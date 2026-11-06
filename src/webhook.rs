//! Outbound webhooks for server alerts (ADR-0042).
//!
//! The URL comes from a stranger (registration is open, ADR-0036) and the
//! server sits on a home network, so a request only leaves for a public
//! address on port 443 over https. The name is resolved here, every address it
//! gives must be public, and the connection is pinned to those addresses, so a
//! rebind between the check and the request changes nothing. Redirects are not
//! followed. The response body is never read and the status is all that is kept.
//! A name with no dot, a host this server fetches from and its own host are refused
//! too, and whether a name failed to resolve or resolved to a private address reads
//! the same, so the answer says nothing about the network inside.
//! Error texts are fixed strings, because a request error prints its URL, which
//! may hold a token.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use hmac::{Hmac, Mac};
use reqwest::{Client, Url};
use sha2::Sha256;
use tokio::sync::Semaphore;

/// Longest webhook URL accepted.
pub const MAX_URL_LEN: usize = 512;
/// How long a name may take to resolve.
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(4);
/// Names that only mean something inside a network.
const LOCAL_NAMES: [&str; 12] = [
    "localhost",
    "localdomain",
    "local",
    "internal",
    "intranet",
    "lan",
    "home",
    "home.arpa",
    "corp",
    "docker",
    "test",
    "invalid",
];
/// Where this server gets its data: a webhook at them spends its standing there.
const UPSTREAM_DOMAINS: [&str; 4] = [
    "binance.com",
    "binance.vision",
    "yahoo.com",
    "alternative.me",
];

pub const SIGNATURE_HEADER: &str = "X-Chartgen-Signature";
pub const TIMESTAMP_HEADER: &str = "X-Chartgen-Timestamp";
pub const EVENT_HEADER: &str = "X-Chartgen-Event";

/// Longest push endpoint accepted: Windows' carry a long token.
pub const MAX_PUSH_URL_LEN: usize = 1024;
/// Where a browser's push subscription may point (ADR-0045).
const PUSH_HOSTS: [&str; 2] = ["fcm.googleapis.com", "updates.push.services.mozilla.com"];
/// The same, for services that hand out subdomains.
const PUSH_DOMAINS: [&str; 2] = [".push.apple.com", ".notify.windows.com"];
/// The longest a push service may ask a retry to wait: past this, its word is taken and
/// the delivery ends, instead of holding a task for it.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(60);

/// Is `ip` an address a webhook may reach? IPv6 must be global unicast, which
/// leaves out loopback, link-local, unique-local, multicast and the IPv4-mapped,
/// IPv4-compatible and NAT64 forms; the transition ranges inside it are refused
/// too, because they lead to an IPv4 address nobody checked.
pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, d] = ip.octets();
    !(a == 0                                    // "this network"
        || a == 10                              // private
        || a == 127                             // loopback
        || a >= 224                             // multicast, reserved, broadcast
        || (a == 100 && (64..128).contains(&b)) // CGNAT, Tailscale, Alibaba metadata
        || (a == 169 && b == 254)               // link-local, cloud metadata
        || (a == 172 && (16..32).contains(&b))  // private
        || (a == 192 && b == 168)               // private
        || (a == 192 && b == 0 && c == 0)       // IETF, Oracle metadata
        || (a == 192 && b == 0 && c == 2)       // documentation
        || (a == 192 && b == 88 && c == 99)     // 6to4 relay
        || (a == 198 && (b == 18 || b == 19))   // benchmarking
        || (a == 198 && b == 51 && c == 100)    // documentation
        || (a == 203 && b == 0 && c == 113)     // documentation
        || (a == 168 && b == 63 && c == 129 && d == 16)) // Azure metadata
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    let s = ip.segments();
    let global_unicast = s[0] & 0xe000 == 0x2000;
    global_unicast
        && !(s[0] == 0x2001 && s[1] < 0x0200)   // IETF: Teredo, benchmarking, ORCHID
        && !(s[0] == 0x2001 && s[1] == 0x0db8)  // documentation
        && s[0] != 0x2002                       // 6to4
        && !(s[0] == 0x3fff && s[1] < 0x1000) // documentation
}

/// An address range: `203.0.113.7/32`, `2001:db8::/48`, or one address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Prefix {
    net: IpAddr,
    len: u32,
}

impl Prefix {
    fn parse(raw: &str) -> Option<Self> {
        let (ip, len) = match raw.split_once('/') {
            Some((ip, len)) => (ip.parse().ok()?, Some(len.parse().ok()?)),
            None => (raw.parse().ok()?, None),
        };
        let max = if matches!(ip, IpAddr::V4(_)) { 32 } else { 128 };
        let len = len.unwrap_or(max);
        (len <= max).then_some(Self { net: ip, len })
    }

    fn contains(&self, ip: IpAddr) -> bool {
        match (self.net, ip) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => {
                let mask = u32::MAX.checked_shl(32 - self.len).unwrap_or(0);
                u32::from(net) & mask == u32::from(ip) & mask
            }
            (IpAddr::V6(net), IpAddr::V6(ip)) => {
                let mask = u128::MAX.checked_shl(128 - self.len).unwrap_or(0);
                u128::from(net) & mask == u128::from(ip) & mask
            }
            _ => false,
        }
    }
}

/// What the operator set, read once from the environment at startup: no request
/// can change any of it.
#[derive(Clone, Debug, Default)]
pub struct Policy {
    /// The origin of a local test receiver, which may be reached besides public
    /// addresses. It only counts with the sample data source (`from_env`).
    pub test_origin: Option<String>,
    /// This server's own host (`CHARTGEN_BASE_URL`).
    pub own_host: Option<String>,
    /// Public addresses that are still off limits, such as this network's own.
    pub blocked: Vec<Prefix>,
    /// Send webhooks through this proxy, so that a receiver never sees this
    /// server's address. Unset means direct, ignoring any proxy in the environment.
    pub proxy: Option<String>,
}

