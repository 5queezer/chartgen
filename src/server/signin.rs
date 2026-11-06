//! Sign-in through GitHub and Google (ADR-0044).
//!
//! chartgen stays the OAuth authorization server for its clients. With a provider
//! configured, `/authorize` stops auto-approving: it needs a signed-in user, sends
//! them through the provider if there is none, and binds the code it issues to that
//! user. Nothing configured means `signin` in `OAuthStore` is `None` and none of
//! this runs.

use super::*;
use axum::extract::Path;
use axum::Form;

/// A pending client request or provider round trip older than this is gone.
const LOGIN_TTL: Duration = Duration::from_secs(600);
/// Open client requests, and open provider round trips, in the whole server. At the
/// limit new sign-ins get a 429 until entries expire; sessions in use are not touched.
const MAX_PENDING_LOGINS: usize = 2000;
/// Open client requests per browser, and per signed-in user: past it the oldest goes.
const MAX_PENDING_PER_BROWSER: usize = 5;
/// Login hits that create state, per client address per `LOGIN_WINDOW`. Junk creates
/// none and costs nothing. Loopback peers are exempt as for `/register`, so behind a
/// same-host proxy with no trusted header only the cap above applies.
const MAX_LOGIN_HITS_PER_IP: usize = 120;
const LOGIN_WINDOW: Duration = Duration::from_secs(60);
const SESSION_TTL_SECS: i64 = 30 * 24 * 3600;
const MAX_SESSIONS_PER_USER: usize = 20;
/// Clients one user can approve; past it the oldest approval is forgotten.
const MAX_GRANTS_PER_USER: usize = 50;
/// Clients one user can keep live. Past it the tokens of the client they got a
/// token for longest ago are dropped, which makes that client evictable, so holding
/// the client table full takes many accounts (ADR-0036).
const MAX_LIVE_CLIENTS_PER_USER: usize = 20;
/// Tokens per user and client; the longest out of use go first.
const MAX_TOKENS_PER_CLIENT: usize = 20;
/// The web app gets a token per page load and each tab keeps using its own, because the
/// chart push limits are per token. A cap below the tabs someone has open would have each
/// tab's sign-in end another's, in a loop that never settles, so this one is roomy.
const MAX_TOKENS_PER_WEB_CLIENT: usize = 100;
const MAX_CODES_PER_USER: usize = 20;
/// Tool calls one user can make per `LOGIN_WINDOW`: chart rendering and upstream
/// quota are shared, and a free account is all it takes to ask for them. A 4-chart
/// grid polling everything makes about 145 a minute per visible window (ADR-0043), so
/// this leaves room for four.
pub(super) const MAX_TOOL_CALLS_PER_USER: usize = 600;
const PROVIDER_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_NAME_CHARS: usize = 100;

const SESSION_COOKIE: &str = "chartgen_session";
/// Random per browser: what ties a client request and a provider round trip to the
/// browser that started them.
const BROWSER_COOKIE: &str = "chartgen_browser";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Provider {
    GitHub,
    Google,
}

impl Provider {
    fn slug(self) -> &'static str {
        match self {
            Provider::GitHub => "github",
            Provider::Google => "google",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Provider::GitHub => "GitHub",
            Provider::Google => "Google",
        }
    }

    /// Identity only: GitHub's public profile needs no scope.
    fn scope(self) -> Option<&'static str> {
        match self {
            Provider::GitHub => None,
            Provider::Google => Some("openid profile email"),
        }
    }
}

pub(super) struct ProviderConfig {
    kind: Provider,
    client_id: String,
    client_secret: String,
    authorize_url: String,
    token_url: String,
    userinfo_url: String,
}

pub(super) struct Config {
    providers: Vec<ProviderConfig>,
    http: reqwest::Client,
}

impl Config {
    /// The providers named by the environment, `None` if there are none. A provider
    /// with an id and no secret, or the reverse, is an error and not "off": a typo
    /// must not quietly leave the server open.
    pub(super) fn from_env() -> Result<Option<Self>, String> {
        Self::from_vars(|name| std::env::var(name).ok().filter(|v| !v.trim().is_empty()))
    }

    fn from_vars(var: impl Fn(&str) -> Option<String>) -> Result<Option<Self>, String> {
        let sample = var("CHARTGEN_DATA_SOURCE").as_deref() == Some("sample");
        let test_origin = test_provider_origin(
            var("CHARTGEN_SIGNIN_TEST_PROVIDER_ORIGIN").as_deref(),
            sample,
        );
        let mut providers = Vec::new();
        for kind in [Provider::GitHub, Provider::Google] {
            let upper = kind.slug().to_ascii_uppercase();
            let id = format!("CHARTGEN_{upper}_CLIENT_ID");
            let secret = format!("CHARTGEN_{upper}_CLIENT_SECRET");
            let (client_id, client_secret) = match (var(&id), var(&secret)) {
                (Some(i), Some(s)) => (i, s),
                (None, None) => continue,
                (Some(_), None) => return Err(format!("{id} is set but {secret} is not")),
                (None, Some(_)) => return Err(format!("{secret} is set but {id} is not")),
            };
            let (authorize_url, token_url, userinfo_url) = match (&test_origin, kind) {
                (Some(o), _) => (
                    format!("{o}/{}/authorize", kind.slug()),
                    format!("{o}/{}/token", kind.slug()),
                    format!("{o}/{}/userinfo", kind.slug()),
                ),
                (None, Provider::GitHub) => (
                    "https://github.com/login/oauth/authorize".to_string(),
                    "https://github.com/login/oauth/access_token".to_string(),
                    "https://api.github.com/user".to_string(),
                ),
                (None, Provider::Google) => (
                    "https://accounts.google.com/o/oauth2/v2/auth".to_string(),
                    "https://oauth2.googleapis.com/token".to_string(),
                    "https://openidconnect.googleapis.com/v1/userinfo".to_string(),
                ),
            };
            providers.push(ProviderConfig {
                kind,
                client_id,
                client_secret,
                authorize_url,
                token_url,
                userinfo_url,
            });
        }
        if providers.is_empty() {
            return Ok(None);
        }
        let http = reqwest::Client::builder()
            .timeout(PROVIDER_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("chartgen")
            .build()
            .map_err(|e| format!("cannot build the HTTP client: {e}"))?;
        Ok(Some(Self { providers, http }))
    }

    pub(super) fn provider_labels(&self) -> String {
        let labels: Vec<_> = self.providers.iter().map(|p| p.kind.label()).collect();
        labels.join(", ")
    }

    fn provider(&self, slug: &str) -> Option<&ProviderConfig> {
        self.providers.iter().find(|p| p.kind.slug() == slug)
    }
}

/// The fake provider of the tests: a loopback `http` origin, honored only on the
/// sample data source. Environment only, so no request can turn it on, and a server
/// on live data ignores it.
fn test_provider_origin(raw: Option<&str>, sample: bool) -> Option<String> {
    let origin = raw.filter(|_| sample).and_then(|raw| {
        let url = reqwest::Url::parse(raw).ok()?;
        let loopback = url
            .host_str()
            .and_then(|h| h.parse::<std::net::Ipv4Addr>().ok())
            .is_some_and(|ip| ip.is_loopback());
        (loopback && url.scheme() == "http").then(|| url.origin().ascii_serialization())
    });
    if let Some(origin) = &origin {
        eprintln!("[OAuth] WARNING: sign-in uses the test provider at {origin}");
    }
    origin
}

/// `Some` when the base URL is a public one, the only case that warrants a warning
/// about an open server: local use and tests have no `CHARTGEN_BASE_URL`.
pub(super) fn public_base_url() -> Option<String> {
    let raw = std::env::var("CHARTGEN_BASE_URL").ok()?;
    let url = reqwest::Url::parse(&raw).ok()?;
    let host = url.host_str()?.trim_matches(['[', ']']);
    let local = host == "localhost"
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|a| a.is_loopback());
    (!local).then_some(raw)
}

// --- State ------------------------------------------------------------------

#[derive(Clone)]
struct Session {
    user: String,
    expires_at: i64,
    created_at: i64,
}

/// A client's `/authorize` request while its user signs in or decides. It is kept
/// here and only its random id travels, so nothing in the browser can change where
/// the code goes, and it is checked against the client again before use.
#[derive(Clone)]
pub(super) struct PendingRequest {
    pub(super) client_id: String,
    pub(super) redirect_uri: String,
    pub(super) code_challenge: String,
    pub(super) code_challenge_method: String,
    pub(super) state: Option<String>,
    browser: String,
    /// The signed-in user who asked, for the limit per user.
    user: Option<String>,
    /// The web app's own client, asked from its own pages: needs no consent. Read at
    /// `/authorize`, because the provider's callback always looks cross-site.
    own_app: bool,
    created_at: Instant,
    /// Set when the consent page was shown: the session it was shown to and the
    /// token its form must return.
    consent: Option<(String, String)>,
}

/// One round trip to a provider, keyed by its `state`.
struct ProviderFlow {
    provider: Provider,
    request: String,
    browser: String,
    verifier: String,
    created_at: Instant,
}

