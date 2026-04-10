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
}

impl BinanceFeed {
    pub fn new(symbol: &str, interval: &str) -> Self {
        Self {
            symbol: symbol.to_lowercase(),
            interval: interval.to_string(),
        }
    }

    /// Start the feed. Runs until the sender is dropped or an unrecoverable error occurs.
    /// Sends `FeedEvent::Bar` for closed candles, `FeedEvent::Tick` for live updates.
    pub async fn run(&self, tx: mpsc::Sender<FeedEvent>) -> Result<(), String> {
        let mut backoff_secs = 1u64;
        let max_backoff_secs = 60u64;
        let mut first_connect = true;

        loop {
            let url = format!(
                "wss://stream.binance.com:9443/ws/{}@kline_{}",
                self.symbol, self.interval
            );

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
                                match self.parse_kline_event(&text) {
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

    /// Parse a Binance kline WebSocket message into a `FeedEvent`.
    /// Returns `Ok(None)` when the message is not a kline event.
    fn parse_kline_event(&self, text: &str) -> Result<Option<FeedEvent>, String> {
        let v: serde_json::Value =
            serde_json::from_str(text).map_err(|e| format!("json parse: {e}"))?;

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

/// Aggregates 1-minute bars into higher-timeframe bars using timestamp-based bucket boundaries.
pub struct BarAggregator {
    interval_minutes: u64,
    current: Option<Bar>,
    current_bucket: i64,
}

impl BarAggregator {
    /// Create a new aggregator from an interval string (e.g. "5m", "15m", "1h", "4h").
    pub fn new(interval: &str) -> Self {
        let interval_minutes = parse_interval_minutes(interval);
        Self {
            interval_minutes,
            current: None,
            current_bucket: 0,
        }
    }

    /// Feed a 1-minute bar. Returns `Some(Bar)` when a completed higher-timeframe bar is ready.
    pub fn update(&mut self, bar: &Bar) -> Option<Bar> {
        let ts: i64 = bar.date.parse().unwrap_or(0);
        let bucket = ts / (self.interval_minutes as i64 * 60);

        match self.current.take() {
            None => {
                self.current = Some(bar.clone());
                self.current_bucket = bucket;
                None
            }
            Some(acc) => {
                if bucket == self.current_bucket {
                    // Same bucket: merge into accumulator.
                    self.current = Some(Bar {
                        open: acc.open,
                        high: acc.high.max(bar.high),
                        low: acc.low.min(bar.low),
                        close: bar.close,
                        volume: acc.volume + bar.volume,
                        date: acc.date,
                    });
                    None
                } else {
                    // New bucket: emit the completed bar, start fresh.
                    self.current = Some(bar.clone());
                    self.current_bucket = bucket;
                    Some(acc)
                }
            }
        }
    }
}

/// Parse an interval string like "1m", "5m", "15m", "1h", "4h" into minutes.
fn parse_interval_minutes(interval: &str) -> u64 {
    let s = interval.trim();
    if let Some(hours) = s.strip_suffix('h') {
        hours.parse::<u64>().unwrap_or(1) * 60
    } else if let Some(mins) = s.strip_suffix('m') {
        mins.parse::<u64>().unwrap_or(1)
    } else {
        // Fallback: treat as minutes.
        s.parse::<u64>().unwrap_or(1)
    }
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

    // --- BarAggregator tests ---

    /// Helper: create a Bar at a given unix-second timestamp.
    fn make_bar(ts: i64, open: f64, high: f64, low: f64, close: f64, volume: f64) -> Bar {
        Bar {
            open,
            high,
            low,
            close,
            volume,
            date: ts.to_string(),
        }
    }

    #[test]
    fn test_parse_interval_minutes() {
        assert_eq!(parse_interval_minutes("1m"), 1);
        assert_eq!(parse_interval_minutes("5m"), 5);
        assert_eq!(parse_interval_minutes("15m"), 15);
        assert_eq!(parse_interval_minutes("1h"), 60);
        assert_eq!(parse_interval_minutes("4h"), 240);
    }

    #[test]
    fn test_aggregator_first_bar_initializes() {
        let mut agg = BarAggregator::new("5m");
        let bar = make_bar(300, 100.0, 105.0, 95.0, 102.0, 10.0);
        // First bar should not emit anything.
        assert!(agg.update(&bar).is_none());
        assert!(agg.current.is_some());
    }

    #[test]
    fn test_aggregator_5m_from_1m_bars() {
        let mut agg = BarAggregator::new("5m");

        // 5 bars in the same 5-minute bucket: ts 300..599 all map to bucket 1 (300/300=1).
        let bars = vec![
            make_bar(300, 100.0, 110.0, 98.0, 105.0, 10.0),
            make_bar(360, 105.0, 112.0, 103.0, 108.0, 20.0),
            make_bar(420, 108.0, 109.0, 100.0, 101.0, 15.0),
            make_bar(480, 101.0, 107.0, 99.0, 106.0, 25.0),
            make_bar(540, 106.0, 111.0, 104.0, 110.0, 30.0),
        ];

        for bar in &bars[..5] {
            assert!(agg.update(bar).is_none());
        }

        // Next bar in a new bucket triggers emit.
        let next = make_bar(600, 110.0, 115.0, 109.0, 113.0, 5.0);
        let emitted = agg.update(&next).expect("should emit completed bar");

        assert!(
            (emitted.open - 100.0).abs() < f64::EPSILON,
            "O = first open"
        );
        assert!((emitted.high - 112.0).abs() < f64::EPSILON, "H = max high");
        assert!((emitted.low - 98.0).abs() < f64::EPSILON, "L = min low");
        assert!(
            (emitted.close - 110.0).abs() < f64::EPSILON,
            "C = last close"
        );
        assert!(
            (emitted.volume - 100.0).abs() < f64::EPSILON,
            "V = sum volumes"
        );
        assert_eq!(emitted.date, "300");
    }

    #[test]
    fn test_aggregator_1h_from_1m_bars() {
        let mut agg = BarAggregator::new("1h");

        // 60 bars in the same 1-hour bucket: ts 3600..7199 → bucket 1 (ts / 3600).
        let base_ts = 3600_i64;
        for i in 0..60 {
            let ts = base_ts + i * 60;
            let bar = make_bar(ts, 50.0 + i as f64, 55.0 + i as f64, 48.0, 51.0, 1.0);
            assert!(
                agg.update(&bar).is_none(),
                "should not emit within same bucket"
            );
        }

        // Trigger with first bar of next hour.
        let trigger = make_bar(7200, 999.0, 999.0, 999.0, 999.0, 1.0);
        let result = agg.update(&trigger).expect("should emit 1h bar");

        assert!((result.open - 50.0).abs() < f64::EPSILON, "O = first open");
        // Max high = 55.0 + 59 = 114.0
        assert!((result.high - 114.0).abs() < f64::EPSILON, "H = max high");
        assert!((result.low - 48.0).abs() < f64::EPSILON, "L = min low");
        assert!((result.close - 51.0).abs() < f64::EPSILON, "C = last close");
        assert!(
            (result.volume - 60.0).abs() < f64::EPSILON,
            "V = sum of 60 × 1.0"
        );
        assert_eq!(result.date, "3600");
    }

    #[test]
    fn test_aggregator_partial_interval_emits_on_bucket_change() {
        let mut agg = BarAggregator::new("5m");

        // Only 3 bars in the first bucket (partial).
        let bars = vec![
            make_bar(300, 100.0, 110.0, 90.0, 105.0, 10.0),
            make_bar(360, 105.0, 108.0, 102.0, 107.0, 20.0),
            make_bar(420, 107.0, 109.0, 101.0, 103.0, 15.0),
        ];

        for bar in &bars {
            assert!(agg.update(bar).is_none());
        }

        // Jump to the next bucket.
        let next = make_bar(600, 103.0, 106.0, 100.0, 104.0, 5.0);
        let emitted = agg.update(&next).expect("should emit partial bar");

        assert!((emitted.open - 100.0).abs() < f64::EPSILON);
        assert!((emitted.high - 110.0).abs() < f64::EPSILON);
        assert!((emitted.low - 90.0).abs() < f64::EPSILON);
        assert!((emitted.close - 103.0).abs() < f64::EPSILON);
        assert!((emitted.volume - 45.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_aggregator_ohlcv_correctness() {
        let mut agg = BarAggregator::new("5m");

        // Carefully chosen values to verify each OHLCV rule.
        let bars = vec![
            make_bar(0, 10.0, 15.0, 8.0, 12.0, 100.0),    // O=10
            make_bar(60, 12.0, 20.0, 11.0, 14.0, 200.0),  // H=20 (max)
            make_bar(120, 14.0, 16.0, 5.0, 9.0, 50.0),    // L=5 (min)
            make_bar(180, 9.0, 13.0, 7.0, 11.0, 150.0),   // continuing
            make_bar(240, 11.0, 14.0, 10.0, 13.0, 300.0), // C=13 (last)
        ];

        for bar in &bars {
            assert!(agg.update(bar).is_none());
        }

        // Trigger emit.
        let next = make_bar(300, 13.0, 14.0, 12.0, 13.5, 10.0);
        let emitted = agg.update(&next).unwrap();

        assert!((emitted.open - 10.0).abs() < f64::EPSILON, "O = first");
        assert!((emitted.high - 20.0).abs() < f64::EPSILON, "H = max");
        assert!((emitted.low - 5.0).abs() < f64::EPSILON, "L = min");
        assert!((emitted.close - 13.0).abs() < f64::EPSILON, "C = last");
        assert!(
            (emitted.volume - 800.0).abs() < f64::EPSILON,
            "V = 100+200+50+150+300"
        );
    }
}
