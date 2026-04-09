use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tower_http::cors::CorsLayer;
use uuid::Uuid;

use crate::mcp::handle_mcp_request;

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

type SharedStore = Arc<Mutex<OAuthStore>>;

fn base_url(port: u16) -> String {
    std::env::var("CHARTGEN_BASE_URL").unwrap_or_else(|_| format!("http://localhost:{}", port))
}

fn port_from_base_url() -> u16 {
    // Extract port from CHARTGEN_BASE_URL or default
    if let Ok(url) = std::env::var("CHARTGEN_BASE_URL") {
        if let Some(port_str) = url.rsplit(':').next() {
            if let Ok(p) = port_str.trim_end_matches('/').parse::<u16>() {
                return p;
            }
        }
    }
    9315
}

// --- OAuth endpoints ---

async fn oauth_metadata(State(store): State<SharedStore>) -> Json<Value> {
    let _ = store; // unused but kept for consistent handler signatures
    let base = base_url(port_from_base_url());
    Json(json!({
        "issuer": base,
        "authorization_endpoint": format!("{}/authorize", base),
        "token_endpoint": format!("{}/token", base),
        "registration_endpoint": format!("{}/register", base),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none", "client_secret_post"]
    }))
}

#[derive(Deserialize)]
struct RegisterRequest {
    redirect_uris: Vec<String>,
    #[serde(default)]
    client_name: Option<String>,
}

async fn oauth_register(
    State(store): State<SharedStore>,
    Json(body): Json<RegisterRequest>,
) -> (StatusCode, Json<Value>) {
    let client_id = Uuid::new_v4().to_string();
    let client_secret = Uuid::new_v4().to_string();

    let client = OAuthClient {
        _client_secret: Some(client_secret.clone()),
        redirect_uris: body.redirect_uris.clone(),
    };

    store
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
    State(store): State<SharedStore>,
    Query(params): Query<AuthorizeQuery>,
) -> Response {
    let mut s = store.lock().unwrap();
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

async fn oauth_token(
    State(store): State<SharedStore>,
    axum::extract::Form(body): axum::extract::Form<TokenRequest>,
) -> Response {
    if body.grant_type != "authorization_code" {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "unsupported_grant_type"})),
        )
            .into_response();
    }

    let mut s = store.lock().unwrap();
    s.cleanup_expired();

    // Look up and remove auth code
    let auth_code = match s.codes.remove(&body.code) {
        Some(c) => c,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "invalid_grant", "error_description": "Invalid or expired code"})),
            )
                .into_response();
        }
    };

    // Validate client_id and redirect_uri
    if auth_code.client_id != body.client_id {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid_grant", "error_description": "client_id mismatch"})),
        )
            .into_response();
    }
    if auth_code.redirect_uri != body.redirect_uri {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid_grant", "error_description": "redirect_uri mismatch"})),
        )
            .into_response();
    }

    // PKCE S256 verification
    if !verify_pkce(&body.code_verifier, &auth_code.code_challenge) {
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
    let expires_in = Duration::from_secs(3600);
    s.tokens.insert(
        token.clone(),
        AccessToken {
            _token: token.clone(),
            _client_id: body.client_id.clone(),
            created_at: Instant::now(),
            expires_in,
        },
    );

    Json(json!({
        "access_token": token,
        "token_type": "Bearer",
        "expires_in": 3600
    }))
    .into_response()
}

fn verify_pkce(code_verifier: &str, code_challenge: &str) -> bool {
    let mut hasher = Sha256::new();
    hasher.update(code_verifier.as_bytes());
    let hash = hasher.finalize();
    let computed = URL_SAFE_NO_PAD.encode(hash);
    computed == code_challenge
}

// --- MCP Streamable HTTP transport ---

fn extract_bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
}

async fn mcp_handler(
    State(store): State<SharedStore>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let method = body.get("method").and_then(|m| m.as_str()).unwrap_or("");

    // Allow initialize without auth; require Bearer token for everything else
    if method != "initialize" {
        let token = match extract_bearer_token(&headers) {
            Some(t) => t.to_string(),
            None => {
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({
                        "jsonrpc": "2.0",
                        "id": body.get("id").cloned(),
                        "error": {"code": -32000, "message": "Missing Authorization header"}
                    })),
                )
                    .into_response();
            }
        };
        if !store.lock().unwrap().validate_token(&token) {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({
                    "jsonrpc": "2.0",
                    "id": body.get("id").cloned(),
                    "error": {"code": -32000, "message": "Invalid or expired token"}
                })),
            )
                .into_response();
        }
    }

    let id = body.get("id").cloned();
    let params = body.get("params");

    // Notifications: no response needed
    if method == "initialized" || method == "notifications/initialized" {
        return StatusCode::NO_CONTENT.into_response();
    }

    // Run MCP handler in a blocking task (chart rendering + data fetching use blocking I/O)
    let method_owned = method.to_string();
    let params_owned = params.cloned();
    let result = tokio::task::spawn_blocking(move || {
        handle_mcp_request(&method_owned, id, params_owned.as_ref())
    })
    .await
    .unwrap();

    // Generate session ID header
    let session_id = Uuid::new_v4().to_string();
    let mut response = Json(result).into_response();
    response
        .headers_mut()
        .insert("Mcp-Session-Id", session_id.parse().unwrap());
    response
}

// --- Server startup ---

pub async fn run_server(port: u16) {
    let store = Arc::new(Mutex::new(OAuthStore::new()));

    // Store port for metadata endpoint
    std::env::set_var("_CHARTGEN_PORT", port.to_string());

    let app = Router::new()
        .route(
            "/.well-known/oauth-authorization-server",
            get(oauth_metadata),
        )
        .route("/register", post(oauth_register))
        .route("/authorize", get(oauth_authorize))
        .route("/token", post(oauth_token))
        .route("/mcp", post(mcp_handler))
        .layer(CorsLayer::permissive())
        .with_state(store);

    let addr = format!("0.0.0.0:{}", port);
    println!("MCP server listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
