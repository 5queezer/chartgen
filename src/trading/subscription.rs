use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::trading::alert::{AlertCondition, TriggeredAlert};

/// Info returned by `list()` for MCP tool responses.
#[derive(Debug, Clone, Serialize)]
pub struct SubscriptionInfo {
    pub symbols: Option<HashSet<String>>,
    pub alert_types: Option<HashSet<String>>,
    pub offline_queue_size: usize,
    pub connected: bool,
}

/// Persisted form of a subscription (filters only, no sender/queue).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedSubscription {
    symbols: Option<HashSet<String>>,
    alert_types: Option<HashSet<String>>,
}

pub struct ClientSubscription {
    pub symbols: Option<HashSet<String>>,
    pub alert_types: Option<HashSet<String>>,
    pub sender: Option<mpsc::UnboundedSender<String>>,
    pub offline_queue: Vec<(TriggeredAlert, Instant)>,
}

pub struct SubscriptionRegistry {
    subscriptions: HashMap<String, ClientSubscription>,
    ttl: Duration,
    max_queue_size: usize,
    persist_path: PathBuf,
}

impl SubscriptionRegistry {
    /// Create a new registry, loading existing subscriptions from disk.
    pub fn new(ttl: Duration, max_queue_size: usize, persist_path: PathBuf) -> Self {
        let subscriptions = match Self::load(&persist_path) {
            Ok(persisted) => persisted
                .into_iter()
                .map(|(token, p)| {
                    (
                        token,
                        ClientSubscription {
                            symbols: p.symbols,
                            alert_types: p.alert_types,
                            sender: None,
                            offline_queue: Vec::new(),
                        },
                    )
                })
                .collect(),
            Err(_) => HashMap::new(),
        };

        Self {
            subscriptions,
            ttl,
            max_queue_size,
            persist_path,
        }
    }

    /// Idempotent subscribe. Overwrites filters but preserves existing sender.
    pub fn subscribe(
        &mut self,
        token: &str,
        symbols: Option<HashSet<String>>,
        alert_types: Option<HashSet<String>>,
    ) {
        if let Some(existing) = self.subscriptions.get_mut(token) {
            existing.symbols = symbols;
            existing.alert_types = alert_types;
        } else {
            self.subscriptions.insert(
                token.to_string(),
                ClientSubscription {
                    symbols,
                    alert_types,
                    sender: None,
                    offline_queue: Vec::new(),
                },
            );
        }
    }

    /// Remove subscription completely. Returns true if it existed.
    pub fn unsubscribe(&mut self, token: &str) -> bool {
        self.subscriptions.remove(token).is_some()
    }

    /// Return filters and queue size for a token.
    pub fn list(&self, token: &str) -> Option<SubscriptionInfo> {
        self.subscriptions.get(token).map(|sub| SubscriptionInfo {
            symbols: sub.symbols.clone(),
            alert_types: sub.alert_types.clone(),
            offline_queue_size: sub.offline_queue.len(),
            connected: sub.sender.is_some(),
        })
    }

    /// Dispatch triggered alerts to all matching subscriptions.
    /// Connected subscribers receive JSON via sender; offline subscribers get queued.
    pub fn dispatch(&mut self, alerts: &[TriggeredAlert]) {
        for alert in alerts {
            let alert_type = condition_type_name(&alert.alert.condition);
            let symbol = &alert.alert.symbol;

            for sub in self.subscriptions.values_mut() {
                if !matches_filter(&sub.symbols, symbol) {
                    continue;
                }
                if !matches_filter(&sub.alert_types, alert_type) {
                    continue;
                }

                let json = format_notification(alert);

                if let Some(ref sender) = sub.sender {
                    if sender.send(json).is_err() {
                        // Sender disconnected — queue instead
                        sub.sender = None;
                        push_to_queue(&mut sub.offline_queue, alert.clone(), self.max_queue_size);
                    }
                } else {
                    push_to_queue(&mut sub.offline_queue, alert.clone(), self.max_queue_size);
                }
            }
        }
    }

    /// Associate an SSE sender with a subscription. Flushes the offline queue.
    pub fn link_sender(&mut self, token: &str, sender: mpsc::UnboundedSender<String>) {
        if let Some(sub) = self.subscriptions.get_mut(token) {
            // Flush offline queue through the sender
            let queue = std::mem::take(&mut sub.offline_queue);
            for (alert, _instant) in queue {
                let json = format_notification(&alert);
                if sender.send(json).is_err() {
                    // Sender broken immediately — stop flushing
                    break;
                }
            }
            sub.sender = Some(sender);
        }
    }

