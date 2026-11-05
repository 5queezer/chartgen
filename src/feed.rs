/// Binance WebSocket kline feed with reconnect logic and REST backfill.
use crate::data::Bar;
use futures_util::StreamExt;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

/// Events emitted by the feed.
#[derive(Clone, Debug)]
pub enum FeedEvent {
    /// Completed/closed candle.
    Bar(Bar),
    /// Live updating candle (not yet closed).
    Tick(Bar),
}

/// Binance WebSocket kline feed for a single symbol/interval pair.
pub struct BinanceFeed {
    symbol: String,
    interval: String,
    trades: bool,
}

impl BinanceFeed {
    pub fn new(symbol: &str, interval: &str) -> Self {
        Self {
            symbol: symbol.to_lowercase(),
            interval: interval.to_string(),
            trades: false,
        }
    }

    /// Also fold the `aggTrade` stream into the forming candle. Binance
    /// pushes kline updates only every 2 s; trades arrive many times a
    /// second, so this makes `Tick`s sub-second (chart push, ADR-0021).
    pub fn with_trades(mut self) -> Self {
        self.trades = true;
        self
    }

    fn url(&self) -> String {
        if self.trades {
            format!(
                "wss://stream.binance.com:9443/stream?streams={0}@kline_{1}/{0}@aggTrade",
                self.symbol, self.interval
            )
        } else {
            format!(
                "wss://stream.binance.com:9443/ws/{}@kline_{}",
                self.symbol, self.interval
            )
        }
    }

