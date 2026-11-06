//! The process-wide budget in front of Binance's REST APIs (ADR-0038).
//!
//! Binance answers too many requests with 429 and then, if they keep coming,
//! bans the IP with an escalating 418, and this server's IP is a home one. So
//! every REST request asks `admit` first and reports its response to `observe`:
//! after a 429, 418 or 403 nothing is sent until `Retry-After` has passed, and
//! nothing starts once the used weight Binance reports nears the limit for the
//! minute. A refused request is an ordinary error for the one call that made it.

use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

/// Cooldown after a 429, 418 or 403 that names no `Retry-After`.
const DEFAULT_COOLDOWN: Duration = Duration::from_secs(60);
/// Longest cooldown: Binance documents bans of up to 3 days, so a longer
/// `Retry-After` is a bad header and must not stop every chart until a restart.
const MAX_COOLDOWN: Duration = Duration::from_secs(3 * 86_400);

#[derive(Default)]
struct State {
    /// When sending may resume, and the status that caused the wait. An `Instant`,
    /// so a step of the wall clock cannot lengthen it.
    cooldown: Option<(Instant, u16)>,
    /// Highest used weight reported in `minute` (unix minute).
    used: u32,
    minute: i64,
}

/// A request the gate admitted, for `observe`: the unix minute it left in.
#[derive(Clone, Copy, Debug)]
pub struct Sent(i64);

pub struct Gate {
    threshold: u32,
    limit: u32,
    state: Mutex<State>,
    wall: Box<dyn Fn() -> i64 + Send + Sync>,
    mono: Box<dyn Fn() -> Instant + Send + Sync>,
}

impl Gate {
    /// A gate refusing from 80% of `limit`, Binance's weight per minute.
    pub fn new(limit: u32) -> Self {
        Self::with_clocks(
            limit,
            Box::new(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs() as i64)
            }),
            Box::new(Instant::now),
        )
    }

    /// `new` with its clocks supplied, for tests: unix seconds, for Binance's
    /// minutes, and a monotonic one, for cooldowns.
    pub fn with_clocks(
        limit: u32,
        wall: Box<dyn Fn() -> i64 + Send + Sync>,
        mono: Box<dyn Fn() -> Instant + Send + Sync>,
    ) -> Self {
        Self {
            threshold: limit / 5 * 4,
            limit,
            state: Mutex::default(),
            wall,
            mono,
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// May a request start now? The error says why not and for how long.
    pub fn admit(&self) -> Result<Sent, String> {
        let now = (self.wall)();
        let st = self.lock();
        if let Some((until, status)) = st.cooldown {
            let left = until.saturating_duration_since((self.mono)());
            if !left.is_zero() {
                return Err(format!(
                    "Binance rate limit: waiting {}s after HTTP {status}",
                    left.as_millis().div_ceil(1000)
                ));
            }
        }
        if st.minute == now.div_euclid(60) && st.used >= self.threshold {
            return Err(format!(
                "Binance rate limit: {} of {} request weight used this minute; try again in {}s",
                st.used,
                self.limit,
                60 - now.rem_euclid(60)
            ));
        }
        Ok(Sent(now.div_euclid(60)))
    }

    /// Record the response to `sent`: its status, and its `Retry-After` and
    /// `x-mbx-used-weight-1m` headers. A reading from a request that left in an
    /// earlier minute describes that minute, so it does not count against this one.
    pub fn observe(
        &self,
        sent: Sent,
        status: u16,
        retry_after: Option<&str>,
        used_weight: Option<&str>,
    ) {
        let minute = (self.wall)().div_euclid(60);
        let mut st = self.lock();
        if matches!(status, 429 | 418 | 403) {
            let wait = retry_after
                .and_then(|r| r.trim().parse::<u64>().ok())
                .filter(|s| *s > 0)
                .map_or(DEFAULT_COOLDOWN, Duration::from_secs)
                .min(MAX_COOLDOWN);
            let until = (self.mono)() + wait;
            if st.cooldown.is_none_or(|(u, _)| u < until) {
                st.cooldown = Some((until, status));
            }
        }
        if let Some(used) = used_weight.and_then(|u| u.trim().parse::<u32>().ok()) {
            if sent.0 == minute {
                if st.minute != minute {
                    st.minute = minute;
                    st.used = 0;
                }
                st.used = st.used.max(used);
            }
        }
    }

    /// `observe` from a response's status and headers.
    pub fn observe_response(&self, sent: Sent, status: u16, headers: &reqwest::header::HeaderMap) {
        let text = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
        self.observe(
            sent,
            status,
            text("retry-after"),
            text("x-mbx-used-weight-1m"),
        );
    }
}

/// `api.binance.com`: 6000 weight a minute (`rateLimits` of its exchangeInfo).
pub fn spot() -> &'static Gate {
    static GATE: OnceLock<Gate> = OnceLock::new();
    GATE.get_or_init(|| Gate::new(6_000))
}

