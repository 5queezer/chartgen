/// Continuous indicator computation on a rolling window of bars.
use std::collections::HashMap;

use crate::data::{Bar, OhlcvData};
use crate::indicator::PanelResult;
use crate::indicators::by_name;

/// Manages rolling windows of bars and recomputes indicators on each new bar.
pub struct IndicatorStateManager {
    /// Rolling window of bars per (symbol, interval) key.
    windows: HashMap<String, RollingWindow>,
    /// Which indicators to compute.
    indicator_names: Vec<String>,
    /// Max bars to keep in each window.
    max_bars: usize,
}

struct RollingWindow {
    bars: Vec<Bar>,
    /// Cached indicator results, updated on each new bar.
    results: HashMap<String, PanelResult>,
}

fn make_key(symbol: &str, interval: &str) -> String {
    format!("{}:{}", symbol.to_uppercase(), interval)
}

impl IndicatorStateManager {
    pub fn new(indicator_names: Vec<String>, max_bars: usize) -> Self {
        Self {
            windows: HashMap::new(),
            indicator_names,
            max_bars,
        }
    }

    /// Push a new bar and recompute all indicators. Returns the updated results.
    pub fn push_bar(
        &mut self,
        symbol: &str,
        interval: &str,
        bar: Bar,
    ) -> &HashMap<String, PanelResult> {
        let key = make_key(symbol, interval);
        let window = self
            .windows
            .entry(key.clone())
            .or_insert_with(|| RollingWindow {
                bars: Vec::new(),
                results: HashMap::new(),
            });

        window.bars.push(bar);
        if window.bars.len() > self.max_bars {
            let excess = window.bars.len() - self.max_bars;
            window.bars.drain(..excess);
        }

        let data = OhlcvData {
            bars: window.bars.clone(),
            symbol: Some(symbol.to_uppercase()),
            interval: Some(interval.to_string()),
        };

        for name in &self.indicator_names {
            if let Some(ind) = by_name(name) {
                let result = ind.compute(&data);
                window.results.insert(name.clone(), result);
            }
        }

        &self.windows[&key].results
    }

    /// Get the latest indicator results for a symbol/interval.
    pub fn get_results(
        &self,
        symbol: &str,
        interval: &str,
    ) -> Option<&HashMap<String, PanelResult>> {
        let key = make_key(symbol, interval);
        self.windows.get(&key).map(|w| &w.results)
    }

    /// Get the current bar window for a symbol/interval.
    pub fn get_bars(&self, symbol: &str, interval: &str) -> Option<&[Bar]> {
        let key = make_key(symbol, interval);
        self.windows.get(&key).map(|w| w.bars.as_slice())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_bar(close: f64, i: usize) -> Bar {
        Bar {
            open: close - 1.0,
            high: close + 1.0,
            low: close - 2.0,
            close,
            volume: 100.0,
            date: format!("D{i}"),
        }
    }

    #[test]
    fn push_bars_and_verify_results() {
        let mut mgr = IndicatorStateManager::new(vec!["rsi".into()], 200);
        for i in 0..20 {
            mgr.push_bar("BTCUSDT", "1h", make_bar(100.0 + i as f64, i));
        }
        let results = mgr.get_results("BTCUSDT", "1h").unwrap();
        assert!(results.contains_key("rsi"));
    }

    #[test]
    fn rolling_window_truncates_at_max_bars() {
        let max = 10;
        let mut mgr = IndicatorStateManager::new(vec!["rsi".into()], max);
        for i in 0..25 {
            mgr.push_bar("ETHUSDT", "5m", make_bar(50.0 + i as f64, i));
        }
        let bars = mgr.get_bars("ETHUSDT", "5m").unwrap();
        assert_eq!(bars.len(), max);
        // Oldest bar should be the 16th pushed (index 15), close = 65.0
        assert!((bars[0].close - 65.0).abs() < f64::EPSILON);
    }

    #[test]
    fn multiple_symbols_tracked_independently() {
        let mut mgr = IndicatorStateManager::new(vec!["rsi".into()], 200);
        for i in 0..5 {
            mgr.push_bar("BTCUSDT", "1h", make_bar(100.0 + i as f64, i));
            mgr.push_bar("ETHUSDT", "1h", make_bar(50.0 + i as f64, i));
        }
        let btc_bars = mgr.get_bars("BTCUSDT", "1h").unwrap();
        let eth_bars = mgr.get_bars("ETHUSDT", "1h").unwrap();
        assert_eq!(btc_bars.len(), 5);
        assert_eq!(eth_bars.len(), 5);
        assert!((btc_bars[0].close - 100.0).abs() < f64::EPSILON);
        assert!((eth_bars[0].close - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn unknown_symbol_returns_none() {
        let mgr = IndicatorStateManager::new(vec!["rsi".into()], 200);
        assert!(mgr.get_results("DOESNOTEXIST", "1h").is_none());
        assert!(mgr.get_bars("DOESNOTEXIST", "1h").is_none());
    }

    #[test]
    fn rsi_result_has_correct_line_count() {
        let mut mgr = IndicatorStateManager::new(vec!["rsi".into()], 200);
        for i in 0..20 {
            mgr.push_bar("BTCUSDT", "1h", make_bar(100.0 + i as f64, i));
        }
        let results = mgr.get_results("BTCUSDT", "1h").unwrap();
        let rsi = results.get("rsi").unwrap();
        // RSI indicator produces at least 1 line (the RSI line itself).
        assert!(
            !rsi.lines.is_empty(),
            "RSI should produce at least one line"
        );
        // Each line should have as many y values as bars in the window.
        assert_eq!(rsi.lines[0].y.len(), 20);
    }
}