    /// Set sender to None (client disconnected).
    pub fn unlink_sender(&mut self, token: &str) {
        if let Some(sub) = self.subscriptions.get_mut(token) {
            sub.sender = None;
        }
    }

    /// Remove TTL-expired entries from all offline queues.
    pub fn cleanup_expired(&mut self) {
        let now = Instant::now();
        for sub in self.subscriptions.values_mut() {
            sub.offline_queue
                .retain(|(_, queued_at)| now.duration_since(*queued_at) < self.ttl);
        }
    }

    /// Persist filter config to JSON (atomic write via .tmp + rename).
    pub fn save(&self) -> Result<(), String> {
        let persisted: HashMap<String, PersistedSubscription> = self
            .subscriptions
            .iter()
            .map(|(token, sub)| {
                (
                    token.clone(),
                    PersistedSubscription {
                        symbols: sub.symbols.clone(),
                        alert_types: sub.alert_types.clone(),
                    },
                )
            })
            .collect();

        if let Some(parent) = self.persist_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create parent directories: {e}"))?;
        }

        let tmp_path = self.persist_path.with_extension("tmp");
        let json = serde_json::to_string_pretty(&persisted)
            .map_err(|e| format!("failed to serialize subscriptions: {e}"))?;

        fs::write(&tmp_path, json).map_err(|e| format!("failed to write tmp file: {e}"))?;
        fs::rename(&tmp_path, &self.persist_path)
            .map_err(|e| format!("failed to rename tmp to target: {e}"))?;

        Ok(())
    }

    /// Load persisted subscriptions from disk.
    fn load(path: &Path) -> Result<HashMap<String, PersistedSubscription>, String> {
        match fs::read_to_string(path) {
            Ok(contents) => serde_json::from_str(&contents)
                .map_err(|e| format!("failed to parse subscriptions: {e}")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
            Err(e) => Err(format!("failed to read subscriptions file: {e}")),
        }
    }
}

/// Map AlertCondition variant to its type name string.
fn condition_type_name(condition: &AlertCondition) -> &'static str {
    match condition {
        AlertCondition::PriceAbove(_) => "price_above",
        AlertCondition::PriceBelow(_) => "price_below",
        AlertCondition::IndicatorSignal { .. } => "indicator_signal",
    }
}

/// Check if a value matches an optional filter set (None = all).
fn matches_filter(filter: &Option<HashSet<String>>, value: &str) -> bool {
    match filter {
        None => true,
        Some(set) => set.contains(value),
    }
}

/// Push an alert to the offline queue, discarding oldest if over capacity.
fn push_to_queue(queue: &mut Vec<(TriggeredAlert, Instant)>, alert: TriggeredAlert, max: usize) {
    if queue.len() >= max {
        queue.remove(0);
    }
    queue.push((alert, Instant::now()));
}

