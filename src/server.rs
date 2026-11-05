use axum::extract::{Query, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as Base64Engine;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};
use uuid::Uuid;

use crate::chart_push::{ChartHub, PushCtx, STREAM_BUFFER};
use crate::mcp::handle_mcp_request;
use chartgen::engine::Engine as TradingEngine;
#[cfg(feature = "scripting")]
use chartgen::scripting;

// --- OAuth 2.1 PKCE in-memory store ---

struct OAuthClient {
    _client_secret: Option<String>,
    redirect_uris: Vec<String>,
    created_at: Instant,
    /// Received a token at least once (unused clients are collected).
    tokened: bool,
}

/// Dynamic client registration limits: `/authorize` auto-approves, so
/// anyone can register; these keep the client table bounded.
const MAX_REDIRECT_URIS: usize = 5;
const MAX_REDIRECT_URI_LEN: usize = 512;
const MAX_CLIENT_NAME_LEN: usize = 256;
const MAX_CLIENTS: usize = 1000;
/// Registrations per client address (the TCP peer) per `REGISTER_WINDOW`.
/// Loopback peers are exempt: local tests and dev clients register freely,
/// and behind a reverse proxy on the same host the peer is always loopback,
/// so there only `MAX_CLIENTS` applies.
const MAX_REGISTRATIONS_PER_IP: usize = 20;
const REGISTER_WINDOW: Duration = Duration::from_secs(600);
/// A client that never received a token is forgotten after this long.
const UNUSED_CLIENT_TTL: Duration = Duration::from_secs(24 * 3600);

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
    #[cfg_attr(not(feature = "scripting"), allow(dead_code))]
    client_id: String,
    created_at: Instant,
    expires_in: Duration,
}

struct OAuthStore {
    clients: HashMap<String, OAuthClient>,
    codes: HashMap<String, AuthCode>,
    tokens: HashMap<String, AccessToken>,
    /// Recent `/register` calls per client address.
    registrations: HashMap<String, Vec<Instant>>,
}

impl OAuthStore {
    fn new() -> Self {
        Self {
            clients: HashMap::new(),
            codes: HashMap::new(),
            tokens: HashMap::new(),
            registrations: HashMap::new(),
        }
    }

    /// Check a registration from `ip` against the limits, and count it.
    /// Forgets clients that never got a token within `UNUSED_CLIENT_TTL`.
    fn admit_registration(
        &mut self,
        ip: &str,
        body: &RegisterRequest,
    ) -> Result<(), (StatusCode, Value)> {
        let bad = |what: String| {
            Err((
                StatusCode::BAD_REQUEST,
                json!({"error": "invalid_client_metadata", "error_description": what}),
            ))
        };
        if body.redirect_uris.is_empty() || body.redirect_uris.len() > MAX_REDIRECT_URIS {
            return bad(format!("1 to {MAX_REDIRECT_URIS} redirect_uris required"));
        }
        if body
            .redirect_uris
            .iter()
            .any(|u| u.len() > MAX_REDIRECT_URI_LEN)
        {
            return bad(format!(
                "redirect_uris are limited to {MAX_REDIRECT_URI_LEN} characters"
            ));
        }
        if body
            .client_name
            .as_ref()
            .is_some_and(|n| n.len() > MAX_CLIENT_NAME_LEN)
        {
            return bad(format!(
                "client_name is limited to {MAX_CLIENT_NAME_LEN} characters"
            ));
        }
        let now = Instant::now();
        self.registrations.retain(|_, hits| {
            hits.retain(|t| now.duration_since(*t) < REGISTER_WINDOW);
            !hits.is_empty()
        });
        let loopback = ip
            .parse::<std::net::IpAddr>()
            .is_ok_and(|a| a.is_loopback());
        let hits = self.registrations.entry(ip.to_owned()).or_default();
        if !loopback && hits.len() >= MAX_REGISTRATIONS_PER_IP {
            return Err((
                StatusCode::TOO_MANY_REQUESTS,
                json!({"error": "slow_down", "error_description": "too many client registrations; try again later"}),
            ));
        }
        hits.push(now);
        self.clients
            .retain(|_, c| c.tokened || now.duration_since(c.created_at) < UNUSED_CLIENT_TTL);
        if self.clients.len() >= MAX_CLIENTS {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                json!({"error": "temporarily_unavailable", "error_description": "client registry is full"}),
            ));
        }
        Ok(())
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

    /// The OAuth client a valid token was issued to: the owner of that
    /// caller's scripted indicators (ADR-0025).
    #[cfg(feature = "scripting")]
    fn client_for_token(&mut self, token: &str) -> Option<String> {
        self.cleanup_expired();
        self.tokens.get(token).map(|t| t.client_id.clone())
    }
}

