//! The bars of stock setups (ADR-0070). Yahoo has no feed, so each stock symbol with
//! setups being scored is read for its 1m bars of the regular session, once a minute.
//! A read that fails moves nothing and is made again; only bars older than Yahoo
//! keeps are a gap.

use super::*;
use chartgen::setups::{Adjustment, Market};

/// How often a stock symbol is read.
const POLL_EVERY: Duration = Duration::from_secs(60);
/// A minute is applied once it is this old, so that trades reported late have arrived.
const SETTLE_MS: i64 = 60_000;
/// The longest window one read asks Yahoo for.
const PAGE_MS: i64 = 7 * 86_400_000;
/// How far back Yahoo's 1m bars reach: a day short of its 30.
const REACH_MS: i64 = 29 * 86_400_000;
/// The longest wait after reads that failed, and the least after a 429.
const MAX_BACKOFF: Duration = Duration::from_secs(15 * 60);
const AFTER_429: Duration = Duration::from_secs(5 * 60);
/// A first price this far from the last close, as a share of it, is held back unless the
/// read holds a split: it takes in every split from 3:2 on and every reverse split from
/// 2:3 on (ADR-0070).
const PLAUSIBLE: (f64, f64) = (0.7, 1.4);
/// Polls a read is held back before its first price is taken as a real gap: half an
/// hour of the session at one a minute.
const HOLD_POLLS: u32 = 30;

/// Why a stock setup cannot expire at `expires_at`: past the years the exchange
/// calendar holds, holidays would count as sessions.
pub(super) fn calendar_problem(market: Market, expires_at: i64) -> Option<Problem> {
    (!market.covers(expires_at)).then(|| {
        Problem::new(
            "expires_at",
            format!(
                "the exchange calendar here ends with {}",
                chartgen::data::US_CALENDAR_LAST_YEAR
            ),
        )
        .hint("a stock setup's sessions and holidays are known through that year; expire it before")
    })
}

/// Move `record` by a read whose bars closed by `through`: its splits first, since
/// Yahoo answers every bar in the terms of the latest split, a split after the page
/// included; then each bar, with the dividends that went ex by it; then an expiry the
/// read passed with no bar. A Binance setup takes the bars only.
pub(super) fn score(
    record: &mut Record,
    read: &fetch::StockMinutes,
    through: i64,
) -> Vec<Revision> {
    let mut log = Vec::new();
    let (splits, dividends): (Vec<&Adjustment>, Vec<&Adjustment>) = read
        .adjustments
        .iter()
        .partition(|a| matches!(a, Adjustment::Split { .. }));
    for split in splits {
        log.extend(record.adjust(*split));
    }
    let dividends: Vec<&Adjustment> = dividends
        .into_iter()
        .filter(|a| a.at() <= through)
        .collect();
    for bar in read.bars.iter().filter_map(MinuteBar::of) {
        for dividend in dividends.iter().filter(|d| d.at() <= bar.time) {
            log.extend(record.adjust(**dividend));
        }
        log.extend(record.apply_bar(&bar));
    }
    for dividend in &dividends {
        log.extend(record.adjust(**dividend));
    }
    log.extend(record.expire_by(through));
    log
}

/// The time through which a read leaves nothing to come: the end of the newest minute
/// it answered for, or with no session after that up to `to`, `to`. A minute Yahoo has
/// not published yet is read by the next poll, not taken for one without a trade.
fn covered(read: &fetch::StockMinutes, from: i64, to: i64) -> i64 {
    let reached = read.reached.unwrap_or(from).clamp(from, to);
    match Market::UsEquity.session_ms_between(reached, to) {
        0 => to,
        _ => reached,
    }
}

/// Whether `open`, the first price of a read, is too far from `before`, the last close
/// scored, to be taken without a split: a split whose event Yahoo has not sent yet
/// would read as a crash (ADR-0070).
fn implausible(open: f64, before: f64) -> bool {
    let move_ = open / before;
    !(PLAUSIBLE.0..=PLAUSIBLE.1).contains(&move_)
}

impl Setups {
    /// Read `symbol`'s 1m bars from Yahoo and move its stock setups by them. Blocking.
    /// An error is a read that failed: nothing moved, and the next poll reads the same
    /// bars. With the market closed since the last read there is nothing to read, but an
    /// expiry may have passed.
    pub(super) fn poll(&self, symbol: &str) -> Result<(), String> {
        let now = fetch::now_ms();
        let through = (now - SETTLE_MS).div_euclid(MINUTE_MS) * MINUTE_MS;
        let reach = (now - REACH_MS).div_euclid(MINUTE_MS) * MINUTE_MS;
        let (needed, read_before) = {
            let st = self.lock();
            let needed = self
                .stock_setups(&st, symbol)
                .map(Record::next_bar_time)
                .min();
            (needed, st.stock_reads.get(symbol).copied())
        };
        let Some(needed) = needed else {
            return Ok(());
        };
        // Minutes read before are not read again: one with no bar had no trade.
        let from = needed.max(read_before.unwrap_or(i64::MIN)).max(reach);
        let to = through.min(from.saturating_add(PAGE_MS)).max(from);
        let read = match Market::UsEquity.session_ms_between(from, to) > 0 {
            true => fetch::fetch_stock_minutes(self.source, symbol, from, to)?,
            false => fetch::StockMinutes {
                reached: Some(to),
                ..Default::default()
            },
        };
        self.apply_read(symbol, &read, from, to, now)
    }