impl Policy {
    /// `CHARTGEN_WEBHOOK_TEST_ORIGIN` (honored only with `CHARTGEN_DATA_SOURCE=sample`
    /// and only for a loopback address), `CHARTGEN_BASE_URL`,
    /// `CHARTGEN_WEBHOOK_BLOCKED_PREFIXES` and `CHARTGEN_WEBHOOK_PROXY`. A security
    /// setting that does not parse is an error: nothing falls back to no protection.
    pub fn from_env() -> Result<Self, String> {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        let sample = std::env::var("CHARTGEN_DATA_SOURCE").is_ok_and(|v| v == "sample");
        let mut policy =
            Self::for_test_origin(var("CHARTGEN_WEBHOOK_TEST_ORIGIN").as_deref(), sample);
        policy.own_host = var("CHARTGEN_BASE_URL")
            .and_then(|u| Url::parse(&u).ok()?.host_str().map(str::to_ascii_lowercase));
        policy.blocked = parse_prefixes(var("CHARTGEN_WEBHOOK_BLOCKED_PREFIXES").as_deref())?;
        policy.proxy = parse_proxy(var("CHARTGEN_WEBHOOK_PROXY"))?;
        Ok(policy)
    }

    fn for_test_origin(raw: Option<&str>, sample: bool) -> Self {
        let test_origin = raw.filter(|_| sample).and_then(|raw| {
            let url = Url::parse(raw).ok()?;
            let loopback = matches!(url.host(), Some(url::Host::Ipv4(ip)) if ip.is_loopback());
            loopback.then(|| url.origin().ascii_serialization())
        });
        if let Some(origin) = &test_origin {
            eprintln!("[webhook] test receiver allowed at {origin}");
        }
        Self {
            test_origin,
            ..Self::default()
        }
    }

    fn allows(&self, url: &Url) -> bool {
        self.test_origin.as_deref() == Some(url.origin().ascii_serialization().as_str())
    }

    fn permits(&self, ip: IpAddr) -> bool {
        is_public_ip(ip) && !self.blocked.iter().any(|p| p.contains(ip))
    }
}

fn parse_prefixes(raw: Option<&str>) -> Result<Vec<Prefix>, String> {
    raw.unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| {
            Prefix::parse(p)
                .ok_or_else(|| format!("CHARTGEN_WEBHOOK_BLOCKED_PREFIXES: cannot read {p}"))
        })
        .collect()
}

/// The error never repeats the value: a proxy URL can hold a password.
fn parse_proxy(raw: Option<String>) -> Result<Option<String>, String> {
    let Some(raw) = raw else { return Ok(None) };
    let http = Url::parse(&raw).is_ok_and(|u| matches!(u.scheme(), "http" | "https"));
    if http && reqwest::Proxy::all(&raw).is_ok() {
        Ok(Some(raw))
    } else {
        Err("CHARTGEN_WEBHOOK_PROXY must be an http or https URL".into())
    }
}

/// A webhook URL that passed every check that needs no DNS.
#[derive(Clone)]
pub struct Target {
    url: Url,
}

impl Target {
    /// Refuse what is wrong with the URL itself. The reason never names more
    /// of the URL than its scheme or port.
    pub fn parse(raw: &str, policy: &Policy) -> Result<Self, String> {
        if raw.len() > MAX_URL_LEN {
            return Err(format!("webhook URL is longer than {MAX_URL_LEN} bytes"));
        }
        let url = Url::parse(raw).map_err(|_| "webhook URL does not parse".to_string())?;
        if policy.allows(&url) {
            return Ok(Self { url });
        }
        if url.scheme() != "https" {
            return Err("webhook URL must use https".into());
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err("webhook URL must not hold a user or password".into());
        }
        if url.port_or_known_default() != Some(443) {
            return Err("webhook URL must use port 443".into());
        }
        match url.host() {
            Some(url::Host::Ipv4(ip)) if !policy.permits(ip.into()) => Err(REFUSED_ADDRESS.into()),
            Some(url::Host::Ipv6(ip)) if !policy.permits(ip.into()) => Err(REFUSED_ADDRESS.into()),
            Some(url::Host::Domain(name)) if is_local_name(name) => Err(REFUSED_ADDRESS.into()),
            Some(url::Host::Domain(name)) if is_our_host(name, policy) => Err(REFUSED_HOST.into()),
            None => Err("webhook URL has no host".into()),
            _ => Ok(Self { url }),
        }
    }

    /// A push endpoint: the same rules, and only a known push service. A subscription
    /// comes from the browser, so nothing but this list says it points at one.
    pub fn parse_push(raw: &str, policy: &Policy) -> Result<Self, String> {
        if raw.len() > MAX_PUSH_URL_LEN {
            return Err(format!(
                "push endpoint is longer than {MAX_PUSH_URL_LEN} bytes"
            ));
        }
        let url = Url::parse(raw).map_err(|_| "push endpoint does not parse".to_string())?;
        if policy.allows(&url) {
            return Ok(Self { url });
        }
        let plain = url.scheme() == "https"
            && url.username().is_empty()
            && url.password().is_none()
            && url.port_or_known_default() == Some(443);
        match url.host() {
            Some(url::Host::Domain(name)) if plain && is_push_host(name) => Ok(Self { url }),
            _ => Err("push endpoint is not a known push service".into()),
        }
    }

    /// The URL as parsed, so that one endpoint written two ways is one device.
    pub fn as_str(&self) -> &str {
        self.url.as_str()
    }

    /// `https://host`, what a VAPID token is issued for.
    pub fn origin(&self) -> String {
        self.url.origin().ascii_serialization()
    }

    /// The host, which is all a list shows: the path may hold a token.
    pub fn host(&self) -> &str {
        self.url.host_str().unwrap_or("")
    }

    fn port(&self) -> u16 {
        self.url.port_or_known_default().unwrap_or(443)
    }
}

const REFUSED_ADDRESS: &str = "webhook URL points at a private or reserved address";
const REFUSED_HOST: &str = "webhook URL points at a host this server itself uses";
/// For a name that does not resolve and one that resolves to a private address alike.
const REFUSED_NAME: &str = "webhook host does not resolve to a public address";

