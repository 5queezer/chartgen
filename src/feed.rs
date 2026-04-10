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
}
