//! A signed-in user's workspace: settings, layouts, watchlists, drawings and browser
//! alerts, as opaque documents the web app keeps in sync (ADR-0050).
//!
//! The account comes from the bearer token and nowhere else, and anyone with an account
//! can sign in, so everything here is limited per user and per document.

use super::*;
use axum::body::Bytes;
use axum::extract::FromRequest;
#[cfg(feature = "scripting")]
use chartgen::scripting::persistence::{
    WorkspaceDoc, WorkspaceLimits, WorkspaceOutcome, WorkspaceWrite,
};

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
/// Deleted documents remembered per user, on top of the time limit. A row is a key and a
/// version, so an account's come to about 200 KB, however many it deletes.
#[cfg(feature = "scripting")]
const MAX_USER_TOMBSTONES: i64 = 1000;
const MAX_WRITES_PER_REQUEST: usize = 50;
/// What the web app sends at most (`MAX_BODY_BYTES` in `lib/workspaceSync.ts`).
pub(super) const MAX_BODY_BYTES: usize = 1024 * 1024;
const MAX_KEY_LEN: usize = 128;
/// To send a request's body. A client that takes longer is holding a slot from others.
const BODY_TIMEOUT: Duration = Duration::from_secs(10);
/// The longest a request keeps its share of the syncs running. A client that does not
/// read its answer keeps the one sync of its account running, but not others waiting.
const SLOT_TIMEOUT: Duration = Duration::from_secs(30);

/// The keys the app syncs (`isSynced` in `lib/workspaceSync.ts`), and what those are made of.
#[cfg_attr(not(feature = "scripting"), allow(dead_code))]
fn is_doc_key(key: &str) -> bool {
    let family = ["chartgen.layouts", "chartgen.indicators.active"]
        .into_iter()
        .chain(["chartgen.alerts.list", "chartgen.chartSettings"])
        .chain(["chartgen.orderflow"])
        .any(|exact| key == exact)
        || [
            "chartgen.layout.",
            "chartgen.pane.",
            "chartgen.watchlist.",
            "chartgen.drawings.",
        ]
        .into_iter()
        .any(|prefix| key.len() > prefix.len() && key.starts_with(prefix));
    family
        && key != "chartgen.drawings.index"
        && key.len() <= MAX_KEY_LEN
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._^=-:".contains(c))
}

/// A running sync: one per user and `MAX_WORKSPACE_SYNCS` in all, until its answer is
/// out. Both are refused rather than queued, as the body is not read before this.
struct Running {
    state: SharedState,
    user: String,
    slot: Arc<Mutex<Option<tokio::sync::OwnedSemaphorePermit>>>,
}

impl Running {
    fn new(state: SharedState, user: String, slot: tokio::sync::OwnedSemaphorePermit) -> Self {
        let slot = Arc::new(Mutex::new(Some(slot)));
        let held = slot.clone();
        tokio::spawn(async move {
            tokio::time::sleep(SLOT_TIMEOUT).await;
            held.lock().unwrap().take();
        });
        Self { state, user, slot }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.slot.lock().unwrap().take();
        self.state
            .oauth
            .lock()
            .unwrap()
            .workspace_call_finished(&self.user);
    }
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

fn refused(status: StatusCode, retry_after: u64, error: &str) -> Response {
    (
        status,
        [(header::RETRY_AFTER, retry_after.to_string())],
        Json(json!({ "error": error })),
    )
        .into_response()
}

/// `POST /workspace/sync`: apply the caller's writes and answer with what became of each
/// and what changed since `since`. `404` where there is no sign-in or no store, because
/// without an account there is nothing to sync with. The body is read only for a caller
/// who may send it, and only as many as are allowed at once.
pub(super) async fn sync(State(state): State<SharedState>, request: Request) -> Response {
    let (user, slot) = {
        let mut s = state.oauth.lock().unwrap();
        if s.signin.is_none() {
            return StatusCode::NOT_FOUND.into_response();
        }
        let Some(token) = extract_bearer_token(request.headers()).filter(|t| s.validate_token(t))
        else {
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
        match s.admit_workspace_call(&user) {
            Ok(slot) => (user, slot),
            Err((retry_after, why)) => {
                return refused(StatusCode::TOO_MANY_REQUESTS, retry_after, why);
            }
        }
    };
    let running = Running::new(state.clone(), user.clone(), slot);

    let body = match tokio::time::timeout(BODY_TIMEOUT, Bytes::from_request(request, &())).await {
        Ok(Ok(body)) => body,
        Ok(Err(rejection)) => return rejection.into_response(),
        Err(_) => return StatusCode::REQUEST_TIMEOUT.into_response(),
    };
    state
        .oauth
        .lock()
        .unwrap()
        .charge_workspace_bytes(&user, body.len());
    let request: SyncRequest = match Json::from_bytes(&body) {
        Ok(Json(request)) => request,
        Err(rejection) => return rejection.into_response(),
    };
    drop(body);
    if request.writes.len() > MAX_WRITES_PER_REQUEST {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "too many writes in one request" })),
        )
            .into_response();
    }
    answer(running, user, request).await
}