struct AppState {
    oauth: Mutex<OAuthStore>,
    engine: Option<Arc<RwLock<TradingEngine>>>,
    /// Chart push subscriptions (ADR-0021); runs with or without the engine.
    charts: Arc<ChartHub>,
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

/// RFC 9728 protected-resource metadata. `/mcp` is the canonical MCP
/// endpoint (path-suffixed document); the root document covers the
/// deprecated `POST /` alias (ADR-0027).
async fn oauth_protected_resource(uri: Uri) -> Json<Value> {
    let base = base_url();
    let suffix = uri
        .path()
        .strip_prefix("/.well-known/oauth-protected-resource")
        .unwrap_or("");
    Json(json!({
        "resource": format!("{base}{suffix}"),
        "authorization_servers": [base],
        "bearer_methods_supported": ["header"]
    }))
}

/// Adds the RFC 9728 `resource_metadata` hint to a 401 from an MCP endpoint.
fn with_resource_metadata(mut resp: Response, path: &str) -> Response {
    let suffix = if path == "/mcp" { "/mcp" } else { "" };
    let value = format!(
        "Bearer resource_metadata=\"{}/.well-known/oauth-protected-resource{suffix}\"",
        base_url()
    );
    if let Ok(v) = HeaderValue::from_str(&value) {
        resp.headers_mut().insert(header::WWW_AUTHENTICATE, v);
    }
    resp
}

#[derive(Deserialize)]
struct RegisterRequest {
    redirect_uris: Vec<String>,
    #[serde(default)]
    client_name: Option<String>,
}

async fn oauth_register(
    State(state): State<SharedState>,
    ext: axum::http::Extensions,
    Json(body): Json<RegisterRequest>,
) -> (StatusCode, Json<Value>) {
    eprintln!(
        "[OAuth] POST /register redirect_uris={}",
        body.redirect_uris.len()
    );
    if let Err((status, err)) = state
        .oauth
        .lock()
        .unwrap()
        .admit_registration(&client_addr(&ext), &body)
    {
        return (status, Json(err));
    }
    let client_id = Uuid::new_v4().to_string();
    let client_secret = Uuid::new_v4().to_string();

    let client = OAuthClient {
        _client_secret: Some(client_secret.clone()),
        redirect_uris: body.redirect_uris.clone(),
        created_at: Instant::now(),
        tokened: false,
    };

    state
        .oauth
        .lock()
        .unwrap()
        .clients
        .insert(client_id.clone(), client);
    // With scripting on, the client id owns stored indicators, so it must
    // survive restarts: persist the registration (ADR-0025).
    #[cfg(feature = "scripting")]
    if scripting::enabled() {
        if let Some(store) = scripting::persistence::global() {
            let saved = store
                .gc_oauth_clients(UNUSED_CLIENT_TTL.as_secs() as i64)
                .and_then(|_| store.save_oauth_client(&client_id, &body.redirect_uris));
            if let Err(e) = saved {
                eprintln!("[OAuth] could not persist a client registration: {e}");
            }
        }
    }

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
    eprintln!("[OAuth] GET /authorize");
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
    // Never log the body: it carries the code, verifier and client_id.
    eprintln!(
        "[OAuth] POST /token content-type={} body_len={}",
        ct,
        body_bytes.len()
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
    eprintln!("[OAuth] POST /token PKCE verify: ok={}", pkce_ok);
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
            client_id: body.client_id.clone(),
            created_at: Instant::now(),
            expires_in,
        },
    );
    if let Some(c) = s.clients.get_mut(&body.client_id) {
        c.tokened = true;
    }
    #[cfg(feature = "scripting")]
    if scripting::enabled() {
        if let Some(store) = scripting::persistence::global() {
            if let Err(e) = store.mark_oauth_client_used(&body.client_id) {
                eprintln!("[OAuth] could not record client use: {e}");
            }
        }
    }

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

/// Identifies one client's SSE stream for chart push (ADR-0021): the bearer
/// token plus the `Mcp-Session-Id` (minted on `initialize`, then sent by the
/// client on every request). The token part keeps one user from addressing
/// another user's stream.
fn stream_key(token: &str, session_id: Option<&str>) -> String {
    format!("{token}\n{}", session_id.unwrap_or(""))
}

/// The peer address of the connection, for the per-client chart push cap.
/// chartgen trusts no forwarding header (`X-Forwarded-For` and the like),
/// so behind a reverse proxy this is the proxy's address (ADR-0021).
fn client_addr(ext: &axum::http::Extensions) -> String {
    ext.get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|c| c.0.ip().to_string())
        .unwrap_or_else(|| "unknown".into())
}