/// The user behind a code or token.
#[derive(Clone)]
pub(super) struct TokenUser {
    pub(super) id: String,
    /// The browser session, for the web app's own tokens only: signing out ends
    /// them, and leaves what other clients were given.
    pub(super) session: Option<String>,
    /// When it last made a call; the limit per client drops the longest out of use.
    pub(super) last_used: Instant,
}

pub(super) struct SignIn {
    pub(super) config: Arc<Config>,
    users: HashMap<String, String>,
    /// By the hash of the cookie value.
    sessions: HashMap<String, Session>,
    /// Per user, oldest first.
    grants: HashMap<String, Vec<String>>,
    pending: HashMap<String, PendingRequest>,
    flows: HashMap<String, ProviderFlow>,
    login_hits: HashMap<String, Vec<Instant>>,
    tool_calls: HashMap<String, Vec<Instant>>,
}

/// What a handler must write to the store once it has let go of the OAuth lock.
#[cfg_attr(not(feature = "scripting"), allow(dead_code))]
enum Write {
    User {
        id: String,
        name: String,
    },
    Session {
        token_hash: String,
        user: String,
        created_at: i64,
        expires_at: i64,
    },
    DeleteSession(String),
    Grant {
        user: String,
        client: String,
    },
    DeleteGrant {
        user: String,
        client: String,
    },
}

/// Apply writes off the async workers: every authenticated call needs the OAuth lock
/// and a busy store must not hold up whatever else a blocked worker has. A build
/// without the store keeps all of this in memory.
#[cfg_attr(not(feature = "scripting"), allow(unused_variables))]
async fn persist(writes: Vec<Write>) {
    #[cfg(feature = "scripting")]
    if !writes.is_empty() {
        let _ = tokio::task::spawn_blocking(move || {
            let Some(store) = scripting::persistence::global() else {
                return;
            };
            for write in writes {
                let done = match &write {
                    Write::User { id, name } => store.save_user(id, name),
                    Write::Session {
                        token_hash,
                        user,
                        created_at,
                        expires_at,
                    } => store.save_session(&scripting::persistence::StoredSession {
                        token_hash: token_hash.clone(),
                        user_id: user.clone(),
                        created_at: *created_at,
                        expires_at: *expires_at,
                    }),
                    Write::DeleteSession(hash) => store.delete_session(hash),
                    Write::Grant { user, client } => store.save_grant(user, client),
                    Write::DeleteGrant { user, client } => store.delete_grant(user, client),
                };
                if let Err(e) = done {
                    eprintln!("[OAuth] could not persist a sign-in change: {e}");
                }
            }
        })
        .await;
    }
}

