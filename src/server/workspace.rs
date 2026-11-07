//! A signed-in user's workspace: settings, layouts, watchlists, drawings and browser
//! alerts, as opaque documents the web app keeps in sync (ADR-0050).
//!
//! The account comes from the bearer token and nowhere else, and anyone with an account
//! can sign in, so everything here is limited per user and per document.

use super::*;
#[cfg(feature = "scripting")]
use chartgen::scripting::persistence::{WorkspaceLimits, WorkspaceOutcome, WorkspaceWrite};

#[cfg(feature = "scripting")]
const MAX_DOC_BYTES: usize = 512 * 1024;
/// Per user. The app keeps a copy of all of it in `localStorage`, plus what it needs to
/// merge, and browsers give an origin about 5 MB, so more could not be kept anyway.
#[cfg(feature = "scripting")]
const MAX_USER_BYTES: i64 = 2 * 1024 * 1024;
#[cfg(feature = "scripting")]
const MAX_USER_DOCS: i64 = 1000;
/// In the whole file. Anyone can sign in, and each account can only add so much, but
/// accounts are free and the disk is not. Past it nobody can grow; existing rows stay.
#[cfg(feature = "scripting")]
const MAX_TOTAL_BYTES: i64 = 1024 * 1024 * 1024;
const MAX_WRITES_PER_REQUEST: usize = 50;
const MAX_KEY_LEN: usize = 128;

/// A document key as the app names them: what `chartgen.` keys and symbols are made of.
#[cfg_attr(not(feature = "scripting"), allow(dead_code))]
fn is_doc_key(key: &str) -> bool {
    key.len() <= MAX_KEY_LEN
        && key.len() > "chartgen.".len()
        && key.starts_with("chartgen.")
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._^=-:".contains(c))
}

#[derive(Deserialize)]
#[cfg_attr(not(feature = "scripting"), allow(dead_code))]
pub(super) struct SyncWrite {
    key: String,
    value: Option<String>,
    base_version: i64,
}

#[derive(Deserialize)]
#[cfg_attr(not(feature = "scripting"), allow(dead_code))]
pub(super) struct SyncRequest {
    since: i64,
    #[serde(default)]
    writes: Vec<SyncWrite>,
}

/// `POST /workspace/sync`: apply the caller's writes and answer with what became of each
/// and what changed since `since`. `404` where there is no sign-in or no store, because
/// without an account there is nothing to sync with.
pub(super) async fn sync(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<SyncRequest>,
) -> Response {
    let user = {
        let mut s = state.oauth.lock().unwrap();
        if s.signin.is_none() {
            return StatusCode::NOT_FOUND.into_response();
        }
        let Some(token) = extract_bearer_token(&headers).filter(|t| s.validate_token(t)) else {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "invalid_token" })),
            )
                .into_response();
        };
        // An application the user approved has the tools, not their browser's data.
        let Some(user) = s.app_user_for_token(token) else {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({ "error": "insufficient_scope" })),
            )
                .into_response();
        };
        if !s.admit_workspace_call(&user) {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                [(header::RETRY_AFTER, "60")],
                Json(json!({ "error": "too many sync requests" })),
            )
                .into_response();
        }
        user
    };
    if request.writes.len() > MAX_WRITES_PER_REQUEST {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "too many writes in one request" })),
        )
            .into_response();
    }
    answer(user, request).await
}

#[cfg(not(feature = "scripting"))]
async fn answer(_user: String, _request: SyncRequest) -> Response {
    StatusCode::NOT_FOUND.into_response()
}

#[cfg(feature = "scripting")]
async fn answer(user: String, request: SyncRequest) -> Response {
    let done = tokio::task::spawn_blocking(move || {
        let store = scripting::persistence::global()?;
        Some(apply(store, &user, request))
    })
    .await;
    match done {
        Ok(Some(Ok(body))) => Json(body).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Ok(Some(Err(e))) => {
            eprintln!("[workspace] cannot sync: {e}");
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[cfg(feature = "scripting")]
fn apply(
    store: &scripting::persistence::Store,
    user: &str,
    request: SyncRequest,
) -> rusqlite::Result<Value> {
    let limits = WorkspaceLimits {
        doc_bytes: MAX_DOC_BYTES,
        user_bytes: MAX_USER_BYTES,
        user_docs: MAX_USER_DOCS,
        total_bytes: MAX_TOTAL_BYTES,
    };
    let (good, bad): (Vec<_>, Vec<_>) =
        request.writes.into_iter().partition(|w| is_doc_key(&w.key));
    let writes: Vec<WorkspaceWrite> = good
        .into_iter()
        .map(|w| WorkspaceWrite {
            key: w.key,
            value: w.value,
            base_version: w.base_version,
        })
        .collect();
    let (outcomes, changes, cursor) =
        store.workspace_sync(user, request.since, &writes, &limits)?;
    let mut results: Vec<Value> = writes
        .iter()
        .zip(outcomes)
        .map(|(write, outcome)| {
            let key = &write.key;
            match outcome {
                WorkspaceOutcome::Stored { version } => {
                    json!({ "key": key, "status": "ok", "version": version })
                }
                WorkspaceOutcome::Conflict { version, value } => {
                    json!({ "key": key, "status": "conflict", "version": version, "value": value })
                }
                WorkspaceOutcome::TooLarge => json!({ "key": key, "status": "too_large" }),
                WorkspaceOutcome::OverQuota => json!({ "key": key, "status": "quota" }),
            }
        })
        .collect();
    results.extend(
        bad.iter()
            .map(|w| json!({ "key": w.key, "status": "bad_key" })),
    );
    let changes: Vec<Value> = changes
        .iter()
        .map(|d| json!({ "key": d.key, "value": d.value, "version": d.version }))
        .collect();
    Ok(json!({ "cursor": cursor, "results": results, "changes": changes }))
}

#[cfg(all(test, feature = "scripting"))]
mod tests {
    use super::*;

    #[test]
    fn a_doc_key_is_what_the_app_names_them() {
        for good in [
            "chartgen.layouts",
            "chartgen.drawings.BRK.B",
            "chartgen.drawings.^GSPC",
            "chartgen.drawings.EURUSD=X",
            "chartgen.pane.1.symbol",
        ] {
            assert!(is_doc_key(good), "{good}");
        }
        let long = format!("chartgen.{}", "a".repeat(MAX_KEY_LEN));
        for bad in [
            "",
            "chartgen.",
            "other.layouts",
            "chartgen.a b",
            "chartgen.a/../b",
            "chartgen.a\nb",
            long.as_str(),
        ] {
            assert!(!is_doc_key(bad), "{bad}");
        }
    }
}
