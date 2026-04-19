use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as Base64Engine;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use tower_http::cors::CorsLayer;
use uuid::Uuid;

use crate::mcp::handle_mcp_request;
use chartgen::engine::Engine as TradingEngine;

// --- OAuth 2.1 PKCE in-memory store ---

struct OAuthClient {
    _client_secret: Option<String>,
    redirect_uris: Vec<String>,
}

struct AuthCode {
    _code: String,
    client_id: String,
    redirect_uri: String,
    code_challenge: String,
    _code_challenge_method: String,
    created_at: Instant,
}

struct AccessToken {
    _token: String,
    _client_id: String,
    created_at: Instant,
    expires_in: Duration,
}

struct OAuthStore {
    clients: HashMap<String, OAuthClient>,
    codes: HashMap<String, AuthCode>,
    tokens: HashMap<String, AccessToken>,
}

impl OAuthStore {
    fn new() -> Self {
        Self {
            clients: HashMap::new(),
            codes: HashMap::new(),
            tokens: HashMap::new(),
        }
    }

    fn cleanup_expired(&mut self) {
        let now = Instant::now();
        self.codes
            .retain(|_, c| now.duration_since(c.created_at) < Duration::from_secs(300));
        self.tokens
            .retain(|_, t| now.duration_since(t.created_at) < t.expires_in);
    }

    fn validate_token(&mut self, token: &str) -> bool {
        self.cleanup_expired();
        self.tokens.contains_key(token)
    }
}

struct AppState {
    oauth: Mutex<OAuthStore>,
    engine: Option<Arc<RwLock<TradingEngine>>>,
}

type SharedState = Arc<AppState>;

fn base_url() -> String {
    std::env::var("CHARTGEN_BASE_URL").unwrap_or_else(|_| {
        let port = std::env::var("_CHARTGEN_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(9315u16);
        format!("http://localhost:{}", port)
    })
}

// --- OAuth endpoints ---

async fn oauth_metadata(State(state): State<SharedState>) -> Json<Value> {
    let _ = state;
    let base = base_url();
    eprintln!(
        "[OAuth] GET /.well-known/oauth-authorization-server → issuer={}",
        base
    );
    Json(json!({
        "issuer": base,
        "authorization_endpoint": format!("{}/authorize", base),
        "token_endpoint": format!("{}/token", base),
        "registration_endpoint": format!("{}/register", base),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none", "client_secret_post"],
        "logo_uri": format!("{}/favicon.svg", base)
    }))
}

#[derive(Deserialize)]
struct RegisterRequest {
    redirect_uris: Vec<String>,
    #[serde(default)]
    client_name: Option<String>,
}

async fn oauth_register(
    State(state): State<SharedState>,
    Json(body): Json<RegisterRequest>,
) -> (StatusCode, Json<Value>) {
    eprintln!(
        "[OAuth] POST /register client_name={:?} redirect_uris={:?}",
        body.client_name, body.redirect_uris
    );
    let client_id = Uuid::new_v4().to_string();
    let client_secret = Uuid::new_v4().to_string();

    let client = OAuthClient {
        _client_secret: Some(client_secret.clone()),
        redirect_uris: body.redirect_uris.clone(),
    };

    state
        .oauth
        .lock()
        .unwrap()
        .clients
        .insert(client_id.clone(), client);

    (
        StatusCode::CREATED,
        Json(json!({
            "client_id": client_id,
            "client_secret": client_secret,
            "client_name": body.client_name,
            "redirect_uris": body.redirect_uris,
            "grant_types": ["authorization_code"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none",
            "logo_uri": format!("{}/favicon.svg", base_url()),
        })),
    )
}

#[derive(Deserialize)]
struct AuthorizeQuery {
    client_id: String,
    redirect_uri: String,
    #[allow(dead_code)]
    response_type: String,
    code_challenge: String,
    #[serde(default = "default_challenge_method")]
    code_challenge_method: String,
    #[serde(default)]
    state: Option<String>,
}

fn default_challenge_method() -> String {
    "S256".to_string()
}

async fn oauth_authorize(
    State(state): State<SharedState>,
    Query(params): Query<AuthorizeQuery>,
) -> Response {
    eprintln!(
        "[OAuth] GET /authorize client_id={} redirect_uri={}",
        params.client_id, params.redirect_uri
    );
    let mut s = state.oauth.lock().unwrap();
    s.cleanup_expired();

    // Validate client_id
    let client = match s.clients.get(&params.client_id) {
        Some(c) => c,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "invalid_client", "error_description": "Unknown client_id"})),
            )
                .into_response();
        }
    };

    // Validate redirect_uri
    if !client.redirect_uris.contains(&params.redirect_uri) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid_request", "error_description": "redirect_uri mismatch"})),
        )
            .into_response();
    }

    // Auto-approve: generate auth code
    let code = Uuid::new_v4().to_string();
    let auth_code = AuthCode {
        _code: code.clone(),
        client_id: params.client_id.clone(),
        redirect_uri: params.redirect_uri.clone(),
        code_challenge: params.code_challenge.clone(),
        _code_challenge_method: params.code_challenge_method.clone(),
        created_at: Instant::now(),
    };
    s.codes.insert(code.clone(), auth_code);

    // Build redirect URL
    let mut redirect = params.redirect_uri.clone();
    redirect.push_str(if redirect.contains('?') { "&" } else { "?" });
    redirect.push_str(&format!("code={}", code));
    if let Some(state) = &params.state {
        redirect.push_str(&format!("&state={}", state));
    }

    Redirect::temporary(&redirect).into_response()
}

