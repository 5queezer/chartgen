//! Refresh tokens (ADR-0058): what lets a client that was authorized once keep working
//! across a restart, a deploy and the 7 days of an access token, with nobody at a
//! browser. Only a token's SHA-256 is kept, in memory and in the store. The web app's
//! own client never gets one, so nothing durable is first-party (ADR-0050).

use super::signin::{hash_of, now_unix, random_token, TokenUser, MAX_TOKENS_PER_CLIENT};
use super::*;

/// A family ends when its newest token is this old. Each exchange starts it again.
pub(super) const REFRESH_TTL_SECS: i64 = 30 * 24 * 3600;
/// A client that lost the answer to an exchange may send the same token again for
/// this long. It gets a new successor and the first one is spent, so a family never
/// has two usable tokens.
const RETRY_WINDOW_SECS: i64 = 60;
/// The `spent_at` of a successor that a retry replaced before it was ever exchanged.
/// It is spent "long ago": no retry window covers it, so its next use ends the family.
/// A token spent by its own exchange carries the time of that exchange.
pub(super) const REPLACED: i64 = 0;
/// Spent tokens kept per family, so that using one again is recognised. An older one
/// is merely unknown, and refused all the same.
const MAX_SPENT_PER_FAMILY: i64 = 100;

/// One refresh token, by the hash of its value. A family is one authorization: its
/// tokens follow each other by `seq`, and at most one of them is unspent.
#[derive(Clone)]
pub(super) struct RefreshToken {
    pub(super) family: String,
    pub(super) seq: i64,
    pub(super) client_id: String,
    /// The signed-in user it was issued to; `None` with sign-in off.
    pub(super) user: Option<String>,
    pub(super) created_at: i64,
    pub(super) spent_at: Option<i64>,
    /// The hash of the access token issued with it. Stored while this token is
    /// unspent, so that the access token is known again after a restart: a client
    /// holding a token it believes valid would otherwise get a `401`, and the common
    /// MCP client answers that with a new authorization, not with a refresh.
    #[cfg_attr(not(feature = "scripting"), allow(dead_code))]
    pub(super) access_hash: Option<String>,
}

/// What a change to the families writes to the store.
#[cfg_attr(not(feature = "scripting"), allow(dead_code))]
pub(super) enum Change {
    /// A new token, and the ones it makes spent as of `spent_at`: together or not
    /// at all.
    Save {
        hash: String,
        token: RefreshToken,
        spent: Vec<String>,
        spent_at: i64,
    },
    Delete(Vec<String>),
}

/// How a change reaches the store: `store_write` when serving, and a test's own to
/// see what a failed write does. Blocking.
pub(super) type WriteFn = Arc<dyn Fn(Change) -> Result<(), String> + Send + Sync>;

/// A build without the store, or a store that could not be opened, keeps families in
/// memory only, as it does sessions.
pub(super) fn store_write() -> WriteFn {
    Arc::new(|change| {
        #[cfg(feature = "scripting")]
        if let Some(store) = scripting::persistence::global() {
            return write_to(store, change);
        }
        let _ = change;
        Ok(())
    })
}

#[cfg(feature = "scripting")]
fn write_to(store: &scripting::persistence::Store, change: Change) -> Result<(), String> {
    match change {
        Change::Save {
            hash,
            token,
            spent,
            spent_at,
        } => store.save_refresh_token(
            &scripting::persistence::StoredRefreshToken {
                token_hash: hash,
                family: token.family,
                seq: token.seq,
                client_id: token.client_id,
                user_id: token.user,
                created_at: token.created_at,
                spent_at: token.spent_at,
                access_hash: token.access_hash,
            },
            &spent,
            spent_at,
            token.seq - MAX_SPENT_PER_FAMILY,
        ),
        Change::Delete(families) => store.delete_refresh_families(&families),
    }
    .map_err(|e| e.to_string())
}

async fn write_off_worker(write: &WriteFn, change: Change) -> Result<(), String> {
    let write = write.clone();
    tokio::task::spawn_blocking(move || write(change))
        .await
        .unwrap_or_else(|e| Err(e.to_string()))
}

/// Delete families whose loss on disk only matters after a restart, where a row left
/// behind would bring a family back.
pub(super) async fn forget_families(write: &WriteFn, families: Vec<String>) {
    if families.is_empty() {
        return;
    }
    if let Err(e) = write_off_worker(write, Change::Delete(families)).await {
        eprintln!("[OAuth] could not delete ended refresh tokens: {e}");
    }
}

/// Exchanges and revocations run one at a time: each reads the families, writes the
/// store and then changes the families, and the store is written off the OAuth lock.
static GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// A refresh token's value is `<family part>.<secret>`, both random. The family part is
/// the same in every token of one family, and the family's id is its hash. So a token
/// the server no longer has a row for still says which family it was of, to someone
/// who held it, while nothing in the store leads back to a family part: the id cannot
/// be turned into one.
fn family_of(value: &str) -> Option<(&str, String)> {
    let (part, _) = value.split_once('.')?;
    (!part.is_empty()).then(|| (part, hash_of(part)))
}

/// A token to hand out once the store has it.
pub(super) struct Rotation {
    /// The value for the client. Never stored or logged.
    pub(super) value: String,
    /// The access token that goes out with it.
    access: String,
    hash: String,
    token: RefreshToken,
    spent: Vec<String>,
    /// The time of the exchange, or `REPLACED` for the successor a retry replaces.
    spent_at: i64,
}

impl Rotation {
    fn change(&self) -> Change {
        Change::Save {
            hash: self.hash.clone(),
            token: self.token.clone(),
            spent: self.spent.clone(),
            spent_at: self.spent_at,
        }
    }
}

enum Refused {
    Invalid,
    /// A spent token came back: whoever holds the family's tokens, one of them is not
    /// the client, so the family ends.
    Reused(String),
}

impl OAuthStore {
    /// Drop the families nobody used for `REFRESH_TTL_SECS`.
    pub(super) fn expire_refresh(&mut self, now: i64) {
        let live: HashSet<String> = self
            .refresh
            .values()
            .filter(|t| t.spent_at.is_none() && now - t.created_at < REFRESH_TTL_SECS)
            .map(|t| t.family.clone())
            .collect();
        self.refresh.retain(|_, t| live.contains(&t.family));
        self.restored
            .retain(|_, t| t.family.as_ref().is_some_and(|f| live.contains(f)));
    }