    /// Start the feed. Runs until the sender is dropped or an unrecoverable error occurs.
    /// Sends `FeedEvent::Bar` for closed candles, `FeedEvent::Tick` for live updates.
    pub async fn run(&self, tx: mpsc::Sender<FeedEvent>) -> Result<(), String> {
        let mut backoff_secs = 1u64;
        let max_backoff_secs = 60u64;
        let mut first_connect = true;

        loop {
            let url = self.url();
            // Forming candle that trades fold into, and the event time of
            // the kline it came from: trades up to then are already in it.
            let mut forming: Option<(Bar, i64)> = None;

            match tokio_tungstenite::connect_async(&url).await {
                Ok((ws_stream, _)) => {
                    eprintln!(
                        "[feed] connected to Binance WS for {}@kline_{}",
                        self.symbol, self.interval
                    );
                    backoff_secs = 1;

                    // Backfill missed bars on reconnect (skip on first connect).
                    if !first_connect {
                        if let Err(e) = self.backfill(&tx).await {
                            eprintln!("[feed] backfill failed: {e}");
                        }
                    }
                    first_connect = false;

                    let (_write, mut read) = ws_stream.split();

                    loop {
                        match read.next().await {
                            Some(Ok(Message::Text(text))) => {
                                match self.parse_event(&text, &mut forming) {
                                    Ok(Some(event)) => {
                                        if tx.send(event).await.is_err() {
                                            // Receiver dropped.
                                            return Ok(());
                                        }
                                    }
                                    Ok(None) => {} // not a kline event
                                    Err(e) => {
                                        eprintln!("[feed] parse error: {e}");
                                    }
                                }
                            }
                            Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => {}
                            Some(Ok(Message::Close(_))) => {
                                eprintln!("[feed] server sent close frame");
                                break;
                            }
                            Some(Err(e)) => {
                                eprintln!("[feed] WS read error: {e}");
                                break;
                            }
                            None => {
                                eprintln!("[feed] WS stream ended");
                                break;
                            }
                            // Binary / Frame variants — ignore.
                            Some(Ok(_)) => {}
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[feed] connect failed (backoff {backoff_secs}s): {e}");
                }
            }

            // Exponential backoff before reconnect.
            tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;
            backoff_secs = (backoff_secs * 2).min(max_backoff_secs);

            if tx.is_closed() {
                return Ok(());
            }
            eprintln!(
                "[feed] reconnecting to {}@kline_{} ...",
                self.symbol, self.interval
            );
        }
    }

    /// Parse a raw or combined-stream (`{"stream", "data"}`) message. A
    /// kline replaces the forming candle; with `with_trades`, an `aggTrade`
    /// newer than it is folded in and emitted as a `Tick`.
    fn parse_event(
        &self,
        text: &str,
        forming: &mut Option<(Bar, i64)>,
    ) -> Result<Option<FeedEvent>, String> {
        let mut v: serde_json::Value =
            serde_json::from_str(text).map_err(|e| format!("json parse: {e}"))?;
        if let Some(data) = v.get_mut("data") {
            v = data.take();
        }
        match v.get("e").and_then(|e| e.as_str()) {
            Some("kline") => {
                let event = self.kline_event(&v)?;
                if let Some(event) = &event {
                    let (FeedEvent::Bar(bar) | FeedEvent::Tick(bar)) = event;
                    let at = v.get("E").and_then(|e| e.as_i64()).unwrap_or(0);
                    // A late close of the previous candle must not replace
                    // a newer candle that trades already started.
                    let newer = forming
                        .as_ref()
                        .is_none_or(|(f, _)| bar_secs(bar) >= bar_secs(f));
                    if newer {
                        *forming = Some((bar.clone(), at));
                    }
                }
                Ok(event)
            }
            Some("aggTrade") if self.trades => {
                let num = |k: &str| {
                    v.get(k)
                        .and_then(|x| x.as_str())
                        .and_then(|x| x.parse::<f64>().ok())
                };
                let (Some(price), Some(qty), Some(t)) =
                    (num("p"), num("q"), v.get("T").and_then(|x| x.as_i64()))
                else {
                    return Err("malformed aggTrade".into());
                };
                Ok(fold_trade(forming, &self.interval, price, qty, t).map(FeedEvent::Tick))
            }
            _ => Ok(None),
        }
    }

    /// Parse a Binance kline WebSocket message into a `FeedEvent`.
    /// Returns `Ok(None)` when the message is not a kline event.
    #[cfg(test)]
    fn parse_kline_event(&self, text: &str) -> Result<Option<FeedEvent>, String> {
        self.parse_event(text, &mut None)
    }

    fn kline_event(&self, v: &serde_json::Value) -> Result<Option<FeedEvent>, String> {
        if v.get("e").and_then(|e| e.as_str()) != Some("kline") {
            return Ok(None);
        }

        let k = v.get("k").ok_or("missing k field")?;
        let bar = parse_kline_object(k)?;
        let closed = k.get("x").and_then(|x| x.as_bool()).unwrap_or(false);

        if closed {
            Ok(Some(FeedEvent::Bar(bar)))
        } else {
            Ok(Some(FeedEvent::Tick(bar)))
        }
    }

    /// Fetch recent bars via REST to cover any gap during a disconnect.
    async fn backfill(&self, tx: &mpsc::Sender<FeedEvent>) -> Result<(), String> {
        let bars = fetch_binance_async(&self.symbol, &self.interval, 10).await?;
        for bar in bars {
            if tx.send(FeedEvent::Bar(bar)).await.is_err() {
                return Ok(()); // receiver dropped
            }
        }
        Ok(())
    }
}

fn bar_secs(bar: &Bar) -> i64 {
    bar.date.parse().unwrap_or(i64::MIN)
}

/// Open time (ms) of the candle after the one opening at `open_ms`, for a
/// Binance interval. Months are calendar months; every other interval has
/// a fixed length.
fn next_open_ms(open_ms: i64, interval: &str) -> Option<i64> {
    use chrono::{Datelike, TimeZone, Utc};
    if interval == "1M" {
        let d = Utc.timestamp_millis_opt(open_ms).single()?;
        let (y, m) = if d.month() == 12 {
            (d.year() + 1, 1)
        } else {
            (d.year(), d.month() + 1)
        };
        return Some(
            Utc.with_ymd_and_hms(y, m, 1, 0, 0, 0)
                .single()?
                .timestamp_millis(),
        );
    }
    Some(open_ms + crate::data::interval_secs(interval)? * 1000)
}

/// Fold a trade at `t_ms` into the forming candle and return the updated
/// candle. A trade past the candle's end opens the next one. Trades the
/// last kline already covered, trades before its open, and trades more than
/// one candle ahead (wait for the kline) are skipped.
fn fold_trade(
    forming: &mut Option<(Bar, i64)>,
    interval: &str,
    price: f64,
    qty: f64,
    t_ms: i64,
) -> Option<Bar> {
    let (bar, synced_ms) = forming.as_mut()?;
    let open_ms = bar_secs(bar).checked_mul(1000)?;
    if t_ms <= *synced_ms || t_ms < open_ms {
        return None;
    }
    let end = next_open_ms(open_ms, interval)?;
    if t_ms >= end {
        if t_ms >= next_open_ms(end, interval)? {
            return None;
        }
        *bar = Bar {
            open: price,
            high: price,
            low: price,
            close: price,
            volume: qty,
            date: (end / 1000).to_string(),
        };
    } else {
        bar.close = price;
        bar.high = bar.high.max(price);
        bar.low = bar.low.min(price);
        bar.volume += qty;
    }
    Some(bar.clone())
}

/// Parse a single kline JSON object (`k` field) into a `Bar`.
fn parse_kline_object(k: &serde_json::Value) -> Result<Bar, String> {
    let t = k
        .get("t")
        .and_then(|v| v.as_i64())
        .ok_or("missing t (kline start time)")?;
    let open: f64 = k
        .get("o")
        .and_then(|v| v.as_str())
        .ok_or("missing o")?
        .parse()
        .map_err(|e| format!("parse open: {e}"))?;
    let high: f64 = k
        .get("h")
        .and_then(|v| v.as_str())
        .ok_or("missing h")?
        .parse()
        .map_err(|e| format!("parse high: {e}"))?;
    let low: f64 = k
        .get("l")
        .and_then(|v| v.as_str())
        .ok_or("missing l")?
        .parse()
        .map_err(|e| format!("parse low: {e}"))?;
    let close: f64 = k
        .get("c")
        .and_then(|v| v.as_str())
        .ok_or("missing c")?
        .parse()
        .map_err(|e| format!("parse close: {e}"))?;
    let volume: f64 = k
        .get("v")
        .and_then(|v| v.as_str())
        .ok_or("missing v")?
        .parse()
        .map_err(|e| format!("parse volume: {e}"))?;

    Ok(Bar {
        open,
        high,
        low,
        close,
        volume,
        date: format!("{}", t / 1000),
    })
}

/// Async version of Binance REST kline fetch (mirrors `fetch::fetch_binance` logic).
async fn fetch_binance_async(
    symbol: &str,
    interval: &str,
    limit: usize,
) -> Result<Vec<Bar>, String> {
    let url = format!(
        "https://api.binance.com/api/v3/klines?symbol={}&interval={}&limit={}",
        symbol.to_uppercase(),
        interval,
        limit
    );

    let resp = reqwest::get(&url)
        .await
        .map_err(|e| format!("Binance REST request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("Binance REST returned status {}", resp.status()));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse Binance REST response: {e}"))?;

    let klines = body
        .as_array()
        .ok_or("Binance REST response is not an array")?;

    let mut bars = Vec::with_capacity(klines.len());
    for kline in klines {
        let arr = kline
            .as_array()
            .ok_or("Binance kline entry is not an array")?;
        if arr.len() < 6 {
            return Err("Binance kline entry has fewer than 6 elements".into());
        }

        let open_time = arr[0].as_i64().ok_or("open_time is not a number")?;
        let open: f64 = arr[1]
            .as_str()
            .ok_or("open is not a string")?
            .parse()
            .map_err(|e| format!("Failed to parse open: {e}"))?;
        let high: f64 = arr[2]
            .as_str()
            .ok_or("high is not a string")?
            .parse()
            .map_err(|e| format!("Failed to parse high: {e}"))?;
        let low: f64 = arr[3]
            .as_str()
            .ok_or("low is not a string")?
            .parse()
            .map_err(|e| format!("Failed to parse low: {e}"))?;
        let close: f64 = arr[4]
            .as_str()
            .ok_or("close is not a string")?
            .parse()
            .map_err(|e| format!("Failed to parse close: {e}"))?;
        let volume: f64 = arr[5]
            .as_str()
            .ok_or("volume is not a string")?
            .parse()
            .map_err(|e| format!("Failed to parse volume: {e}"))?;

        bars.push(Bar {
            open,
            high,
            low,
            close,
            volume,
            date: format!("{}", open_time / 1000),
        });
    }

    Ok(bars)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_creates_feed() {
        let feed = BinanceFeed::new("BTCUSDT", "1m");
        assert_eq!(feed.symbol, "btcusdt");
        assert_eq!(feed.interval, "1m");
    }

    #[test]
    fn test_parse_kline_event_closed() {
        let feed = BinanceFeed::new("BTCUSDT", "1m");
        let msg = r#"{
            "e": "kline",
            "k": {
                "t": 1672531200000,
                "o": "16500.00",
                "h": "16550.00",
                "l": "16480.00",
                "c": "16520.00",
                "v": "123.45",
                "x": true
            }
        }"#;
        let result = feed.parse_kline_event(msg).unwrap().unwrap();
        match result {
            FeedEvent::Bar(bar) => {
                assert!((bar.open - 16500.0).abs() < f64::EPSILON);
                assert!((bar.close - 16520.0).abs() < f64::EPSILON);
                assert_eq!(bar.date, "1672531200");
            }
            FeedEvent::Tick(_) => panic!("expected Bar, got Tick"),
        }
    }

    #[test]
    fn test_parse_kline_event_live() {
        let feed = BinanceFeed::new("ETHUSDT", "5m");
        let msg = r#"{
            "e": "kline",
            "k": {
                "t": 1672531200000,
                "o": "1200.00",
                "h": "1210.00",
                "l": "1195.00",
                "c": "1205.00",
                "v": "50.00",
                "x": false
            }
        }"#;
        let result = feed.parse_kline_event(msg).unwrap().unwrap();
        match result {
            FeedEvent::Tick(bar) => {
                assert!((bar.close - 1205.0).abs() < f64::EPSILON);
            }
            FeedEvent::Bar(_) => panic!("expected Tick, got Bar"),
        }
    }