/// `fapi.binance.com`: a separate counter of 2400 weight a minute.
pub fn futures() -> &'static Gate {
    static GATE: OnceLock<Gate> = OnceLock::new();
    GATE.get_or_init(|| Gate::new(2_400))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI64, Ordering};
    use std::sync::Arc;

    /// A gate, its wall clock and its monotonic one (seconds since the start).
    fn clocked(at: i64) -> (Gate, Arc<AtomicI64>, Arc<AtomicI64>) {
        let (wall, mono) = (Arc::new(AtomicI64::new(at)), Arc::new(AtomicI64::new(0)));
        let (w, m, base) = (wall.clone(), mono.clone(), Instant::now());
        let g = Gate::with_clocks(
            6_000,
            Box::new(move || w.load(Ordering::SeqCst)),
            Box::new(move || base + Duration::from_secs(m.load(Ordering::SeqCst) as u64)),
        );
        (g, wall, mono)
    }

    /// Both clocks move together.
    fn gate(at: i64) -> (Gate, impl Fn(i64)) {
        let (g, wall, mono) = clocked(at);
        (g, move |secs| {
            wall.store(at + secs, Ordering::SeqCst);
            mono.store(secs, Ordering::SeqCst);
        })
    }

    #[test]
    fn a_429_418_or_403_stops_everything_until_retry_after() {
        for status in [429, 418, 403] {
            let (g, advance) = gate(600);
            let sent = g.admit().unwrap();
            g.observe(sent, status, Some("90"), None);
            let e = g.admit().unwrap_err();
            assert!(
                e.contains(&format!("HTTP {status}")) && e.contains("90s"),
                "{e}"
            );
            advance(89);
            assert!(g.admit().is_err());
            advance(90);
            assert!(g.admit().is_ok());
        }
    }

    #[test]
    fn a_cooldown_needs_no_header_and_only_ever_grows() {
        let (g, advance) = gate(600);
        let sent = g.admit().unwrap();
        g.observe(sent, 429, None, None);
        advance(DEFAULT_COOLDOWN.as_secs() as i64 - 1);
        assert!(g.admit().is_err());
        advance(DEFAULT_COOLDOWN.as_secs() as i64);
        assert!(g.admit().is_ok());
        // Nonsense in the header is the default, not a free pass.
        let (g, _) = gate(600);
        let sent = g.admit().unwrap();
        g.observe(sent, 429, Some("soon"), None);
        assert!(g.admit().is_err());
        // A later, shorter answer does not shorten a ban.
        let (g, advance) = gate(600);
        let sent = g.admit().unwrap();
        g.observe(sent, 418, Some("7200"), None);
        g.observe(sent, 429, Some("1"), None);
        advance(100);
        assert!(g.admit().unwrap_err().contains("HTTP 418"));
    }

    #[test]
    fn a_cooldown_is_capped_at_three_days() {
        let (g, advance) = gate(600);
        let sent = g.admit().unwrap();
        g.observe(sent, 418, Some("99999999999"), None);
        advance(MAX_COOLDOWN.as_secs() as i64 - 1);
        assert!(g.admit().is_err(), "a real ban is honoured");
        advance(MAX_COOLDOWN.as_secs() as i64);
        assert!(g.admit().is_ok(), "a bad header ends with the longest ban");
    }

    #[test]
    fn a_wall_clock_step_does_not_lengthen_a_cooldown() {
        let (g, wall, mono) = clocked(10 * 86_400);
        let sent = g.admit().unwrap();
        g.observe(sent, 429, Some("60"), None);
        // Stepped back 10 days, as an NTP fix might.
        wall.store(600, Ordering::SeqCst);
        assert!(g.admit().unwrap_err().contains("60s"));
        mono.store(60, Ordering::SeqCst);
        assert!(g.admit().is_ok());
    }

    #[test]
    fn other_statuses_do_not_cool_down() {
        let (g, _) = gate(600);
        let sent = g.admit().unwrap();
        for status in [200, 400, 404, 500, 503] {
            g.observe(sent, status, Some("90"), None);
        }
        assert!(g.admit().is_ok());
    }

    #[test]
    fn the_weight_threshold_holds_for_the_minute_it_was_read_in() {
        let (g, advance) = gate(600);
        let sent = g.admit().unwrap();
        g.observe(sent, 200, None, Some("4798"));
        assert!(g.admit().is_ok(), "below 80% of 6000");
        advance(30);
        let sent = g.admit().unwrap();
        g.observe(sent, 200, None, Some("4800"));
        let e = g.admit().unwrap_err();
        assert!(e.contains("4800 of 6000") && e.contains("30s"), "{e}");
        // A response that left earlier cannot lower the reading.
        g.observe(sent, 200, None, Some("10"));
        assert!(g.admit().is_err());
        // Binance's counter is per minute, so the next one starts clean.
        advance(60);
        let sent = g.admit().unwrap();
        g.observe(sent, 200, None, Some("2"));
        assert!(g.admit().is_ok());
    }

    #[test]
    fn a_reading_from_an_earlier_minute_does_not_refuse_this_one() {
        let (g, advance) = gate(600);
        advance(59);
        let sent = g.admit().unwrap();
        // It left at :59 and came back at :00, over the threshold.
        advance(60);
        g.observe(sent, 200, None, Some("4900"));
        assert!(g.admit().is_ok());
        // Its status still counts, whenever it arrives.
        g.observe(sent, 429, Some("30"), None);
        assert!(g.admit().is_err());
    }
}