fn normalized(name: &str) -> String {
    name.trim_end_matches('.').to_ascii_lowercase()
}

fn is_or_under(name: &str, domain: &str) -> bool {
    name == domain || name.ends_with(&format!(".{domain}"))
}

/// A name with no dot only means something inside a network.
fn is_local_name(name: &str) -> bool {
    let name = normalized(name);
    !name.contains('.') || LOCAL_NAMES.iter().any(|l| is_or_under(&name, l))
}

/// `name` comes lowercased from the URL parser. A trailing dot is not allowed for: no
/// browser hands one out.
fn is_push_host(name: &str) -> bool {
    PUSH_HOSTS.contains(&name) || PUSH_DOMAINS.iter().any(|d| name.ends_with(d))
}

fn is_our_host(name: &str, policy: &Policy) -> bool {
    let name = normalized(name);
    UPSTREAM_DOMAINS.iter().any(|d| is_or_under(&name, d))
        || policy
            .own_host
            .as_deref()
            .is_some_and(|own| is_or_under(&name, own))
}

/// Every address must be permitted: one private answer among public ones is how
/// a name reaches the inside.
fn check_addresses(
    addrs: Vec<SocketAddr>,
    target: &Target,
    policy: &Policy,
) -> Result<Vec<SocketAddr>, String> {
    if policy.allows(&target.url) {
        return Ok(addrs);
    }
    if addrs.is_empty() || addrs.iter().any(|a| !policy.permits(a.ip())) {
        return Err(REFUSED_NAME.into());
    }
    Ok(addrs)
}

/// The addresses to pin `target` to.
pub async fn resolve(target: &Target, policy: &Policy) -> Result<Vec<SocketAddr>, String> {
    let lookup = tokio::time::timeout(
        RESOLVE_TIMEOUT,
        tokio::net::lookup_host((target.host().trim_matches(['[', ']']), target.port())),
    )
    .await;
    let addrs = match lookup {
        Ok(Ok(addrs)) => addrs.collect(),
        _ => return Err(REFUSED_NAME.into()),
    };
    check_addresses(addrs, target, policy)
}

/// How long a delivery may take.
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    pub connect: Duration,
    pub total: Duration,
    /// The wait before each retry; its length bounds the retries.
    pub backoff: [Duration; 2],
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(3),
            total: Duration::from_secs(5),
            backoff: [Duration::from_secs(2), Duration::from_secs(8)],
        }
    }
}

/// How a delivery ended. `error` is safe to show and to log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub status: Option<u16>,
    pub error: Option<String>,
    pub attempts: u32,
}