#[cfg(not(feature = "scripting"))]
async fn answer(_running: Running, _user: String, _request: SyncRequest) -> Response {
    StatusCode::NOT_FOUND.into_response()
}

#[cfg(feature = "scripting")]
async fn answer(running: Running, user: String, request: SyncRequest) -> Response {
    // `running` goes along, so a caller that hangs up does not free its place while the
    // work goes on.
    let done = tokio::task::spawn_blocking(move || {
        let result = scripting::persistence::global().map(|store| apply(store, &user, request));
        (running, result)
    })
    .await;
    match done {
        Ok((running, Some(Ok(body)))) => answer_held_by(running, body),
        Ok((_, None)) => StatusCode::NOT_FOUND.into_response(),
        Ok((_, Some(Err(e)))) => {
            eprintln!("[workspace] cannot sync: {e}");
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// The answer, keeping its place until it is written out: answers a client does not read
/// are memory too, and it is free to ask for 120 of them a minute.
#[cfg(feature = "scripting")]
fn answer_held_by(running: Running, body: Vec<u8>) -> Response {
    let stream = async_stream::stream! {
        yield Ok::<_, std::convert::Infallible>(Bytes::from(body));
        drop(running);
    };
    (
        [(header::CONTENT_TYPE, "application/json")],
        axum::body::Body::from_stream(stream),
    )
        .into_response()
}

#[cfg(feature = "scripting")]
#[derive(serde::Serialize)]
struct Answer<'a> {
    cursor: i64,
    restored: bool,
    results: Vec<Value>,
    changes: &'a [WorkspaceDoc],
}

#[cfg(feature = "scripting")]
fn apply(
    store: &scripting::persistence::Store,
    user: &str,
    request: SyncRequest,
) -> rusqlite::Result<Vec<u8>> {
    let limits = WorkspaceLimits {
        doc_bytes: MAX_DOC_BYTES,
        user_bytes: MAX_USER_BYTES,
        user_docs: MAX_USER_DOCS,
        user_tombstones: MAX_USER_TOMBSTONES,
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
    let synced = store.workspace_sync(user, request.since, &writes, &limits)?;
    let mut results: Vec<Value> = writes
        .iter()
        .zip(synced.outcomes)
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
    drop(writes);
    // Serialized from the rows as they are: a `Value` of them would be one copy more.
    Ok(serde_json::to_vec(&Answer {
        cursor: synced.cursor,
        restored: synced.restored,
        results,
        changes: &synced.changes,
    })
    .expect("strings and numbers serialize"))
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
            "chartgen.chartSettings",
            "chartgen.orderflow",
            "chartgen.watchlist.v2",
        ] {
            assert!(is_doc_key(good), "{good}");
        }
        let long = format!("chartgen.pane.{}", "a".repeat(MAX_KEY_LEN));
        for bad in [
            "",
            "chartgen.",
            "other.layouts",
            "chartgen.layout.",
            "chartgen.drawings.index",
            "chartgen.drawings.",
            "chartgen.layouts.bak",
            "chartgen.sync.cursor",
            "chartgen.workspace.account",
            "chartgen.oauth.client_secret",
            "chartgen.sidebar.tab",
            "chartgen.scripts.drafts",
            "chartgen.pane.a b",
            "chartgen.pane.a/../b",
            "chartgen.pane.a\nb",
            long.as_str(),
        ] {
            assert!(!is_doc_key(bad), "{bad}");
        }
    }
}