/// True for a JSON-RPC message that gets no response: a notification (no
/// `id`) or a response to a server request (`result`/`error`, no `method`).
fn is_notification_or_response(m: &Value) -> bool {
    m.is_object()
        && (m.get("id").is_none()
            || (m.get("method").is_none()
                && (m.get("result").is_some() || m.get("error").is_some())))
}

/// A POST body made only of notifications and responses (one, or a batch).
/// Streamable HTTP answers it with `202 Accepted` and no body.
fn needs_no_response(body: &Value) -> bool {
    match body.as_array() {
        Some(batch) => !batch.is_empty() && batch.iter().all(is_notification_or_response),
        None => is_notification_or_response(body),
    }
}

/// Check whether the client's Accept header allows an `application/json`
/// response. Missing Accept is tolerated (treated as "accepts anything").
/// Returns `true` if JSON is acceptable; `false` means the server should
/// reply with 406 Not Acceptable.
///
/// Per RFC 7231 §5.3.1, a quality value of `q=0` means "not acceptable", so
/// a media range with `q=0` is skipped even when the type matches.
fn accepts_json(headers: &HeaderMap) -> bool {
    let accept = match headers.get(header::ACCEPT).and_then(|v| v.to_str().ok()) {
        Some(v) => v,
        None => return true, // tolerant default
    };
    // A bare `*/*` or a type/range that matches `application/json` or
    // `application/*` is acceptable. Streamable HTTP clients typically send
    // `application/json, text/event-stream`.
    for raw in accept.split(',') {
        let mut parts = raw.split(';');
        let token = parts.next().unwrap_or("").trim();
        let matches_json = token.eq_ignore_ascii_case("*/*")
            || token.eq_ignore_ascii_case("application/*")
            || token.eq_ignore_ascii_case("application/json");
        if !matches_json {
            continue;
        }
        // Honor `q=0` ("not acceptable"). Other q values still mean
        // acceptable; we don't perform full quality-weighted negotiation.
        let rejected = parts.any(|p| is_q_zero(p.trim()));
        if !rejected {
            return true;
        }
    }
    false
}