#[derive(Deserialize)]
struct TokenRequest {
    grant_type: String,
    code: String,
    redirect_uri: String,
    client_id: String,
    code_verifier: String,
}

/// Accept token request as either form-encoded or JSON (Claude.ai may send either).
async fn oauth_token(
    State(state): State<SharedState>,
    headers: HeaderMap,
    body_bytes: axum::body::Bytes,
) -> Response {
    let ct = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("none");
    let body_preview = String::from_utf8_lossy(&body_bytes);
    eprintln!(
        "[OAuth] POST /token content-type={} body_len={} body={}",
        ct,
        body_bytes.len(),
        &body_preview[..body_preview.len().min(500)]
    );
    let body: TokenRequest = if headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.contains("application/json"))
    {
        match serde_json::from_slice(&body_bytes) {
            Ok(b) => b,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "invalid_request", "error_description": format!("JSON parse error: {}", e)})),
                )
                    .into_response();
            }
        }
    } else {
        // Parse as application/x-www-form-urlencoded
        let body_str = String::from_utf8_lossy(&body_bytes);
        let parsed_json = form_to_json(&body_str);
        eprintln!("[OAuth] POST /token parsed_form={}", parsed_json);
        match serde_json::from_value(parsed_json) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("[OAuth] POST /token FORM PARSE ERROR: {}", e);
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "invalid_request", "error_description": format!("Form parse error: {}", e)})),
                )
                    .into_response();
            }
        }
    };
    if body.grant_type != "authorization_code" {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "unsupported_grant_type"})),
        )
            .into_response();
    }

    let mut s = state.oauth.lock().unwrap();
    s.cleanup_expired();

    // Extract code data (clone to release borrow before mutating store)
    let code_data = s.codes.get(&body.code).map(|c| {
        (
            c.client_id.clone(),
            c.redirect_uri.clone(),
            c.code_challenge.clone(),
        )
    });

    let (stored_client_id, stored_redirect_uri, stored_challenge) = match code_data {
        Some(d) => d,
        None => {
            eprintln!("[OAuth] POST /token code not found (possibly already exchanged)");
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "invalid_grant", "error_description": "Invalid or expired code"})),
            )
                .into_response();
        }
    };

    // Validate client_id and redirect_uri
    if stored_client_id != body.client_id {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid_grant", "error_description": "client_id mismatch"})),
        )
            .into_response();
    }
    if stored_redirect_uri != body.redirect_uri {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid_grant", "error_description": "redirect_uri mismatch"})),
        )
            .into_response();
    }

    // PKCE S256 verification
    let pkce_ok = verify_pkce(&body.code_verifier, &stored_challenge);
    eprintln!(
        "[OAuth] POST /token PKCE verify: ok={} verifier_len={} challenge={}",
        pkce_ok,
        body.code_verifier.len(),
        &stored_challenge
    );
    if !pkce_ok {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                json!({"error": "invalid_grant", "error_description": "PKCE verification failed"}),
            ),
        )
            .into_response();
    }

    // Issue access token
    let token = Uuid::new_v4().to_string();
    let expires_in = Duration::from_secs(7 * 24 * 3600); // 7 days
    s.tokens.insert(
        token.clone(),
        AccessToken {
            _token: token.clone(),
            _client_id: body.client_id.clone(),
            created_at: Instant::now(),
            expires_in,
        },
    );

    eprintln!(
        "[OAuth] POST /token SUCCESS — token issued (len={})",
        token.len()
    );

    Json(json!({
        "access_token": token,
        "token_type": "Bearer",
        "expires_in": 7 * 24 * 3600
    }))
    .into_response()
}