/// `CHARTGEN_CLAIM_OWNER`: the one account that inherits what clients owned before
/// sign-in. Unset means nobody does. A client id travels in URLs, so holding one must
/// never be what hands anything over (ADR-0044).
#[cfg(feature = "scripting")]
pub(super) fn claim_owner() -> Option<String> {
    std::env::var("CHARTGEN_CLAIM_OWNER")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// A user got a token: that keeps their server alerts alive, and the named owner
/// collects what clients still own, through any client (ADR-0044). Blocking.
#[cfg(feature = "scripting")]
pub(super) fn after_token(user: &str) {
    let Some(store) = scripting::persistence::global() else {
        return;
    };
    if let Err(e) = store.mark_user_used(user) {
        eprintln!("[OAuth] could not record user activity: {e}");
    }
    if claim_owner().as_deref() != Some(user) {
        return;
    }
    match store.claim_waiting_scripts(user) {
        Ok(0) => {}
        Ok(n) => eprintln!("[OAuth] {n} script(s) from before sign-in moved to the named owner"),
        Err(e) => eprintln!("[OAuth] could not move scripts to the named owner: {e}"),
    }
    if let Some(alerts) = crate::server_alerts::global() {
        match alerts.claim_waiting(user) {
            0 => {}
            n => eprintln!(
                "[OAuth] {n} server alert(s) from before sign-in moved to the named owner"
            ),
        }
    }
}

/// What startup says about rows from before sign-in: counts only, an id is a credential.
#[cfg(feature = "scripting")]
pub(super) fn waiting_rows_notice(
    scripts: i64,
    alerts: i64,
    owner: Option<&str>,
) -> Option<String> {
    if scripts == 0 && alerts == 0 {
        return None;
    }
    let rows = format!("{scripts} script(s) and {alerts} server alert(s) from before sign-in");
    Some(match owner {
        Some(_) => format!("{rows} wait for the owner CHARTGEN_CLAIM_OWNER names, until their next sign-in."),
        None => format!("{rows} stay with their clients, out of everyone's reach. To collect them, set CHARTGEN_CLAIM_OWNER=github:<id> or google:<sub> to your own account and sign in."),
    })
}

/// How many entries of `CHARTGEN_SERVER_ALERT_CLIENTS` can match nobody with sign-in
/// on: owners are then accounts, so a client id on the list grants nothing.
pub(super) fn allowlist_entries_not_accounts(raw: &str) -> usize {
    raw.split(',')
        .map(str::trim)
        .filter(|id| !id.is_empty() && *id != "*")
        .filter(|id| {
            ![Provider::GitHub, Provider::Google]
                .iter()
                .any(|p| id.starts_with(&format!("{}:", p.slug())))
        })
        .count()
}

impl SignIn {
    pub(super) fn new(config: Config) -> Self {
        Self {
            config: Arc::new(config),
            users: HashMap::new(),
            sessions: HashMap::new(),
            grants: HashMap::new(),
            pending: HashMap::new(),
            flows: HashMap::new(),
            login_hits: HashMap::new(),
            tool_calls: HashMap::new(),
        }
    }

    /// Load users, sessions and approvals; returns how many users and sessions.
    #[cfg(feature = "scripting")]
    pub(super) fn restore(
        &mut self,
        store: &scripting::persistence::Store,
    ) -> rusqlite::Result<(usize, usize)> {
        for u in store.users()? {
            self.users.insert(u.id, u.name);
        }
        for s in store.live_sessions(now_unix())? {
            self.sessions.insert(
                s.token_hash,
                Session {
                    user: s.user_id,
                    expires_at: s.expires_at,
                    created_at: s.created_at,
                },
            );
        }
        for (user, client) in store.grants()? {
            self.grants.entry(user).or_default().push(client);
        }
        Ok((self.users.len(), self.sessions.len()))
    }

    fn cleanup(&mut self) {
        let now = Instant::now();
        self.pending
            .retain(|_, p| now.duration_since(p.created_at) < LOGIN_TTL);
        let pending = &self.pending;
        self.flows.retain(|_, f| {
            now.duration_since(f.created_at) < LOGIN_TTL && pending.contains_key(&f.request)
        });
        let unix = now_unix();
        self.sessions.retain(|_, s| s.expires_at > unix);
    }

    /// Keep this browser's and this user's open requests below their limit for one
    /// more, oldest going first with their provider round trips.
    fn make_room_for_request(&mut self, browser: &str, user: Option<&str>) {
        let mut mine: Vec<(Instant, String)> = self
            .pending
            .iter()
            .filter(|(_, p)| p.browser == browser || (user.is_some() && p.user.as_deref() == user))
            .map(|(id, p)| (p.created_at, id.clone()))
            .collect();
        mine.sort();
        let excess = (mine.len() + 1).saturating_sub(MAX_PENDING_PER_BROWSER);
        for (_, id) in mine.into_iter().take(excess) {
            self.pending.remove(&id);
            self.flows.retain(|_, f| f.request != id);
        }
    }

    /// Count a login hit that creates state from `peer`; false when over the limit.
    fn admit_login(&mut self, peer: &str, now: Instant) -> bool {
        self.login_hits.retain(|_, hits| {
            hits.retain(|t| now.duration_since(*t) < LOGIN_WINDOW);
            !hits.is_empty()
        });
        if peer
            .parse::<std::net::IpAddr>()
            .is_ok_and(|a| a.is_loopback())
        {
            return true;
        }
        let hits = self.login_hits.entry(peer.to_owned()).or_default();
        if hits.len() >= MAX_LOGIN_HITS_PER_IP {
            return false;
        }
        hits.push(now);
        true
    }

    /// Count a tool call by `user`; false when over the limit.
    fn admit_tool_call(&mut self, user: &str, now: Instant) -> bool {
        self.tool_calls.retain(|_, hits| {
            hits.retain(|t| now.duration_since(*t) < LOGIN_WINDOW);
            !hits.is_empty()
        });
        let hits = self.tool_calls.entry(user.to_owned()).or_default();
        if hits.len() >= MAX_TOOL_CALLS_PER_USER {
            return false;
        }
        hits.push(now);
        true
    }

    /// The session a request carries, as `(cookie hash, user)`.
    fn session_of(&mut self, headers: &HeaderMap) -> Option<(String, String)> {
        self.cleanup();
        let hash = hash_of(&read_cookie(headers, SESSION_COOKIE)?);
        let user = self.sessions.get(&hash)?.user.clone();
        Some((hash, user))
    }

    fn pending_for(&self, id: &str, browser: Option<&str>) -> Option<&PendingRequest> {
        self.pending
            .get(id)
            .filter(|p| browser.is_some_and(|b| p.browser == b))
    }

    fn has_grant(&self, user: &str, client: &str) -> bool {
        self.grants
            .get(user)
            .is_some_and(|clients| clients.iter().any(|c| c == client))
    }

    /// Open a session, dropping what it replaces and what exceeds the user's limit.
    /// Returns the cookie value and the store writes.
    fn open_session(&mut self, user: &str, replaces: Option<String>) -> (String, Vec<Write>) {
        let mut writes = Vec::new();
        if let Some(old) = replaces {
            if self.sessions.remove(&old).is_some() {
                writes.push(Write::DeleteSession(old));
            }
        }
        let mut mine: Vec<(i64, String)> = self
            .sessions
            .iter()
            .filter(|(_, s)| s.user == user)
            .map(|(h, s)| (s.created_at, h.clone()))
            .collect();
        mine.sort();
        let excess = (mine.len() + 1).saturating_sub(MAX_SESSIONS_PER_USER);
        for (_, hash) in mine.into_iter().take(excess) {
            self.sessions.remove(&hash);
            writes.push(Write::DeleteSession(hash));
        }
        let token = random_token();
        let token_hash = hash_of(&token);
        let created_at = now_unix();
        let expires_at = created_at + SESSION_TTL_SECS;
        self.sessions.insert(
            token_hash.clone(),
            Session {
                user: user.to_string(),
                expires_at,
                created_at,
            },
        );
        writes.push(Write::Session {
            token_hash,
            user: user.to_string(),
            created_at,
            expires_at,
        });
        (token, writes)
    }

    fn add_grant(&mut self, user: &str, client: &str) -> Vec<Write> {
        let mut writes = vec![Write::Grant {
            user: user.to_string(),
            client: client.to_string(),
        }];
        let clients = self.grants.entry(user.to_string()).or_default();
        clients.push(client.to_string());
        if clients.len() > MAX_GRANTS_PER_USER {
            writes.push(Write::DeleteGrant {
                user: user.to_string(),
                client: clients.remove(0),
            });
        }
        writes
    }
}

impl OAuthStore {
    /// Hold a user's codes and tokens to their limits, after issuing one. Oldest go
    /// first; the holder of a dropped token signs in again without noticing much,
    /// since a session or a new approval round is all it takes.
    pub(super) fn trim_user_tokens(&mut self, user: &str) {
        let mut by_client: HashMap<String, Vec<(Instant, String)>> = HashMap::new();
        let mut web_clients = HashSet::new();
        for (token, t) in &self.tokens {
            let Some(u) = t.user.as_ref().filter(|u| u.id == user) else {
                continue;
            };
            by_client
                .entry(t.client_id.clone())
                .or_default()
                .push((u.last_used, token.clone()));
            if u.session.is_some() {
                web_clients.insert(t.client_id.clone());
            }
        }
        let mut clients: Vec<_> = by_client
            .into_iter()
            .map(|(client, mut tokens)| {
                tokens.sort();
                let cap = if web_clients.contains(&client) {
                    MAX_TOKENS_PER_WEB_CLIENT
                } else {
                    MAX_TOKENS_PER_CLIENT
                };
                (
                    tokens.last().map(|t| t.0).unwrap_or_else(Instant::now),
                    cap,
                    tokens,
                )
            })
            .collect();
        clients.sort_by_key(|c| std::cmp::Reverse(c.0));
        for (rank, (_, cap, tokens)) in clients.into_iter().enumerate() {
            let keep = if rank < MAX_LIVE_CLIENTS_PER_USER {
                cap
            } else {
                0
            };
            let excess = tokens.len().saturating_sub(keep);
            for (_, token) in tokens.into_iter().take(excess) {
                self.tokens.remove(&token);
            }
        }
    }

    fn trim_user_codes(&mut self, user: &str) {
        let mut mine: Vec<(Instant, String)> = self
            .codes
            .iter()
            .filter(|(_, c)| c.user.as_ref().is_some_and(|u| u.id == user))
            .map(|(code, c)| (c.created_at, code.clone()))
            .collect();
        mine.sort();
        let excess = (mine.len() + 1).saturating_sub(MAX_CODES_PER_USER);
        for (_, code) in mine.into_iter().take(excess) {
            self.codes.remove(&code);
        }
    }

    /// Count a tool call against the token's user, if it has one.
    pub(super) fn admit_tool_call(&mut self, token: &str) -> bool {
        let Some(user) = self.user_for_token(token) else {
            return true;
        };
        self.signin
            .as_mut()
            .is_none_or(|s| s.admit_tool_call(&user, Instant::now()))
    }

    /// `(display name, provider slug)` of a signed-in user.
    pub(super) fn user_profile(&self, user: &str) -> Option<(String, String)> {
        let signin = self.signin.as_ref()?;
        let name = signin.users.get(user)?.clone();
        let provider = user.split(':').next().unwrap_or_default().to_string();
        Some((name, provider))
    }

    /// Issue a code for `request` to `user`; the answer is the redirect to the client.
    fn issue_code(
        &mut self,
        request: &PendingRequest,
        user: &str,
        session: &str,
    ) -> Option<String> {
        self.trim_user_codes(user);
        let first_party = is_first_party(&base_url(), &request.redirect_uri);
        let code = Uuid::new_v4().to_string();
        self.codes.insert(
            code.clone(),
            AuthCode {
                _code: code.clone(),
                client_id: request.client_id.clone(),
                redirect_uri: request.redirect_uri.clone(),
                code_challenge: request.code_challenge.clone(),
                _code_challenge_method: request.code_challenge_method.clone(),
                created_at: Instant::now(),
                user: Some(TokenUser {
                    id: user.to_string(),
                    session: first_party.then(|| session.to_string()),
                    last_used: Instant::now(),
                }),
            },
        );
        client_redirect(
            &request.redirect_uri,
            &[("code", code.as_str())],
            request.state.as_deref(),
        )
    }

    /// What comes next for a signed-in user's request: the code if the client needs
    /// no approval, else the consent page. A client gone in the meantime is answered
    /// as an unknown one; the request is dropped in every case but consent.
    fn continue_request(&mut self, id: &str, user: &str, session: &str) -> Response {
        let Some(request) = self
            .signin
            .as_ref()
            .and_then(|s| s.pending.get(id))
            .cloned()
        else {
            return expired_page();
        };
        let registered = self
            .clients
            .get(&request.client_id)
            .is_some_and(|c| c.redirect_uris.contains(&request.redirect_uri));
        if !registered {
            if let Some(s) = self.signin.as_mut() {
                s.pending.remove(id);
            }
            return unknown_client_answer(&request.redirect_uri, request.state.as_deref());
        }
        let approved = request.own_app
            || self
                .signin
                .as_ref()
                .is_some_and(|s| s.has_grant(user, &request.client_id));
        if !approved {
            return Redirect::to(&format!("/oauth/consent?request={id}")).into_response();
        }
        if let Some(s) = self.signin.as_mut() {
            s.pending.remove(id);
        }
        match self.issue_code(&request, user, session) {
            Some(target) => Redirect::to(&target).into_response(),
            None => expired_page(),
        }
    }
}

// --- The web app's own client ------------------------------------------------

/// Whether a browser says this navigation started on our own origin, or at the address
/// bar. A stranger's link is `cross-site`, and one with no such header proves nothing.
fn from_own_origin(headers: &HeaderMap) -> bool {
    headers
        .get("sec-fetch-site")
        .is_some_and(|v| v == "same-origin" || v == "none")
}

/// The web app's redirect: exactly `/` on this server's origin. Such a client skips
/// consent when asked from our own origin (`from_own_origin`), whoever registered it:
/// its code lands on our own page, which discards a code whose `state` it did not issue.
fn is_first_party(base: &str, redirect_uri: &str) -> bool {
    let (Ok(base), Ok(uri)) = (reqwest::Url::parse(base), reqwest::Url::parse(redirect_uri)) else {
        return false;
    };
    uri.origin() == base.origin()
        && uri.username().is_empty()
        && uri.password().is_none()
        && uri.fragment().is_none()
        && uri.query().is_none()
        && uri.path() == "/"
}

fn client_redirect(
    redirect_uri: &str,
    params: &[(&str, &str)],
    state: Option<&str>,
) -> Option<String> {
    let mut target = reqwest::Url::parse(redirect_uri).ok()?;
    {
        let mut query = target.query_pairs_mut();
        for (k, v) in params {
            query.append_pair(k, v);
        }
        if let Some(state) = state {
            query.append_pair("state", state);
        }
    }
    Some(target.into())
}

/// The answer for a client that disappeared mid sign-in: the web app recovers as
/// from a stale client (ADR-0036), anyone else gets a page saying what to do.
fn unknown_client_answer(redirect_uri: &str, state: Option<&str>) -> Response {
    if std::env::var_os("CHARTGEN_WEB_DIR").is_some() {
        if let Some(target) = invalid_client_redirect(&base_url(), redirect_uri, state) {
            return Redirect::to(&target).into_response();
        }
    }
    page(
        StatusCode::BAD_REQUEST,
        "This application is no longer registered",
        "<p>The application you were signing in to is no longer registered with chartgen. Go back to it and connect again.</p>",
    )
}

// --- Cookies and tokens -----------------------------------------------------

fn is_https() -> bool {
    base_url().starts_with("https://")
}

/// With https the name carries `__Host-`, so a sibling subdomain cannot plant it:
/// that would undo the browser binding that stops login CSRF.
fn cookie_name(name: &str) -> String {
    if is_https() {
        format!("__Host-{name}")
    } else {
        name.to_string()
    }
}

fn cookie_header(name: &str, value: &str, max_age: i64) -> HeaderValue {
    let secure = if is_https() { "; Secure" } else { "" };
    let name = cookie_name(name);
    let raw = format!("{name}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure}");
    HeaderValue::from_str(&raw).unwrap_or_else(|_| HeaderValue::from_static(""))
}

fn read_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    let name = cookie_name(name);
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_string())
}

