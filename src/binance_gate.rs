//! The process-wide budget in front of Binance's REST APIs (ADR-0038).
//!
//! Binance answers too many requests with 429 and then, if they keep coming,
//! bans the IP with an escalating 418, and this server's IP is a home one. So
//! every REST request asks `admit` first and reports its response to `observe`:
//! after a 429 or 418 nothing is sent until `Retry-After` has passed, and nothing
//! starts once the used weight Binance reports nears the limit for the minute.
//! A refused request is an ordinary error for the one call that made it.

use std::sync::{Mutex, MutexGuard, OnceLock};

/// Cooldown after a 429 or 418 that names no `Retry-After`.
const DEFAULT_COOLDOWN_SECS: i64 = 60;

#[derive(Default)]
struct State {
    /// Unix second until which nothing is sent, and the status that caused it.
    cooldown: Option<(i64, u16)>,
    /// Highest used weight reported in `minute` (unix minute).
    used: u32,
    minute: i64,
}

pub struct Gate {
    threshold: u32,
    limit: u32,
    state: Mutex<State>,
    clock: Box<dyn Fn() -> i64 + Send + Sync>,
}

impl Gate {
    /// A gate refusing from 80% of `limit`, Binance's weight per minute.
    pub fn new(limit: u32) -> Self {
        Self::with_clock(
            limit,
            Box::new(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs() as i64)
            }),
        )
    }

    /// `new` with the unix-seconds clock supplied, for tests.
    pub fn with_clock(limit: u32, clock: Box<dyn Fn() -> i64 + Send + Sync>) -> Self {
        Self {
            threshold: limit / 5 * 4,
            limit,
            state: Mutex::default(),
            clock,
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// May a request start now? The error says why not and for how long.
    pub fn admit(&self) -> Result<(), String> {
        let now = (self.clock)();
        let st = self.lock();
        if let Some((until, status)) = st.cooldown.filter(|(until, _)| now < *until) {
            return Err(format!(
                "Binance rate limit: waiting {}s after HTTP {status}",
                until - now
            ));
        }
        if st.minute == now.div_euclid(60) && st.used >= self.threshold {
            return Err(format!(
                "Binance rate limit: {} of {} request weight used this minute; try again in {}s",
                st.used,
                self.limit,
                60 - now.rem_euclid(60)
            ));
        }
        Ok(())
    }

    /// Record a response: its status, and its `Retry-After` and
    /// `x-mbx-used-weight-1m` headers.
    pub fn observe(&self, status: u16, retry_after: Option<&str>, used_weight: Option<&str>) {
        let now = (self.clock)();
        let mut st = self.lock();
        if matches!(status, 429 | 418) {
            let wait = retry_after
                .and_then(|r| r.trim().parse::<i64>().ok())
                .filter(|s| *s > 0)
                .unwrap_or(DEFAULT_COOLDOWN_SECS);
            let until = now.saturating_add(wait);
            if st.cooldown.is_none_or(|(u, _)| u < until) {
                st.cooldown = Some((until, status));
            }
        }
        if let Some(used) = used_weight.and_then(|u| u.trim().parse::<u32>().ok()) {
            let minute = now.div_euclid(60);
            if st.minute != minute {
                st.minute = minute;
                st.used = 0;
            }
            st.used = st.used.max(used);
        }
    }

    /// `observe` from a response's status and headers.
    pub fn observe_response(&self, status: u16, headers: &reqwest::header::HeaderMap) {
        let text = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
        self.observe(status, text("retry-after"), text("x-mbx-used-weight-1m"));
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

    fn gate(at: i64) -> (Gate, Arc<AtomicI64>) {
        let now = Arc::new(AtomicI64::new(at));
        let clock = now.clone();
        let g = Gate::with_clock(6_000, Box::new(move || clock.load(Ordering::SeqCst)));
        (g, now)
    }

    #[test]
    fn a_429_or_418_stops_everything_until_retry_after() {
        for status in [429, 418] {
            let (g, now) = gate(600);
            assert!(g.admit().is_ok());
            g.observe(status, Some("90"), None);
            let e = g.admit().unwrap_err();
            assert!(
                e.contains(&format!("HTTP {status}")) && e.contains("90s"),
                "{e}"
            );
            now.store(689, Ordering::SeqCst);
            assert!(g.admit().is_err());
            now.store(690, Ordering::SeqCst);
            assert!(g.admit().is_ok());
        }
    }

    #[test]
    fn a_cooldown_needs_no_header_and_only_ever_grows() {
        let (g, now) = gate(600);
        g.observe(429, None, None);
        now.store(600 + DEFAULT_COOLDOWN_SECS - 1, Ordering::SeqCst);
        assert!(g.admit().is_err());
        now.store(600 + DEFAULT_COOLDOWN_SECS, Ordering::SeqCst);
        assert!(g.admit().is_ok());
        // Nonsense in the header is the default, not a free pass.
        let (g, _) = gate(600);
        g.observe(429, Some("soon"), None);
        assert!(g.admit().is_err());
        // A later, shorter answer does not shorten a ban.
        let (g, now) = gate(600);
        g.observe(418, Some("7200"), None);
        g.observe(429, Some("1"), None);
        now.store(700, Ordering::SeqCst);
        assert!(g.admit().unwrap_err().contains("HTTP 418"));
    }

    #[test]
    fn other_statuses_do_not_cool_down() {
        let (g, _) = gate(600);
        for status in [200, 400, 403, 500, 503] {
            g.observe(status, Some("90"), None);
        }
        assert!(g.admit().is_ok());
    }

    #[test]
    fn the_weight_threshold_holds_for_the_minute_it_was_read_in() {
        let (g, now) = gate(600);
        g.observe(200, None, Some("4798"));
        assert!(g.admit().is_ok(), "below 80% of 6000");
        now.store(630, Ordering::SeqCst);
        g.observe(200, None, Some("4800"));
        let e = g.admit().unwrap_err();
        assert!(e.contains("4800 of 6000") && e.contains("30s"), "{e}");
        // A response that left earlier cannot lower the reading.
        g.observe(200, None, Some("10"));
        assert!(g.admit().is_err());
        // Binance's counter is per minute, so the next one starts clean.
        now.store(660, Ordering::SeqCst);
        assert!(g.admit().is_ok());
        g.observe(200, None, Some("2"));
        assert!(g.admit().is_ok());
    }
}