/// Parse application/x-www-form-urlencoded into a JSON object.
fn form_to_json(body: &str) -> Value {
    let mut map = serde_json::Map::new();
    for pair in body.split('&') {
        if let Some((key, value)) = pair.split_once('=') {
            let key = urlencoding_decode(key);
            let value = urlencoding_decode(value);
            map.insert(key, Value::String(value));
        }
    }
    Value::Object(map)
}

fn urlencoding_decode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.bytes();
    while let Some(b) = chars.next() {
        match b {
            b'+' => result.push(' '),
            b'%' => {
                let hi = chars.next().unwrap_or(b'0');
                let lo = chars.next().unwrap_or(b'0');
                let hex = [hi, lo];
                if let Ok(s) = std::str::from_utf8(&hex) {
                    if let Ok(val) = u8::from_str_radix(s, 16) {
                        result.push(val as char);
                    } else {
                        result.push('%');
                        result.push(hi as char);
                        result.push(lo as char);
                    }
                }
            }
            _ => result.push(b as char),
        }
    }
    result
}

fn verify_pkce(code_verifier: &str, code_challenge: &str) -> bool {
    let mut hasher = Sha256::new();
    hasher.update(code_verifier.as_bytes());
    let hash = hasher.finalize();
    let computed = URL_SAFE_NO_PAD.encode(hash);
    computed == code_challenge
}

// --- Favicon ---

const FAVICON_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64">
  <rect width="64" height="64" rx="8" fill="#1a1a2e"/>
  <line x1="14" y1="12" x2="14" y2="52" stroke="#22c55e" stroke-width="2"/>
  <rect x="10" y="20" width="8" height="18" rx="1" fill="#22c55e"/>
  <line x1="28" y1="8" x2="28" y2="48" stroke="#ef4444" stroke-width="2"/>
  <rect x="24" y="16" width="8" height="22" rx="1" fill="#ef4444"/>
  <line x1="42" y1="18" x2="42" y2="56" stroke="#22c55e" stroke-width="2"/>
  <rect x="38" y="26" width="8" height="16" rx="1" fill="#22c55e"/>
  <line x1="56" y1="10" x2="56" y2="50" stroke="#ef4444" stroke-width="2"/>
  <rect x="52" y="18" width="8" height="20" rx="1" fill="#ef4444"/>
</svg>"##;

async fn favicon_handler() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "image/svg+xml")], FAVICON_SVG)
}

// --- MCP Streamable HTTP transport ---

fn extract_bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
}

/// Check whether the client's Accept header allows an `application/json`
/// response. Missing Accept is tolerated (treated as "accepts anything").
/// Returns `true` if JSON is acceptable; `false` means the server should
/// reply with 406 Not Acceptable.
fn accepts_json(headers: &HeaderMap) -> bool {
    let accept = match headers.get(header::ACCEPT).and_then(|v| v.to_str().ok()) {
        Some(v) => v,
        None => return true, // tolerant default
    };
    // A bare `*/*` or a type/range that matches `application/json` or
    // `application/*` is acceptable. Streamable HTTP clients typically send
    // `application/json, text/event-stream`.
    for raw in accept.split(',') {
        let token = raw.split(';').next().unwrap_or("").trim();
        if token.eq_ignore_ascii_case("*/*")
            || token.eq_ignore_ascii_case("application/*")
            || token.eq_ignore_ascii_case("application/json")
        {
            return true;
        }
    }
    false
}

/// Build a JSON-RPC 2.0 error response with an explicit id and code.
fn jsonrpc_error(id: Option<Value>, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    })
}