fn with_cookie(mut resp: Response, cookie: HeaderValue) -> Response {
    resp.headers_mut().append(header::SET_COOKIE, cookie);
    resp
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::rng(), &mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn hash_of(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

/// The browser's binding value as stored (a hash), minting the cookie if it has none.
fn browser_binding(headers: &HeaderMap) -> (String, HeaderValue) {
    let value = read_cookie(headers, BROWSER_COOKIE).unwrap_or_else(random_token);
    let cookie = cookie_header(BROWSER_COOKIE, &value, LOGIN_TTL.as_secs() as i64);
    (hash_of(&value), cookie)
}

fn browser_of(headers: &HeaderMap) -> Option<String> {
    read_cookie(headers, BROWSER_COOKIE).map(|v| hash_of(&v))
}

/// A state-changing request from a browser must come from our own origin. SameSite
/// already keeps the cookies off a cross-site POST; this holds without them too.
fn same_origin(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN) else {
        return true;
    };
    let ours = reqwest::Url::parse(&base_url()).map(|u| u.origin().ascii_serialization());
    origin
        .to_str()
        .ok()
        .zip(ours.ok())
        .is_some_and(|(o, b)| o == b)
}

// --- Pages -------------------------------------------------------------------

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

const PAGE_STYLE: &str = "body{margin:0;min-height:100vh;display:flex;align-items:center;justify-content:center;background:#131722;color:#d1d4dc;font:15px/1.5 system-ui,sans-serif;padding:16px;box-sizing:border-box}main{width:100%;max-width:26rem;background:#1e222d;border:1px solid #2a2e39;border-radius:10px;padding:24px;box-sizing:border-box}h1{font-size:20px;margin:0 0 12px}a.b,button{display:block;width:100%;box-sizing:border-box;margin-top:10px;padding:11px;border-radius:6px;border:1px solid #2a2e39;background:#2a2e39;color:#d1d4dc;font:inherit;text-align:center;text-decoration:none;cursor:pointer}a.b.p,button.p{background:#2962ff;border-color:#2962ff;color:#fff}.host{font-size:18px;font-weight:600;word-break:break-all;color:#fff}.m{color:#9598a1;font-size:13px;word-break:break-word}";

/// A page of ours: no scripts, nothing framed, nothing cached, no referrer to other
/// sites. There is no `form-action`, which browsers also apply to the redirect to
/// the client.
fn page(status: StatusCode, title: &str, body: &str) -> Response {
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{title} · chartgen</title><style>{PAGE_STYLE}</style></head><body><main><h1>{title}</h1>{body}</main></body></html>",
        title = escape(title),
    );
    (
        status,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; frame-ancestors 'none'",
            ),
            (header::X_FRAME_OPTIONS, "DENY"),
            // Not `no-referrer`: that makes a browser send `Origin: null` on the consent
            // form, which `same_origin` rightly refuses.
            (header::REFERRER_POLICY, "same-origin"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        html,
    )
        .into_response()
}

const BACK_LINK: &str = "<a class=\"b\" href=\"/\">Back to chartgen</a><p class=\"m\">Connecting another application? Go back to it and start again.</p>";

fn expired_page() -> Response {
    page(
        StatusCode::BAD_REQUEST,
        "This sign-in has expired",
        &format!("<p>It took too long, was already used, or started in another browser. Sign-in needs cookies, so check they are on if this keeps happening.</p>{BACK_LINK}"),
    )
}

/// A failure the user can retry: back to the choice of provider, or give up, which
/// tells the client. Without a usable request only the way back is left.
fn failure_page(status: StatusCode, title: &str, message: &str, request: Option<&str>) -> Response {
    let actions = match request {
        Some(id) => format!(
            "<a class=\"b p\" href=\"/oauth/signin?request={id}\">Try again</a><a class=\"b\" href=\"/oauth/cancel?request={id}\">Cancel</a>",
            id = escape(id),
        ),
        None => BACK_LINK.to_string(),
    };
    page(
        status,
        title,
        &format!("<p>{}</p>{actions}", escape(message)),
    )
}

fn too_many_page() -> Response {
    let mut resp = failure_page(
        StatusCode::TOO_MANY_REQUESTS,
        "Too many sign-in attempts",
        "Too many sign-ins are in progress right now. Wait a minute and try again.",
        None,
    );
    resp.headers_mut()
        .insert(header::RETRY_AFTER, HeaderValue::from_static("60"));
    resp
}

fn chooser_page(config: &Config, request: &str) -> Response {
    let mut links = String::new();
    for p in &config.providers {
        links.push_str(&format!(
            "<a class=\"b p\" href=\"/oauth/login/{slug}?request={id}\">Continue with {label}</a>",
            slug = p.kind.slug(),
            id = escape(request),
            label = p.kind.label(),
        ));
    }
    page(
        StatusCode::OK,
        "Sign in to chartgen",
        &format!("<p>Choose how to sign in. chartgen keeps only your account id and name.</p>{links}<p class=\"m\">A GitHub and a Google account are two separate chartgen users, each with their own scripts.</p>"),
    )
}

/// Where the code would go, as a person should read it: the host, or for a native
/// app its scheme. The client's name proves nothing, this does.
fn redirect_target_label(redirect_uri: &str) -> String {
    match reqwest::Url::parse(redirect_uri) {
        Ok(u) => match u.host_str() {
            Some(host) => host.to_string(),
            None => format!("{}:", u.scheme()),
        },
        Err(_) => redirect_uri.to_string(),
    }
}

fn consent_page(
    request: &str,
    csrf: &str,
    client: &str,
    redirect_uri: &str,
    user: &str,
) -> Response {
    let body = format!(
        "<p>An application calling itself <b>{client}</b> wants to use chartgen as you. That name is chosen by the application and proves nothing, so check where it will send you:</p>\
         <p class=\"host\" data-testid=\"redirect-host\">{host}</p>\
         <p class=\"m\">{uri}</p>\
         <p>It will be able to read charts, and to use and change your scripted indicators. Signed in as <b>{user}</b>.</p>\
         <form method=\"post\" action=\"/oauth/consent\">\
         <input type=\"hidden\" name=\"request\" value=\"{request}\">\
         <input type=\"hidden\" name=\"csrf\" value=\"{csrf}\">\
         <button class=\"p\" name=\"decision\" value=\"approve\">Approve</button>\
         <button name=\"decision\" value=\"deny\">Deny</button>\
         </form>",
        client = escape(client),
        host = escape(&redirect_target_label(redirect_uri)),
        uri = escape(redirect_uri),
        user = escape(user),
        request = escape(request),
        csrf = escape(csrf),
    );
    page(StatusCode::OK, "Approve this application?", &body)
}

// --- Endpoints ---------------------------------------------------------------

/// `GET /authorize` with sign-in on, after the client and `redirect_uri` were checked:
/// a session and an approved client get the code, anything else goes on to the
/// provider or to consent with the request held here.
pub(super) fn authorize(
    s: &mut OAuthStore,
    params: &AuthorizeQuery,
    headers: &HeaderMap,
    peer: &str,
) -> Response {
    let Some(signin) = s.signin.as_mut() else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let session = signin.session_of(headers);
    let (browser, browser_cookie) = browser_binding(headers);
    let request = PendingRequest {
        client_id: params.client_id.clone(),
        redirect_uri: params.redirect_uri.clone(),
        code_challenge: params.code_challenge.clone(),
        code_challenge_method: params.code_challenge_method.clone(),
        state: params.state.clone(),
        browser: browser.clone(),
        user: session.as_ref().map(|(_, user)| user.clone()),
        own_app: is_first_party(&base_url(), &params.redirect_uri) && from_own_origin(headers),
        created_at: Instant::now(),
        consent: None,
    };
    if let Some((session, user)) = &session {
        let approved = request.own_app || signin.has_grant(user, &params.client_id);
        if approved {
            return match s.issue_code(&request, user, session) {
                Some(target) => Redirect::temporary(&target).into_response(),
                None => expired_page(),
            };
        }
    }
    if !signin.admit_login(peer, Instant::now()) || signin.pending.len() >= MAX_PENDING_LOGINS {
        return too_many_page();
    }
    signin.make_room_for_request(&browser, session.as_ref().map(|(_, user)| user.as_str()));
    let id = random_token();
    signin.pending.insert(id.clone(), request);
    let next = if session.is_some() {
        format!("/oauth/consent?request={id}")
    } else {
        match signin.config.providers.as_slice() {
            [only] => format!("/oauth/login/{}?request={id}", only.kind.slug()),
            _ => format!("/oauth/signin?request={id}"),
        }
    };
    with_cookie(Redirect::to(&next).into_response(), browser_cookie)
}