    /// End families, with the access tokens that came from them.
    pub(super) fn end_families(&mut self, families: &[String]) {
        let ended = |t: &AccessToken| t.family.as_ref().is_some_and(|f| families.contains(f));
        self.refresh.retain(|_, t| !families.contains(&t.family));
        self.tokens.retain(|_, t| !ended(t));
        self.restored.retain(|_, t| !ended(t));
    }

    /// The families `user` holds through `client`.
    pub(super) fn families_of(&self, user: &str, client: &str) -> Vec<String> {
        self.refresh
            .values()
            .filter(|t| {
                t.spent_at.is_none() && t.client_id == client && t.user.as_deref() == Some(user)
            })
            .map(|t| t.family.clone())
            .collect()
    }

    /// Load the stored families at startup. A token of a client no longer registered
    /// is left out. One issued under the other sign-in mode is kept and refused, so
    /// switching back finds it again. Returns how many families are usable.
    #[cfg(feature = "scripting")]
    pub(super) fn restore_refresh(
        &mut self,
        rows: Vec<scripting::persistence::StoredRefreshToken>,
    ) -> usize {
        let now = now_unix();
        let sign_in = self.signin.is_some();
        for r in rows {
            if !self.clients.contains_key(&r.client_id) {
                continue;
            }
            // The 7 days count from when it was issued: a restart adds nothing. What is
            // left is counted from now: not every platform's `Instant` can name a time
            // before the process could have started.
            let left = ACCESS_TOKEN_SECS.saturating_sub((now - r.created_at).max(0) as u64);
            let access = r
                .access_hash
                .clone()
                .filter(|_| r.spent_at.is_none() && r.user_id.is_some() == sign_in)
                .filter(|_| left > 0);
            if let Some(access) = access {
                self.restored.insert(
                    access,
                    AccessToken {
                        _token: String::new(),
                        family: Some(r.family.clone()),
                        client_id: r.client_id.clone(),
                        created_at: Instant::now(),
                        expires_in: Duration::from_secs(left),
                        // Never a browser session, so never the web app's (ADR-0050).
                        user: r.user_id.as_ref().map(|id| TokenUser {
                            id: id.clone(),
                            session: None,
                            last_used: Instant::now(),
                        }),
                    },
                );
            }
            self.refresh.insert(
                r.token_hash,
                RefreshToken {
                    family: r.family,
                    seq: r.seq,
                    client_id: r.client_id,
                    user: r.user_id,
                    created_at: r.created_at,
                    spent_at: r.spent_at,
                    access_hash: r.access_hash,
                },
            );
        }
        self.expire_refresh(now);
        self.refresh
            .values()
            .filter(|t| t.spent_at.is_none() && t.user.is_some() == sign_in)
            .count()
    }

    /// The first token of a new family, for a code that went to `redirect_uri`. `None`
    /// for the web app's own client, in either sign-in mode: it takes a token per page
    /// load and must stay bound to its browser session (ADR-0050). With sign-in off a
    /// client id is all it takes to authorize, so a client at its limit gets no new
    /// family rather than losing one in use to whoever asks (ADR-0036).
    pub(super) fn plan_family(
        &mut self,
        client_id: &str,
        redirect_uri: &str,
        user: Option<&str>,
        access: &str,
    ) -> Option<Rotation> {
        if signin::is_first_party(&base_url(), redirect_uri) {
            return None;
        }
        let now = now_unix();
        self.expire_refresh(now);
        let held = self
            .refresh
            .values()
            .filter(|t| t.spent_at.is_none() && t.client_id == client_id && t.user.is_none())
            .count();
        if user.is_none() && held >= MAX_TOKENS_PER_CLIENT {
            return None;
        }
        let family_part = random_token();
        let value = format!("{family_part}.{}", random_token());
        Some(Rotation {
            hash: hash_of(&value),
            value,
            access: access.to_string(),
            spent: Vec::new(),
            spent_at: now,
            token: RefreshToken {
                family: hash_of(&family_part),
                seq: 0,
                client_id: client_id.to_string(),
                user: user.map(str::to_string),
                created_at: now,
                spent_at: None,
                access_hash: Some(hash_of(access)),
            },
        })
    }

    /// Check a presented refresh token and prepare its successor. Changes nothing.
    fn plan_rotation(
        &mut self,
        presented: &str,
        client_id: &str,
        now: i64,
    ) -> Result<Rotation, Refused> {
        self.expire_refresh(now);
        let sign_in = self.signin.is_some();
        let (family_part, family) = family_of(presented).ok_or(Refused::Invalid)?;
        // This client's, and issued under this sign-in mode: with sign-in on a token's
        // owner is its user and never its client (ADR-0044).
        let ours = |t: &RefreshToken| {
            t.family == family && t.client_id == client_id && t.user.is_some() == sign_in
        };
        if !self.clients.contains_key(client_id) {
            return Err(Refused::Invalid);
        }
        let Some(t) = self.refresh.get(&hash_of(presented)).filter(|t| ours(t)) else {
            // No row for it. If it names a family of this client that is in use, it is
            // a token of that family from before the spent ones that are kept, and it
            // came back: the family ends, as for a spent token that is still known.
            let in_use = self
                .refresh
                .values()
                .any(|t| ours(t) && t.spent_at.is_none());
            return Err(if in_use {
                Refused::Reused(family)
            } else {
                Refused::Invalid
            });
        };
        let t = t.clone();
        let live = self
            .refresh
            .iter()
            .find(|(_, l)| l.family == t.family && l.spent_at.is_none())
            .map(|(hash, l)| (hash.clone(), l.seq));
        let (spent, spent_at) = match (t.spent_at, live) {
            (None, _) => (vec![hash_of(presented)], now),
            // A retry by the holder of the token exchanged last: the successor it
            // never got is replaced, and marked so that it can never retry in turn.
            (Some(at), Some((successor, seq)))
                if now - at <= RETRY_WINDOW_SECS && seq == t.seq + 1 =>
            {
                (vec![successor], REPLACED)
            }
            _ => return Err(Refused::Reused(t.family)),
        };
        let value = format!("{family_part}.{}", random_token());
        let access = Uuid::new_v4().to_string();
        Ok(Rotation {
            hash: hash_of(&value),
            value,
            spent,
            spent_at,
            token: RefreshToken {
                seq: t.seq + 1,
                created_at: now,
                spent_at: None,
                access_hash: Some(hash_of(&access)),
                ..t
            },
            access,
        })
    }

