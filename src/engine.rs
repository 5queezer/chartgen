/// Top-level orchestrator wiring: feed -> indicators -> alerts -> trading.
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::RwLock;

use crate::data::Bar;
use crate::feed::{BinanceFeed, FeedEvent};
use crate::indicator_state::IndicatorStateManager;
use crate::trading::alert::{AlertCondition, AlertEngine, TriggeredAlert};
use crate::trading::order::OrderTracker;
use crate::trading::persistence::{load_alerts, save_alerts, AuditLog};
use crate::trading::position::PositionTracker;
use crate::trading::subscription::SubscriptionRegistry;

pub struct EngineConfig {
    pub symbol: String,
    pub interval: String,
    pub indicators: Vec<String>,
    /// Data directory (e.g. ~/.chartgen/).
    pub data_dir: PathBuf,
    /// TTL for offline notification queue entries (seconds). Default: 3600.
    pub notification_ttl_secs: u64,
    /// Max offline queue size per subscription. Default: 1000.
    pub max_queue_size: usize,
}

pub struct Engine {
    pub config: EngineConfig,
    pub indicator_state: IndicatorStateManager,
    pub alert_engine: AlertEngine,
    pub order_tracker: OrderTracker,
    pub position_tracker: PositionTracker,
    pub audit_log: AuditLog,
    /// Pending triggered-alert notifications (drained by MCP polling).
    pub notifications: Vec<TriggeredAlert>,
    /// Push notification subscription registry.
    pub subscription_registry: SubscriptionRegistry,
}

impl Engine {
    /// Create engine, loading any persisted alerts from disk.
    pub fn new(config: EngineConfig) -> Self {
        let alerts_path = config.data_dir.join("alerts.json");
        let mut alert_engine = AlertEngine::new();

        if let Ok(alerts) = load_alerts(&alerts_path) {
            alert_engine.load(alerts);
        }

        let audit_log = AuditLog::new(config.data_dir.join("trades.log"));
        let indicator_state = IndicatorStateManager::new(config.indicators.clone(), 200);

        let subscription_registry = SubscriptionRegistry::new(
            std::time::Duration::from_secs(config.notification_ttl_secs),
            config.max_queue_size,
            config.data_dir.join("subscriptions.json"),
        );

        Self {
            config,
            indicator_state,
            alert_engine,
            order_tracker: OrderTracker::new(),
            position_tracker: PositionTracker::new(),
            audit_log,
            notifications: Vec::new(),
            subscription_registry,
        }
    }

    /// Process a new completed bar. Returns any triggered alerts.
    pub fn on_bar(&mut self, bar: Bar) -> Vec<TriggeredAlert> {
        // 1. Update indicators
        let indicators = self
            .indicator_state
            .push_bar(&self.config.symbol, &self.config.interval, bar.clone())
            .clone();

        // 2. Update position prices
        self.position_tracker
            .update_price(&self.config.symbol, bar.close);

        // 3. Evaluate alerts
        let triggered = self
            .alert_engine
            .evaluate(&self.config.symbol, &bar, &indicators);

        // 4. Log triggered alerts and queue notifications
        for t in &triggered {
            let condition_str = format!("{:?}", t.alert.condition);
            self.audit_log.log_alert_triggered(
                &t.alert.id,
                &condition_str,
                &self.config.symbol,
                &self.config.interval,
            );
            self.notifications.push(t.clone());
        }

        // 4b. Dispatch to push-notification subscribers
        if !triggered.is_empty() {
            self.subscription_registry.dispatch(&triggered);
        }

        // 5. Persist alerts (triggered ones were removed)
        let alerts_path = self.config.data_dir.join("alerts.json");
        let _ = save_alerts(&alerts_path, self.alert_engine.list());

        triggered
    }

    /// Add an alert and persist to disk.
    pub fn add_alert(&mut self, symbol: String, condition: AlertCondition) -> String {
        let id = self.alert_engine.add(symbol, condition);
        let alerts_path = self.config.data_dir.join("alerts.json");
        let _ = save_alerts(&alerts_path, self.alert_engine.list());
        id
    }

    /// Drain all pending notifications (returns and clears the queue).
    pub fn drain_notifications(&mut self) -> Vec<TriggeredAlert> {
        std::mem::take(&mut self.notifications)
    }

    /// Remove an alert and persist to disk.
    pub fn remove_alert(&mut self, id: &str) -> bool {
        let removed = self.alert_engine.remove(id);
        if removed {
            let alerts_path = self.config.data_dir.join("alerts.json");
            let _ = save_alerts(&alerts_path, self.alert_engine.list());
        }
        removed
    }
}