    #[test]
    fn test_parse_non_kline_event() {
        let feed = BinanceFeed::new("BTCUSDT", "1m");
        let msg = r#"{"e": "trade", "p": "16500.00"}"#;
        assert!(feed.parse_kline_event(msg).unwrap().is_none());
    }

    #[test]
    fn test_parse_kline_object() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{
                "t": 1672531200000,
                "o": "100.50",
                "h": "101.00",
                "l": "99.80",
                "c": "100.75",
                "v": "999.99"
            }"#,
        )
        .unwrap();
        let bar = parse_kline_object(&v).unwrap();
        assert!((bar.open - 100.50).abs() < f64::EPSILON);
        assert!((bar.high - 101.0).abs() < f64::EPSILON);
        assert!((bar.low - 99.8).abs() < f64::EPSILON);
        assert!((bar.close - 100.75).abs() < f64::EPSILON);
        assert!((bar.volume - 999.99).abs() < f64::EPSILON);
        assert_eq!(bar.date, "1672531200");
    }

    fn bar_at(t_secs: i64, close: f64) -> Bar {
        Bar {
            open: 100.0,
            high: 101.0,
            low: 99.0,
            close,
            volume: 10.0,
            date: t_secs.to_string(),
        }
    }

    #[test]
    fn fold_trade_updates_forming_and_rolls_over() {
        // 1m candle opening at 60 s, last kline event at 70 s.
        let mut forming = Some((bar_at(60, 100.0), 70_000));
        // Already covered by the kline: skipped.
        assert!(fold_trade(&mut forming, "1m", 105.0, 1.0, 69_000).is_none());
        let b = fold_trade(&mut forming, "1m", 102.5, 2.0, 71_000).unwrap();
        assert_eq!(
            (b.close, b.high, b.low, b.volume),
            (102.5, 102.5, 99.0, 12.0)
        );
        let b = fold_trade(&mut forming, "1m", 98.0, 1.0, 72_000).unwrap();
        assert_eq!((b.close, b.low), (98.0, 98.0));
        // Past the candle's end: the next candle opens with this trade.
        let b = fold_trade(&mut forming, "1m", 97.0, 0.5, 125_000).unwrap();
        assert_eq!(b.date, "120");
        assert_eq!(
            (b.open, b.high, b.low, b.close, b.volume),
            (97.0, 97.0, 97.0, 97.0, 0.5)
        );
        // More than one candle ahead: wait for the kline.
        assert!(fold_trade(&mut forming, "1m", 97.0, 0.5, 300_000).is_none());
        // No kline yet: nothing to fold into.
        assert!(fold_trade(&mut None, "1m", 1.0, 1.0, 1).is_none());
    }

    #[test]
    fn next_open_handles_calendar_months() {
        // 2024-01-01 → 2024-02-01, 2024-12-01 → 2025-01-01.
        assert_eq!(
            next_open_ms(1_704_067_200_000, "1M"),
            Some(1_706_745_600_000)
        );
        assert_eq!(
            next_open_ms(1_733_011_200_000, "1M"),
            Some(1_735_689_600_000)
        );
        assert_eq!(next_open_ms(0, "1h"), Some(3_600_000));
    }

    #[test]
    fn combined_stream_kline_then_trade() {
        let feed = BinanceFeed::new("BTCUSDT", "1m").with_trades();
        assert!(feed
            .url()
            .ends_with("streams=btcusdt@kline_1m/btcusdt@aggTrade"));
        let mut forming = None;
        let kline = r#"{"stream":"btcusdt@kline_1m","data":{"e":"kline","E":70000,
            "k":{"t":60000,"o":"1","h":"2","l":"0.5","c":"1.5","v":"3","x":false}}}"#;
        assert!(matches!(
            feed.parse_event(kline, &mut forming).unwrap(),
            Some(FeedEvent::Tick(_))
        ));
        let trade = r#"{"stream":"btcusdt@aggTrade","data":{"e":"aggTrade","E":71000,
            "p":"2.5","q":"1","T":71000}}"#;
        match feed.parse_event(trade, &mut forming).unwrap() {
            Some(FeedEvent::Tick(b)) => assert_eq!((b.close, b.high, b.volume), (2.5, 2.5, 4.0)),
            other => panic!("expected a trade tick, got {other:?}"),
        }
        // A late close of an older candle does not replace the forming one.
        let late = r#"{"e":"kline","E":72000,
            "k":{"t":0,"o":"1","h":"1","l":"1","c":"1","v":"1","x":true}}"#;
        assert!(matches!(
            feed.parse_event(late, &mut forming).unwrap(),
            Some(FeedEvent::Bar(_))
        ));
        assert_eq!(forming.unwrap().0.date, "60");
        // Without with_trades, aggTrade frames are ignored.
        let plain = BinanceFeed::new("BTCUSDT", "1m");
        assert!(plain.parse_event(trade, &mut None).unwrap().is_none());
    }
}