    /// Take a stored rotation into the families. `false` if the family ended or its
    /// client was evicted while the store was written.
    pub(super) fn apply_rotation(&mut self, r: &Rotation) -> bool {
        let unspent = |h: &String| self.refresh.get(h).is_some_and(|t| t.spent_at.is_none());
        if !self.clients.contains_key(&r.token.client_id) || !r.spent.iter().all(unspent) {
            return false;
        }
        for hash in &r.spent {
            if let Some(t) = self.refresh.get_mut(hash) {
                t.spent_at = Some(r.spent_at);
            }
        }
        let floor = r.token.seq - MAX_SPENT_PER_FAMILY;
        self.refresh
            .retain(|_, t| t.family != r.token.family || t.spent_at.is_none() || t.seq >= floor);
        self.refresh.insert(r.hash.clone(), r.token.clone());
        true
    }
}

/// Store the first token of a family and tie `access_token` to it. `Ok(None)` if the
/// client was evicted meanwhile, or the user's limits ended the family at once.
pub(super) async fn start_family(
    state: &SharedState,
    access_token: &str,
    first: Rotation,
    write: &WriteFn,
) -> Result<Option<String>, String> {
    write_off_worker(write, first.change()).await?;
    let (kept, ended) = {
        let mut s = state.oauth.lock().unwrap();
        let kept = s.apply_rotation(&first);
        if kept {
            if let Some(t) = s.tokens.get_mut(access_token) {
                t.family = Some(first.token.family.clone());
            }
        }
        let ended = match (&first.token.user, kept) {
            (Some(user), true) => s.trim_user_tokens(user),
            (_, true) => Vec::new(),
            (_, false) => vec![first.token.family.clone()],
        };
        (kept && !ended.contains(&first.token.family), ended)
    };
    forget_families(write, ended).await;
    Ok(kept.then_some(first.value))
}

fn invalid_grant() -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"error": "invalid_grant", "error_description": "Invalid or expired refresh token"})),
    )
        .into_response()
}

fn store_failed() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({"error": "server_error", "error_description": "The token store is unavailable. Try again."})),
    )
        .into_response()
}

/// `POST /token` with `grant_type=refresh_token`: a new access token and the next
/// refresh token. The successor is in the store before the answer leaves: answered
/// first, a failed write would leave the client holding a token that the next start
/// does not know. When the write fails nothing is issued and the presented token
/// stays valid.
pub(super) async fn grant(
    state: &SharedState,
    presented: &str,
    client_id: &str,
    write: &WriteFn,
) -> Response {
    let _gate = GATE.lock().await;
    let planned = state
        .oauth
        .lock()
        .unwrap()
        .plan_rotation(presented, client_id, now_unix());
    let rotation = match planned {
        Ok(rotation) => rotation,
        Err(Refused::Invalid) => return invalid_grant(),
        Err(Refused::Reused(family)) => {
            let families = vec![family];
            if let Err(e) = write_off_worker(write, Change::Delete(families.clone())).await {
                eprintln!("[OAuth] could not revoke a refresh token used twice: {e}");
                return store_failed();
            }
            state.oauth.lock().unwrap().end_families(&families);
            eprintln!("[OAuth] a spent refresh token was used again: its authorization is revoked");
            return invalid_grant();
        }
    };
    if let Err(e) = write_off_worker(write, rotation.change()).await {
        eprintln!("[OAuth] could not store a refresh token: {e}");
        return store_failed();
    }
    let token = {
        let mut s = state.oauth.lock().unwrap();
        s.apply_rotation(&rotation).then(|| {
            let user = rotation.token.user.as_ref().map(|id| TokenUser {
                id: id.clone(),
                session: None,
                last_used: Instant::now(),
            });
            let family = Some(rotation.token.family.clone());
            let token = s.issue_access(rotation.access.clone(), client_id, user, family);
            // The user's limits count what a refresh adds, as they do a code exchange.
            let ended = rotation.token.user.as_ref().map(|u| s.trim_user_tokens(u));
            (token, ended.unwrap_or_default())
        })
    };
    let Some((token, ended)) = token else {
        forget_families(write, vec![rotation.token.family]).await;
        return invalid_grant();
    };
    forget_families(write, ended).await;
    token_issued(client_id.to_string(), rotation.token.user).await;
    eprintln!("[OAuth] POST /token SUCCESS — token refreshed");
    Json(json!({
        "access_token": token,
        "token_type": "Bearer",
        "expires_in": ACCESS_TOKEN_SECS,
        "refresh_token": rotation.value,
    }))
    .into_response()
}

/// `POST /revoke` (RFC 7009), as form or JSON: `token` and `client_id`.
pub(super) async fn revoke_endpoint(
    State(state): State<SharedState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let json = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.contains("application/json"));
    let body: Value = if json {
        serde_json::from_slice(&body).unwrap_or(Value::Null)
    } else {
        form_to_json(&String::from_utf8_lossy(&body))
    };
    let (Some(token), Some(client_id)) = (body["token"].as_str(), body["client_id"].as_str())
    else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid_request", "error_description": "token and client_id are required"})),
        )
            .into_response();
    };
    revoke(&state, token, client_id, &store_write()).await
}

/// End what `token` stands for: an access token, and for a refresh token or an access
/// token that came from one, the whole family with its access tokens. The answer is
/// `200` whether or not the token was known, and a token of another client is left
/// alone, so the endpoint tells nobody which tokens exist. `503` when the store could
/// not be written: the family then still works, here and after a restart.
pub(super) async fn revoke(
    state: &SharedState,
    token: &str,
    client_id: &str,
    write: &WriteFn,
) -> Response {
    let _gate = GATE.lock().await;
    let family = {
        let mut s = state.oauth.lock().unwrap();
        s.adopt(token);
        let of_client = |c: &str| c == client_id;
        match s.refresh.get(&hash_of(token)) {
            Some(t) => of_client(&t.client_id).then(|| t.family.clone()),
            None => match s.tokens.get(token) {
                Some(t) => of_client(&t.client_id).then(|| t.family.clone()).flatten(),
                // A refresh token too old to have a row still names its family.
                None => family_of(token).map(|f| f.1).filter(|family| {
                    let mut tokens = s.refresh.values();
                    tokens.any(|t| &t.family == family && of_client(&t.client_id))
                }),
            },
        }
    };
    let families: Vec<String> = family.into_iter().collect();
    if !families.is_empty() {
        if let Err(e) = write_off_worker(write, Change::Delete(families.clone())).await {
            eprintln!("[OAuth] could not revoke a refresh token: {e}");
            return store_failed();
        }
    }
    let mut s = state.oauth.lock().unwrap();
    s.end_families(&families);
    if s.tokens
        .get(token)
        .is_some_and(|t| t.client_id == client_id)
    {
        s.tokens.remove(token);
    }
    StatusCode::OK.into_response()
}