/// Run the trading engine event loop.
/// Connects to the WebSocket feed and processes bars through the engine.
/// Uses `std::sync::RwLock` so the engine can also be accessed by the MCP
/// handler without async infection.
pub async fn run_engine(
    engine: Arc<RwLock<Engine>>,
    on_alert: impl Fn(TriggeredAlert) + Send + 'static,
) {
    let (symbol, interval) = {
        let e = engine.read().unwrap();
        (e.config.symbol.clone(), e.config.interval.clone())
    };

    let feed = BinanceFeed::new(&symbol, &interval);
    let (tx, mut rx) = tokio::sync::mpsc::channel(100);

    tokio::spawn(async move {
        if let Err(e) = feed.run(tx).await {
            eprintln!("[engine] feed error: {e}");
        }
    });

    let mut cleanup_interval = tokio::time::interval(std::time::Duration::from_secs(60));
    cleanup_interval.tick().await; // consume the immediate first tick

    loop {
        tokio::select! {
            event = rx.recv() => {
                match event {
                    Some(FeedEvent::Bar(bar)) => {
                        let triggered = {
                            let mut e = engine.write().unwrap();
                            e.on_bar(bar)
                        };
                        for t in triggered {
                            on_alert(t);
                        }
                    }
                    Some(FeedEvent::Tick(bar)) => {
                        let mut e = engine.write().unwrap();
                        e.position_tracker.update_price(&symbol, bar.close);
                    }
                    None => break,
                }
            }
            _ = cleanup_interval.tick() => {
                let mut e = engine.write().unwrap();
                e.subscription_registry.cleanup_expired();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trading::alert::AlertCondition;
    use std::fs;

    fn make_bar(close: f64) -> Bar {
        Bar {
            open: close,
            high: close + 1.0,
            low: close - 1.0,
            close,
            volume: 100.0,
            date: "2026-01-01".to_string(),
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("chartgen_engine_test_{name}"));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);
        dir
    }

    fn make_engine(data_dir: PathBuf) -> Engine {
        Engine::new(EngineConfig {
            symbol: "BTCUSDT".to_string(),
            interval: "1h".to_string(),
            indicators: vec!["rsi".to_string()],
            data_dir,
            notification_ttl_secs: 3600,
            max_queue_size: 1000,
        })
    }

    #[test]
    fn new_creates_empty_state() {
        let dir = temp_dir("new_empty");
        let engine = make_engine(dir.clone());

        assert!(engine.alert_engine.list().is_empty());
        assert!(engine.order_tracker.all().is_empty());
        assert!(engine.position_tracker.positions().is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn on_bar_updates_indicator_state() {
        let dir = temp_dir("on_bar_ind");
        let mut engine = make_engine(dir.clone());

        for i in 0..20 {
            engine.on_bar(make_bar(100.0 + i as f64));
        }

        let results = engine.indicator_state.get_results("BTCUSDT", "1h").unwrap();
        assert!(results.contains_key("rsi"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn on_bar_triggers_price_alert() {
        let dir = temp_dir("on_bar_alert");
        let mut engine = make_engine(dir.clone());

        engine.add_alert("BTCUSDT".to_string(), AlertCondition::PriceAbove(100.0));
        assert_eq!(engine.alert_engine.list().len(), 1);

        let triggered = engine.on_bar(make_bar(105.0));
        assert_eq!(triggered.len(), 1);
        assert!((triggered[0].value - 105.0).abs() < f64::EPSILON);
        // Alert should be removed after triggering
        assert!(engine.alert_engine.list().is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn add_alert_persists_to_disk() {
        let dir = temp_dir("add_persist");
        let mut engine = make_engine(dir.clone());

        engine.add_alert("BTCUSDT".to_string(), AlertCondition::PriceAbove(50000.0));

        let alerts_path = dir.join("alerts.json");
        assert!(alerts_path.exists());

        let loaded = load_alerts(&alerts_path).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].symbol, "BTCUSDT");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn remove_alert_persists_to_disk() {
        let dir = temp_dir("remove_persist");
        let mut engine = make_engine(dir.clone());

        let id = engine.add_alert("BTCUSDT".to_string(), AlertCondition::PriceBelow(40000.0));
        assert_eq!(engine.alert_engine.list().len(), 1);

        let removed = engine.remove_alert(&id);
        assert!(removed);
        assert!(engine.alert_engine.list().is_empty());

        let alerts_path = dir.join("alerts.json");
        let loaded = load_alerts(&alerts_path).unwrap();
        assert!(loaded.is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn persisted_alerts_restored_on_new() {
        let dir = temp_dir("restore");

        // Create engine, add alert, drop it
        {
            let mut engine = make_engine(dir.clone());
            engine.add_alert("ETHUSDT".to_string(), AlertCondition::PriceAbove(5000.0));
        }

        // Create new engine from same directory — alert should be restored
        let engine = make_engine(dir.clone());
        assert_eq!(engine.alert_engine.list().len(), 1);
        assert_eq!(engine.alert_engine.list()[0].symbol, "ETHUSDT");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn on_bar_pushes_to_notifications() {
        let dir = temp_dir("notif_push");
        let mut engine = make_engine(dir.clone());

        engine.add_alert("BTCUSDT".to_string(), AlertCondition::PriceAbove(100.0));
        engine.on_bar(make_bar(105.0));

        // Notifications should contain the triggered alert
        assert_eq!(engine.notifications.len(), 1);
        assert!((engine.notifications[0].value - 105.0).abs() < f64::EPSILON);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn drain_notifications_clears_queue() {
        let dir = temp_dir("notif_drain");
        let mut engine = make_engine(dir.clone());

        engine.add_alert("BTCUSDT".to_string(), AlertCondition::PriceAbove(100.0));
        engine.on_bar(make_bar(105.0));
        assert_eq!(engine.notifications.len(), 1);

        let drained = engine.drain_notifications();
        assert_eq!(drained.len(), 1);
        assert!(engine.notifications.is_empty());

        // Second drain returns empty
        let drained2 = engine.drain_notifications();
        assert!(drained2.is_empty());

        let _ = fs::remove_dir_all(&dir);
    }
}
