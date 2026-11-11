//! The operator page's one read, `GET /admin/overview` (ADR-0063): who signed in, which
//! clients registered and how the server is, for a signed-in operator in the web app.
//!
//! It is a layer and not a route. For anyone who is not an operator the request goes on
//! untouched, and since no route of that name exists it is answered as any unknown path
//! is: nobody can tell from an answer that this server has such a page, or who may see
//! it. Only counts, names, ids and times leave here.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::{json, Value};

use super::SharedState;
use chartgen::scripting::persistence::{self, ClientSummary, UserSummary};

/// The path, compared exactly: another case, a trailing slash or an encoded letter is
/// another path, and so not this.
pub(super) const PATH: &str = "/admin/overview";
/// Accounts shown, those that signed in last. The table of users has no limit.
const USERS_SHOWN: usize = 200;
/// Clients shown, those registered last.
const CLIENTS_SHOWN: usize = 50;
/// How long a built answer is handed out again: reloading costs one build in this time.
const KEPT: Duration = Duration::from_secs(10);
/// Characters of a name shown. Names are text a stranger chose.
const NAME_CHARS: usize = 80;
/// Characters of a client id shown: enough to tell rows apart, and nothing to use.
const CLIENT_ID_CHARS: usize = 8;

static STARTED: OnceLock<i64> = OnceLock::new();

/// Note when the server started, for the page.
pub(super) fn mark_started() {
    STARTED.get_or_init(|| chrono::Utc::now().timestamp());
}