#[cfg(all(test, feature = "scripting"))]
mod tests {
    use super::*;
    use http_body_util::BodyExt;
    use scripting::persistence::Store;

    const CLIENT: &str = "c1";
    const REDIRECT: &str = "https://client.example/cb";

    fn temp_store(name: &str) -> (Arc<Store>, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("chartgen-refresh-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("s.db");
        (Arc::new(Store::open(&path).unwrap()), path)
    }

    fn to(store: &Arc<Store>) -> WriteFn {
        let store = store.clone();
        Arc::new(move |change| write_to(&store, change))
    }

    fn failing() -> WriteFn {
        Arc::new(|_| Err("disk full".to_string()))
    }

    fn sign_in() -> signin::SignIn {
        let vars = [
            ("CHARTGEN_GITHUB_CLIENT_ID", "h"),
            ("CHARTGEN_GITHUB_CLIENT_SECRET", "hs"),
        ];
        let config = signin::Config::from_vars(|name| {
            vars.iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.to_string())
        });
        signin::SignIn::new(config.unwrap().unwrap())
    }

    /// Server state with `clients` registered, as a start of the process builds it:
    /// nothing in memory but what `restore_refresh` reads from the store.
    fn started(store: &Store, with_sign_in: bool, clients: &[&str]) -> SharedState {
        let mut oauth = OAuthStore::new();
        oauth.signin = with_sign_in.then(sign_in);
        for id in clients {
            oauth.clients.insert(
                id.to_string(),
                OAuthClient {
                    _client_secret: None,
                    redirect_uris: vec![REDIRECT.to_string()],
                    created_at: Instant::now(),
                    last_used: None,
                    name: None,
                },
            );
        }
        oauth.restore_refresh(store.refresh_tokens(now_unix() - REFRESH_TTL_SECS).unwrap());
        Arc::new(AppState {
            oauth: Mutex::new(oauth),
            engine: None,
            charts: ChartHub::new(
                Arc::new(crate::chart_push::SampleSource),
                crate::chart_push::Limits::default(),
            ),
        })
    }

    /// What `/token` does once a code is accepted: `(access token, refresh token)`.
    async fn authorize(
        state: &SharedState,
        client: &str,
        user: Option<&str>,
        write: &WriteFn,
    ) -> (String, Option<String>) {
        let (access, family) = {
            let mut s = state.oauth.lock().unwrap();
            let token_user = user.map(|id| TokenUser {
                id: id.to_string(),
                session: None,
                last_used: Instant::now(),
            });
            let access = s.issue_access(Uuid::new_v4().to_string(), client, token_user, None);
            let family = s.plan_family(client, REDIRECT, user, &access);
            (access, family)
        };
        let refresh = match family {
            Some(family) => start_family(state, &access, family, write)
                .await
                .unwrap_or(None),
            None => None,
        };
        (access, refresh)
    }

