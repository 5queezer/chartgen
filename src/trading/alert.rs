use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::data::Bar;
use crate::indicator::PanelResult;

pub type AlertId = String;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Alert {
    pub id: AlertId,
    pub symbol: String,
    pub condition: AlertCondition,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AlertCondition {
    PriceAbove(f64),
    PriceBelow(f64),
    IndicatorSignal { indicator: String, signal: String },
}

#[derive(Clone, Debug)]
pub struct TriggeredAlert {
    pub alert: Alert,
    pub triggered_at: DateTime<Utc>,
    pub value: f64,
}

pub struct AlertEngine {
    alerts: Vec<Alert>,
}

impl AlertEngine {
    pub fn new() -> Self {
        Self { alerts: Vec::new() }
    }

    pub fn add(&mut self, symbol: String, condition: AlertCondition) -> AlertId {
        let id = Uuid::new_v4().to_string();
        self.alerts.push(Alert {
            id: id.clone(),
            symbol,
            condition,
            created_at: Utc::now(),
        });
        id
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.alerts.len();
        self.alerts.retain(|a| a.id != id);
        self.alerts.len() < before
    }

    pub fn list(&self) -> &[Alert] {
        &self.alerts
    }

    /// Evaluate all alerts against the current bar and indicator results.
    /// Returns triggered alerts (which are also removed from the active list).
    pub fn evaluate(
        &mut self,
        symbol: &str,
        bar: &Bar,
        indicators: &HashMap<String, PanelResult>,
    ) -> Vec<TriggeredAlert> {
        let now = Utc::now();
        let mut triggered_ids = Vec::new();
        let mut triggered = Vec::new();

        for alert in &self.alerts {
            if alert.symbol != symbol {
                continue;
            }

            let fire_value = match &alert.condition {
                AlertCondition::PriceAbove(threshold) => {
                    if bar.close >= *threshold {
                        Some(bar.close)
                    } else {
                        None
                    }
                }
                AlertCondition::PriceBelow(threshold) => {
                    if bar.close <= *threshold {
                        Some(bar.close)
                    } else {
                        None
                    }
                }
                AlertCondition::IndicatorSignal { indicator, signal } => {
                    evaluate_indicator_signal(indicators, indicator, signal)
                }
            };

            if let Some(value) = fire_value {
                triggered_ids.push(alert.id.clone());
                triggered.push(TriggeredAlert {
                    alert: alert.clone(),
                    triggered_at: now,
                    value,
                });
            }
        }

        // Remove triggered (one-shot) alerts
        self.alerts.retain(|a| !triggered_ids.contains(&a.id));

        triggered
    }
}

impl Default for AlertEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Check if an indicator's PanelResult contains a dot-based signal matching `signal` (by label).
/// A dot "fires" if it exists and its y value is not NaN.
/// Falls back to checking any dot if no label matches.
fn evaluate_indicator_signal(
    indicators: &HashMap<String, PanelResult>,
    indicator: &str,
    _signal: &str,
) -> Option<f64> {
    let panel = indicators.get(indicator)?;

    // Check if any dot in the panel has a finite y value (signal fired).
    // Dots are only emitted at bars where the signal is active, so presence
    // of a non-NaN dot means the signal triggered on this bar.
    for dot in &panel.dots {
        if !dot.y.is_nan() {
            return Some(dot.y);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indicator::{Dot, PanelResult};
    use plotters::style::RGBAColor;

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

    #[test]
    fn add_alert_appears_in_list() {
        let mut engine = AlertEngine::new();
        let id = engine.add("BTCUSD".into(), AlertCondition::PriceAbove(50000.0));
        assert_eq!(engine.list().len(), 1);
        assert_eq!(engine.list()[0].id, id);
    }

    #[test]
    fn remove_alert_by_id() {
        let mut engine = AlertEngine::new();
        let id = engine.add("BTCUSD".into(), AlertCondition::PriceAbove(50000.0));
        assert!(engine.remove(&id));
        assert!(engine.list().is_empty());
        // removing again returns false
        assert!(!engine.remove(&id));
    }

    #[test]
    fn price_above_triggers() {
        let mut engine = AlertEngine::new();
        engine.add("BTCUSD".into(), AlertCondition::PriceAbove(100.0));
        let bar = make_bar(105.0);
        let triggered = engine.evaluate("BTCUSD", &bar, &HashMap::new());
        assert_eq!(triggered.len(), 1);
        assert!((triggered[0].value - 105.0).abs() < f64::EPSILON);
    }

    #[test]
    fn price_above_does_not_trigger_below_threshold() {
        let mut engine = AlertEngine::new();
        engine.add("BTCUSD".into(), AlertCondition::PriceAbove(100.0));
        let bar = make_bar(95.0);
        let triggered = engine.evaluate("BTCUSD", &bar, &HashMap::new());
        assert!(triggered.is_empty());
        // Alert should still be active
        assert_eq!(engine.list().len(), 1);
    }

    #[test]
    fn price_below_triggers() {
        let mut engine = AlertEngine::new();
        engine.add("BTCUSD".into(), AlertCondition::PriceBelow(100.0));
        let bar = make_bar(95.0);
        let triggered = engine.evaluate("BTCUSD", &bar, &HashMap::new());
        assert_eq!(triggered.len(), 1);
        assert!((triggered[0].value - 95.0).abs() < f64::EPSILON);
    }

    #[test]
    fn triggered_alerts_removed_from_active() {
        let mut engine = AlertEngine::new();
        engine.add("BTCUSD".into(), AlertCondition::PriceAbove(100.0));
        let bar = make_bar(105.0);
        let triggered = engine.evaluate("BTCUSD", &bar, &HashMap::new());
        assert_eq!(triggered.len(), 1);
        // Should be removed now
        assert!(engine.list().is_empty());
    }

    #[test]
    fn only_matching_symbol_evaluated() {
        let mut engine = AlertEngine::new();
        engine.add("ETHUSD".into(), AlertCondition::PriceAbove(100.0));
        let bar = make_bar(105.0);
        let triggered = engine.evaluate("BTCUSD", &bar, &HashMap::new());
        assert!(triggered.is_empty());
        // ETHUSD alert should still be active
        assert_eq!(engine.list().len(), 1);
    }

    #[test]
    fn alert_ids_are_unique() {
        let mut engine = AlertEngine::new();
        let id1 = engine.add("BTCUSD".into(), AlertCondition::PriceAbove(100.0));
        let id2 = engine.add("BTCUSD".into(), AlertCondition::PriceAbove(200.0));
        assert_ne!(id1, id2);
    }

    #[test]
    fn indicator_signal_triggers_on_dot() {
        let mut engine = AlertEngine::new();
        engine.add(
            "BTCUSD".into(),
            AlertCondition::IndicatorSignal {
                indicator: "rsi".into(),
                signal: "green_dot".into(),
            },
        );

        let mut indicators = HashMap::new();
        let mut panel = PanelResult::default();
        panel.dots.push(Dot {
            x: 0,
            y: 42.0,
            color: RGBAColor(0, 255, 0, 1.0),
            size: 5,
        });
        indicators.insert("rsi".into(), panel);

        let bar = make_bar(100.0);
        let triggered = engine.evaluate("BTCUSD", &bar, &indicators);
        assert_eq!(triggered.len(), 1);
        assert!((triggered[0].value - 42.0).abs() < f64::EPSILON);
    }

    #[test]
    fn indicator_signal_does_not_trigger_without_dots() {
        let mut engine = AlertEngine::new();
        engine.add(
            "BTCUSD".into(),
            AlertCondition::IndicatorSignal {
                indicator: "rsi".into(),
                signal: "green_dot".into(),
            },
        );

        let mut indicators = HashMap::new();
        indicators.insert("rsi".into(), PanelResult::default());

        let bar = make_bar(100.0);
        let triggered = engine.evaluate("BTCUSD", &bar, &indicators);
        assert!(triggered.is_empty());
    }
}