#[derive(Deserialize)]
pub(super) struct RequestRef {
    request: String,
}

/// `GET /oauth/signin`: the choice of provider, and where a failed attempt retries.
pub(super) async fn signin_page(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Query(q): Query<RequestRef>,
) -> Response {
    let mut guard = state.oauth.lock().unwrap();
    let s = &mut *guard;
    let Some(signin) = s.signin.as_mut() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    signin.cleanup();
    if signin
        .pending_for(&q.request, browser_of(&headers).as_deref())
        .is_none()
    {
        return expired_page();
    }
    if let Some((session, user)) = signin.session_of(&headers) {
        return s.continue_request(&q.request, &user, &session);
    }
    match signin.config.providers.as_slice() {
        [only] => Redirect::to(&format!(
            "/oauth/login/{}?request={}",
            only.kind.slug(),
            q.request
        ))
        .into_response(),
        _ => chooser_page(&signin.config, &q.request),
    }
}

/// `GET /oauth/login/{provider}`: start one round trip to the provider.
pub(super) async fn login(
    State(state): State<SharedState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    ext: axum::http::Extensions,
    Query(q): Query<RequestRef>,
) -> Response {
    let mut guard = state.oauth.lock().unwrap();
    let Some(signin) = guard.signin.as_mut() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    signin.cleanup();
    let config = signin.config.clone();
    let Some(provider) = config.provider(&slug) else {
        return page(
            StatusCode::NOT_FOUND,
            "Unknown sign-in provider",
            &format!("<p>This server does not offer that way to sign in.</p>{BACK_LINK}"),
        );
    };
    let Some(browser) =
        browser_of(&headers).filter(|b| signin.pending_for(&q.request, Some(b)).is_some())
    else {
        return expired_page();
    };
    // One round trip per request, so these cannot outnumber the requests. Starting
    // another for the same request replaces it and creates nothing, so it is not counted.
    let replacing = signin.flows.values().any(|f| f.request == q.request);
    if !replacing && !signin.admit_login(&client_addr(&headers, &ext), Instant::now()) {
        return too_many_page();
    }
    signin.flows.retain(|_, f| f.request != q.request);
    let flow_state = random_token();
    let verifier = random_token();
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let callback = callback_url(provider.kind);
    let mut query = vec![
        ("client_id", provider.client_id.as_str()),
        ("redirect_uri", callback.as_str()),
        ("response_type", "code"),
        ("state", flow_state.as_str()),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
    ];
    if let Some(scope) = provider.kind.scope() {
        query.push(("scope", scope));
    }
    let Ok(target) = reqwest::Url::parse_with_params(&provider.authorize_url, &query) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    signin.flows.insert(
        flow_state,
        ProviderFlow {
            provider: provider.kind,
            request: q.request,
            browser,
            verifier,
            created_at: Instant::now(),
        },
    );
    eprintln!("[OAuth] sign-in started provider={}", provider.kind.slug());
    Redirect::to(target.as_str()).into_response()
}

/// The exact redirect URI registered with the provider.
fn callback_url(provider: Provider) -> String {
    format!(
        "{}/oauth/callback/{}",
        base_url().trim_end_matches('/'),
        provider.slug()
    )
}

#[derive(Deserialize)]
pub(super) struct CallbackQuery {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

/// `GET /oauth/callback/{provider}`: the provider's answer. The `state` is used up
/// on first sight, must belong to this provider and to this browser, and only then
/// does anything of the answer count.
pub(super) async fn callback(
    State(state): State<SharedState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    ext: axum::http::Extensions,
    Query(q): Query<CallbackQuery>,
) -> Response {
    let taken = {
        let mut guard = state.oauth.lock().unwrap();
        let Some(signin) = guard.signin.as_mut() else {
            return StatusCode::NOT_FOUND.into_response();
        };
        signin.cleanup();
        // Only a `state` of ours counts, and a refusal leaves it for a retry.
        let known = q
            .state
            .as_deref()
            .is_some_and(|flow_state| signin.flows.contains_key(flow_state));
        if known && !signin.admit_login(&client_addr(&headers, &ext), Instant::now()) {
            return too_many_page();
        }
        let config = signin.config.clone();
        q.state
            .as_deref()
            .and_then(|flow_state| signin.flows.remove(flow_state))
            .map(|flow| (flow, config))
    };
    let Some((flow, config)) = taken else {
        return expired_page();
    };
    if flow.provider.slug() != slug
        || browser_of(&headers).as_deref() != Some(flow.browser.as_str())
    {
        eprintln!("[OAuth] sign-in callback refused: not this flow's provider or browser");
        return expired_page();
    }
    let provider = flow.provider;
    let Some(provider_config) = config.provider(provider.slug()) else {
        return expired_page();
    };
    let retry = Some(flow.request.as_str());
    let label = provider.label();
    if let Some(error) = q.error.as_deref() {
        // The value is whatever the URL says: it picks a message and is not shown.
        return if error == "access_denied" {
            failure_page(
                StatusCode::OK,
                "Sign-in cancelled",
                &format!("{label} sign-in was cancelled, so you are not signed in."),
                retry,
            )
        } else {
            eprintln!(
                "[OAuth] provider answered with an error provider={}",
                provider.slug()
            );
            failure_page(
                StatusCode::BAD_GATEWAY,
                "Sign-in failed",
                &format!("{label} reported a problem with this sign-in."),
                retry,
            )
        };
    }
    let Some(code) = q.code.as_deref().filter(|c| !c.is_empty()) else {
        return failure_page(
            StatusCode::BAD_GATEWAY,
            "Sign-in failed",
            &format!("{label} sent back an incomplete answer."),
            retry,
        );
    };
    let identity = match fetch_identity(provider_config, &config.http, code, &flow.verifier).await {
        Ok(identity) => identity,
        Err(failure) => {
            eprintln!(
                "[OAuth] sign-in failed provider={} reason={}",
                provider.slug(),
                failure.log_reason()
            );
            return failure_page(
                StatusCode::BAD_GATEWAY,
                "Sign-in failed",
                &failure.message(label),
                retry,
            );
        }
    };

    let (response, token, mut writes) = {
        let mut guard = state.oauth.lock().unwrap();
        let s = &mut *guard;
        let Some(signin) = s.signin.as_mut() else {
            return StatusCode::NOT_FOUND.into_response();
        };
        let replaced = read_cookie(&headers, SESSION_COOKIE).map(|v| hash_of(&v));
        signin
            .users
            .insert(identity.id.clone(), identity.name.clone());
        let (token, writes) = signin.open_session(&identity.id, replaced);
        let session = hash_of(&token);
        let response = s.continue_request(&flow.request, &identity.id, &session);
        (response, token, writes)
    };
    writes.insert(
        0,
        Write::User {
            id: identity.id,
            name: identity.name,
        },
    );
    persist(writes).await;
    eprintln!("[OAuth] sign-in succeeded provider={}", provider.slug());
    with_cookie(
        response,
        cookie_header(SESSION_COOKIE, &token, SESSION_TTL_SECS),
    )
}

/// `GET /oauth/cancel`: give up on a failed sign-in and tell the client, as RFC 6749
/// section 4.1.2.1 asks. The web app then offers **Sign in again**.
pub(super) async fn cancel(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Query(q): Query<RequestRef>,
) -> Response {
    let mut guard = state.oauth.lock().unwrap();
    let Some(signin) = guard.signin.as_mut() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    signin.cleanup();
    if signin
        .pending_for(&q.request, browser_of(&headers).as_deref())
        .is_none()
    {
        return expired_page();
    }
    let Some(request) = signin.pending.remove(&q.request) else {
        return expired_page();
    };
    deny_redirect(&request)
}

fn deny_redirect(request: &PendingRequest) -> Response {
    match client_redirect(
        &request.redirect_uri,
        &[("error", "access_denied")],
        request.state.as_deref(),
    ) {
        Some(target) => Redirect::to(&target).into_response(),
        None => expired_page(),
    }
}

/// `GET /oauth/consent`: shown to a signed-in user for a client they never approved.
pub(super) async fn consent_form(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Query(q): Query<RequestRef>,
) -> Response {
    let mut guard = state.oauth.lock().unwrap();
    let s = &mut *guard;
    let Some(signin) = s.signin.as_mut() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    signin.cleanup();
    if signin
        .pending_for(&q.request, browser_of(&headers).as_deref())
        .is_none()
    {
        return expired_page();
    }
    let Some((session, user)) = signin.session_of(&headers) else {
        return Redirect::to(&format!("/oauth/signin?request={}", q.request)).into_response();
    };
    let user_name = signin
        .users
        .get(&user)
        .cloned()
        .unwrap_or_else(|| user.clone());
    let csrf = random_token();
    let Some(request) = signin.pending.get_mut(&q.request) else {
        return expired_page();
    };
    request.consent = Some((session, csrf.clone()));
    let (client_id, redirect_uri) = (request.client_id.clone(), request.redirect_uri.clone());
    let Some(client) = s
        .clients
        .get(&client_id)
        .filter(|c| c.redirect_uris.contains(&redirect_uri))
    else {
        return expired_page();
    };
    let name = client.name.as_deref().unwrap_or("Unnamed application");
    consent_page(&q.request, &csrf, name, &redirect_uri, &user_name)
}

#[derive(Deserialize)]
pub(super) struct ConsentForm {
    request: String,
    csrf: String,
    decision: String,
}

/// `POST /oauth/consent`: the decision. It counts only from the session the page was
/// shown to, in the browser that started the request, with the page's token.
pub(super) async fn consent_decision(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Form(form): Form<ConsentForm>,
) -> Response {
    let (response, writes) = {
        let mut guard = state.oauth.lock().unwrap();
        let s = &mut *guard;
        let Some(signin) = s.signin.as_mut() else {
            return StatusCode::NOT_FOUND.into_response();
        };
        signin.cleanup();
        if !same_origin(&headers) {
            return expired_page();
        }
        let Some((session, user)) = signin.session_of(&headers) else {
            return expired_page();
        };
        let matches = signin
            .pending_for(&form.request, browser_of(&headers).as_deref())
            .and_then(|p| p.consent.as_ref())
            .is_some_and(|(shown_to, csrf)| *shown_to == session && *csrf == form.csrf);
        if !matches {
            return expired_page();
        }
        let Some(request) = signin.pending.remove(&form.request) else {
            return expired_page();
        };
        if form.decision != "approve" {
            eprintln!("[OAuth] consent denied");
            return deny_redirect(&request);
        }
        let writes = signin.add_grant(&user, &request.client_id);
        let registered = s
            .clients
            .get(&request.client_id)
            .is_some_and(|c| c.redirect_uris.contains(&request.redirect_uri));
        let response = if !registered {
            unknown_client_answer(&request.redirect_uri, request.state.as_deref())
        } else {
            match s.issue_code(&request, &user, &session) {
                Some(target) => Redirect::to(&target).into_response(),
                None => expired_page(),
            }
        };
        (response, writes)
    };
    persist(writes).await;
    response
}

/// `POST /oauth/logout`: end this browser's session and the web app's tokens from it.
/// Tokens given to other clients keep working until they expire: signing out of the
/// web app does not disconnect a connector approved on purpose.
pub(super) async fn logout(State(state): State<SharedState>, headers: HeaderMap) -> Response {
    if !same_origin(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let hash = read_cookie(&headers, SESSION_COOKIE).map(|v| hash_of(&v));
    {
        let mut guard = state.oauth.lock().unwrap();
        let s = &mut *guard;
        let Some(signin) = s.signin.as_mut() else {
            return StatusCode::NOT_FOUND.into_response();
        };
        if let Some(hash) = &hash {
            signin.sessions.remove(hash);
            let ours = |u: &Option<TokenUser>| {
                u.as_ref()
                    .is_some_and(|u| u.session.as_deref() == Some(hash.as_str()))
            };
            s.tokens.retain(|_, t| !ours(&t.user));
            s.codes.retain(|_, c| !ours(&c.user));
        }
    }
    persist(hash.map(Write::DeleteSession).into_iter().collect()).await;
    eprintln!("[OAuth] signed out");
    with_cookie(
        StatusCode::NO_CONTENT.into_response(),
        cookie_header(SESSION_COOKIE, "", 0),
    )
}

/// `GET /oauth/signed-out`: where the web app goes after signing out. Going straight
/// back to the app would sign the user in again: with one provider there is no chooser
/// in between, and the provider remembers them.
pub(super) async fn signed_out(State(state): State<SharedState>) -> Response {
    if state.oauth.lock().unwrap().signin.is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    page(
        StatusCode::OK,
        "You are signed out",
        "<p>Your session in this browser has ended.</p><a class=\"b p\" href=\"/\">Sign in again</a>",
    )
}

/// `GET /oauth/me`: who a bearer token belongs to, for the web app's header.
pub(super) async fn me(State(state): State<SharedState>, headers: HeaderMap) -> Response {
    let mut s = state.oauth.lock().unwrap();
    if s.signin.is_none() {
        return Json(json!({ "sign_in": false, "user": null })).into_response();
    }
    let user = extract_bearer_token(&headers).and_then(|t| s.user_for_token(t));
    match user.and_then(|u| s.user_profile(&u)) {
        Some((name, provider)) => {
            Json(json!({ "sign_in": true, "user": { "name": name, "provider": provider } }))
                .into_response()
        }
        None => (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "sign_in": true, "user": null })),
        )
            .into_response(),
    }
}