/// `sha256=<hex>` of HMAC-SHA256 over `<timestamp>.<body>`.
pub fn signature(secret: &str, timestamp: i64, body: &str) -> String {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(secret.as_bytes())
        .expect("HMAC takes a key of any length");
    mac.update(format!("{timestamp}.{body}").as_bytes());
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

struct Failure {
    status: Option<u16>,
    error: String,
    retry: bool,
    retry_after: Option<Duration>,
}

/// What one attempt sends.
#[derive(Clone)]
pub struct Request {
    pub headers: Vec<(&'static str, String)>,
    pub body: Vec<u8>,
    /// Wait as long as the receiver asks before a retry. For a push service, whose
    /// standing matters: a webhook receiver is whoever a client named, and could use it to
    /// keep a delivery going.
    pub honor_retry_after: bool,
}

/// A webhook's request. Its timestamp is the attempt's, so a retry signs anew.
fn webhook_request(body: &str, event_id: &str, secret: Option<&str>) -> Request {
    let timestamp = chrono::Utc::now().timestamp();
    let mut headers = vec![
        ("content-type", "application/json".to_string()),
        (EVENT_HEADER, event_id.to_string()),
        (TIMESTAMP_HEADER, timestamp.to_string()),
    ];
    if let Some(secret) = secret {
        headers.push((SIGNATURE_HEADER, signature(secret, timestamp, body)));
    }
    Request {
        headers,
        body: body.as_bytes().to_vec(),
        honor_retry_after: false,
    }
}

/// One POST to `addrs`. The name in `target` only reaches the TLS handshake and
/// the Host header: `resolve_to_addrs` keeps the client from looking it up
/// again, `no_proxy` from handing it to a proxy that would. With a configured
/// proxy, the proxy connects instead, and the addresses it reaches are its own.
async fn post(
    target: &Target,
    addrs: &[SocketAddr],
    sending: Request,
    timing: &Timing,
    proxy: Option<&str>,
) -> Result<u16, Failure> {
    let failed = |error: &str, retry| Failure {
        status: None,
        error: error.to_string(),
        retry,
        retry_after: None,
    };
    let honor_retry_after = sending.honor_retry_after;
    let builder = match proxy.map(reqwest::Proxy::all) {
        None => Client::builder().no_proxy(),
        Some(Ok(proxy)) => Client::builder().proxy(proxy),
        // Never direct instead: the proxy is there to hide this server's address.
        Some(Err(_)) => return Err(failed("could not prepare the request", false)),
    };
    let client = builder
        .resolve_to_addrs(target.host(), addrs)
        .redirect(reqwest::redirect::Policy::none())
        .pool_max_idle_per_host(0)
        .connect_timeout(timing.connect)
        .timeout(timing.total)
        .user_agent("chartgen-webhook/1")
        .build()
        .map_err(|_| failed("could not prepare the request", false))?;
    let mut request = client.post(target.url.clone()).body(sending.body);
    for (name, value) in sending.headers {
        request = request.header(name, value);
    }
    match request.send().await {
        Ok(resp) => {
            let status = resp.status();
            let code = status.as_u16();
            if status.is_success() {
                return Ok(code);
            }
            let retry = status.is_server_error() || code == 429;
            let error = if status.is_redirection() {
                format!("redirect not followed (HTTP {code})")
            } else {
                format!("HTTP {code}")
            };
            let retry_after = resp
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok()?.trim().parse().ok())
                .map(Duration::from_secs)
                .filter(|_| honor_retry_after && retry);
            Err(Failure {
                status: Some(code),
                error,
                retry,
                retry_after,
            })
        }
        Err(e) if e.is_timeout() => Err(failed("timed out", true)),
        Err(e) if e.is_connect() => Err(failed("could not connect", true)),
        Err(_) => Err(failed("request failed", true)),
    }
}

/// POST a webhook's `body`, as `deliver_request` does.
pub async fn deliver(
    target: &Target,
    policy: &Policy,
    body: &str,
    event_id: &str,
    secret: Option<&str>,
    timing: &Timing,
    slots: &[&Semaphore],
) -> Outcome {
    let make = || webhook_request(body, event_id, secret);
    deliver_request(target, policy, &make, timing, slots).await
}

/// POST what `make` builds for each attempt, retrying what may pass later. Every
/// attempt resolves and checks the name again, so an address that turned private is
/// refused, without retry. `slots` are held for each attempt but not during the waits
/// between them.
pub async fn deliver_request(
    target: &Target,
    policy: &Policy,
    make: &(dyn Fn() -> Request + Sync),
    timing: &Timing,
    slots: &[&Semaphore],
) -> Outcome {
    let mut attempts = 0;
    let mut waits = timing.backoff.iter();
    loop {
        attempts += 1;
        let mut permits = Vec::with_capacity(slots.len());
        for slot in slots {
            permits.push(slot.acquire().await);
        }
        let failure = match resolve(target, policy).await {
            Ok(addrs) => {
                let proxy = policy.proxy.as_deref();
                match post(target, &addrs, make(), timing, proxy).await {
                    Ok(code) => {
                        return Outcome {
                            status: Some(code),
                            error: None,
                            attempts,
                        }
                    }
                    Err(f) => f,
                }
            }
            Err(error) => Failure {
                status: None,
                error,
                retry: false,
                retry_after: None,
            },
        };
        drop(permits);
        let asked = failure.retry_after.unwrap_or_default();
        let next = waits
            .next()
            .filter(|_| failure.retry && asked <= MAX_RETRY_AFTER);
        match next {
            Some(wait) => tokio::time::sleep((*wait).max(asked)).await,
            None => {
                return Outcome {
                    status: failure.status,
                    error: Some(failure.error),
                    attempts,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const OPEN: Policy = Policy {
        test_origin: None,
        own_host: None,
        blocked: Vec::new(),
        proxy: None,
    };

    fn refused(url: &str) {
        assert!(Target::parse(url, &OPEN).is_err(), "{url} passed");
    }

    #[test]
    fn a_push_endpoint_must_be_a_known_push_service() {
        for url in [
            "https://fcm.googleapis.com/fcm/send/abc",
            "https://updates.push.services.mozilla.com/wpush/v2/abc",
            "https://web.push.apple.com/QGabc",
            "https://api.push.apple.com/abc",
            "https://wns2-par02p.notify.windows.com/w/?token=abc",
            "https://FCM.googleapis.com/fcm/send/abc",
        ] {
            assert!(Target::parse_push(url, &OPEN).is_ok(), "{url}");
        }
        for url in [
            "http://fcm.googleapis.com/fcm/send/abc",
            "https://fcm.googleapis.com:8443/fcm/send/abc",
            "https://user:pw@fcm.googleapis.com/fcm/send/abc",
            "https://fcm.googleapis.com./abc",
            "https://push.apple.com/abc",
            "https://notify.windows.com/abc",
            "https://fcm.googleapis.com.evil.example/abc",
            "https://evilfcm.googleapis.com/abc",
            "https://googleapis.com/abc",
            "https://evilpush.apple.com/abc",
            "https://push.apple.com.evil.example/abc",
            "https://notify.windows.com.evil.example/abc",
            "https://example.com/hook",
            "https://127.0.0.1/abc",
            "https://[::1]/abc",
            "https://169.254.169.254/latest",
            "https://localhost/abc",
            "not a url",
        ] {
            assert!(Target::parse_push(url, &OPEN).is_err(), "{url}");
        }
        let long = format!(
            "https://fcm.googleapis.com/{}",
            "a".repeat(MAX_PUSH_URL_LEN)
        );
        assert!(Target::parse_push(&long, &OPEN).is_err());
    }

    #[test]
    fn one_endpoint_written_two_ways_reads_the_same() {
        let parsed = |raw| Target::parse_push(raw, &OPEN).unwrap().as_str().to_string();
        assert_eq!(
            parsed("https://FCM.googleapis.com:443/fcm/send/a"),
            parsed("https://fcm.googleapis.com/fcm/send/a"),
        );
    }

    /// Answers `reply` and counts the requests, with when each arrived.
    async fn asking_receiver(reply: &'static str) -> (u16, Arc<Mutex<Vec<std::time::Instant>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let times = Arc::new(Mutex::new(Vec::new()));
        let log = times.clone();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = listener.accept().await {
                let mut buf = vec![0u8; 8192];
                let n = s.read(&mut buf).await.unwrap_or(0);
                if buf[..n].starts_with(b"POST ") {
                    log.lock().unwrap().push(std::time::Instant::now());
                }
                let _ = s.write_all(reply.as_bytes()).await;
                let _ = s.shutdown().await;
            }
        });
        (port, times)
    }

    fn pushing() -> Request {
        Request {
            headers: Vec::new(),
            body: b"x".to_vec(),
            honor_retry_after: true,
        }
    }

    #[tokio::test]
    async fn a_push_retry_waits_as_long_as_the_service_asks() {
        const WAIT: &str =
            "HTTP/1.1 429 Slow\r\nretry-after: 1\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
        let (port, times) = asking_receiver(WAIT).await;
        let (policy, target) = local(port);
        let make = pushing;
        let out = deliver_request(&target, &policy, &make, &quick(), &[]).await;
        assert_eq!((out.status, out.attempts), (Some(429), 3));
        let times = times.lock().unwrap();
        for pair in times.windows(2) {
            assert!(
                pair[1] - pair[0] >= Duration::from_millis(990),
                "{:?}",
                pair[1] - pair[0]
            );
        }
    }

    #[tokio::test]
    async fn a_service_that_asks_for_too_long_ends_the_delivery() {
        const WAIT: &str =
            "HTTP/1.1 429 Slow\r\nretry-after: 3600\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
        let (port, times) = asking_receiver(WAIT).await;
        let (policy, target) = local(port);
        let make = pushing;
        // Waiting as asked would take an hour, so a mistake here fails rather than hangs.
        let out = tokio::time::timeout(
            Duration::from_secs(10),
            deliver_request(&target, &policy, &make, &quick(), &[]),
        )
        .await
        .expect("it should end, not wait");
        assert_eq!((out.status, out.attempts), (Some(429), 1));
        assert_eq!(times.lock().unwrap().len(), 1);
    }

    /// A webhook receiver is whoever a client named, so it gets no say in this.
    #[tokio::test]
    async fn a_webhook_retry_does_not_wait_for_the_receiver() {
        const WAIT: &str =
            "HTTP/1.1 429 Slow\r\nretry-after: 3600\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
        let (port, times) = asking_receiver(WAIT).await;
        let (policy, target) = local(port);
        let started = std::time::Instant::now();
        let out = deliver(&target, &policy, "{}", "e", None, &quick(), &[]).await;
        assert_eq!(out.attempts, 3);
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(times.lock().unwrap().len(), 3);
    }

    #[test]
    fn a_test_origin_is_the_only_way_round_the_push_list_and_needs_the_sample_source() {
        let on = Policy::for_test_origin(Some("http://127.0.0.1:9000"), true);
        assert!(Target::parse_push("http://127.0.0.1:9000/push/a", &on).is_ok());
        assert!(Target::parse_push("http://127.0.0.1:9001/push/a", &on).is_err());
        assert!(Target::parse_push("https://example.com/a", &on).is_err());
        let live = Policy::for_test_origin(Some("http://127.0.0.1:9000"), false);
        assert!(Target::parse_push("http://127.0.0.1:9000/push/a", &live).is_err());
    }

    #[test]
    fn every_private_or_reserved_ipv4_range_is_refused() {
        for ip in [
            "0.0.0.0",
            "0.255.255.255",
            "10.0.0.1",
            "10.255.255.255",
            "100.64.0.1",
            "100.100.100.200",
            "100.127.255.255",
            "127.0.0.1",
            "127.255.255.254",
            "169.254.0.1",
            "169.254.169.254",
            "172.16.0.1",
            "172.31.255.255",
            "192.0.0.192",
            "192.0.2.1",
            "192.88.99.1",
            "192.168.1.1",
            "198.18.0.1",
            "198.19.255.255",
            "198.51.100.1",
            "203.0.113.1",
            "168.63.129.16",
            "224.0.0.1",
            "239.255.255.255",
            "240.0.0.1",
            "255.255.255.255",
        ] {
            assert!(!is_public_ip(ip.parse().unwrap()), "{ip}");
            refused(&format!("https://{ip}/hook"));
        }
    }

    #[test]
    fn the_neighbours_of_those_ranges_are_public() {
        for ip in [
            "1.1.1.1",
            "9.255.255.255",
            "11.0.0.1",
            "100.63.255.255",
            "100.128.0.1",
            "169.253.255.255",
            "172.15.255.255",
            "172.32.0.1",
            "192.0.1.1",
            "192.167.255.255",
            "198.17.255.255",
            "198.20.0.1",
            "223.255.255.255",
            "168.63.129.17",
        ] {
            assert!(is_public_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["2606:4700:4700::1111", "2a00:1450:4001:81a::200e"] {
            assert!(is_public_ip(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn every_private_reserved_or_mapped_ipv6_form_is_refused() {
        for ip in [
            "::",
            "::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "::ffff:169.254.169.254",
            "::ffff:8.8.8.8",
            "::127.0.0.1",
            "64:ff9b::7f00:1",
            "64:ff9b:1::1",
            "100::1",
            "fc00::1",
            "fd00:ec2::254",
            "fe80::1",
            "fec0::1",
            "ff02::1",
            "2001::1",
            "2001:db8::1",
            "2002:7f00:1::",
            "3fff::1",
        ] {
            assert!(!is_public_ip(ip.parse().unwrap()), "{ip}");
            refused(&format!("https://[{ip}]/hook"));
        }
    }

    #[test]
    fn other_spellings_of_loopback_are_refused() {
        for url in [
            "https://2130706433/",
            "https://0x7f.0.0.1/",
            "https://0177.0.0.1/",
            "https://127.1/",
            "https://0/",
            "https://[0:0:0:0:0:ffff:7f00:1]/",
        ] {
            refused(url);
        }
    }

    #[test]
    fn local_names_are_refused_without_a_lookup() {
        for url in [
            "https://localhost/",
            "https://LOCALHOST./",
            "https://x.localhost/",
            "https://printer.local/",
            "https://metadata.google.internal/",
            "https://nas.lan/",
            "https://router.home.arpa/",
        ] {
            refused(url);
        }
        assert!(Target::parse("https://local.example.com/", &OPEN).is_ok());
    }

    #[test]
    fn a_name_with_no_dot_or_a_private_suffix_is_refused() {
        for url in [
            "https://redis/",
            "https://coolify/hook",
            "https://fedora./",
            "https://db.localdomain/",
            "https://git.corp/",
            "https://app.docker/",
            "https://x.test/",
            "https://x.invalid/",
            "https://nas.home/",
        ] {
            assert_eq!(
                Target::parse(url, &OPEN).err().unwrap(),
                REFUSED_ADDRESS,
                "{url}"
            );
        }
    }

    #[test]
    fn this_servers_own_host_and_data_sources_are_refused() {
        let policy = Policy {
            own_host: Some("chartgen.example.net".into()),
            ..Policy::default()
        };
        for url in [
            "https://api.binance.com/api/v3/klines",
            "https://fapi.binance.com/",
            "https://API.BINANCE.COM./",
            "https://binance.com/",
            "https://testnet.binance.vision/",
            "https://query1.finance.yahoo.com/v8/",
            "https://api.alternative.me/fng/",
            "https://chartgen.example.net/mcp",
        ] {
            assert_eq!(
                Target::parse(url, &policy).err().unwrap(),
                REFUSED_HOST,
                "{url}"
            );
        }
        for url in [
            "https://notbinance.com/",
            "https://example.net/",
            "https://binance.com.example.org/",
        ] {
            assert!(Target::parse(url, &policy).is_ok(), "{url}");
        }
        // Only because of the setting.
        assert!(Target::parse("https://chartgen.example.net/", &OPEN).is_ok());
    }

    #[test]
    fn a_private_answer_and_no_answer_read_the_same() {
        let t = Target::parse("https://example.com/", &OPEN).unwrap();
        let private = check_addresses(vec![sock("10.0.0.1")], &t, &OPEN).unwrap_err();
        let none = check_addresses(vec![], &t, &OPEN).unwrap_err();
        assert_eq!(private, none);
        assert_eq!(private, REFUSED_NAME);
    }

    #[tokio::test]
    async fn a_name_that_does_not_resolve_reads_as_a_private_one() {
        let policy = Policy {
            test_origin: Some("https://no-such-host.invalid".into()),
            ..Policy::default()
        };
        let t = Target::parse("https://no-such-host.invalid/", &policy).unwrap();
        assert_eq!(resolve(&t, &OPEN).await.unwrap_err(), REFUSED_NAME);
    }

    #[test]
    fn prefixes_read_and_match() {
        let p = |raw: &str| Prefix::parse(raw).unwrap();
        assert!(p("203.0.113.0/24").contains("203.0.113.200".parse().unwrap()));
        assert!(!p("203.0.113.0/24").contains("203.0.114.1".parse().unwrap()));
        assert!(p("8.8.8.8").contains("8.8.8.8".parse().unwrap()));
        assert!(!p("8.8.8.8").contains("8.8.8.9".parse().unwrap()));
        assert!(p("0.0.0.0/0").contains("8.8.8.8".parse().unwrap()));
        assert!(p("2a00:1450::/32").contains("2a00:1450:4001::1".parse().unwrap()));
        assert!(!p("2a00:1450::/32").contains("2a00:1451::1".parse().unwrap()));
        assert!(!p("8.8.8.8/32").contains("2a00:1450::1".parse().unwrap()));
        for bad in ["8.8.8.8/33", "2a00::/129", "nonsense", "8.8.8.8/x", "/24"] {
            assert_eq!(Prefix::parse(bad), None, "{bad}");
        }
        assert_eq!(parse_prefixes(None).unwrap(), []);
        assert_eq!(
            parse_prefixes(Some(" 8.8.8.8 , ,1.1.1.1/32"))
                .unwrap()
                .len(),
            2
        );
        assert!(parse_prefixes(Some("8.8.8.8,oops")).is_err());
    }

    #[test]
    fn an_operators_blocked_prefix_refuses_a_public_address() {
        let policy = Policy {
            blocked: vec![Prefix::parse("8.8.8.0/24").unwrap()],
            ..Policy::default()
        };
        assert_eq!(
            Target::parse("https://8.8.8.8/", &policy).err().unwrap(),
            REFUSED_ADDRESS
        );
        assert!(Target::parse("https://1.1.1.1/", &policy).is_ok());
        let t = Target::parse("https://example.com/", &policy).unwrap();
        let e = check_addresses(vec![sock("1.1.1.1"), sock("8.8.8.8")], &t, &policy);
        assert_eq!(e.unwrap_err(), REFUSED_NAME);
        assert!(check_addresses(vec![sock("1.1.1.1")], &t, &policy).is_ok());
    }

    #[test]
    fn a_bad_proxy_is_an_error_that_does_not_repeat_it() {
        assert_eq!(parse_proxy(None).unwrap(), None);
        let ok = parse_proxy(Some("http://relay.example:3128".into())).unwrap();
        assert_eq!(ok.as_deref(), Some("http://relay.example:3128"));
        for bad in [
            "ftp://user:hunter2@relay/",
            "hunter2",
            "socks5://user:hunter2@relay/",
        ] {
            let e = parse_proxy(Some(bad.into())).unwrap_err();
            assert!(!e.contains("hunter2"), "{e}");
        }
    }

    #[tokio::test]
    async fn with_a_proxy_the_request_goes_there_and_not_to_the_target() {
        let (proxy_port, proxied, _) = receiver(OK).await;
        let (target_port, _, target_hits) = receiver(OK).await;
        let policy = Policy {
            test_origin: Some(format!("http://127.0.0.1:{target_port}")),
            proxy: Some(format!("http://127.0.0.1:{proxy_port}")),
            ..Policy::default()
        };
        let target =
            Target::parse(&format!("http://127.0.0.1:{target_port}/hook"), &policy).unwrap();
        let out = deliver(&target, &policy, "{}", "e", None, &quick(), &[]).await;
        assert_eq!(out.status, Some(200));
        assert_eq!(target_hits.load(Ordering::SeqCst), 0);
        let req = proxied.lock().unwrap()[0].clone();
        assert!(
            req.contains(&format!("127.0.0.1:{target_port}/hook")),
            "{req}"
        );
    }

    #[tokio::test]
    async fn a_broken_proxy_never_falls_back_to_direct() {
        let (port, _, hits) = receiver(OK).await;
        let (mut policy, target) = local(port);
        policy.proxy = Some("::not a url::".into());
        let out = deliver(&target, &policy, "{}", "e", None, &quick(), &[]).await;
        assert_eq!(out.error.as_deref(), Some("could not prepare the request"));
        assert_eq!(hits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn the_slots_are_free_during_the_wait_between_attempts() {
        let reply = "HTTP/1.1 503 Busy\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
        let (port, _, hits) = receiver(reply).await;
        let (policy, target) = local(port);
        let slots = Arc::new(Semaphore::new(1));
        let timing = Timing {
            backoff: [Duration::from_millis(400), Duration::from_millis(400)],
            ..quick()
        };
        let held = slots.clone();
        let run = tokio::spawn(async move {
            deliver(&target, &policy, "{}", "e", None, &timing, &[&held]).await
        });
        for _ in 0..100 {
            if hits.load(Ordering::SeqCst) == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(slots.available_permits(), 1, "held through the wait");
        assert_eq!(run.await.unwrap().attempts, 3);
        assert_eq!(slots.available_permits(), 1);
    }

    #[test]
    fn only_https_on_443_without_credentials_passes() {
        for url in [
            "http://example.com/",
            "http://example.com:443/",
            "ftp://example.com:443/",
            "ftp://example.com/",
            "file:///etc/passwd",
            "gopher://example.com/",
            "https://example.com:8443/",
            "https://example.com:80/",
            "https://user@example.com/",
            "https://user:pw@example.com/",
            "not a url",
            "",
        ] {
            refused(url);
        }
        refused(&format!("https://example.com/{}", "a".repeat(MAX_URL_LEN)));
        let ok = Target::parse("https://example.com:443/hook?token=x", &OPEN).unwrap();
        assert_eq!(ok.host(), "example.com");
    }

    #[test]
    fn a_refusal_names_no_part_of_the_url() {
        for url in [
            "http://secret-host.example/token",
            "https://secret-host.example:1/token",
            "https://secret:secret@secret-host.example/",
        ] {
            let e = Target::parse(url, &OPEN).err().unwrap();
            assert!(!e.contains("secret"), "{e}");
        }
    }

    fn sock(ip: &str) -> SocketAddr {
        SocketAddr::new(ip.parse().unwrap(), 443)
    }

    #[test]
    fn one_private_answer_among_public_ones_refuses_the_name() {
        let t = Target::parse("https://example.com/", &OPEN).unwrap();
        assert!(check_addresses(vec![], &t, &OPEN).is_err());
        for bad in [
            "10.0.0.1",
            "127.0.0.1",
            "169.254.169.254",
            "::1",
            "::ffff:10.0.0.1",
        ] {
            let e = check_addresses(vec![sock("1.1.1.1"), sock(bad)], &t, &OPEN).unwrap_err();
            assert_eq!(e, REFUSED_NAME, "{bad}");
        }
        let good = vec![sock("1.1.1.1"), sock("2606:4700:4700::1111")];
        assert_eq!(check_addresses(good.clone(), &t, &OPEN).unwrap(), good);
    }

    #[tokio::test]
    async fn a_name_resolving_to_loopback_is_refused_at_delivery() {
        // An IP literal skips the resolver, so `localhost` itself is the case
        // that needs one; `Target::parse` would refuse it, so build the target
        // through a policy and check under an open one.
        let policy = Policy {
            test_origin: Some("https://localhost".into()),
            ..Policy::default()
        };
        let t = Target::parse("https://localhost/", &policy).unwrap();
        assert_eq!(resolve(&t, &OPEN).await.unwrap_err(), REFUSED_NAME);
        let out = deliver(&t, &OPEN, "{}", "e", None, &quick(), &[]).await;
        assert_eq!(out.error.as_deref(), Some(REFUSED_NAME));
        assert_eq!(out.attempts, 1, "a refusal is not retried");
    }

    #[test]
    fn the_test_origin_needs_the_sample_source_and_a_loopback_address() {
        let on = |raw, sample| Policy::for_test_origin(Some(raw), sample).test_origin;
        assert_eq!(
            on("http://127.0.0.1:9000", true).as_deref(),
            Some("http://127.0.0.1:9000")
        );
        assert_eq!(on("http://127.0.0.1:9000", false), None);
        assert_eq!(on("http://example.com:9000", true), None);
        assert_eq!(on("http://10.0.0.1:9000", true), None);
        assert_eq!(on("nonsense", true), None);

        let p = Policy::for_test_origin(Some("http://127.0.0.1:9000"), true);
        assert!(Target::parse("http://127.0.0.1:9000/hook", &p).is_ok());
        // Exactly that origin: not another port, not another loopback.
        assert!(Target::parse("http://127.0.0.1:9001/hook", &p).is_err());
        assert!(Target::parse("http://127.0.0.2:9000/hook", &p).is_err());
        assert!(Target::parse("https://127.0.0.1:9000/hook", &p).is_err());
    }

    #[test]
    fn the_signature_is_hmac_sha256_over_timestamp_and_body() {
        // Worked out independently with:
        //   printf '1700000000.{"a":1}' | openssl dgst -sha256 -hmac secret
        assert_eq!(
            signature("secret", 1_700_000_000, r#"{"a":1}"#),
            "sha256=49f24e537407743fa4a0242bb63b94b9a47ee99cbbe071ccd8a22550ae411686"
        );
        assert_ne!(
            signature("secret", 1_700_000_001, r#"{"a":1}"#),
            signature("secret", 1_700_000_000, r#"{"a":1}"#)
        );
    }

    fn quick() -> Timing {
        Timing {
            connect: Duration::from_millis(300),
            total: Duration::from_millis(400),
            backoff: [Duration::from_millis(10), Duration::from_millis(10)],
        }
    }

    /// Answers each request with `reply`; records heads and bodies.
    async fn receiver(reply: &'static str) -> (u16, Arc<Mutex<Vec<String>>>, Arc<AtomicUsize>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let hits = Arc::new(AtomicUsize::new(0));
        let (seen2, hits2) = (seen.clone(), hits.clone());
        tokio::spawn(async move {
            while let Ok((mut s, _)) = listener.accept().await {
                let mut buf = vec![0u8; 8192];
                let n = s.read(&mut buf).await.unwrap_or(0);
                // Other programs on this host probe local ports: only a POST is a delivery.
                if buf.starts_with(b"POST ") {
                    hits2.fetch_add(1, Ordering::SeqCst);
                    seen2
                        .lock()
                        .unwrap()
                        .push(String::from_utf8_lossy(&buf[..n]).to_string());
                }
                let _ = s.write_all(reply.as_bytes()).await;
                let _ = s.shutdown().await;
            }
        });
        (port, seen, hits)
    }

    const OK: &str = "HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";

    fn local(port: u16) -> (Policy, Target) {
        let policy = Policy {
            test_origin: Some(format!("http://127.0.0.1:{port}")),
            ..Policy::default()
        };
        let target = Target::parse(&format!("http://127.0.0.1:{port}/hook"), &policy).unwrap();
        (policy, target)
    }

    #[tokio::test]
    async fn a_delivery_carries_the_event_and_signature_and_returns_the_status() {
        let (port, seen, _) = receiver(OK).await;
        let (policy, target) = local(port);
        let body = r#"{"id":"e1"}"#;
        let out = deliver(&target, &policy, body, "e1", Some("s3cret"), &quick(), &[]).await;
        assert_eq!((out.status, out.error, out.attempts), (Some(200), None, 1));
        let req = seen.lock().unwrap()[0].to_lowercase();
        assert!(req.starts_with("post /hook"), "{req}");
        assert!(req.contains("x-chartgen-event: e1"), "{req}");
        assert!(req.contains("content-type: application/json"), "{req}");
        assert!(req.ends_with(r#"{"id":"e1"}"#), "{req}");
        let ts: i64 = req
            .lines()
            .find_map(|l| l.strip_prefix("x-chartgen-timestamp: "))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let sig = signature("s3cret", ts, body);
        assert!(
            req.contains(&format!("x-chartgen-signature: {sig}")),
            "{req}"
        );
        assert!(!req.contains("s3cret"), "the secret itself never travels");
    }

    #[tokio::test]
    async fn without_a_secret_there_is_no_signature() {
        let (port, seen, _) = receiver(OK).await;
        let (policy, target) = local(port);
        deliver(&target, &policy, "{}", "e", None, &quick(), &[]).await;
        let req = seen.lock().unwrap()[0].to_lowercase();
        assert!(!req.contains("x-chartgen-signature"), "{req}");
    }

    #[tokio::test]
    async fn a_redirect_is_not_followed() {
        let (next_port, _, next_hits) = receiver(OK).await;
        let reply: &'static str = Box::leak(
            format!(
                "HTTP/1.1 302 Found\r\nlocation: http://127.0.0.1:{next_port}/next\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
            )
            .into_boxed_str(),
        );
        let (port, _, hits) = receiver(reply).await;
        let (policy, target) = local(port);
        let out = deliver(&target, &policy, "{}", "e", None, &quick(), &[]).await;
        assert_eq!(out.status, Some(302));
        assert_eq!(
            out.error.as_deref(),
            Some("redirect not followed (HTTP 302)")
        );
        assert_eq!(hits.load(Ordering::SeqCst), 1, "a redirect is not retried");
        assert_eq!(next_hits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn server_errors_are_retried_a_bounded_number_of_times() {
        let reply = "HTTP/1.1 503 Busy\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
        let (port, _, hits) = receiver(reply).await;
        let (policy, target) = local(port);
        let out = deliver(&target, &policy, "{}", "e", None, &quick(), &[]).await;
        assert_eq!(
            (out.status, out.error.as_deref(), out.attempts),
            (Some(503), Some("HTTP 503"), 3)
        );
        assert_eq!(hits.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn a_client_error_is_final_and_its_body_stays_unread() {
        let reply =
            "HTTP/1.1 404 Nope\r\ncontent-length: 11\r\nconnection: close\r\n\r\nSECRET-BODY";
        let (port, _, hits) = receiver(reply).await;
        let (policy, target) = local(port);
        let out = deliver(&target, &policy, "{}", "e", None, &quick(), &[]).await;
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert_eq!(out.error.as_deref(), Some("HTTP 404"));
        assert!(!format!("{out:?}").contains("SECRET-BODY"));
    }

    #[tokio::test]
    async fn a_silent_peer_times_out() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((s, _)) = listener.accept().await {
                held.push(s);
            }
        });
        let (policy, target) = local(port);
        let started = std::time::Instant::now();
        let out = deliver(&target, &policy, "{}", "e", None, &quick(), &[]).await;
        assert_eq!(out.error.as_deref(), Some("timed out"));
        assert_eq!((out.status, out.attempts), (None, 3));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn a_failure_never_repeats_the_url() {
        // Nothing listens on this port.
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let policy = Policy {
            test_origin: Some(format!("http://127.0.0.1:{port}")),
            ..Policy::default()
        };
        let target = Target::parse(&format!("http://127.0.0.1:{port}/token-abc"), &policy).unwrap();
        let out = deliver(&target, &policy, "{}", "e", None, &quick(), &[]).await;
        assert_eq!(out.error.as_deref(), Some("could not connect"));
        assert!(!format!("{out:?}").contains("token-abc"));
    }

    #[tokio::test]
    async fn the_connection_goes_to_the_pinned_address_whatever_the_name_says() {
        // `.invalid` never resolves, so reaching the receiver proves the client
        // used the pinned address and looked nothing up.
        let (port, seen, _) = receiver(OK).await;
        let policy = Policy {
            test_origin: Some(format!("http://rebind.invalid:{port}")),
            ..Policy::default()
        };
        let target = Target::parse(&format!("http://rebind.invalid:{port}/h"), &policy).unwrap();
        let addrs = [SocketAddr::new("127.0.0.1".parse().unwrap(), port)];
        let code = post(
            &target,
            &addrs,
            webhook_request("{}", "e", None),
            &quick(),
            None,
        )
        .await;
        assert_eq!(code.ok(), Some(200));
        let req = seen.lock().unwrap()[0].to_lowercase();
        assert!(
            req.contains(&format!("host: rebind.invalid:{port}")),
            "{req}"
        );
    }
}