/// Format a TriggeredAlert as an MCP notification JSON-RPC message.
fn format_notification(alert: &TriggeredAlert) -> String {
    let condition = match &alert.alert.condition {
        AlertCondition::PriceAbove(threshold) => {
            serde_json::json!({
                "type": "price_above",
                "threshold": threshold,
            })
        }
        AlertCondition::PriceBelow(threshold) => {
            serde_json::json!({
                "type": "price_below",
                "threshold": threshold,
            })
        }
        AlertCondition::IndicatorSignal { indicator, signal } => {
            serde_json::json!({
                "type": "indicator_signal",
                "indicator": indicator,
                "signal": signal,
            })
        }
    };

    let notification = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/alert_triggered",
        "params": {
            "alert_id": alert.alert.id,
            "symbol": alert.alert.symbol,
            "interval": "",
            "condition": condition,
            "value": alert.value,
            "triggered_at": alert.triggered_at.to_rfc3339(),
        }
    });

    notification.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trading::alert::{Alert, AlertCondition};
    use chrono::Utc;

    fn make_alert(id: &str, symbol: &str, condition: AlertCondition, value: f64) -> TriggeredAlert {
        TriggeredAlert {
            alert: Alert {
                id: id.to_string(),
                symbol: symbol.to_string(),
                condition,
                created_at: Utc::now(),
            },
            triggered_at: Utc::now(),
            value,
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("chartgen_subscription_test_{name}"));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);
        dir
    }

    fn make_registry(dir: &Path) -> SubscriptionRegistry {
        SubscriptionRegistry::new(
            Duration::from_secs(3600),
            1000,
            dir.join("subscriptions.json"),
        )
    }

    #[test]
    fn subscribe_and_list() {
        let dir = temp_dir("sub_list");
        let mut reg = make_registry(&dir);

        assert!(reg.list("tok1").is_none());

        reg.subscribe("tok1", None, None);
        let info = reg.list("tok1").unwrap();
        assert!(info.symbols.is_none());
        assert!(info.alert_types.is_none());
        assert_eq!(info.offline_queue_size, 0);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn subscribe_idempotent_overwrites_filters() {
        let dir = temp_dir("sub_idempotent");
        let mut reg = make_registry(&dir);

        reg.subscribe("tok1", None, None);
        let syms = Some(HashSet::from(["BTCUSDT".to_string()]));
        reg.subscribe("tok1", syms.clone(), None);

        let info = reg.list("tok1").unwrap();
        assert_eq!(info.symbols, syms);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn subscribe_idempotent_preserves_sender() {
        let dir = temp_dir("sub_preserves_sender");
        let mut reg = make_registry(&dir);

        let (tx, _rx) = mpsc::unbounded_channel();
        reg.subscribe("tok1", None, None);
        reg.link_sender("tok1", tx);

        // Re-subscribe with new filters
        let syms = Some(HashSet::from(["BTCUSDT".to_string()]));
        reg.subscribe("tok1", syms, None);

        // Sender should still be present
        assert!(reg.subscriptions.get("tok1").unwrap().sender.is_some());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unsubscribe_removes_completely() {
        let dir = temp_dir("unsub");
        let mut reg = make_registry(&dir);

        reg.subscribe("tok1", None, None);
        assert!(reg.list("tok1").is_some());

        reg.unsubscribe("tok1");
        assert!(reg.list("tok1").is_none());

        // Unsubscribing again is a no-op
        reg.unsubscribe("tok1");
        assert!(reg.list("tok1").is_none());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn filter_match_symbol_only() {
        let dir = temp_dir("filter_sym");
        let mut reg = make_registry(&dir);

        let syms = Some(HashSet::from(["BTCUSDT".to_string()]));
        reg.subscribe("tok1", syms, None);

        let btc_alert = make_alert("a1", "BTCUSDT", AlertCondition::PriceAbove(100.0), 105.0);
        let eth_alert = make_alert("a2", "ETHUSDT", AlertCondition::PriceAbove(100.0), 105.0);

        reg.dispatch(&[btc_alert, eth_alert]);

        // Only BTC should be queued
        let info = reg.list("tok1").unwrap();
        assert_eq!(info.offline_queue_size, 1);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn filter_match_alert_type_only() {
        let dir = temp_dir("filter_type");
        let mut reg = make_registry(&dir);

        let types = Some(HashSet::from(["price_above".to_string()]));
        reg.subscribe("tok1", None, types);

        let above = make_alert("a1", "BTCUSDT", AlertCondition::PriceAbove(100.0), 105.0);
        let below = make_alert("a2", "BTCUSDT", AlertCondition::PriceBelow(100.0), 95.0);

        reg.dispatch(&[above, below]);

        let info = reg.list("tok1").unwrap();
        assert_eq!(info.offline_queue_size, 1);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn filter_match_both_symbol_and_type() {
        let dir = temp_dir("filter_both");
        let mut reg = make_registry(&dir);

        let syms = Some(HashSet::from(["BTCUSDT".to_string()]));
        let types = Some(HashSet::from(["indicator_signal".to_string()]));
        reg.subscribe("tok1", syms, types);

        let sig = make_alert(
            "a1",
            "BTCUSDT",
            AlertCondition::IndicatorSignal {
                indicator: "rsi".into(),
                signal: "green_dot".into(),
            },
            42.0,
        );
        let price = make_alert("a2", "BTCUSDT", AlertCondition::PriceAbove(100.0), 105.0);
        let wrong_sym = make_alert(
            "a3",
            "ETHUSDT",
            AlertCondition::IndicatorSignal {
                indicator: "rsi".into(),
                signal: "green_dot".into(),
            },
            42.0,
        );

        reg.dispatch(&[sig, price, wrong_sym]);

        // Only BTC + indicator_signal should match
        let info = reg.list("tok1").unwrap();
        assert_eq!(info.offline_queue_size, 1);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn filter_none_matches_all() {
        let dir = temp_dir("filter_none");
        let mut reg = make_registry(&dir);

        reg.subscribe("tok1", None, None);

        let a1 = make_alert("a1", "BTCUSDT", AlertCondition::PriceAbove(100.0), 105.0);
        let a2 = make_alert("a2", "ETHUSDT", AlertCondition::PriceBelow(100.0), 95.0);
        let a3 = make_alert(
            "a3",
            "XRPUSDT",
            AlertCondition::IndicatorSignal {
                indicator: "rsi".into(),
                signal: "green_dot".into(),
            },
            42.0,
        );

        reg.dispatch(&[a1, a2, a3]);

        let info = reg.list("tok1").unwrap();
        assert_eq!(info.offline_queue_size, 3);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ttl_expiry() {
        let dir = temp_dir("ttl");
        let mut reg = SubscriptionRegistry::new(
            Duration::from_millis(50),
            1000,
            dir.join("subscriptions.json"),
        );

        reg.subscribe("tok1", None, None);

        let alert = make_alert("a1", "BTCUSDT", AlertCondition::PriceAbove(100.0), 105.0);
        reg.dispatch(&[alert]);

        assert_eq!(reg.list("tok1").unwrap().offline_queue_size, 1);

        // Wait for TTL to expire
        std::thread::sleep(Duration::from_millis(60));
        reg.cleanup_expired();

        assert_eq!(reg.list("tok1").unwrap().offline_queue_size, 0);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn queue_overflow_discards_oldest() {
        let dir = temp_dir("overflow");
        let mut reg =
            SubscriptionRegistry::new(Duration::from_secs(3600), 3, dir.join("subscriptions.json"));

        reg.subscribe("tok1", None, None);

        for i in 0..5 {
            let alert = make_alert(
                &format!("a{i}"),
                "BTCUSDT",
                AlertCondition::PriceAbove(100.0 + i as f64),
                105.0,
            );
            reg.dispatch(&[alert]);
        }

        let info = reg.list("tok1").unwrap();
        assert_eq!(info.offline_queue_size, 3);

        // Verify the oldest were discarded: remaining should be a2, a3, a4
        let sub = reg.subscriptions.get("tok1").unwrap();
        assert_eq!(sub.offline_queue[0].0.alert.id, "a2");
        assert_eq!(sub.offline_queue[1].0.alert.id, "a3");
        assert_eq!(sub.offline_queue[2].0.alert.id, "a4");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn persistence_roundtrip() {
        let dir = temp_dir("persist");
        let path = dir.join("subscriptions.json");

        {
            let mut reg = SubscriptionRegistry::new(Duration::from_secs(3600), 1000, path.clone());
            let syms = Some(HashSet::from([
                "XRPUSDT".to_string(),
                "BTCUSDT".to_string(),
            ]));
            let types = Some(HashSet::from(["indicator_signal".to_string()]));
            reg.subscribe("tok1", syms, types);
            reg.subscribe("tok2", None, None);
            reg.save().unwrap();
        }

        // Load from disk
        let reg = SubscriptionRegistry::new(Duration::from_secs(3600), 1000, path);

        let info1 = reg.list("tok1").unwrap();
        assert!(info1.symbols.as_ref().unwrap().contains("XRPUSDT"));
        assert!(info1.symbols.as_ref().unwrap().contains("BTCUSDT"));
        assert!(info1
            .alert_types
            .as_ref()
            .unwrap()
            .contains("indicator_signal"));

        let info2 = reg.list("tok2").unwrap();
        assert!(info2.symbols.is_none());
        assert!(info2.alert_types.is_none());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn dispatch_to_connected_subscriber() {
        let dir = temp_dir("dispatch_connected");
        let mut reg = make_registry(&dir);

        let (tx, mut rx) = mpsc::unbounded_channel();
        reg.subscribe("tok1", None, None);
        reg.link_sender("tok1", tx);

        let alert = make_alert("a1", "BTCUSDT", AlertCondition::PriceAbove(100.0), 105.0);
        reg.dispatch(&[alert]);

        // Should receive via channel, not queue
        let msg = rx.try_recv().unwrap();
        assert!(msg.contains("notifications/alert_triggered"));
        assert!(msg.contains("BTCUSDT"));
        assert!(msg.contains("a1"));

        let info = reg.list("tok1").unwrap();
        assert_eq!(info.offline_queue_size, 0);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn dispatch_to_offline_subscriber_queues() {
        let dir = temp_dir("dispatch_offline");
        let mut reg = make_registry(&dir);

        reg.subscribe("tok1", None, None);

        let alert = make_alert("a1", "BTCUSDT", AlertCondition::PriceAbove(100.0), 105.0);
        reg.dispatch(&[alert]);

        let info = reg.list("tok1").unwrap();
        assert_eq!(info.offline_queue_size, 1);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn link_sender_flushes_queue() {
        let dir = temp_dir("link_flush");
        let mut reg = make_registry(&dir);

        reg.subscribe("tok1", None, None);

        // Queue some alerts while offline
        let a1 = make_alert("a1", "BTCUSDT", AlertCondition::PriceAbove(100.0), 105.0);
        let a2 = make_alert("a2", "ETHUSDT", AlertCondition::PriceBelow(50.0), 45.0);
        reg.dispatch(&[a1, a2]);

        assert_eq!(reg.list("tok1").unwrap().offline_queue_size, 2);

        // Link sender — should flush queue
        let (tx, mut rx) = mpsc::unbounded_channel();
        reg.link_sender("tok1", tx);

        assert_eq!(reg.list("tok1").unwrap().offline_queue_size, 0);

        let msg1 = rx.try_recv().unwrap();
        let msg2 = rx.try_recv().unwrap();
        assert!(msg1.contains("a1"));
        assert!(msg2.contains("a2"));
        assert!(rx.try_recv().is_err());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unlink_sender_sets_to_none() {
        let dir = temp_dir("unlink");
        let mut reg = make_registry(&dir);

        let (tx, _rx) = mpsc::unbounded_channel();
        reg.subscribe("tok1", None, None);
        reg.link_sender("tok1", tx);

        assert!(reg.subscriptions.get("tok1").unwrap().sender.is_some());

        reg.unlink_sender("tok1");
        assert!(reg.subscriptions.get("tok1").unwrap().sender.is_none());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn dispatch_broken_sender_falls_back_to_queue() {
        let dir = temp_dir("broken_sender");
        let mut reg = make_registry(&dir);

        let (tx, rx) = mpsc::unbounded_channel();
        reg.subscribe("tok1", None, None);
        reg.link_sender("tok1", tx);

        // Drop receiver to break the channel
        drop(rx);

        let alert = make_alert("a1", "BTCUSDT", AlertCondition::PriceAbove(100.0), 105.0);
        reg.dispatch(&[alert]);

        // Sender should be cleared and alert queued
        assert!(reg.subscriptions.get("tok1").unwrap().sender.is_none());
        assert_eq!(reg.list("tok1").unwrap().offline_queue_size, 1);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn notification_json_format() {
        let alert = make_alert(
            "abc123",
            "XRPUSDT",
            AlertCondition::IndicatorSignal {
                indicator: "cipher_b".into(),
                signal: "green_dot".into(),
            },
            0.812,
        );

        let json = format_notification(&alert);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["jsonrpc"], "2.0");
        assert_eq!(parsed["method"], "notifications/alert_triggered");
        assert_eq!(parsed["params"]["alert_id"], "abc123");
        assert_eq!(parsed["params"]["symbol"], "XRPUSDT");
        assert_eq!(parsed["params"]["condition"]["type"], "indicator_signal");
        assert_eq!(parsed["params"]["condition"]["indicator"], "cipher_b");
        assert_eq!(parsed["params"]["condition"]["signal"], "green_dot");
        assert!((parsed["params"]["value"].as_f64().unwrap() - 0.812).abs() < f64::EPSILON);
    }

    #[test]
    fn condition_type_names() {
        assert_eq!(
            condition_type_name(&AlertCondition::PriceAbove(1.0)),
            "price_above"
        );
        assert_eq!(
            condition_type_name(&AlertCondition::PriceBelow(1.0)),
            "price_below"
        );
        assert_eq!(
            condition_type_name(&AlertCondition::IndicatorSignal {
                indicator: "x".into(),
                signal: "y".into(),
            }),
            "indicator_signal"
        );
    }
}