/// Answer the overview to an operator, and let every other request go on as it is.
/// Method and path are looked at first: no other request takes a lock here.
pub(super) async fn layer(
    State(state): State<SharedState>,
    request: Request,
    next: Next,
) -> Response {
    if request.method() != Method::GET || request.uri().path() != PATH {
        return next.run(request).await;
    }
    let operator = state.oauth.lock().unwrap().is_operator(request.headers());
    if !operator {
        return next.run(request).await;
    }
    match overview(state).await {
        Some(body) => (
            [
                (
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("application/json"),
                ),
                // For one person, and not to be kept by anything in between.
                (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
                (header::VARY, HeaderValue::from_static("Authorization")),
                (header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY")),
                (
                    header::X_CONTENT_TYPE_OPTIONS,
                    HeaderValue::from_static("nosniff"),
                ),
            ],
            body.as_ref().clone(),
        )
            .into_response(),
        None => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

/// The answer, built at most once in `KEPT` and by one caller at a time. Off the async
/// workers and with no lock of the OAuth store held: the store has one connection, which
/// token rotations wait on too.
async fn overview(state: SharedState) -> Option<Arc<String>> {
    static LAST: tokio::sync::Mutex<Option<(Instant, Arc<String>)>> =
        tokio::sync::Mutex::const_new(None);
    let mut last = LAST.lock().await;
    if let Some((at, body)) = last.as_ref() {
        if at.elapsed() < KEPT {
            return Some(body.clone());
        }
    }
    let server = server_block(&state);
    let body = tokio::task::spawn_blocking(move || build(server))
        .await
        .ok()
        .flatten()
        .map(Arc::new)?;
    *last = Some((Instant::now(), body.clone()));
    Some(body)
}

/// What is read from memory, on the caller's thread.
fn server_block(state: &SharedState) -> Value {
    let (push_subscriptions, _, push_feeds) = state.charts.in_use();
    let alerts = crate::server_alerts::global();
    let (pairs, pair_limit) = alerts.map_or((0, 0), |a| a.watched_pairs());
    json!({
        "version": env!("CARGO_PKG_VERSION"),
        "started_at": STARTED.get().copied(),
        "features": {
            "sign_in": true,
            "server_alerts": alerts.is_some(),
            "setups": crate::setup_service::global().is_some(),
            "scripting": std::env::var("CHARTGEN_SCRIPTING").is_ok_and(|v| v == "1"),
            "trading": state.engine.is_some(),
            // A replay server has no such page.
            "replay": false,
        },
        "feeds": {
            "watched_pairs": pairs,
            "watched_pairs_limit": pair_limit,
            "push_subscriptions": push_subscriptions,
            "push_feeds": push_feeds,
        },
    })
}

fn build(mut server: Value) -> Option<String> {
    let store = persistence::global()?;
    let summary = store
        .operator_summary(USERS_SHOWN, CLIENTS_SHOWN)
        .inspect_err(|e| eprintln!("[admin] could not read the store: {e}"))
        .ok()?;
    let open = crate::setup_service::global()
        .map(|s| s.open_by_owner())
        .unwrap_or_default();
    server["schema_version"] = json!(summary.schema_version);
    server["pending_deliveries"] = json!(summary.pending_deliveries);
    server["store_bytes"] = json!(std::fs::metadata(persistence::db_path())
        .ok()
        .map(|m| m.len()));
    server["setups_total"] = json!(summary.setups_total);
    let body = json!({
        "server": server,
        "users": {
            "total": summary.users_total,
            "shown": summary.users.iter().map(|u| user_row(u, &open)).collect::<Vec<_>>(),
        },
        "clients": {
            "total": summary.clients_total,
            "limit": super::MAX_CLIENTS,
            "shown": summary.clients.iter().map(client_row).collect::<Vec<_>>(),
        },
    });
    Some(body.to_string())
}

fn user_row(user: &UserSummary, open: &HashMap<String, usize>) -> Value {
    let open = open.get(&user.id).copied().unwrap_or(0) as i64;
    json!({
        "id": user.id,
        "name": shown_text(&user.name),
        "provider": user.id.split(':').next().unwrap_or_default(),
        "created_at": user.created_at,
        "last_login_at": user.last_login_at,
        "last_token_at": user.last_token_at,
        "workspace_docs": user.workspace_docs,
        "workspace_bytes": user.workspace_bytes,
        "setups_open": open.min(user.setups),
        "setups_closed": (user.setups - open).max(0),
        "server_alerts": user.server_alerts,
        "authorisations": user.authorisations,
        "push_devices": user.push_devices,
    })
}

fn client_row(client: &ClientSummary) -> Value {
    json!({
        "id": client.id.chars().take(CLIENT_ID_CHARS).collect::<String>(),
        "name": client.name.as_deref().map(shown_text),
        "created_at": client.created_at,
        "last_token_at": client.last_token_at,
        "has_authorisation": client.has_family,
        "owns_setups": client.owns_setups,
    })
}

/// A name as it may be shown: no control character and none that changes the direction
/// of text, which could make one row read like another, and no longer than `NAME_CHARS`.
fn shown_text(name: &str) -> String {
    name.chars()
        .filter(|c| {
            !c.is_control()
                && !matches!(c, '\u{061C}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}')
        })
        .take(NAME_CHARS)
        .collect()
}

/// The user ids in `CHARTGEN_ADMIN_USERS`. A setting that cannot be read as written is
/// an error and not "nobody": the operator would believe something the server does not
/// do. So is the setting on a server without sign-in, where it can never take effect.
pub(super) fn operators(
    raw: Option<&str>,
    sign_in: bool,
) -> Result<std::collections::HashSet<String>, String> {
    let Some(raw) = raw.filter(|r| !r.trim().is_empty()) else {
        return Ok(Default::default());
    };
    if !sign_in {
        return Err(
            "CHARTGEN_ADMIN_USERS is set but sign-in is off: there is no operator without accounts"
                .into(),
        );
    }
    let mut ids = std::collections::HashSet::new();
    for (i, entry) in raw.split(',').enumerate() {
        let entry = entry.trim();
        if !could_be_a_user_id(entry) {
            return Err(format!(
                "entry {} of CHARTGEN_ADMIN_USERS is not a user id (github:<number> or google:<sub>)",
                i + 1
            ));
        }
        ids.insert(entry.to_string());
    }
    Ok(ids)
}

/// Whether `entry` is an id sign-in could have stored (`identity_from_profile`): no
/// less, or a real account could not be named, and no more. GitHub's is its numeric
/// id, above 0 and written as a number is. Google's is the `sub` it sends, which the
/// server takes as any text; it is digits in practice, but that is Google's to say.
fn could_be_a_user_id(entry: &str) -> bool {
    if let Some(id) = entry.strip_prefix("github:") {
        return id
            .parse::<u64>()
            .is_ok_and(|n| n > 0 && n.to_string() == id);
    }
    entry.strip_prefix("google:").is_some_and(|sub| {
        !sub.is_empty()
            && sub.len() <= 255
            && sub.trim() == sub
            && !sub.chars().any(char::is_control)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_operator_list_is_exact_ids_or_an_error() {
        assert!(operators(None, true).unwrap().is_empty());
        assert!(operators(Some("  "), false).unwrap().is_empty());
        let two = operators(Some("github:1000001, google:2000002"), true).unwrap();
        assert_eq!(two.len(), 2);
        assert!(two.contains("github:1000001") && two.contains("google:2000002"));
        // Whatever sign-in stores can be named: Google's `sub` is taken as any text.
        let text = operators(Some("google:a-sub_of.Any-text"), true).unwrap();
        assert!(text.contains("google:a-sub_of.Any-text"));
        for bad in [
            "*",
            "someone",
            "someone@mail.example",
            "github:",
            "github:12ab",
            // Not what sign-in stores for GitHub: its id is a number above 0.
            "github:0",
            "github:007",
            "github:99999999999999999999999",
            "google:",
            "google: 2000002",
            "GitHub:1000001",
            "github:1000001,",
            "github:1000001,,google:2000002",
            "github: 1000001",
            "gitlab:1000001",
        ] {
            let err = operators(Some(bad), true).unwrap_err();
            assert!(err.contains("CHARTGEN_ADMIN_USERS"), "{bad}: {err}");
            // The value itself is not echoed.
            assert!(!err.contains("someone"), "{err}");
        }
        let off = operators(Some("github:1000001"), false).unwrap_err();
        assert!(off.contains("sign-in is off"), "{off}");
    }

    #[test]
    fn a_name_is_shown_without_what_could_disguise_a_row() {
        assert_eq!(shown_text("Ann"), "Ann");
        // Right-to-left override, a zero-width space, a newline and a bell.
        assert_eq!(shown_text("A\u{202E}nn\u{200B}\n\u{7}!"), "Ann!");
        assert_eq!(shown_text(&"x".repeat(500)).chars().count(), NAME_CHARS);
        // Markup is not this function's business: the page writes text, never markup.
        assert_eq!(shown_text("<b>x</b>"), "<b>x</b>");
    }

    #[test]
    fn a_client_row_shows_the_start_of_its_id_only() {
        let row = client_row(&ClientSummary {
            id: "0123456789abcdef-rest-of-the-id".into(),
            name: Some("An\u{202E} agent".into()),
            created_at: 1,
            last_token_at: None,
            has_family: true,
            owns_setups: false,
        });
        assert_eq!(row["id"], "01234567");
        assert_eq!(row["name"], "An agent");
        assert!(!row.to_string().contains("rest-of-the-id"));
    }

    #[test]
    fn a_user_row_splits_setups_into_open_and_closed() {
        let user = UserSummary {
            id: "github:1000001".into(),
            name: "Ann".into(),
            created_at: 1,
            last_login_at: 2,
            last_token_at: None,
            workspace_docs: 0,
            workspace_bytes: 0,
            setups: 5,
            server_alerts: 0,
            authorisations: 0,
            push_devices: 0,
        };
        let open = HashMap::from([("github:1000001".to_string(), 2)]);
        let row = user_row(&user, &open);
        assert_eq!(
            (row["setups_open"].as_i64(), row["setups_closed"].as_i64()),
            (Some(2), Some(3))
        );
        assert_eq!(row["provider"], "github");
    }
}