/// Build the Streamable-HTTP response: `Content-Type: application/json`,
/// echo any incoming `Mcp-Session-Id` header (spec-compliant correlation),
/// and return the serialized JSON-RPC envelope as the body.
fn streamable_http_json_response(
    status: StatusCode,
    body: Value,
    session_id: Option<&str>,
) -> Response {
    let payload = serde_json::to_vec(&body).unwrap_or_else(|_| b"{}".to_vec());
    let mut builder = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(sid) = session_id {
        // Echo the client-supplied session id unchanged. chartgen is
        // stateless on the sync path, so no server-side session tracking is
        // required — the header is surfaced for clients that correlate on it.
        builder = builder.header("Mcp-Session-Id", sid);
    }
    builder.body(axum::body::Body::from(payload)).unwrap()
}

/// Streamable HTTP (MCP spec 2025-03-26) request handler for `POST /mcp`
/// (and the `POST /` / `POST /message` aliases).
///
/// Behavior:
/// - If the client's `Accept` header does not allow `application/json`,
///   respond with `406 Not Acceptable`.
/// - Parse the body as JSON-RPC 2.0. Malformed bodies return
///   `{"error": {"code": -32700}}` (Parse error).
/// - `initialize` and `tools/list` are accepted without auth (tool
///   discovery); everything else requires a valid bearer token.
/// - The `Mcp-Session-Id` header, if present, is echoed back unchanged.
///   chartgen does not track server-side session state.
/// - Notifications return `204 No Content`.
/// - Successful responses set `Content-Type: application/json`.
async fn mcp_handler(
    State(state): State<SharedState>,
    headers: HeaderMap,
    body_bytes: axum::body::Bytes,
) -> Response {
    // Echo any incoming session id (see spec §Session management).
    let session_id = headers
        .get("Mcp-Session-Id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    // Accept negotiation: reject when the client explicitly excludes JSON.
    if !accepts_json(&headers) {
        eprintln!("[MCP] POST /mcp → 406 Not Acceptable (Accept excludes application/json)");
        let err = jsonrpc_error(
            None,
            -32600,
            "Not Acceptable: server returns application/json",
        );
        return streamable_http_json_response(
            StatusCode::NOT_ACCEPTABLE,
            err,
            session_id.as_deref(),
        );
    }

    // Parse body as JSON-RPC 2.0. An empty body is treated as a parse error.
    let body: Value = match serde_json::from_slice(&body_bytes) {
        Ok(v) => v,
        Err(e) => {
            let err = jsonrpc_error(None, -32700, &format!("Parse error: {}", e));
            return streamable_http_json_response(
                StatusCode::BAD_REQUEST,
                err,
                session_id.as_deref(),
            );
        }
    };

    let method = body.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let has_auth = extract_bearer_token(&headers).is_some();
    eprintln!("[MCP] POST /mcp method={} has_auth={}", method, has_auth);

    // Allow initialize and tools/list without auth for tool discovery
    if method != "initialize" && method != "tools/list" {
        let token = match extract_bearer_token(&headers) {
            Some(t) => t.to_string(),
            None => {
                let err = jsonrpc_error(
                    body.get("id").cloned(),
                    -32000,
                    "Missing Authorization header",
                );
                return streamable_http_json_response(
                    StatusCode::UNAUTHORIZED,
                    err,
                    session_id.as_deref(),
                );
            }
        };
        if !state.oauth.lock().unwrap().validate_token(&token) {
            let err = jsonrpc_error(body.get("id").cloned(), -32000, "Invalid or expired token");
            return streamable_http_json_response(
                StatusCode::UNAUTHORIZED,
                err,
                session_id.as_deref(),
            );
        }
    }

    let id = body.get("id").cloned();
    let params = body.get("params");

    // Notifications: no response body. Still echo the session id.
    if method == "initialized" || method == "notifications/initialized" {
        let mut resp = StatusCode::NO_CONTENT.into_response();
        if let Some(sid) = session_id.as_deref() {
            if let Ok(val) = sid.parse() {
                resp.headers_mut().insert("Mcp-Session-Id", val);
            }
        }
        return resp;
    }

    // Extract token for subscription tools
    let token_owned = extract_bearer_token(&headers).map(|t| t.to_string());

    // Run MCP handler in a blocking task (chart rendering + data fetching use blocking I/O)
    let method_owned = method.to_string();
    let params_owned = params.cloned();
    let engine = state.engine.clone();
    let result = tokio::task::spawn_blocking(move || {
        handle_mcp_request(
            &method_owned,
            id,
            params_owned.as_ref(),
            engine.as_ref(),
            token_owned.as_deref(),
        )
    })
    .await
    .unwrap();

    streamable_http_json_response(StatusCode::OK, result, session_id.as_deref())
}

// --- Health / discovery handler (no auth) ---

async fn health_handler() -> Json<Value> {
    eprintln!("[MCP] GET / (health/discovery check)");
    Json(json!({
        "name": "chartgen",
        "version": "0.1.0",
        "protocol": "MCP",
        "protocolVersion": "2025-03-26",
        "status": "ok"
    }))
}

// --- SSE Transport handler ---

/// Streamable HTTP (MCP 2025-03-26) server-initiated SSE stream.
///
/// Mounted at `GET /mcp` and aliased at `GET /sse` for backwards compatibility
/// with existing Claude.ai connector deployments.
///
/// Unlike the legacy 2024-11-05 HTTP+SSE transport, this endpoint does **not**
/// emit an `event: endpoint` frame or rotate a `session_id` — the client sends
/// every JSON-RPC request to `POST /mcp` and reads server-initiated
/// notifications (e.g. `notifications/alert_triggered`) from this stream
/// directly as JSON-RPC notification frames.
async fn sse_handler(State(state): State<SharedState>, headers: HeaderMap) -> Response {
    eprintln!("[MCP] GET /mcp (Streamable HTTP SSE stream requested)");

    // Validate bearer token
    let token = match extract_bearer_token(&headers) {
        Some(t) => t.to_string(),
        None => {
            return (StatusCode::UNAUTHORIZED, "Missing Authorization header").into_response();
        }
    };
    if !state.oauth.lock().unwrap().validate_token(&token) {
        return (StatusCode::UNAUTHORIZED, "Invalid or expired token").into_response();
    }

    // Echo the client-supplied Mcp-Session-Id if present, else mint one so
    // clients that expect the header on the SSE response still get a value.
    let session_id = headers
        .get("Mcp-Session-Id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    // Create notification channel
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();

    // Link sender to subscription registry if engine is available
    if let Some(ref engine) = state.engine {
        let mut e = engine.write().unwrap();
        e.subscription_registry.link_sender(&token, tx);
    }

    let token_cleanup = token.clone();
    let engine_cleanup = state.engine.clone();

    // Build SSE stream: notifications + keepalives. No legacy endpoint event
    // — Streamable HTTP clients POST to /mcp directly and read server
    // notifications as raw JSON-RPC frames.
    let stream = async_stream::stream! {
        let mut rx = rx;
        let mut keepalive = tokio::time::interval(std::time::Duration::from_secs(30));
        keepalive.tick().await; // consume immediate first tick

        loop {
            tokio::select! {
                msg = rx.recv() => {
                    match msg {
                        Some(payload) => {
                            yield Ok::<_, std::convert::Infallible>(format!("event: message\ndata: {}\n\n", payload));
                        }
                        None => break,
                    }
                }
                _ = keepalive.tick() => {
                    yield Ok(": keepalive\n\n".to_string());
                }
            }
        }
    };

    // Wrap in a finalizer that cleans up on disconnect
    let body_stream = CleanupStream {
        inner: Box::pin(stream),
        token: token_cleanup,
        engine: engine_cleanup,
        cleaned_up: false,
    };

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        .header("Mcp-Session-Id", &session_id)
        .body(axum::body::Body::from_stream(body_stream))
        .unwrap()
}

/// Stream wrapper that cleans up the subscription sender on drop.
struct CleanupStream {
    inner: std::pin::Pin<
        Box<dyn futures_util::Stream<Item = Result<String, std::convert::Infallible>> + Send>,
    >,
    token: String,
    engine: Option<Arc<RwLock<TradingEngine>>>,
    cleaned_up: bool,
}

impl futures_util::Stream for CleanupStream {
    type Item = Result<String, std::convert::Infallible>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}

impl Drop for CleanupStream {
    fn drop(&mut self) {
        if !self.cleaned_up {
            self.cleaned_up = true;
            if let Some(ref engine) = self.engine {
                match engine.write() {
                    Ok(mut e) => {
                        e.subscription_registry.unlink_sender(&self.token);
                        eprintln!("[MCP] SSE disconnected, unlinked sender for token");
                    }
                    Err(poisoned) => {
                        let mut e = poisoned.into_inner();
                        e.subscription_registry.unlink_sender(&self.token);
                        eprintln!("[MCP] SSE disconnected, unlinked sender (lock was poisoned)");
                    }
                }
            }
        }
    }
}

// --- Server startup ---

pub async fn run_server(port: u16, engine: Option<Arc<RwLock<TradingEngine>>>) {
    let state = Arc::new(AppState {
        oauth: Mutex::new(OAuthStore::new()),
        engine,
    });

    // Store port for metadata endpoint
    std::env::set_var("_CHARTGEN_PORT", port.to_string());

    let cors = CorsLayer::permissive();

    let app = Router::new()
        .route(
            "/.well-known/oauth-authorization-server",
            get(oauth_metadata),
        )
        .route("/register", post(oauth_register))
        .route("/authorize", get(oauth_authorize))
        .route("/token", post(oauth_token))
        .route("/favicon.ico", get(favicon_handler))
        .route("/favicon.svg", get(favicon_handler))
        // MCP endpoints — Claude.ai may try root, /mcp, /message, or /sse
        .route("/", get(health_handler).post(mcp_handler))
        .route("/mcp", get(sse_handler).post(mcp_handler))
        .route("/message", post(mcp_handler))
        .route("/sse", get(sse_handler))
        .layer(cors)
        .with_state(state);

    let addr = format!("0.0.0.0:{}", port);
    println!("MCP server listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn test_app() -> Router {
        let state = Arc::new(AppState {
            oauth: Mutex::new(OAuthStore::new()),
            engine: None,
        });
        Router::new()
            .route("/", get(health_handler).post(mcp_handler))
            .route("/mcp", get(sse_handler).post(mcp_handler))
            .route("/message", post(mcp_handler))
            .route("/sse", get(sse_handler))
            .with_state(state)
    }

    fn headers_with(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                header::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                v.parse().unwrap(),
            );
        }
        h
    }

    // --- accepts_json unit tests ---

    #[test]
    fn accepts_json_missing_accept_is_tolerant() {
        let h = HeaderMap::new();
        assert!(accepts_json(&h));
    }

    #[test]
    fn accepts_json_streamable_http_default() {
        let h = headers_with(&[("accept", "application/json, text/event-stream")]);
        assert!(accepts_json(&h));
    }

    #[test]
    fn accepts_json_wildcard() {
        let h = headers_with(&[("accept", "*/*")]);
        assert!(accepts_json(&h));
    }

    #[test]
    fn accepts_json_app_wildcard() {
        let h = headers_with(&[("accept", "application/*")]);
        assert!(accepts_json(&h));
    }

    #[test]
    fn accepts_json_text_plain_only_rejected() {
        let h = headers_with(&[("accept", "text/plain")]);
        assert!(!accepts_json(&h));
    }

    #[test]
    fn accepts_json_sse_only_rejected() {
        let h = headers_with(&[("accept", "text/event-stream")]);
        assert!(!accepts_json(&h));
    }

    #[test]
    fn accepts_json_ignores_quality_parameters() {
        let h = headers_with(&[("accept", "application/json;q=0.9, text/event-stream")]);
        assert!(accepts_json(&h));
    }

    // --- Streamable HTTP POST /mcp tests ---

    #[tokio::test]
    async fn post_mcp_tools_list_returns_json() {
        let app = test_app();
        let body = br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
        let req = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("accept", "application/json, text/event-stream")
            .header("content-type", "application/json")
            .body(Body::from(&body[..]))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/json"
        );

        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let parsed: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed["jsonrpc"], "2.0");
        assert_eq!(parsed["id"], 1);
        assert!(parsed["result"]["tools"].is_array());
    }

    #[tokio::test]
    async fn post_mcp_initialize_advertises_new_protocol_version() {
        let app = test_app();
        let body = br#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#;
        let req = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("accept", "application/json, text/event-stream")
            .header("content-type", "application/json")
            .body(Body::from(&body[..]))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let parsed: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed["result"]["protocolVersion"], "2025-03-26");
    }

    #[tokio::test]
    async fn post_mcp_accept_excludes_json_returns_406() {
        let app = test_app();
        let body = br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
        let req = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("accept", "text/plain")
            .header("content-type", "application/json")
            .body(Body::from(&body[..]))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_ACCEPTABLE);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/json"
        );

        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let parsed: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed["jsonrpc"], "2.0");
        assert!(parsed["error"].is_object());
    }

    #[tokio::test]
    async fn post_mcp_echoes_session_id() {
        let app = test_app();
        let body = br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
        let req = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("accept", "application/json, text/event-stream")
            .header("content-type", "application/json")
            .header("Mcp-Session-Id", "client-session-abc")
            .body(Body::from(&body[..]))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let sid = resp.headers().get("Mcp-Session-Id").unwrap();
        assert_eq!(sid, "client-session-abc");
    }

    #[tokio::test]
    async fn post_mcp_no_accept_header_defaults_to_json() {
        // Tolerant default: no Accept header → treated as application/json.
        let app = test_app();
        let body = br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
        let req = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("content-type", "application/json")
            .body(Body::from(&body[..]))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/json"
        );
    }

    #[tokio::test]
    async fn post_mcp_malformed_body_is_parse_error() {
        let app = test_app();
        let req = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("accept", "application/json, text/event-stream")
            .header("content-type", "application/json")
            .body(Body::from("not-json"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let parsed: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed["error"]["code"], -32700);
    }

    #[tokio::test]
    async fn post_mcp_tools_call_without_auth_returns_401() {
        let app = test_app();
        let body =
            br#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_indicators"}}"#;
        let req = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("accept", "application/json, text/event-stream")
            .header("content-type", "application/json")
            .body(Body::from(&body[..]))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/json"
        );
    }

    #[tokio::test]
    async fn post_message_alias_also_works() {
        // /message is retained as an alias of /mcp for backward compat.
        let app = test_app();
        let body = br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
        let req = Request::builder()
            .method("POST")
            .uri("/message")
            .header("accept", "application/json, text/event-stream")
            .header("content-type", "application/json")
            .body(Body::from(&body[..]))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // --- GET /mcp SSE tests ---

    #[tokio::test]
    async fn get_mcp_without_auth_is_unauthorized() {
        let app = test_app();
        let req = Request::builder()
            .method("GET")
            .uri("/mcp")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn get_mcp_with_valid_token_opens_sse_without_endpoint_event() {
        // Build an app that shares state with us so we can inject a token.
        let state = Arc::new(AppState {
            oauth: Mutex::new(OAuthStore::new()),
            engine: None,
        });
        // Seed a valid token.
        {
            let mut s = state.oauth.lock().unwrap();
            s.tokens.insert(
                "test-token".to_string(),
                AccessToken {
                    _token: "test-token".to_string(),
                    _client_id: "test".to_string(),
                    created_at: Instant::now(),
                    expires_in: Duration::from_secs(3600),
                },
            );
        }
        let app: Router = Router::new()
            .route("/mcp", get(sse_handler).post(mcp_handler))
            .with_state(state);

        let req = Request::builder()
            .method("GET")
            .uri("/mcp")
            .header("authorization", "Bearer test-token")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/event-stream"
        );

        // Pull a small chunk of the body — with a 30s keepalive and no
        // subscription, no frame should arrive promptly. Race an immediate
        // frame read against a short timeout: anything that does land must
        // NOT contain a legacy `event: endpoint` frame.
        let mut body = resp.into_body();
        let first = tokio::time::timeout(
            std::time::Duration::from_millis(200),
            http_body_util::BodyExt::frame(&mut body),
        )
        .await;
        if let Ok(Some(Ok(frame))) = first {
            if let Ok(data) = frame.into_data() {
                let s = String::from_utf8_lossy(&data);
                assert!(
                    !s.contains("event: endpoint"),
                    "Streamable HTTP must not emit the legacy endpoint event, got: {}",
                    s
                );
            }
        }
    }

    #[tokio::test]
    async fn get_sse_alias_behaves_like_get_mcp() {
        // /sse is retained as an alias for legacy Claude.ai deployments.
        let app = test_app();
        let req = Request::builder()
            .method("GET")
            .uri("/sse")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // --- Health handler ---

    #[tokio::test]
    async fn get_root_advertises_new_protocol_version() {
        let app = test_app();
        let req = Request::builder()
            .method("GET")
            .uri("/")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let parsed: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed["protocolVersion"], "2025-03-26");
    }
}