// --- Asking the provider who this is ----------------------------------------

struct Identity {
    id: String,
    name: String,
}

enum ProviderFailure {
    Unreachable,
    Refused(String),
    Unusable(&'static str),
}

impl ProviderFailure {
    fn message(&self, label: &str) -> String {
        match self {
            ProviderFailure::Unreachable => {
                format!("{label} could not be reached. Try again in a moment.")
            }
            ProviderFailure::Refused(_) => format!("{label} refused this sign-in. Try again."),
            ProviderFailure::Unusable(_) => {
                format!("{label} sent an answer chartgen cannot use. Try again.")
            }
        }
    }

    /// For the log: never carries a code, token or secret.
    fn log_reason(&self) -> String {
        match self {
            ProviderFailure::Unreachable => "unreachable".to_string(),
            ProviderFailure::Refused(why) => format!("refused({why})"),
            ProviderFailure::Unusable(why) => format!("unusable({why})"),
        }
    }
}

fn from_request_error(e: reqwest::Error) -> ProviderFailure {
    if e.is_timeout() || e.is_connect() {
        ProviderFailure::Unreachable
    } else {
        ProviderFailure::Unusable("transport")
    }
}

#[derive(Deserialize)]
struct ProviderToken {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

/// Exchange the code server-side and read the user over TLS. The provider's token
/// is used for this one call and then dropped.
async fn fetch_identity(
    provider: &ProviderConfig,
    http: &reqwest::Client,
    code: &str,
    verifier: &str,
) -> Result<Identity, ProviderFailure> {
    let callback = callback_url(provider.kind);
    let resp = http
        .post(&provider.token_url)
        .header(header::ACCEPT, "application/json")
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", provider.client_id.as_str()),
            ("client_secret", provider.client_secret.as_str()),
            ("code", code),
            ("redirect_uri", callback.as_str()),
            ("code_verifier", verifier),
        ])
        .send()
        .await
        .map_err(from_request_error)?;
    let status = resp.status();
    // GitHub reports a bad code with a 200 and an `error` field.
    let token: ProviderToken = resp
        .json()
        .await
        .map_err(|_| ProviderFailure::Unusable("token answer"))?;
    if let Some(error) = token.error {
        let error: String = error
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
            .take(64)
            .collect();
        return Err(ProviderFailure::Refused(error));
    }
    let Some(access_token) = token.access_token.filter(|_| status.is_success()) else {
        return Err(ProviderFailure::Refused(format!(
            "status {}",
            status.as_u16()
        )));
    };
    let resp = http
        .get(&provider.userinfo_url)
        .bearer_auth(access_token)
        .header(header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(from_request_error)?;
    if !resp.status().is_success() {
        return Err(ProviderFailure::Refused(format!(
            "userinfo status {}",
            resp.status().as_u16()
        )));
    }
    let profile: Value = resp
        .json()
        .await
        .map_err(|_| ProviderFailure::Unusable("userinfo answer"))?;
    identity_from_profile(provider.kind, &profile).ok_or(ProviderFailure::Unusable("no user id"))
}

/// The stable key is the provider plus its immutable user id, never a login or an
/// email. Names are display only; an email only counts as one when verified.
fn identity_from_profile(provider: Provider, profile: &Value) -> Option<Identity> {
    let text = |key: &str| {
        profile[key]
            .as_str()
            .map(str::trim)
            .filter(|v| !v.is_empty())
    };
    let (key, name) = match provider {
        Provider::GitHub => (
            profile["id"].as_u64().filter(|id| *id > 0)?.to_string(),
            text("login"),
        ),
        Provider::Google => {
            let email = text("email").filter(|_| profile["email_verified"].as_bool() == Some(true));
            (text("sub")?.to_string(), text("name").or(email))
        }
    };
    let name = name
        .unwrap_or(provider.label())
        .chars()
        .take(MAX_NAME_CHARS)
        .collect();
    Some(Identity {
        id: format!("{}:{key}", provider.slug()),
        name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(vars: &[(&str, &str)]) -> Result<Option<Config>, String> {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Config::from_vars(|name| vars.get(name).cloned())
    }

    #[test]
    fn nothing_configured_is_off_and_half_a_provider_is_an_error() {
        assert!(config(&[]).unwrap().is_none());
        let id_only = config(&[("CHARTGEN_GITHUB_CLIENT_ID", "x")]).err().unwrap();
        assert!(
            id_only.contains("CHARTGEN_GITHUB_CLIENT_SECRET"),
            "{id_only}"
        );
        let secret_only = config(&[("CHARTGEN_GOOGLE_CLIENT_SECRET", "hunter2")])
            .err()
            .unwrap();
        assert!(
            secret_only.contains("CHARTGEN_GOOGLE_CLIENT_ID"),
            "{secret_only}"
        );
        assert!(
            !secret_only.contains("hunter2"),
            "a secret must not reach the log"
        );
    }

    #[test]
    fn one_or_two_providers_use_the_real_endpoints() {
        let c = config(&[
            ("CHARTGEN_GOOGLE_CLIENT_ID", "g"),
            ("CHARTGEN_GOOGLE_CLIENT_SECRET", "gs"),
        ])
        .unwrap()
        .unwrap();
        assert_eq!(c.provider_labels(), "Google");
        assert!(c
            .provider("google")
            .unwrap()
            .token_url
            .starts_with("https://oauth2.googleapis.com/"));
        assert!(c.provider("github").is_none());
        let c = config(&[
            ("CHARTGEN_GOOGLE_CLIENT_ID", "g"),
            ("CHARTGEN_GOOGLE_CLIENT_SECRET", "gs"),
            ("CHARTGEN_GITHUB_CLIENT_ID", "h"),
            ("CHARTGEN_GITHUB_CLIENT_SECRET", "hs"),
        ])
        .unwrap()
        .unwrap();
        assert_eq!(c.provider_labels(), "GitHub, Google");
        assert!(c
            .provider("github")
            .unwrap()
            .authorize_url
            .starts_with("https://github.com/"));
    }

    #[test]
    fn the_test_provider_needs_the_sample_source_and_a_loopback_origin() {
        let on = |raw, sample| test_provider_origin(Some(raw), sample);
        assert_eq!(
            on("http://127.0.0.1:4000/x", true).as_deref(),
            Some("http://127.0.0.1:4000")
        );
        assert_eq!(
            on("http://127.0.0.1:4000", false),
            None,
            "live data ignores it"
        );
        for raw in [
            "http://10.0.0.1:4000",
            "http://example.com",
            "https://127.0.0.1:4000",
            "http://localhost:4000",
            "http://127.0.0.1.example.com",
            "not a url",
        ] {
            assert_eq!(on(raw, true), None, "{raw}");
        }
        // The whole way through the config too: on live data the real endpoints stay.
        let c = config(&[
            ("CHARTGEN_GITHUB_CLIENT_ID", "h"),
            ("CHARTGEN_GITHUB_CLIENT_SECRET", "hs"),
            (
                "CHARTGEN_SIGNIN_TEST_PROVIDER_ORIGIN",
                "http://127.0.0.1:4000",
            ),
        ])
        .unwrap()
        .unwrap();
        assert!(c
            .provider("github")
            .unwrap()
            .token_url
            .starts_with("https://github.com/"));
    }

    #[test]
    fn the_first_party_client_is_exactly_the_own_root() {
        const BASE: &str = "https://chartgen.example";
        assert!(is_first_party(BASE, "https://chartgen.example/"));
        assert!(is_first_party(BASE, "https://CHARTGEN.example:443/"));
        for uri in [
            "https://chartgen.example/cb",
            "https://chartgen.example/?x=1",
            "https://chartgen.example/#f",
            "https://user@chartgen.example/",
            "https://chartgen.example.evil.com/",
            "https://chartgen.example@evil.com/",
            "http://chartgen.example/",
            "https://chartgen.example:8443/",
            "cursor://chartgen.example/",
            "not a url",
        ] {
            assert!(!is_first_party(BASE, uri), "{uri}");
        }
    }

    #[test]
    fn identity_is_the_provider_and_its_immutable_id() {
        let github = identity_from_profile(
            Provider::GitHub,
            &json!({"id": 42, "login": "octo", "email": "a@b.c"}),
        )
        .unwrap();
        assert_eq!(
            (github.id.as_str(), github.name.as_str()),
            ("github:42", "octo")
        );
        // A renamed account is the same user; a login alone is nobody.
        let renamed =
            identity_from_profile(Provider::GitHub, &json!({"id": 42, "login": "new"})).unwrap();
        assert_eq!(renamed.id, github.id);
        assert!(identity_from_profile(Provider::GitHub, &json!({"login": "octo"})).is_none());
        assert!(identity_from_profile(Provider::GitHub, &json!({"id": "42"})).is_none());
        assert!(identity_from_profile(Provider::GitHub, &json!({"id": 0})).is_none());

        let google =
            identity_from_profile(Provider::Google, &json!({"sub": "abc", "name": "Ann"})).unwrap();
        assert_eq!(
            (google.id.as_str(), google.name.as_str()),
            ("google:abc", "Ann")
        );
        assert!(identity_from_profile(
            Provider::Google,
            &json!({"email": "a@b.c", "email_verified": true})
        )
        .is_none());
        // An email is only shown when the provider verified it.
        let verified = identity_from_profile(
            Provider::Google,
            &json!({"sub": "abc", "email": "a@b.c", "email_verified": true}),
        )
        .unwrap();
        assert_eq!(verified.name, "a@b.c");
        let unverified = identity_from_profile(
            Provider::Google,
            &json!({"sub": "abc", "email": "a@b.c", "email_verified": false}),
        )
        .unwrap();
        assert_eq!(unverified.name, "Google");
    }

    #[test]
    fn pages_escape_what_they_are_given() {
        assert_eq!(escape("<script>\"'&"), "&lt;script&gt;&quot;&#39;&amp;");
        assert_eq!(
            redirect_target_label("https://evil.example:8443/cb"),
            "evil.example"
        );
        assert_eq!(
            redirect_target_label("cursor://anysphere.cursor/cb"),
            "anysphere.cursor"
        );
        assert_eq!(redirect_target_label("my-app:/cb"), "my-app:");
    }

    fn store_with(client: &str, redirect_uri: &str) -> OAuthStore {
        let c = config(&[
            ("CHARTGEN_GITHUB_CLIENT_ID", "h"),
            ("CHARTGEN_GITHUB_CLIENT_SECRET", "hs"),
        ])
        .unwrap()
        .unwrap();
        let mut s = OAuthStore::new();
        s.signin = Some(SignIn::new(c));
        s.clients.insert(
            client.to_string(),
            OAuthClient {
                _client_secret: None,
                redirect_uris: vec![redirect_uri.to_string()],
                created_at: Instant::now(),
                last_used: None,
                name: None,
            },
        );
        s
    }

    fn pending(client: &str, redirect_uri: &str) -> PendingRequest {
        PendingRequest {
            client_id: client.to_string(),
            redirect_uri: redirect_uri.to_string(),
            code_challenge: "c".to_string(),
            code_challenge_method: "S256".to_string(),
            state: Some("st".to_string()),
            browser: "b".to_string(),
            user: None,
            own_app: false,
            created_at: Instant::now(),
            consent: None,
        }
    }

    /// The request is checked again on return: a client evicted while its user was at
    /// the provider, or one no longer listing the redirect, gets no code.
    #[test]
    fn a_request_is_validated_again_after_the_provider() {
        const URI: &str = "https://client.example/cb";
        let mut s = store_with("c1", URI);
        s.signin.as_mut().unwrap().add_grant("github:1", "c1");
        let put = |s: &mut OAuthStore, client: &str, uri: &str| {
            let signin = s.signin.as_mut().unwrap();
            signin.pending.insert("p".to_string(), pending(client, uri));
        };

        put(&mut s, "gone", URI);
        let resp = s.continue_request("p", "github:1", "sess");
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert!(s.codes.is_empty());
        assert!(s.signin.as_ref().unwrap().pending.is_empty());

        put(&mut s, "c1", "https://evil.example/cb");
        let resp = s.continue_request("p", "github:1", "sess");
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert!(s.codes.is_empty());

        // Control: the same request with a good client gets its code, bound to the
        // user, with no session on it because this is not the web app.
        put(&mut s, "c1", URI);
        let resp = s.continue_request("p", "github:1", "sess");
        assert!(resp.status().is_redirection());
        let code = s.codes.values().next().unwrap();
        let user = code.user.as_ref().unwrap();
        assert_eq!(user.id, "github:1");
        assert_eq!(user.session, None);

        // A user without the approval goes to consent, and the request stays for it.
        put(&mut s, "c1", URI);
        let resp = s.continue_request("p", "github:2", "sess");
        let location = resp.headers()[header::LOCATION].to_str().unwrap();
        assert_eq!(location, "/oauth/consent?request=p");
        assert!(s.signin.as_ref().unwrap().pending.contains_key("p"));
    }

    /// ADR-0036 keeps working with users: a live token pins its client, and the
    /// per-user cap is what releases one, so one account cannot pin the table.
    #[test]
    fn the_client_cap_makes_a_users_oldest_client_evictable() {
        let mut s = OAuthStore::new();
        let t0 = Instant::now();
        for i in 0..MAX_CLIENTS {
            s.clients.insert(
                format!("c{i}"),
                OAuthClient {
                    _client_secret: None,
                    redirect_uris: Vec::new(),
                    created_at: t0,
                    last_used: Some(t0),
                    name: None,
                },
            );
        }
        let token_for = |s: &mut OAuthStore, i: usize, user: &str, age: u64| {
            let token = format!("t{i}");
            s.tokens.insert(
                token.clone(),
                AccessToken {
                    _token: token,
                    client_id: format!("c{i}"),
                    created_at: t0 - Duration::from_secs(age),
                    expires_in: Duration::from_secs(3600),
                    user: Some(TokenUser {
                        id: user.to_string(),
                        session: None,
                        last_used: t0 - Duration::from_secs(age),
                    }),
                },
            );
        };
        // One account holds a token on every client: all are pinned, as without a cap.
        for i in 0..MAX_CLIENTS {
            token_for(&mut s, i, "github:1", (MAX_CLIENTS - i) as u64);
        }
        assert!(s.make_room(|| Some(HashSet::new())).is_err(), "all live");
        s.trim_user_tokens("github:1");
        assert_eq!(s.tokens.len(), MAX_LIVE_CLIENTS_PER_USER);
        let keeps = |s: &OAuthStore, i: usize| s.tokens.contains_key(&format!("t{i}"));
        assert!(keeps(&s, MAX_CLIENTS - 1), "the newest stays");
        assert!(!keeps(&s, 0), "the oldest goes");
        let evicted = s.make_room(|| Some(HashSet::new())).unwrap();
        assert_eq!(evicted.len(), 1);
        let live: HashSet<&str> = s.tokens.values().map(|t| t.client_id.as_str()).collect();
        assert!(!live.contains(evicted[0].as_str()), "a live client stays");

        // Another user's tokens do not count against this one's.
        token_for(&mut s, 0, "github:2", 5000);
        s.trim_user_tokens("github:1");
        assert!(keeps(&s, 0));
    }

    /// The tokens a client keeps are the ones in use, and the cap is the client's kind.
    #[test]
    fn the_longest_out_of_use_tokens_go_first() {
        let mut s = OAuthStore::new();
        let t0 = Instant::now();
        let add = |s: &mut OAuthStore, token: String, session: Option<&str>, age: u64| {
            s.tokens.insert(
                token.clone(),
                AccessToken {
                    _token: token,
                    client_id: if session.is_some() { "web" } else { "mcp" }.to_string(),
                    created_at: t0,
                    expires_in: Duration::from_secs(3600),
                    user: Some(TokenUser {
                        id: "github:1".to_string(),
                        session: session.map(str::to_string),
                        last_used: t0 - Duration::from_secs(age),
                    }),
                },
            );
        };
        for i in 0..MAX_TOKENS_PER_CLIENT + 1 {
            add(&mut s, format!("mcp-{i}"), None, 100 + i as u64);
        }
        for i in 0..MAX_TOKENS_PER_WEB_CLIENT + 1 {
            add(&mut s, format!("web-{i}"), Some("sess"), 1000 + i as u64);
        }
        s.trim_user_tokens("github:1");
        let gone = MAX_TOKENS_PER_CLIENT;
        assert!(
            !s.tokens.contains_key(&format!("mcp-{gone}")),
            "out of use longest"
        );
        assert!(s.tokens.contains_key("mcp-0"));
        let gone = MAX_TOKENS_PER_WEB_CLIENT;
        assert!(!s.tokens.contains_key(&format!("web-{gone}")));
        assert!(
            s.tokens.contains_key(&format!("web-{}", gone - 1)),
            "a roomier cap"
        );
    }

    #[test]
    fn a_browser_and_a_user_hold_few_requests_and_the_oldest_goes() {
        let mut s = store_with("c1", "https://client.example/cb");
        let signin = s.signin.as_mut().unwrap();
        let t0 = Instant::now();
        let add = |signin: &mut SignIn, id: &str, browser: &str, user: Option<&str>, age: u64| {
            let mut p = pending("c1", "https://client.example/cb");
            p.browser = browser.to_string();
            p.user = user.map(str::to_string);
            p.created_at = t0 - Duration::from_secs(age);
            signin.pending.insert(id.to_string(), p);
        };
        for i in 0..MAX_PENDING_PER_BROWSER {
            add(signin, &format!("b{i}"), "b", None, 100 - i as u64);
        }
        add(signin, "other", "another-browser", None, 200);
        signin.flows.insert(
            "state-of-b0".to_string(),
            ProviderFlow {
                provider: Provider::GitHub,
                request: "b0".to_string(),
                browser: "b".to_string(),
                verifier: String::new(),
                created_at: t0,
            },
        );
        signin.make_room_for_request("b", None);
        assert!(!signin.pending.contains_key("b0"), "the oldest goes");
        assert!(signin.flows.is_empty(), "with its round trip");
        assert!(signin.pending.contains_key("b1"));
        assert!(
            signin.pending.contains_key("other"),
            "another browser's stays"
        );

        // Leaving the browser cookie out buys a signed-in user nothing.
        for i in 0..MAX_PENDING_PER_BROWSER {
            add(
                signin,
                &format!("u{i}"),
                &format!("fresh-{i}"),
                Some("github:1"),
                50 - i as u64,
            );
        }
        signin.make_room_for_request("fresh-again", Some("github:1"));
        assert!(!signin.pending.contains_key("u0"));
        assert!(signin.pending.contains_key("u1"));
    }

    #[test]
    fn a_round_trip_does_not_outlive_its_request() {
        let mut s = store_with("c1", "https://client.example/cb");
        let signin = s.signin.as_mut().unwrap();
        signin.flows.insert(
            "orphan".to_string(),
            ProviderFlow {
                provider: Provider::GitHub,
                request: "gone".to_string(),
                browser: "b".to_string(),
                verifier: String::new(),
                created_at: Instant::now(),
            },
        );
        signin.cleanup();
        assert!(signin.flows.is_empty());
    }

    #[cfg(feature = "scripting")]
    #[test]
    fn the_notice_counts_rows_and_says_how_to_collect_them() {
        assert_eq!(waiting_rows_notice(0, 0, None), None);
        let unset = waiting_rows_notice(2, 1, None).unwrap();
        assert!(
            unset.contains("2 script(s) and 1 server alert(s)"),
            "{unset}"
        );
        assert!(
            unset.contains("CHARTGEN_CLAIM_OWNER=github:<id>"),
            "{unset}"
        );
        let set = waiting_rows_notice(2, 1, Some("github:7")).unwrap();
        assert!(!set.contains("github:7"), "no ids in the log: {set}");
    }

    #[test]
    fn an_allowlist_with_sign_in_on_names_accounts() {
        assert_eq!(allowlist_entries_not_accounts(""), 0);
        assert_eq!(allowlist_entries_not_accounts("*"), 0);
        assert_eq!(allowlist_entries_not_accounts("github:1, google:abc"), 0);
        assert_eq!(
            allowlist_entries_not_accounts("github:1,3f2a-client-id, gitlab:9"),
            2
        );
    }

    #[test]
    fn an_expired_session_is_nobody() {
        let c = config(&[
            ("CHARTGEN_GITHUB_CLIENT_ID", "h"),
            ("CHARTGEN_GITHUB_CLIENT_SECRET", "hs"),
        ])
        .unwrap()
        .unwrap();
        let mut s = SignIn::new(c);
        let (token, _) = s.open_session("github:1", None);
        let mut headers = HeaderMap::new();
        let cookie = format!("{SESSION_COOKIE}={token}");
        headers.insert(header::COOKIE, cookie.parse().unwrap());
        assert_eq!(s.session_of(&headers).unwrap().1, "github:1");
        s.sessions.get_mut(&hash_of(&token)).unwrap().expires_at = now_unix() - 1;
        assert!(s.session_of(&headers).is_none());
        assert!(s.sessions.is_empty(), "and it is forgotten");
    }

    #[test]
    fn opening_a_session_replaces_the_old_one_and_keeps_to_the_limit() {
        let c = config(&[
            ("CHARTGEN_GITHUB_CLIENT_ID", "h"),
            ("CHARTGEN_GITHUB_CLIENT_SECRET", "hs"),
        ])
        .unwrap()
        .unwrap();
        let mut s = SignIn::new(c);
        let (first, _) = s.open_session("github:1", None);
        let (_, writes) = s.open_session("github:1", Some(hash_of(&first)));
        assert_eq!(
            s.sessions.len(),
            1,
            "a new login replaces the session it came with"
        );
        assert!(matches!(&writes[0], Write::DeleteSession(h) if *h == hash_of(&first)));
        for _ in 0..MAX_SESSIONS_PER_USER + 3 {
            s.open_session("github:1", None);
        }
        assert_eq!(s.sessions.len(), MAX_SESSIONS_PER_USER);
        s.open_session("github:2", None);
        assert_eq!(
            s.sessions.len(),
            MAX_SESSIONS_PER_USER + 1,
            "others are not touched"
        );
    }

    #[test]
    fn login_hits_are_limited_per_address_except_loopback() {
        let c = config(&[
            ("CHARTGEN_GITHUB_CLIENT_ID", "h"),
            ("CHARTGEN_GITHUB_CLIENT_SECRET", "hs"),
        ])
        .unwrap()
        .unwrap();
        let mut s = SignIn::new(c);
        let t0 = Instant::now();
        for _ in 0..MAX_LOGIN_HITS_PER_IP {
            assert!(s.admit_login("203.0.113.9", t0));
        }
        assert!(!s.admit_login("203.0.113.9", t0), "over the limit");
        assert!(
            s.admit_login("203.0.113.10", t0),
            "another address has its own"
        );
        for _ in 0..MAX_LOGIN_HITS_PER_IP + 5 {
            assert!(s.admit_login("127.0.0.1", t0));
        }
        assert!(
            s.admit_login("203.0.113.9", t0 + LOGIN_WINDOW),
            "the window passes"
        );
    }
}