    /// The stock setups of `symbol` being scored.
    fn stock_setups<'a>(
        &'a self,
        st: &'a State,
        symbol: &'a str,
    ) -> impl Iterator<Item = &'a Record> {
        st.setups
            .values()
            .map(|h| &h.record)
            .filter(move |r| r.market.is_some() && r.symbol == symbol && self.runs(r))
    }

    /// Move `symbol`'s stock setups by a read of [`from`, `to`]. Its minutes count as
    /// read once the setups they moved are stored: a write that failed is an error, and
    /// the next poll reads them again. A read whose first price is implausibly far from
    /// the last close, with no split in it, is held for up to `HOLD_POLLS` polls first.
    pub(super) fn apply_read(
        &self,
        symbol: &str,
        read: &fetch::StockMinutes,
        from: i64,
        to: i64,
        now: i64,
    ) -> Result<(), String> {
        let reach = (now - REACH_MS).div_euclid(MINUTE_MS) * MINUTE_MS;
        let through = covered(read, from, to);
        let mut st = self.lock();
        let read_before = st.stock_reads.get(symbol).copied();
        if self.held(&mut st, symbol, read) {
            return Ok(());
        }
        let open: Vec<Record> = self.stock_setups(&st, symbol).cloned().collect();
        let mut changed = Vec::new();
        for mut record in open {
            let before = record.clone();
            let mut log = Vec::new();
            let next = record.next_bar_time().max(read_before.unwrap_or(i64::MIN));
            if next < reach && Market::UsEquity.session_ms_between(next, reach) > 0 {
                log.push(record.note_gap(next, reach, now));
            }
            log.extend(score(&mut record, read, through));
            if record != before {
                changed.push((record, log));
            }
        }
        if !changed.is_empty() {
            self.write(&mut st, changed)?;
        }
        st.stock_reads.insert(symbol.to_string(), through);
        self.sync_feeds(&mut st);
        Ok(())
    }

    /// Whether to hold `read` back: its first bar after the last close scored opens
    /// outside `PLAUSIBLE` of it, and it holds no split. After `HOLD_POLLS` polls of the
    /// same bar it is taken as a real gap, and the log says so.
    fn held(&self, st: &mut State, symbol: &str, read: &fetch::StockMinutes) -> bool {
        let last = self
            .stock_setups(st, symbol)
            .filter_map(|r| r.last_close)
            .max_by_key(|m| m.bar_time);
        let first = last.and_then(|last| {
            read.bars
                .iter()
                .filter_map(MinuteBar::of)
                .find(|b| b.time > last.bar_time)
                .map(|b| (last, b))
        });
        let split = read
            .adjustments
            .iter()
            .any(|a| matches!(a, Adjustment::Split { .. }));
        let Some((last, bar)) = first.filter(|(last, b)| !split && implausible(b.open, last.price))
        else {
            st.stock_holds.remove(symbol);
            return false;
        };
        let polls = match st.stock_holds.get(symbol) {
            Some(&(at, polls)) if at == bar.time => polls + 1,
            _ => 1,
        };
        if polls > HOLD_POLLS {
            eprintln!(
                "[setups] {symbol} opened at {} after {} with no split for {HOLD_POLLS} polls: taken as a real gap",
                bar.open, last.price
            );
            st.stock_holds.remove(symbol);
            return false;
        }
        eprintln!(
            "[setups] {symbol} opened at {} after {} with no split: held ({polls} of {HOLD_POLLS})",
            bar.open, last.price
        );
        st.stock_holds.insert(symbol.to_string(), (bar.time, polls));
        true
    }
}

/// Read `symbol` until its watch is aborted: once a minute, and after a read that
/// failed twice as long as before, up to 15 minutes, and at least 5 after a 429.
pub(super) async fn run_poll(setups: Weak<Setups>, symbol: String) {
    let mut wait = POLL_EVERY;
    loop {
        tokio::time::sleep(wait).await;
        let Some(this) = setups.upgrade() else {
            return;
        };
        let asked = symbol.clone();
        let read = tokio::task::spawn_blocking(move || this.poll(&asked)).await;
        wait = match read {
            Ok(Err(e)) => {
                eprintln!("[setups] cannot read {symbol} from Yahoo: {e}");
                let least = match e.contains("429") {
                    true => AFTER_429,
                    false => POLL_EVERY,
                };
                (wait * 2).clamp(least, MAX_BACKOFF)
            }
            _ => POLL_EVERY,
        };
    }
}
