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
/// How long the access token before a refresh stays valid after it. A request sent
/// with it just before the refresh may arrive just after, and a client answers a `401`
/// for a token it believes valid with a whole new authorization.
const PREVIOUS_ACCESS_SECS: u64 = 60;
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

/// Run `work` to its end, whether or not the request that asked for it is still
/// there. A handler is dropped at whatever it is waiting for when its client hangs
/// up, while a store write it started goes on without it. Dropped between that write
/// and the change in memory that follows, a request would leave the two apart: a
/// token spent in the store and usable in memory, a family revoked in the store and
/// working in memory. So what reads the families, writes the store and then changes
/// the families is a task of its own, and the request only waits for it. `None` if
/// the work panicked.
pub(super) async fn to_the_end<T: Send + 'static>(
    work: impl std::future::Future<Output = T> + Send + 'static,
) -> Option<T> {
    tokio::spawn(work)
        .await
        .map_err(|e| eprintln!("[OAuth] a token request did not finish: {e}"))
        .ok()
}

fn did_not_finish() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": "server_error"})),
    )
        .into_response()
}

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
    /// On a retry: the hash of the access token that went out with the replaced
    /// successor, in the answer the client never got.
    lost_access: Option<String>,
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

    /// After a refresh: a family keeps `newest` and, for `PREVIOUS_ACCESS_SECS` more,
    /// the access token before it. Any older one ends now, so a family never has more
    /// than two. After a retry the one before it is the token the client holds, not
    /// `lost`, the one from the answer it never got, which ends here.
    fn retire_access(&mut self, family: &str, newest: &str, lost: Option<&str>) {
        let of_family = |t: &AccessToken| t.family.as_deref() == Some(family);
        if let Some(lost) = lost {
            self.restored.remove(lost);
            self.tokens
                .retain(|token, t| !of_family(t) || hash_of(token) != lost);
        }
        let previous = self
            .tokens
            .iter()
            .chain(self.restored.iter())
            .filter(|(key, t)| of_family(t) && key.as_str() != newest)
            .max_by_key(|(_, t)| t.created_at)
            .map(|(key, _)| key.clone());
        let keeps = |key: &String| key == newest || Some(key) == previous.as_ref();
        self.tokens.retain(|key, t| !of_family(t) || keeps(key));
        self.restored.retain(|key, t| !of_family(t) || keeps(key));
        let previous = previous.and_then(|key| match self.tokens.get_mut(&key) {
            Some(t) => Some(t),
            None => self.restored.get_mut(&key),
        });
        if let Some(t) = previous {
            let until = t.created_at.elapsed() + Duration::from_secs(PREVIOUS_ACCESS_SECS);
            t.expires_in = t.expires_in.min(until);
        }
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
    ///
    /// The family is stored with the lock released, so `access` is made the family's
    /// here and now, before that. A token without a family is one the limits on
    /// tokens without a refresh token may drop, and other exchanges run meanwhile. For
    /// the same reason a family being stored already counts against the limit.
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
        let stored = self
            .refresh
            .values()
            .filter(|t| t.spent_at.is_none() && t.client_id == client_id && t.user.is_none())
            .map(|t| &t.family);
        let being_stored = self
            .tokens
            .values()
            .filter(|t| t.client_id == client_id && t.user.is_none())
            .filter_map(|t| t.family.as_ref());
        let held: HashSet<&String> = stored.chain(being_stored).collect();
        if user.is_none() && held.len() >= MAX_TOKENS_PER_CLIENT {
            return None;
        }
        let family_part = random_token();
        let family = hash_of(&family_part);
        if let Some(t) = self.tokens.get_mut(access) {
            t.family = Some(family.clone());
        }
        let value = format!("{family_part}.{}", random_token());
        Some(Rotation {
            hash: hash_of(&value),
            value,
            access: access.to_string(),
            spent: Vec::new(),
            spent_at: now,
            lost_access: None,
            token: RefreshToken {
                family,
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
            .map(|(hash, l)| (hash.clone(), l.seq, l.access_hash.clone()));
        let (spent, spent_at, lost_access) = match (t.spent_at, live) {
            (None, _) => (vec![hash_of(presented)], now, None),
            // A retry by the holder of the token exchanged last: the successor it
            // never got is replaced, and marked so that it can never retry in turn.
            (Some(at), Some((successor, seq, access)))
                if now - at <= RETRY_WINDOW_SECS && seq == t.seq + 1 =>
            {
                (vec![successor], REPLACED, access)
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
            lost_access,
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

/// Store the first token of a family, which `plan_family` made `access_token` part of.
/// `Ok(None)` if the access token is gone by then, the client was evicted, or the
/// user's limits ended the family at once: no refresh token is ever handed out beside
/// an access token that does not work, and nothing of the family stays. `Err` if it
/// could not be stored: the access token is then one without a family.
pub(super) async fn start_family(
    state: &SharedState,
    access_token: &str,
    first: Rotation,
    write: &WriteFn,
) -> Result<Option<String>, String> {
    let (state, access_token, write) = (state.clone(), access_token.to_string(), write.clone());
    to_the_end(async move { store_family(&state, &access_token, first, &write).await })
        .await
        .unwrap_or_else(|| Err("the work did not finish".to_string()))
}

async fn store_family(
    state: &SharedState,
    access_token: &str,
    first: Rotation,
    write: &WriteFn,
) -> Result<Option<String>, String> {
    let family = &first.token.family;
    let leave_family = |s: &mut OAuthStore| {
        if let Some(t) = s.tokens.get_mut(access_token) {
            t.family = None;
        }
    };
    if let Err(e) = write_off_worker(write, first.change()).await {
        leave_family(&mut state.oauth.lock().unwrap());
        return Err(e);
    }
    let (kept, ended) = {
        let mut s = state.oauth.lock().unwrap();
        let kept = s.tokens.contains_key(access_token) && s.apply_rotation(&first);
        let ended = match (&first.token.user, kept) {
            (Some(user), true) => s.trim_user_tokens(user, Some(family)),
            (_, true) => Vec::new(),
            (_, false) => {
                leave_family(&mut s);
                vec![family.clone()]
            }
        };
        (kept && !ended.contains(family), ended)
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
    let (state, write) = (state.clone(), write.clone());
    let (presented, client_id) = (presented.to_string(), client_id.to_string());
    to_the_end(async move { rotate(&state, &presented, &client_id, &write).await })
        .await
        .unwrap_or_else(did_not_finish)
}

async fn rotate(
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
            s.retire_access(
                &rotation.token.family,
                &token,
                rotation.lost_access.as_deref(),
            );
            // The user's limits count what a refresh adds, as they do a code exchange.
            let kept = Some(rotation.token.family.as_str());
            let ended = rotation
                .token
                .user
                .as_ref()
                .map(|u| s.trim_user_tokens(u, kept));
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
    let (state, write) = (state.clone(), write.clone());
    let (token, client_id) = (token.to_string(), client_id.to_string());
    to_the_end(async move { end_what_it_stands_for(&state, &token, &client_id, &write).await })
        .await
        .unwrap_or_else(did_not_finish)
}

async fn end_what_it_stands_for(
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
        assert_eq!(held(&state), 2, "the newest and the one before it");

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

    /// A client that lost the answer to an exchange still holds the access token from
    /// before it, and may have a request under way with it. After its retry that is the
    /// token to keep for the 60 seconds, not the one in the answer nobody received.
    #[tokio::test]
    async fn after_a_retry_the_access_token_the_client_holds_keeps_working() {
        let (store, _) = temp_store("retry-access");
        let state = started(&store, false, &[CLIENT]);
        let write = to(&store);
        let (held, first) = authorize(&state, CLIENT, None, &write).await;
        let first = first.unwrap();
        let (lost, _) = exchange(&state, &first, CLIENT, &write).await.unwrap();
        let (newest, _) = exchange(&state, &first, CLIENT, &write).await.unwrap();

        assert!(valid(&state, &newest));
        assert!(valid(&state, &held), "the one the client was using");
        assert!(
            !valid(&state, &lost),
            "the one from the answer that was lost"
        );
        // A second lost answer changes nothing for the one in hand.
        let (newest, _) = exchange(&state, &first, CLIENT, &write).await.unwrap();
        assert!(valid(&state, &newest) && valid(&state, &held));
        assert_eq!(state.oauth.lock().unwrap().tokens.len(), 2);

        // For 60 seconds, as after any refresh: seen 61 seconds on.
        let mut s = state.oauth.lock().unwrap();
        let t = s.tokens.get_mut(&held).unwrap();
        t.expires_in = t.expires_in.saturating_sub(Duration::from_secs(61));
        assert!(!s.validate_token(&held) && s.validate_token(&newest));
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

    /// The store write of a new family runs with the OAuth lock released, and other
    /// exchanges run meanwhile. `meanwhile` runs inside that write, on the thread that
    /// does it: exactly where another request would.
    fn writing_while(store: &Arc<Store>, meanwhile: impl Fn() + Send + Sync + 'static) -> WriteFn {
        let store = store.clone();
        Arc::new(move |change| {
            meanwhile();
            write_to(&store, change)
        })
    }

    /// A new access token used to carry no family until its family was stored. In
    /// between it counted as one without a refresh token, and with sign-in off the
    /// limit on those could drop it: the client then got a refresh token beside an
    /// access token that was already gone.
    #[tokio::test]
    async fn a_new_token_is_its_familys_from_the_start() {
        let (store, _) = temp_store("window-off");
        let state = started(&store, false, &[CLIENT]);
        // Other exchanges for the same client id, all of them while ours is written:
        // more tokens without a refresh token than the client may keep, all newer.
        let others = state.clone();
        let flood = move || {
            let mut s = others.oauth.lock().unwrap();
            for _ in 0..=MAX_LOOSE_TOKENS_PER_CLIENT {
                s.issue_access(Uuid::new_v4().to_string(), CLIENT, None, None);
                s.trim_loose_tokens(CLIENT);
            }
        };
        let (access, refresh) =
            authorize(&state, CLIENT, None, &writing_while(&store, flood)).await;
        assert!(
            valid(&state, &access),
            "pushed out as a token with no family"
        );
        let refresh = refresh.expect("and its refresh token with it");
        assert!(exchange(&state, &refresh, CLIENT, &to(&store))
            .await
            .is_ok());
    }

    /// If the access token is gone all the same when the family has been stored, there
    /// is no refresh token to hand out beside it, and no family left behind.
    #[tokio::test]
    async fn no_refresh_token_is_handed_out_beside_a_dead_access_token() {
        let (store, _) = temp_store("window-gone");
        let state = started(&store, true, &[CLIENT]);
        // As the user's own limit of tokens per client would, with 20 newer ones.
        let others = state.clone();
        let gone = move || others.oauth.lock().unwrap().tokens.clear();
        let user = Some("github:1");
        let (access, refresh) = authorize(&state, CLIENT, user, &writing_while(&store, gone)).await;
        assert!(!valid(&state, &access));
        assert_eq!(refresh, None);
        assert!(state.oauth.lock().unwrap().refresh.is_empty());
        assert!(store.refresh_tokens(0).unwrap().is_empty(), "nor on disk");
    }

    /// With sign-in off a client keeps 20 families and gets no more. Exchanges that
    /// run at the same time must not each see room for the last one.
    #[tokio::test]
    async fn exchanges_at_the_same_time_share_the_limit_of_families() {
        let (store, _) = temp_store("window-cap");
        let state = started(&store, false, &[CLIENT]);
        let write = to(&store);
        for _ in 1..MAX_TOKENS_PER_CLIENT {
            assert!(authorize(&state, CLIENT, None, &write).await.1.is_some());
        }
        // Two more, the second one planned before the first is stored.
        let (first, second) = {
            let mut s = state.oauth.lock().unwrap();
            let a = s.issue_access("a".into(), CLIENT, None, None);
            let first = s.plan_family(CLIENT, REDIRECT, None, &a);
            let b = s.issue_access("b".into(), CLIENT, None, None);
            (first, s.plan_family(CLIENT, REDIRECT, None, &b))
        };
        assert!(first.is_some());
        assert!(second.is_none(), "a 21st family");
        // A family that could not be stored gives its place back.
        let failed = start_family(&state, "a", first.unwrap(), &failing()).await;
        assert!(failed.is_err());
        let mut s = state.oauth.lock().unwrap();
        assert!(
            s.tokens["a"].family.is_none(),
            "the access token alone, as before"
        );
        assert!(s.plan_family(CLIENT, REDIRECT, None, "b").is_some());
    }

    /// A family for `github:1` and `CLIENT` put there directly, with an id of the
    /// test's choosing and issued this second.
    fn put_family(state: &SharedState, id: &str) {
        state.oauth.lock().unwrap().refresh.insert(
            format!("hash-of-{id}"),
            RefreshToken {
                family: id.to_string(),
                seq: 0,
                client_id: CLIENT.to_string(),
                user: Some("github:1".to_string()),
                created_at: now_unix(),
                spent_at: None,
                access_hash: None,
            },
        );
    }

    /// The limit ends a family longest out of use, never the one just issued or just
    /// exchanged. Times are whole seconds, so within a second the newest ties with the
    /// others. Here every family is of the same second and the others have ids that
    /// sort after any real one, so the newest is first in line on every run.
    #[tokio::test]
    async fn the_limit_never_ends_the_family_just_issued_or_exchanged() {
        let (store, _) = temp_store("keep-newest");
        let state = started(&store, true, &[CLIENT]);
        let write = to(&store);
        let families = |state: &SharedState| {
            let mut f = state.oauth.lock().unwrap().families_of("github:1", CLIENT);
            f.sort();
            f
        };
        for i in 0..MAX_TOKENS_PER_CLIENT {
            put_family(&state, &format!("zz{i:02}"));
        }

        // Issued: one more than the limit.
        let (access, issued) = authorize(&state, CLIENT, Some("github:1"), &write).await;
        let issued = issued.expect("the one just issued is kept");
        assert!(valid(&state, &access));
        let left = families(&state);
        assert_eq!(left.len(), MAX_TOKENS_PER_CLIENT);
        // Of the others, tied to the second, the smallest id went: arbitrary, but the
        // same for the same data.
        assert!(!left.contains(&"zz00".to_string()) && left.contains(&"zz01".to_string()));

        // Exchanged while one over the limit: the answer's tokens must work.
        put_family(&state, "zz99");
        let (access, next) = exchange(&state, &issued, CLIENT, &write).await.unwrap();
        assert!(valid(&state, &access), "a 200 with a dead access token");
        let left = families(&state);
        assert_eq!(left.len(), MAX_TOKENS_PER_CLIENT);
        assert!(!left.contains(&"zz01".to_string()) && left.contains(&"zz02".to_string()));
        assert!(exchange(&state, &next, CLIENT, &write).await.is_ok());
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

    /// Where a client hangs up, which drops its request at whatever it was waiting for.
    #[derive(Clone, Copy, Debug)]
    enum HangUp {
        /// Still waiting for its turn: nothing read, nothing written.
        BeforeTheWrite,
        /// The store is being written.
        DuringTheWrite,
        /// The store is written and the request has not gone on yet.
        AfterTheWrite,
        /// Everything is done and only the answer is left to send.
        BeforeTheAnswer,
    }
    const HANG_UPS: [HangUp; 4] = [
        HangUp::BeforeTheWrite,
        HangUp::DuringTheWrite,
        HangUp::AfterTheWrite,
        HangUp::BeforeTheAnswer,
    ];

    /// A store write the test holds open: it says when it begins and when it is done,
    /// and waits in between until let go.
    struct HeldWrite {
        write: WriteFn,
        begun: std::sync::mpsc::Receiver<()>,
        go: std::sync::mpsc::Sender<()>,
        done: std::sync::mpsc::Receiver<()>,
    }

    fn held_write(store: &Arc<Store>) -> HeldWrite {
        let (begun_tx, begun) = std::sync::mpsc::channel();
        let (go, go_rx) = std::sync::mpsc::channel();
        let (done_tx, done) = std::sync::mpsc::channel();
        let (store, go_rx) = (store.clone(), Mutex::new(go_rx));
        let write: WriteFn = Arc::new(move |change| {
            let _ = begun_tx.send(());
            let _ = go_rx.lock().unwrap().recv();
            let result = write_to(&store, change);
            let _ = done_tx.send(());
            result
        });
        HeldWrite {
            write,
            begun,
            go,
            done,
        }
    }

    /// Send `request` and hang up at `at`. Returns once whatever the server still does
    /// for it is done.
    async fn hang_up<F>(at: HangUp, held: &HeldWrite, request: F)
    where
        F: std::future::Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let begun = || async {
            while held.begun.try_recv().is_err() {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        };
        match at {
            HangUp::BeforeTheWrite => {
                let turn = GATE.lock().await;
                let sent = tokio::spawn(request);
                tokio::time::sleep(Duration::from_millis(20)).await;
                sent.abort();
                let _ = sent.await;
                drop(turn);
                held.go.send(()).unwrap();
            }
            HangUp::DuringTheWrite => {
                let sent = tokio::spawn(request);
                begun().await;
                sent.abort();
                let _ = sent.await;
                held.go.send(()).unwrap();
            }
            HangUp::AfterTheWrite => {
                let sent = tokio::spawn(request);
                begun().await;
                held.go.send(()).unwrap();
                // No await from here to the abort: the request is not polled again.
                held.done.recv().unwrap();
                std::thread::sleep(Duration::from_millis(50));
                sent.abort();
                let _ = sent.await;
            }
            HangUp::BeforeTheAnswer => {
                held.go.send(()).unwrap();
                request.await;
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        drop(GATE.lock().await);
    }

    fn usable_rows(store: &Store) -> usize {
        let rows = store.refresh_tokens(0).unwrap();
        rows.iter().filter(|r| r.spent_at.is_none()).count()
    }

    /// A client that hangs up during a refresh never got the answer. The same token
    /// again is then a retry, wherever the request was dropped: the server and its
    /// store must not come apart over a request that went away.
    #[tokio::test]
    async fn a_refresh_whose_client_hangs_up_is_done_or_not_done_and_can_be_retried() {
        for at in HANG_UPS {
            let (store, _) = temp_store(&format!("hang-up-refresh-{at:?}"));
            let state = started(&store, false, &[CLIENT]);
            let write = to(&store);
            let (_, first) = authorize(&state, CLIENT, None, &write).await;
            let first = first.unwrap();
            let held = held_write(&store);
            let (s, token, w) = (state.clone(), first.clone(), held.write.clone());
            hang_up(at, &held, async move {
                grant(&s, &token, CLIENT, &w).await;
            })
            .await;

            let again = exchange(&state, &first, CLIENT, &write).await;
            let (access, next) = again.unwrap_or_else(|status| panic!("{at:?}: {status}"));
            assert!(valid(&state, &access), "{at:?}");
            assert_eq!(usable_rows(&store), 1, "{at:?}");
            assert!(
                exchange(&state, &next, CLIENT, &write).await.is_ok(),
                "{at:?}"
            );
            // And what is in the store is what a restart would bring back.
            let restarted = started(&store, false, &[CLIENT]);
            assert_eq!(restarted.oauth.lock().unwrap().refresh.len(), {
                state.oauth.lock().unwrap().refresh.len()
            });
        }
    }

    /// A revocation whose client hangs up still happens, in memory as in the store.
    #[tokio::test]
    async fn a_revocation_whose_client_hangs_up_still_ends_the_family() {
        for at in HANG_UPS {
            let (store, _) = temp_store(&format!("hang-up-revoke-{at:?}"));
            let state = started(&store, false, &[CLIENT]);
            let write = to(&store);
            let (access, token) = authorize(&state, CLIENT, None, &write).await;
            let token = token.unwrap();
            let held = held_write(&store);
            let (s, t, w) = (state.clone(), token.clone(), held.write.clone());
            hang_up(at, &held, async move {
                revoke(&s, &t, CLIENT, &w).await;
            })
            .await;

            assert!(!valid(&state, &access), "{at:?}");
            assert!(store.refresh_tokens(0).unwrap().is_empty(), "{at:?}");
            assert_eq!(
                exchange(&state, &token, CLIENT, &write).await,
                Err(StatusCode::BAD_REQUEST),
                "{at:?}"
            );
        }
    }

    /// The same for a family that ends because a spent token came back.
    #[tokio::test]
    async fn a_reuse_whose_client_hangs_up_still_ends_the_family() {
        let (store, _) = temp_store("hang-up-reuse");
        let state = started(&store, false, &[CLIENT]);
        let write = to(&store);
        let (_, first) = authorize(&state, CLIENT, None, &write).await;
        let first = first.unwrap();
        let (access, second) = exchange(&state, &first, CLIENT, &write).await.unwrap();
        for t in state.oauth.lock().unwrap().refresh.values_mut() {
            t.spent_at = t.spent_at.map(|at| at - RETRY_WINDOW_SECS - 1);
        }
        let held = held_write(&store);
        let (s, w) = (state.clone(), held.write.clone());
        hang_up(HangUp::DuringTheWrite, &held, async move {
            grant(&s, &first, CLIENT, &w).await;
        })
        .await;

        assert!(!valid(&state, &access));
        assert!(store.refresh_tokens(0).unwrap().is_empty());
        assert!(exchange(&state, &second, CLIENT, &write).await.is_err());
    }

    /// A code exchange whose client hangs up while its family is stored: the family is
    /// then in memory as it is in the store, not in the store alone until a restart.
    #[tokio::test]
    async fn a_new_family_whose_client_hangs_up_is_in_memory_as_in_the_store() {
        let (store, _) = temp_store("hang-up-start");
        let state = started(&store, false, &[CLIENT]);
        let (access, first) = {
            let mut s = state.oauth.lock().unwrap();
            let access = s.issue_access(Uuid::new_v4().to_string(), CLIENT, None, None);
            let first = s.plan_family(CLIENT, REDIRECT, None, &access).unwrap();
            (access, first)
        };
        let held = held_write(&store);
        let (s, a, w) = (state.clone(), access.clone(), held.write.clone());
        hang_up(HangUp::DuringTheWrite, &held, async move {
            let _ = start_family(&s, &a, first, &w).await;
        })
        .await;

        assert_eq!(store.refresh_tokens(0).unwrap().len(), 1);
        let s = state.oauth.lock().unwrap();
        assert_eq!(s.refresh.len(), 1);
        let family = s.refresh.values().next().unwrap().family.clone();
        assert_eq!(s.tokens[&access].family, Some(family));
    }
}