/// Returns true when an Accept parameter is `q=0` (or `q=0.`, `q=0.0`, …) —
/// RFC 7231 §5.3.1 quality value of zero, meaning "not acceptable".
fn is_q_zero(param: &str) -> bool {
    let rest = match param
        .strip_prefix("q=")
        .or_else(|| param.strip_prefix("Q="))
    {
        Some(r) => r.trim(),
        None => return false,
    };
    // Accept "0", "0.", "0.0", "0.00", "0.000".
    matches!(rest, "0" | "0." | "0.0" | "0.00" | "0.000")
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
///   `initialize` without one gets a fresh id. chartgen keeps no session
///   state beyond scoping chart push subscriptions to the client's SSE
///   stream with it.
/// - Notifications and responses (alone or as a batch) return `202
///   Accepted` with no body (the SDK opens its GET SSE stream only after a
///   202 for `notifications/initialized`).
/// - Successful responses set `Content-Type: application/json`.
async fn mcp_handler(
    State(state): State<SharedState>,
    uri: Uri,
    headers: HeaderMap,
    ext: axum::http::Extensions,
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
    let session_id = match session_id {
        None if method == "initialize" => Some(Uuid::new_v4().to_string()),
        sid => sid,
    };
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
                return with_resource_metadata(
                    streamable_http_json_response(
                        StatusCode::UNAUTHORIZED,
                        err,
                        session_id.as_deref(),
                    ),
                    uri.path(),
                );
            }
        };
        if !state.oauth.lock().unwrap().validate_token(&token) {
            let err = jsonrpc_error(body.get("id").cloned(), -32000, "Invalid or expired token");
            return with_resource_metadata(
                streamable_http_json_response(StatusCode::UNAUTHORIZED, err, session_id.as_deref()),
                uri.path(),
            );
        }
    }

    let id = body.get("id").cloned();
    let params = body.get("params");

    // Notifications: no response body. Still echo the session id.
    // Per JSON-RPC 2.0 §4.1, a frame without an `id` is a notification; a
    // frame with an `id` is a request and MUST produce a matching response
    // envelope — even when the method name matches a notification shape.
    if needs_no_response(&body) {
        let mut resp = StatusCode::ACCEPTED.into_response();
        if let Some(sid) = session_id.as_deref() {
            if let Ok(val) = sid.parse() {
                resp.headers_mut().insert("Mcp-Session-Id", val);
            }
        }
        return resp;
    }

    // Extract token for subscription tools
    let token_owned = extract_bearer_token(&headers).map(|t| t.to_string());
    // Scripted indicators belong to the token's OAuth client (ADR-0025).
    #[cfg(feature = "scripting")]
    let user = match &token_owned {
        Some(t) if scripting::enabled() => state.oauth.lock().unwrap().client_for_token(t),
        _ => None,
    };
    #[cfg(not(feature = "scripting"))]
    let user: Option<String> = None;

    // Run MCP handler in a blocking task (chart rendering + data fetching use blocking I/O)
    let method_owned = method.to_string();
    let params_owned = params.cloned();
    let engine = state.engine.clone();
    let push = token_owned.as_deref().map(|t| PushCtx {
        hub: state.charts.clone(),
        stream: stream_key(t, session_id.as_deref()),
        client: client_addr(&ext),
    });
    let result = tokio::task::spawn_blocking(move || {
        handle_mcp_request(
            &method_owned,
            id,
            params_owned.as_ref(),
            engine.as_ref(),
            token_owned.as_deref(),
            push.as_ref(),
            user.as_deref(),
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

// --- Web app (ADR-0027) ---

fn accepts_html(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|a| a.contains("text/html"))
}

/// `GET /`: the web app for browsers, the health JSON for everyone else.
async fn root_handler(dir: PathBuf, req: Request) -> Response {
    if accepts_html(req.headers()) {
        serve_web(dir, req).await
    } else {
        health_handler().await.into_response()
    }
}

/// Serves the built web app in `dir` (router fallback). A missing path is
/// answered with `index.html` when the client accepts HTML (SPA routing);
/// otherwise it is a 404, as are non-GET/HEAD requests.
async fn serve_web(dir: PathBuf, req: Request) -> Response {
    if !matches!(*req.method(), Method::GET | Method::HEAD) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let assets = req.uri().path().starts_with("/assets/");
    let mut resp = if accepts_html(req.headers()) && !assets {
        let index = ServeFile::new(dir.join("index.html"));
        ServeDir::new(&dir).fallback(index).try_call(req).await
    } else {
        ServeDir::new(&dir).try_call(req).await
    }
    .map_or_else(
        |_| StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        IntoResponse::into_response,
    );
    let cache = if assets && resp.status().is_success() {
        // Vite content-hashes everything under /assets/.
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    let h = resp.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    resp
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
/// notifications (e.g. `notifications/alert_triggered`,
/// `notifications/chart_tick`) from this stream directly as JSON-RPC
/// notification frames.
async fn sse_handler(State(state): State<SharedState>, uri: Uri, headers: HeaderMap) -> Response {
    eprintln!("[MCP] GET /mcp (Streamable HTTP SSE stream requested)");

    // Validate bearer token
    let token = match extract_bearer_token(&headers) {
        Some(t) => t.to_string(),
        None => {
            let resp = (StatusCode::UNAUTHORIZED, "Missing Authorization header").into_response();
            return with_resource_metadata(resp, uri.path());
        }
    };
    if !state.oauth.lock().unwrap().validate_token(&token) {
        let resp = (StatusCode::UNAUTHORIZED, "Invalid or expired token").into_response();
        return with_resource_metadata(resp, uri.path());
    }

    // Echo the client-supplied Mcp-Session-Id if present, else mint one so
    // clients that expect the header on the SSE response still get a value.
    let client_session = headers
        .get("Mcp-Session-Id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let chart_stream = stream_key(&token, client_session.as_deref());
    let session_id = client_session.unwrap_or_else(|| Uuid::new_v4().to_string());

    // Create notification channel. Bounded: a peer that stops reading is
    // unlinked once it is full (dead peers are found by the 30 s keepalive
    // write failing).
    let (tx, rx) = tokio::sync::mpsc::channel::<String>(STREAM_BUFFER);

    // Chart ticks for this stream's subscriptions (ADR-0021).
    let chart_link = state.charts.link_stream(&chart_stream, tx.clone());

    // Link sender to subscription registry if engine is available
    if let Some(ref engine) = state.engine {
        let mut e = engine.write().unwrap();
        e.subscription_registry
            .link_sender(&token, &chart_stream, tx);
    }

    let token_cleanup = token.clone();
    let engine_cleanup = state.engine.clone();

    // Build SSE stream: notifications + keepalives. No legacy endpoint event
    // — Streamable HTTP clients POST to /mcp directly and read server
    // notifications as raw JSON-RPC frames.
    let stream = async_stream::stream! {
        // Emit an immediate SSE comment so the response head is deterministic
        // (useful for proxies flushing the response, and for tests that assert
        // on the first frame). Comment lines beginning with ':' are ignored
        // by spec-compliant SSE clients.
        yield Ok::<_, std::convert::Infallible>(": connected\n\n".to_string());

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
        charts: state.charts.clone(),
        stream_key: chart_stream,
        chart_link,
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
    charts: Arc<ChartHub>,
    /// Token + session id: keys this stream in the chart hub and the alert
    /// registry alike.
    stream_key: String,
    chart_link: u64,
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
            // Ends this stream's chart subscriptions, and their feeds when
            // nobody else watches them.
            self.charts.unlink_stream(&self.stream_key, self.chart_link);
            if let Some(ref engine) = self.engine {
                match engine.write() {
                    Ok(mut e) => {
                        e.subscription_registry
                            .unlink_sender(&self.token, &self.stream_key);
                        eprintln!("[MCP] SSE disconnected, unlinked sender for token");
                    }
                    Err(poisoned) => {
                        let mut e = poisoned.into_inner();
                        e.subscription_registry
                            .unlink_sender(&self.token, &self.stream_key);
                        eprintln!("[MCP] SSE disconnected, unlinked sender (lock was poisoned)");
                    }
                }
            }
        }
    }
}

// --- Server startup ---

pub async fn run_server(port: u16, engine: Option<Arc<RwLock<TradingEngine>>>) {
    #[cfg_attr(not(feature = "scripting"), allow(unused_mut))]
    let mut oauth = OAuthStore::new();
    // Scripting: open (and migrate) the store at startup and restore the
    // registered OAuth clients, so their scripts stay theirs (ADR-0025).
    #[cfg(feature = "scripting")]
    if scripting::enabled() {
        let path = scripting::persistence::db_path();
        let ttl = UNUSED_CLIENT_TTL.as_secs() as i64;
        let restored = scripting::persistence::global()
            .map(|s| s.gc_oauth_clients(ttl).and_then(|_| s.oauth_clients()));
        match restored {
            Some(Ok(clients)) => {
                println!(
                    "Scripting enabled: {} ({} OAuth clients restored)",
                    path.display(),
                    clients.len()
                );
                let now_unix = chrono::Utc::now().timestamp();
                for c in clients {
                    let age = Duration::from_secs((now_unix - c.created_at).max(0) as u64);
                    oauth.clients.insert(
                        c.client_id,
                        OAuthClient {
                            _client_secret: None,
                            redirect_uris: c.redirect_uris,
                            created_at: Instant::now()
                                .checked_sub(age)
                                .unwrap_or_else(Instant::now),
                            tokened: c.last_token_at.is_some(),
                        },
                    );
                }
            }
            Some(Err(e)) => eprintln!("[scripting] cannot read OAuth clients: {e}"),
            None => eprintln!("[scripting] store unavailable at {}", path.display()),
        }
    }
    let state = Arc::new(AppState {
        oauth: Mutex::new(oauth),
        engine,
        charts: ChartHub::from_env(),
    });

    // Store port for metadata endpoint
    std::env::set_var("_CHARTGEN_PORT", port.to_string());

    let cors = CorsLayer::permissive();

    // Built web app (ADR-0027); unset keeps the server API-only.
    let web_dir = std::env::var_os("CHARTGEN_WEB_DIR").map(PathBuf::from);
    let root_get = match web_dir.clone() {
        Some(dir) => get(move |req: Request| root_handler(dir.clone(), req)),
        None => get(health_handler),
    };

    let mut app = Router::new()
        .route(
            "/.well-known/oauth-authorization-server",
            get(oauth_metadata),
        )
        .route(
            "/.well-known/oauth-protected-resource",
            get(oauth_protected_resource),
        )
        .route(
            "/.well-known/oauth-protected-resource/mcp",
            get(oauth_protected_resource),
        )
        .route("/register", post(oauth_register))
        .route("/authorize", get(oauth_authorize))
        .route("/token", post(oauth_token))
        .route("/favicon.ico", get(favicon_handler))
        .route("/favicon.svg", get(favicon_handler))
        // MCP endpoints — Claude.ai may try root, /mcp, /message, or /sse
        // /mcp is canonical; POST /, /message and /sse are deprecated
        // aliases for older connectors (ADR-0027).
        .route("/", root_get.post(mcp_handler))
        .route("/mcp", get(sse_handler).post(mcp_handler))
        .route("/message", post(mcp_handler))
        .route("/sse", get(sse_handler));
    if let Some(dir) = web_dir {
        println!("Serving web app from {}", dir.display());
        app = app.fallback(move |req: Request| serve_web(dir.clone(), req));
    }
    let app = app.layer(cors).with_state(state);

    let addr = format!("0.0.0.0:{}", port);
    println!("MCP server listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await
    .unwrap();
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

    fn sample_hub() -> Arc<ChartHub> {
        ChartHub::new(
            Arc::new(crate::chart_push::SampleSource),
            crate::chart_push::Limits::default(),
        )
    }

    fn test_app() -> Router {
        let state = Arc::new(AppState {
            oauth: Mutex::new(OAuthStore::new()),
            engine: None,
            charts: sample_hub(),
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

    #[test]
    fn accepts_json_rejects_q_zero() {
        // RFC 7231 §5.3.1: q=0 means "not acceptable".
        let h = headers_with(&[("accept", "application/json;q=0")]);
        assert!(!accepts_json(&h));
    }

    #[tokio::test]
    async fn post_mcp_accept_q_zero_returns_406() {
        let app = test_app();
        let body = br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
        let req = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("accept", "application/json;q=0")
            .header("content-type", "application/json")
            .body(Body::from(&body[..]))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_ACCEPTABLE);
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
    async fn initialize_request_with_method_initialized_and_id_returns_response() {
        // JSON-RPC 2.0 §4.1: a frame with an `id` is a request and MUST
        // produce a matching response envelope, even if the method name
        // matches a notification shape like `initialized`.
        let app = test_app();
        let body = br#"{"jsonrpc":"2.0","id":7,"method":"initialized"}"#;
        let req = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("accept", "application/json, text/event-stream")
            .header("content-type", "application/json")
            .body(Body::from(&body[..]))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_ne!(
            resp.status(),
            StatusCode::ACCEPTED,
            "requests (id present) must not be treated as notifications"
        );
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/json"
        );

        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let parsed: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed["jsonrpc"], "2.0");
        assert_eq!(parsed["id"], 7);
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
            charts: sample_hub(),
        });
        // Seed a valid token.
        {
            let mut s = state.oauth.lock().unwrap();
            s.tokens.insert(
                "test-token".to_string(),
                AccessToken {
                    _token: "test-token".to_string(),
                    client_id: "test".to_string(),
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

        // The handler emits an immediate `: connected` SSE comment so the
        // response head is deterministic. Assert unconditionally: a frame
        // MUST arrive within the timeout, and it MUST NOT contain the legacy
        // `event: endpoint` line.
        let mut body = resp.into_body();
        let first = tokio::time::timeout(
            std::time::Duration::from_millis(200),
            http_body_util::BodyExt::frame(&mut body),
        )
        .await
        .expect("first SSE frame did not arrive within 200ms")
        .expect("SSE stream ended before first frame")
        .expect("SSE frame read failed");
        let data = first
            .into_data()
            .expect("first SSE frame must be a data frame");
        let s = String::from_utf8_lossy(&data);
        assert!(
            !s.contains("event: endpoint"),
            "Streamable HTTP must not emit the legacy endpoint event, got: {}",
            s
        );
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

    #[test]
    fn notifications_and_responses_need_no_response() {
        for body in [
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": 3}}),
            json!({"jsonrpc": "2.0", "id": 9, "result": {}}),
            json!({"jsonrpc": "2.0", "id": 9, "error": {"code": -1, "message": "x"}}),
            json!([{"jsonrpc": "2.0", "method": "notifications/progress"},
                   {"jsonrpc": "2.0", "id": 1, "result": {}}]),
        ] {
            assert!(needs_no_response(&body), "{body}");
        }
        for body in [
            json!({"jsonrpc": "2.0", "id": 7, "method": "initialized"}),
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
            json!([{"jsonrpc": "2.0", "method": "notifications/progress"},
                   {"jsonrpc": "2.0", "id": 1, "method": "ping"}]),
            json!([]),
        ] {
            assert!(!needs_no_response(&body), "{body}");
        }
    }

    #[tokio::test]
    async fn post_notifications_cancelled_returns_202_without_body() {
        let state = Arc::new(AppState {
            oauth: Mutex::new(OAuthStore::new()),
            engine: None,
            charts: sample_hub(),
        });
        state.oauth.lock().unwrap().tokens.insert(
            "push-token".to_string(),
            AccessToken {
                _token: "push-token".to_string(),
                client_id: "test".to_string(),
                created_at: Instant::now(),
                expires_in: Duration::from_secs(3600),
            },
        );
        let app: Router = Router::new()
            .route("/mcp", post(mcp_handler))
            .with_state(state);
        let body = json!({"jsonrpc": "2.0", "method": "notifications/cancelled",
            "params": {"requestId": 3, "reason": "aborted"}});
        let resp = app.oneshot(rpc(body, Some("sid-1"))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::ACCEPTED);
        assert_eq!(resp.headers()["Mcp-Session-Id"], "sid-1");
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        assert!(bytes.is_empty());
    }

    // --- Chart push (ADR-0021) ---

    fn rpc(body: Value, session: Option<&str>) -> Request<Body> {
        let mut b = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("accept", "application/json, text/event-stream")
            .header("content-type", "application/json")
            .header("authorization", "Bearer push-token");
        if let Some(sid) = session {
            b = b.header("Mcp-Session-Id", sid);
        }
        b.body(Body::from(body.to_string())).unwrap()
    }

    async fn tool_text(
        app: &Router,
        name: &str,
        args: Value,
        session: Option<&str>,
    ) -> (bool, String) {
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": name, "arguments": args}});
        let resp = app.clone().oneshot(rpc(body, session)).await.unwrap();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        let text = v["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_string();
        (v["result"]["isError"] == true, text)
    }

    #[tokio::test]
    async fn chart_ticks_flow_over_the_sse_stream_until_it_closes() {
        let state = Arc::new(AppState {
            oauth: Mutex::new(OAuthStore::new()),
            engine: None,
            charts: sample_hub(),
        });
        state.oauth.lock().unwrap().tokens.insert(
            "push-token".to_string(),
            AccessToken {
                _token: "push-token".to_string(),
                client_id: "test".to_string(),
                created_at: Instant::now(),
                expires_in: Duration::from_secs(3600),
            },
        );
        let app: Router = Router::new()
            .route("/mcp", get(sse_handler).post(mcp_handler))
            .with_state(state.clone());

        // initialize mints the session id that scopes the subscriptions.
        let init = json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {}});
        let resp = app.clone().oneshot(rpc(init, None)).await.unwrap();
        let sid = resp.headers()["Mcp-Session-Id"]
            .to_str()
            .unwrap()
            .to_string();

        let args =
            json!({"ticker": "BTCUSDT", "timeframe": "1m", "indicators": ["rsi"], "warmup": 20});
        let (err, text) = tool_text(&app, "subscribe_chart", args.clone(), Some(&sid)).await;
        assert!(err && text.contains("no open SSE stream"), "{text}");

        let sse = Request::builder()
            .method("GET")
            .uri("/mcp")
            .header("authorization", "Bearer push-token")
            .header("Mcp-Session-Id", &sid)
            .body(Body::empty())
            .unwrap();
        let mut body = app.clone().oneshot(sse).await.unwrap().into_body();

        let (err, text) = tool_text(&app, "subscribe_chart", args.clone(), Some(&sid)).await;
        assert!(!err, "{text}");
        let sub: Value = serde_json::from_str(&text).unwrap();
        // Another session of the same token has no stream of its own.
        let (err, _) = tool_text(&app, "subscribe_chart", args, Some("other")).await;
        assert!(err);
        // Not pushable: the client polls instead.
        let (err, text) = tool_text(
            &app,
            "subscribe_chart",
            json!({"ticker": "AAPL"}),
            Some(&sid),
        )
        .await;
        assert!(err && text.starts_with("not pushable"), "{text}");

        let mut ticks = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while ticks.len() < 3 {
            let frame = tokio::time::timeout_at(deadline, body.frame())
                .await
                .expect("three ticks within 5 s")
                .unwrap()
                .unwrap();
            let chunk = String::from_utf8_lossy(frame.data_ref().unwrap()).to_string();
            if let Some(data) = chunk.strip_prefix("event: message\ndata: ") {
                let n: Value = serde_json::from_str(data.trim()).unwrap();
                assert_eq!(n["method"], "notifications/chart_tick");
                assert_eq!(n["params"]["subscription_id"], sub["subscription_id"]);
                assert_eq!(n["params"]["series"]["bars"].as_array().unwrap().len(), 2);
                ticks.push(n["params"]["series"]["bars"][1]["c"].as_f64().unwrap());
            }
        }

        // Closing the stream ends its subscriptions; the feed lingers 30 s.
        drop(body);
        assert_eq!(state.charts.counts(), (0, 0, 1));
    }

    // --- dynamic client registration limits ---

    fn reg(uris: &[&str]) -> RegisterRequest {
        RegisterRequest {
            redirect_uris: uris.iter().map(|u| u.to_string()).collect(),
            client_name: None,
        }
    }

    fn client(age: Duration, tokened: bool) -> OAuthClient {
        OAuthClient {
            _client_secret: None,
            redirect_uris: vec![],
            created_at: Instant::now() - age,
            tokened,
        }
    }

    #[test]
    fn registration_metadata_is_capped() {
        let mut s = OAuthStore::new();
        let ok = reg(&["http://localhost/cb"]);
        assert!(s.admit_registration("1.1.1.1", &ok).is_ok());
        let (status, _) = s.admit_registration("1.1.1.1", &reg(&[])).unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let six = reg(&["http://a/1"; 6]);
        assert!(s.admit_registration("1.1.1.1", &six).is_err());
        let long = "http://a/".to_string() + &"x".repeat(MAX_REDIRECT_URI_LEN);
        assert!(s.admit_registration("1.1.1.1", &reg(&[&long])).is_err());
        let mut named = reg(&["http://a/1"]);
        named.client_name = Some("n".repeat(MAX_CLIENT_NAME_LEN + 1));
        assert!(s.admit_registration("1.1.1.1", &named).is_err());
    }

    #[test]
    fn registration_is_rate_limited_per_address() {
        let mut s = OAuthStore::new();
        let ok = reg(&["http://localhost/cb"]);
        for _ in 0..MAX_REGISTRATIONS_PER_IP {
            assert!(s.admit_registration("2.2.2.2", &ok).is_ok());
        }
        let (status, _) = s.admit_registration("2.2.2.2", &ok).unwrap_err();
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert!(s.admit_registration("3.3.3.3", &ok).is_ok(), "per address");
        for _ in 0..=MAX_REGISTRATIONS_PER_IP {
            assert!(s.admit_registration("127.0.0.1", &ok).is_ok(), "loopback");
        }
    }

    #[test]
    fn registry_is_bounded_and_unused_clients_expire() {
        let mut s = OAuthStore::new();
        for i in 0..MAX_CLIENTS {
            s.clients
                .insert(format!("c{i}"), client(Duration::from_secs(60), false));
        }
        let ok = reg(&["http://localhost/cb"]);
        let (status, _) = s.admit_registration("4.4.4.4", &ok).unwrap_err();
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        // never-used clients older than a day are collected; used ones stay
        s.clients
            .insert("c0".into(), client(UNUSED_CLIENT_TTL * 2, false));
        s.clients
            .insert("c1".into(), client(UNUSED_CLIENT_TTL * 2, true));
        assert!(s.admit_registration("4.4.4.4", &ok).is_ok());
        assert!(!s.clients.contains_key("c0"));
        assert!(s.clients.contains_key("c1"));
    }
}
