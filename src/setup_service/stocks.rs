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

/// Move `record` by a read whose bars closed by `through`: a split first, since every
/// bar of the read is in its terms; then each bar, with the dividends that went ex by
/// it; then an expiry the read passed with no bar. A Binance setup takes the bars only.
pub(super) fn score(
    record: &mut Record,
    read: &fetch::StockMinutes,
    through: i64,
) -> Vec<Revision> {
    let mut log = Vec::new();
    let reported = read.adjustments.iter().filter(|a| a.at() <= through);
    let (splits, dividends): (Vec<&Adjustment>, Vec<&Adjustment>) =
        reported.partition(|a| matches!(a, Adjustment::Split { .. }));
    for split in splits {
        log.extend(record.adjust(*split));
    }
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

impl Setups {
    /// Read `symbol`'s 1m bars from Yahoo and move its stock setups by them. Blocking.
    /// An error is a read that failed: nothing moved, and the next poll reads the same
    /// bars. With the market closed since the last read there is nothing to read, but an
    /// expiry may have passed.
    pub(super) fn poll(&self, symbol: &str) -> Result<(), String> {
        let market = Market::UsEquity;
        let now = fetch::now_ms();
        let through = (now - SETTLE_MS).div_euclid(MINUTE_MS) * MINUTE_MS;
        let reach = (now - REACH_MS).div_euclid(MINUTE_MS) * MINUTE_MS;
        let on_symbol = |r: &Record| r.market.is_some() && r.symbol == symbol && self.runs(r);
        let (needed, read_before) = {
            let st = self.lock();
            let needed = st
                .setups
                .values()
                .map(|h| &h.record)
                .filter(|r| on_symbol(r))
                .map(Record::next_bar_time)
                .min();
            (needed, st.stock_reads.get(symbol).copied())
        };
        let Some(needed) = needed else {
            return Ok(());
        };
        // Minutes read before are not read again: one with no bar had no trade.
        let from = needed.max(read_before.unwrap_or(i64::MIN)).max(reach);
        let to = through.min(from.saturating_add(PAGE_MS));
        let read = match from < to && market.session_ms_between(from, to) > 0 {
            true => fetch::fetch_stock_minutes(self.source, symbol, from, to)?,
            false => fetch::StockMinutes::default(),
        };
        let to = to.max(from);

        let mut st = self.lock();
        let open: Vec<Record> = st
            .setups
            .values()
            .map(|h| &h.record)
            .filter(|r| on_symbol(r))
            .cloned()
            .collect();
        let mut changed = Vec::new();
        for mut record in open {
            let before = record.clone();
            let mut log = Vec::new();
            let next = record.next_bar_time().max(read_before.unwrap_or(i64::MIN));
            if next < reach && market.session_ms_between(next, reach) > 0 {
                log.push(record.note_gap(next, reach, now));
            }
            log.extend(score(&mut record, &read, to));
            if record != before {
                changed.push((record, log));
            }
        }
        st.stock_reads.insert(symbol.to_string(), to);
        if self.write(&mut st, changed).is_ok() {
            self.sync_feeds(&mut st);
        }
        Ok(())
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