    async fn answer(response: Response) -> (StatusCode, Value) {
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    /// Exchange `token`; the new `(access token, refresh token)` when it is accepted.
    async fn exchange(
        state: &SharedState,
        token: &str,
        client: &str,
        write: &WriteFn,
    ) -> Result<(String, String), StatusCode> {
        let (status, body) = answer(grant(state, token, client, write).await).await;
        if status != StatusCode::OK {
            return Err(status);
        }
        Ok((
            body["access_token"].as_str().unwrap().to_string(),
            body["refresh_token"].as_str().unwrap().to_string(),
        ))
    }

    fn valid(state: &SharedState, access: &str) -> bool {
        state.oauth.lock().unwrap().validate_token(access)
    }

    fn owner(state: &SharedState, access: &str) -> Option<String> {
        state.oauth.lock().unwrap().owner_for_token(access)
    }

    /// The brief's case: authorized once, then the process is replaced. The client's
    /// access token is known again, so it sees no `401` and needs no exchange, and the
    /// refresh token works when the 7 days are over.
    #[tokio::test]
    async fn an_authorization_works_after_a_restart_with_no_exchange() {
        for (with_sign_in, user) in [(true, Some("github:1")), (false, None)] {
            let (store, _) = temp_store(&format!("restart-{with_sign_in}"));
            let before = started(&store, with_sign_in, &[CLIENT]);
            let (first_access, refresh) = authorize(&before, CLIENT, user, &to(&store)).await;
            let refresh = refresh.unwrap();
            assert!(valid(&before, &first_access));
            let was = owner(&before, &first_access);

            let after = started(&store, with_sign_in, &[CLIENT]);
            assert!(after.oauth.lock().unwrap().tokens.is_empty());
            assert!(valid(&after, &first_access), "the same access token");
            assert_eq!(owner(&after, &first_access), was, "the same owner");
            assert_eq!(
                after
                    .oauth
                    .lock()
                    .unwrap()
                    .app_user_for_token(&first_access),
                None,
                "and never the web app's (ADR-0050)"
            );
            let (access, next) = exchange(&after, &refresh, CLIENT, &to(&store))
                .await
                .unwrap();
            assert_eq!(owner(&after, &access), was);
            assert_ne!(next, refresh);

            // After the next one: the newest pair, and not the access token before it.
            let later = started(&store, with_sign_in, &[CLIENT]);
            assert!(valid(&later, &access));
            assert!(!valid(&later, &first_access));
            assert!(exchange(&later, &next, CLIENT, &to(&store)).await.is_ok());
        }
    }

    /// The 7 days of an access token count from when it was issued.
    #[tokio::test]
    async fn a_restart_adds_nothing_to_an_access_tokens_lifetime() {
        let (store, path) = temp_store("lifetime");
        let state = started(&store, false, &[CLIENT]);
        let (access, refresh) = authorize(&state, CLIENT, None, &to(&store)).await;
        let issued_ago = |secs: u64| {
            rusqlite::Connection::open(&path)
                .unwrap()
                .execute(
                    "UPDATE refresh_tokens SET created_at = ?1",
                    [now_unix() - secs as i64],
                )
                .unwrap();
        };
        let day = 24 * 3600;
        issued_ago(6 * day);
        let state = started(&store, false, &[CLIENT]);
        assert!(valid(&state, &access));
        let left = state.oauth.lock().unwrap().tokens[&access].expires_in;
        assert!(left <= Duration::from_secs(day), "{left:?}");

        issued_ago(ACCESS_TOKEN_SECS + 1);
        let state = started(&store, false, &[CLIENT]);
        assert!(!valid(&state, &access));
        // Which is when the client refreshes.
        assert!(exchange(&state, &refresh.unwrap(), CLIENT, &to(&store))
            .await
            .is_ok());
    }

    /// A refresh counts in the user's limits like a code exchange, and so does an
    /// access token from before a restart.
    #[tokio::test]
    async fn refreshing_and_restarting_add_nothing_to_a_users_token_count() {
        let (store, _) = temp_store("count");
        let state = started(&store, true, &[CLIENT]);
        let write = to(&store);
        let held = |state: &SharedState| {
            let s = state.oauth.lock().unwrap();
            s.tokens.len() + s.restored.len()
        };
        let (_, refresh) = authorize(&state, CLIENT, Some("github:1"), &write).await;
        let mut refresh = refresh.unwrap();
        for _ in 0..MAX_TOKENS_PER_CLIENT + 5 {
            refresh = exchange(&state, &refresh, CLIENT, &write).await.unwrap().1;
        }
        assert_eq!(held(&state), MAX_TOKENS_PER_CLIENT);

        for _ in 1..MAX_TOKENS_PER_CLIENT {
            authorize(&state, CLIENT, Some("github:1"), &write).await;
        }
        let state = started(&store, true, &[CLIENT]);
        assert_eq!(held(&state), MAX_TOKENS_PER_CLIENT, "one per family");
        let (access, _) = authorize(&state, CLIENT, Some("github:1"), &write).await;
        assert_eq!(held(&state), MAX_TOKENS_PER_CLIENT);
        assert!(valid(&state, &access), "the new one is not the one dropped");
    }

    #[tokio::test]
    async fn only_the_hash_of_a_refresh_token_is_in_the_store() {
        let (store, path) = temp_store("hashes");
        let state = started(&store, false, &[CLIENT]);
        let (_, first) = authorize(&state, CLIENT, None, &to(&store)).await;
        let first = first.unwrap();
        let (access, second) = exchange(&state, &first, CLIENT, &to(&store)).await.unwrap();
        let rows = store.refresh_tokens(0).unwrap();
        assert_eq!(rows.len(), 2, "the spent token and its successor");
        let of_access: Vec<_> = rows.iter().filter_map(|r| r.access_hash.clone()).collect();
        assert_eq!(
            of_access,
            [hash_of(&access)],
            "the current access token, hashed"
        );
        let hashes: Vec<&str> = rows.iter().map(|r| r.token_hash.as_str()).collect();
        assert!(hashes.contains(&hash_of(&first).as_str()));
        assert!(hashes.contains(&hash_of(&second).as_str()));
        drop(store);
        // Nowhere in the file or beside it, in any column or journal.
        for entry in std::fs::read_dir(path.parent().unwrap()).unwrap() {
            let bytes = std::fs::read(entry.unwrap().path()).unwrap();
            for value in [&first, &second, &access] {
                assert!(
                    !bytes.windows(value.len()).any(|w| w == value.as_bytes()),
                    "a token value is on disk"
                );
            }
        }
    }

    #[tokio::test]
    async fn a_spent_refresh_token_ends_its_family_even_after_a_restart() {
        // Whether or not the access token from before the restart was used since.
        for used_since in [true, false] {
            let (store, _) = temp_store(&format!("reuse-{used_since}"));
            let write = to(&store);
            let state = started(&store, false, &[CLIENT]);
            let (_, first) = authorize(&state, CLIENT, None, &write).await;
            let first = first.unwrap();
            let (_, second) = exchange(&state, &first, CLIENT, &write).await.unwrap();
            let (access, third) = exchange(&state, &second, CLIENT, &write).await.unwrap();

            // Two exchanges back, so no retry: whoever sends it copied it.
            let state = started(&store, false, &[CLIENT]);
            if used_since {
                assert!(valid(&state, &access));
            }
            assert_eq!(
                exchange(&state, &first, CLIENT, &write).await,
                Err(StatusCode::BAD_REQUEST)
            );
            assert!(!valid(&state, &access), "its access token ends too");
            assert!(state.oauth.lock().unwrap().restored.is_empty());
            assert_eq!(
                exchange(&state, &third, CLIENT, &write).await,
                Err(StatusCode::BAD_REQUEST),
                "the newest token of the family is dead"
            );
            assert!(store.refresh_tokens(0).unwrap().is_empty());
            let state = started(&store, false, &[CLIENT]);
            assert!(exchange(&state, &third, CLIENT, &write).await.is_err());
            assert!(!valid(&state, &access));
        }
    }

    /// A lost answer must not cost the client its authorization, and the retry must
    /// not leave two tokens that both work.
    #[tokio::test]
    async fn a_retry_replaces_the_successor_and_never_leaves_two() {
        let (store, _) = temp_store("retry");
        let state = started(&store, false, &[CLIENT]);
        let write = to(&store);
        let (_, first) = authorize(&state, CLIENT, None, &write).await;
        let first = first.unwrap();
        let (_, lost) = exchange(&state, &first, CLIENT, &write).await.unwrap();
        let (_, second) = exchange(&state, &first, CLIENT, &write).await.unwrap();
        assert_ne!(lost, second);
        let live = |s: &SharedState| {
            let s = s.oauth.lock().unwrap();
            s.refresh.values().filter(|t| t.spent_at.is_none()).count()
        };
        assert_eq!(live(&state), 1);
        assert_eq!(live(&started(&store, false, &[CLIENT])), 1, "on disk too");

        // Whoever holds the replaced one is found out by using it.
        assert!(exchange(&state, &lost, CLIENT, &write).await.is_err());
        assert!(exchange(&state, &second, CLIENT, &write).await.is_err());
        assert_eq!(live(&state), 0);
    }

    /// Only the newest spent tokens of a family are kept. One older than that used to
    /// be merely unknown: whoever held a copy of the usable token could exchange it
    /// 101 times, and the owner's token then ended nothing, while the copy lived on.
    #[tokio::test]
    async fn a_token_too_old_to_be_remembered_still_ends_its_family() {
        for restart in [false, true] {
            let (store, _) = temp_store(&format!("forgotten-{restart}"));
            let mut state = started(&store, false, &[CLIENT]);
            let write = to(&store);
            let (_, owners) = authorize(&state, CLIENT, None, &write).await;
            let owners = owners.unwrap();
            // The holder of a copy runs ahead.
            let (mut access, mut copy) = (String::new(), owners.clone());
            for _ in 0..MAX_SPENT_PER_FAMILY + 2 {
                (access, copy) = exchange(&state, &copy, CLIENT, &write).await.unwrap();
            }
            if restart {
                state = started(&store, false, &[CLIENT]);
            }
            let known = |state: &SharedState, token: &str| {
                let s = state.oauth.lock().unwrap();
                s.refresh.contains_key(&hash_of(token))
            };
            assert!(!known(&state, &owners), "its row is gone");

            assert_eq!(
                exchange(&state, &owners, CLIENT, &write).await,
                Err(StatusCode::BAD_REQUEST)
            );
            assert!(
                !valid(&state, &access),
                "restart {restart}: the copy's access token"
            );
            assert!(exchange(&state, &copy, CLIENT, &write).await.is_err());
            assert!(store.refresh_tokens(0).unwrap().is_empty());
        }
    }

    /// Ending a family by a token the server no longer knows takes a token of that
    /// family, presented by its client. Nothing else does it: not a made-up value, not
    /// another client, and not what somebody could read out of the database.
    #[tokio::test]
    async fn only_a_token_of_the_family_and_its_client_ends_it() {
        let (store, _) = temp_store("family-key");
        let state = started(&store, false, &[CLIENT, "c2"]);
        let write = to(&store);
        let (_, mine) = authorize(&state, CLIENT, None, &write).await;
        let (_, theirs) = authorize(&state, "c2", None, &write).await;
        let (mine, theirs) = (mine.unwrap(), theirs.unwrap());
        let family_part = |token: &str| token.split_once('.').unwrap().0.to_string();

        let mut attempts = vec![
            // Names no family at all, or none that exists.
            ("made-up".to_string(), CLIENT),
            ("no-such-family.secret".to_string(), CLIENT),
            (".".to_string(), CLIENT),
            // Names a live family, of another client.
            (format!("{}.made-up", family_part(&theirs)), CLIENT),
            (format!("{}.made-up", family_part(&mine)), "c2"),
        ];
        // Everything the database holds, alone and combined, with the right client.
        for row in store.refresh_tokens(0).unwrap() {
            let client = if row.client_id == CLIENT {
                CLIENT
            } else {
                "c2"
            };
            let access = row.access_hash.clone().unwrap();
            for value in [
                row.family.clone(),
                row.token_hash.clone(),
                format!("{}.{}", row.family, row.token_hash),
                format!("{}.{}", row.family, access),
                format!("{}.x", row.token_hash),
            ] {
                attempts.push((value, client));
            }
        }
        for (value, client) in &attempts {
            assert_eq!(
                exchange(&state, value, client, &write).await,
                Err(StatusCode::BAD_REQUEST),
                "{value}"
            );
            let answer = revoke(&state, value, client, &write).await;
            assert_eq!(answer.status(), StatusCode::OK);
        }
        assert_eq!(store.refresh_tokens(0).unwrap().len(), 2, "nothing ended");
        assert!(exchange(&state, &mine, CLIENT, &write).await.is_ok());

        // A former holder can: the family part of any token of it, with its client,
        // by presenting it or by revoking it.
        let stale = format!("{}.forgotten", family_part(&theirs));
        assert!(exchange(&state, &stale, "c2", &write).await.is_err());
        assert!(exchange(&state, &theirs, "c2", &write).await.is_err());
        let (_, again) = authorize(&state, "c2", None, &write).await;
        let again = again.unwrap();
        let stale = format!("{}.forgotten", family_part(&again));
        assert_eq!(
            revoke(&state, &stale, "c2", &write).await.status(),
            StatusCode::OK
        );
        assert!(exchange(&state, &again, "c2", &write).await.is_err());
    }

    /// A successor that a retry replaced was never exchanged. It must not pass for a
    /// token whose own exchange was lost: using it ends the family, at once, also
    /// when one exchange has happened since and also after a restart.
    #[tokio::test]
    async fn a_replaced_successor_ends_the_family_when_it_is_used() {
        for restart in [false, true] {
            let (store, _) = temp_store(&format!("replaced-{restart}"));
            let mut state = started(&store, false, &[CLIENT]);
            let write = to(&store);
            let (_, first) = authorize(&state, CLIENT, None, &write).await;
            let first = first.unwrap();
            let (_, replaced) = exchange(&state, &first, CLIENT, &write).await.unwrap();
            let (_, successor) = exchange(&state, &first, CLIENT, &write).await.unwrap();
            if restart {
                state = started(&store, false, &[CLIENT]);
            }
            let (access, next) = exchange(&state, &successor, CLIENT, &write).await.unwrap();

            assert_eq!(
                exchange(&state, &replaced, CLIENT, &write).await,
                Err(StatusCode::BAD_REQUEST),
                "restart: {restart}"
            );
            assert!(!valid(&state, &access), "the family ended");
            assert!(exchange(&state, &next, CLIENT, &write).await.is_err());
            assert!(store.refresh_tokens(0).unwrap().is_empty());
        }
    }

    /// A client may lose more than one answer. Each retry leaves one usable token, and
    /// every successor it replaced ends the family when used.
    #[tokio::test]
    async fn any_number_of_retries_leaves_one_usable_token() {
        for stale in [0, 1, 2] {
            let (store, _) = temp_store(&format!("retries-{stale}"));
            let state = started(&store, false, &[CLIENT]);
            let write = to(&store);
            let (_, first) = authorize(&state, CLIENT, None, &write).await;
            let first = first.unwrap();
            let mut successors = Vec::new();
            for _ in 0..3 {
                successors.push(exchange(&state, &first, CLIENT, &write).await.unwrap().1);
            }
            let usable = |state: &SharedState| {
                let s = state.oauth.lock().unwrap();
                s.refresh.values().filter(|t| t.spent_at.is_none()).count()
            };
            assert_eq!(usable(&state), 1);
            if stale == 2 {
                // The last one is the usable one.
                assert!(exchange(&state, &successors[2], CLIENT, &write)
                    .await
                    .is_ok());
            } else {
                assert!(exchange(&state, &successors[stale], CLIENT, &write)
                    .await
                    .is_err());
                assert!(exchange(&state, &successors[2], CLIENT, &write)
                    .await
                    .is_err());
                assert_eq!(usable(&state), 0);
            }
        }
    }

    #[tokio::test]
    async fn a_retry_after_the_window_is_reuse() {
        let (store, _) = temp_store("window");
        let state = started(&store, false, &[CLIENT]);
        let write = to(&store);
        let (_, first) = authorize(&state, CLIENT, None, &write).await;
        let first = first.unwrap();
        let (_, second) = exchange(&state, &first, CLIENT, &write).await.unwrap();
        for t in state.oauth.lock().unwrap().refresh.values_mut() {
            t.spent_at = t.spent_at.map(|at| at - RETRY_WINDOW_SECS - 1);
        }
        assert!(exchange(&state, &first, CLIENT, &write).await.is_err());
        assert!(exchange(&state, &second, CLIENT, &write).await.is_err());
    }

    #[tokio::test]
    async fn a_failed_store_write_issues_nothing_and_keeps_the_presented_token() {
        let (store, _) = temp_store("failed-write");
        let state = started(&store, false, &[CLIENT]);
        let (_, first) = authorize(&state, CLIENT, None, &to(&store)).await;
        let first = first.unwrap();
        let tokens = |s: &SharedState| {
            let s = s.oauth.lock().unwrap();
            (s.tokens.len(), s.refresh.len())
        };
        let before = tokens(&state);

        assert_eq!(
            exchange(&state, &first, CLIENT, &failing()).await,
            Err(StatusCode::SERVICE_UNAVAILABLE)
        );
        assert_eq!(tokens(&state), before, "no access token and no successor");
        assert_eq!(store.refresh_tokens(0).unwrap().len(), 1);
        // The same token works once the store does, here and after a restart.
        let state = started(&store, false, &[CLIENT]);
        assert!(exchange(&state, &first, CLIENT, &to(&store)).await.is_ok());

        // A first token that could not be stored is not handed out.
        let (access, refresh) = authorize(&state, CLIENT, None, &failing()).await;
        assert!(refresh.is_none());
        assert!(valid(&state, &access), "the access token alone, as before");
    }

    #[tokio::test]
    async fn revoking_ends_the_family_now_and_after_a_restart() {
        let (store, _) = temp_store("revoke");
        let state = started(&store, false, &[CLIENT, "c2"]);
        let write = to(&store);
        let ok = |r: Response| assert_eq!(r.status(), StatusCode::OK);
        let (access, refresh) = authorize(&state, CLIENT, None, &write).await;
        let refresh = refresh.unwrap();

        // Unknown tokens and another client's answer the same, and change nothing.
        ok(revoke(&state, "no-such-token", CLIENT, &write).await);
        ok(revoke(&state, &refresh, "c2", &write).await);
        ok(revoke(&state, &access, "c2", &write).await);
        assert!(valid(&state, &access));

        // The store must have it first: otherwise a restart would bring it back.
        let failed = revoke(&state, &refresh, CLIENT, &failing()).await;
        assert_eq!(failed.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(valid(&state, &access));

        ok(revoke(&state, &refresh, CLIENT, &write).await);
        assert!(!valid(&state, &access), "its access tokens end with it");
        assert!(exchange(&state, &refresh, CLIENT, &write).await.is_err());
        let state = started(&store, false, &[CLIENT, "c2"]);
        assert!(!valid(&state, &access), "and stay ended after a restart");
        assert!(exchange(&state, &refresh, CLIENT, &write).await.is_err());

        // By the access token: the family behind it goes too.
        let (access, refresh) = authorize(&state, CLIENT, None, &write).await;
        ok(revoke(&state, &access, CLIENT, &write).await);
        assert!(!valid(&state, &access));
        assert!(exchange(&state, &refresh.unwrap(), CLIENT, &write)
            .await
            .is_err());

        // A restart no longer ends an access token, so revoking must: by the refresh
        // token, with the access token never used since.
        let (access, refresh) = authorize(&state, CLIENT, None, &write).await;
        let state = started(&store, false, &[CLIENT, "c2"]);
        ok(revoke(&state, &refresh.unwrap(), CLIENT, &write).await);
        assert!(!valid(&state, &access));
        // And by the access token itself, which the server knows by its hash only.
        let (access, refresh) = authorize(&state, CLIENT, None, &write).await;
        let state = started(&store, false, &[CLIENT, "c2"]);
        ok(revoke(&state, &access, "c2", &write).await);
        ok(revoke(&state, &access, CLIENT, &write).await);
        assert!(!valid(&state, &access));
        assert!(exchange(&state, &refresh.unwrap(), CLIENT, &write)
            .await
            .is_err());
        assert!(store.refresh_tokens(0).unwrap().is_empty());
    }

    /// The operator's way, for a token nobody holds any more: delete the rows, restart.
    #[tokio::test]
    async fn rows_deleted_by_an_operator_are_gone_after_a_restart() {
        let (store, path) = temp_store("operator");
        let state = started(&store, true, &[CLIENT]);
        let write = to(&store);
        let (my_access, mine) = authorize(&state, CLIENT, Some("github:1"), &write).await;
        let (their_access, theirs) = authorize(&state, CLIENT, Some("github:2"), &write).await;
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute(
                "DELETE FROM refresh_tokens WHERE user_id = ?1",
                ["github:1"],
            )
            .unwrap();
        let state = started(&store, true, &[CLIENT]);
        assert!(
            !valid(&state, &my_access),
            "the access token goes with the rows"
        );
        assert!(valid(&state, &their_access));
        assert!(exchange(&state, &mine.unwrap(), CLIENT, &write)
            .await
            .is_err());
        assert!(exchange(&state, &theirs.unwrap(), CLIENT, &write)
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn the_web_app_gets_no_refresh_token_in_either_mode() {
        let (store, _) = temp_store("first-party");
        let own_root = format!("{}/", base_url());
        for (with_sign_in, user) in [(true, Some("github:1")), (false, None)] {
            let state = started(&store, with_sign_in, &[CLIENT]);
            let mut s = state.oauth.lock().unwrap();
            assert!(s.plan_family(CLIENT, &own_root, user, "a").is_none());
            assert!(s.plan_family(CLIENT, REDIRECT, user, "a").is_some());
        }
    }

    #[tokio::test]
    async fn a_refreshed_token_is_its_users_and_never_first_party() {
        let (store, _) = temp_store("bound");
        let state = started(&store, true, &[CLIENT, "c2"]);
        let write = to(&store);
        let (_, refresh) = authorize(&state, CLIENT, Some("github:1"), &write).await;
        let refresh = refresh.unwrap();

        // Another client cannot use it, and trying does not cost the owner anything.
        assert!(exchange(&state, &refresh, "c2", &write).await.is_err());
        let (access, _) = exchange(&state, &refresh, CLIENT, &write).await.unwrap();
        assert_eq!(owner(&state, &access).as_deref(), Some("github:1"));
        let s = state.oauth.lock().unwrap();
        assert_eq!(
            s.app_user_for_token(&access),
            None,
            "the workspace is for the web app's own token only (ADR-0050)"
        );
    }

    /// With sign-in on a token's owner is its user and never its client (ADR-0044), so
    /// a token from the other mode yields nothing. It is kept for switching back.
    #[tokio::test]
    async fn a_refresh_token_from_the_other_sign_in_mode_is_refused() {
        let (store, _) = temp_store("mode");
        let write = to(&store);
        let off = started(&store, false, &[CLIENT]);
        let (_, clientless) = authorize(&off, CLIENT, None, &write).await;
        let clientless = clientless.unwrap();
        let on = started(&store, true, &[CLIENT]);
        let (users_access, users) = authorize(&on, CLIENT, Some("github:1"), &write).await;
        let users = users.unwrap();

        assert!(exchange(&on, &clientless, CLIENT, &write).await.is_err());
        assert!(on.oauth.lock().unwrap().tokens.len() == 1, "nothing issued");
        let off = started(&store, false, &[CLIENT]);
        assert!(!valid(&off, &users_access), "nor is its access token known");
        assert!(exchange(&off, &users, CLIENT, &write).await.is_err());
        assert!(exchange(&off, &clientless, CLIENT, &write).await.is_ok());
        let on = started(&store, true, &[CLIENT]);
        assert_eq!(owner(&on, &users_access).as_deref(), Some("github:1"));
        assert!(exchange(&on, &users, CLIENT, &write).await.is_ok());
    }

    /// With sign-in off anyone holding a client id can authorize, so the limit refuses
    /// the newcomer instead of ending a family in use (ADR-0036).
    #[tokio::test]
    async fn without_sign_in_a_client_at_its_limit_gets_no_new_family() {
        let (store, _) = temp_store("limit-off");
        let state = started(&store, false, &[CLIENT]);
        let write = to(&store);
        let mut held = Vec::new();
        for _ in 0..MAX_TOKENS_PER_CLIENT {
            held.push(authorize(&state, CLIENT, None, &write).await.1.unwrap());
        }
        let (access, refresh) = authorize(&state, CLIENT, None, &write).await;
        assert!(refresh.is_none());
        assert!(valid(&state, &access));
        for token in held {
            assert!(exchange(&state, &token, CLIENT, &write).await.is_ok());
        }
    }

    #[tokio::test]
    async fn a_users_families_are_held_to_the_token_limits() {
        let (store, _) = temp_store("limit-on");
        let clients: Vec<String> = (0..=signin::MAX_LIVE_CLIENTS_PER_USER)
            .map(|i| format!("c{i}"))
            .collect();
        let ids: Vec<&str> = clients.iter().map(String::as_str).collect();
        let state = started(&store, true, &ids);
        let write = to(&store);
        let age = |state: &SharedState, secs: i64| {
            for t in state.oauth.lock().unwrap().refresh.values_mut() {
                t.created_at -= secs;
            }
        };

        // One more family than a client may hold: the longest out of use goes.
        let (_, oldest) = authorize(&state, "c0", Some("github:1"), &write).await;
        let mut newer = Vec::new();
        for _ in 0..MAX_TOKENS_PER_CLIENT {
            age(&state, 10);
            newer.push(authorize(&state, "c0", Some("github:1"), &write).await.1);
        }
        assert!(newer.iter().all(Option::is_some));
        assert!(exchange(&state, &oldest.unwrap(), "c0", &write)
            .await
            .is_err());
        let rows = store.refresh_tokens(0).unwrap();
        assert_eq!(rows.len(), MAX_TOKENS_PER_CLIENT, "and its row with it");
        // Another user's do not count.
        assert!(authorize(&state, "c0", Some("github:2"), &write)
            .await
            .1
            .is_some());

        // One more client than a user may keep live: the first client's families go.
        for id in &ids[1..] {
            age(&state, 10);
            assert!(authorize(&state, id, Some("github:1"), &write)
                .await
                .1
                .is_some());
        }
        let s = state.oauth.lock().unwrap();
        assert!(s.families_of("github:1", "c0").is_empty());
        assert_eq!(s.families_of("github:2", "c0").len(), 1);
        assert_eq!(s.families_of("github:1", "c1").len(), 1);
    }

    #[tokio::test]
    async fn a_family_nobody_used_for_30_days_ends() {
        let (store, path) = temp_store("expiry");
        let state = started(&store, false, &[CLIENT]);
        let write = to(&store);
        let (stale_access, stale) = authorize(&state, CLIENT, None, &write).await;
        let (_, fresh) = authorize(&state, CLIENT, None, &write).await;
        let stale_hash = hash_of(stale.as_ref().unwrap());
        let then = now_unix() - REFRESH_TTL_SECS - 1;
        state
            .oauth
            .lock()
            .unwrap()
            .refresh
            .get_mut(&stale_hash)
            .unwrap()
            .created_at = then;
        assert!(exchange(&state, &stale.clone().unwrap(), CLIENT, &write)
            .await
            .is_err());

        // At a start the rows go as well.
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute(
                "UPDATE refresh_tokens SET created_at = ?1 WHERE token_hash = ?2",
                rusqlite::params![then, stale_hash],
            )
            .unwrap();
        let state = started(&store, false, &[CLIENT]);
        assert_eq!(store.refresh_tokens(0).unwrap().len(), 1);
        assert_eq!(state.oauth.lock().unwrap().restored.len(), 1);
        assert!(!valid(&state, &stale_access));
        assert!(exchange(&state, &stale.unwrap(), CLIENT, &write)
            .await
            .is_err());
        assert!(exchange(&state, &fresh.unwrap(), CLIENT, &write)
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn a_token_of_a_client_no_longer_registered_is_refused() {
        let (store, _) = temp_store("evicted");
        let state = started(&store, false, &[CLIENT]);
        let write = to(&store);
        let (access, refresh) = authorize(&state, CLIENT, None, &write).await;
        let refresh = refresh.unwrap();
        state.oauth.lock().unwrap().clients.remove(CLIENT);
        assert!(exchange(&state, &refresh, CLIENT, &write).await.is_err());
        let state = started(&store, false, &[]);
        assert!(state.oauth.lock().unwrap().refresh.is_empty());
        assert!(!valid(&state, &access), "nor is its access token");
        assert!(exchange(&state, &refresh, CLIENT, &write).await.is_err());
    }
}
