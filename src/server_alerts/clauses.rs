//! Conditions joined by AND (ADR-0084): an alert's `and` clauses must each hold at
//! the close its own condition fires at, or it does not fire.
//!
//! A clause is a price level or a bar-only indicator, with its own condition. Each is
//! read from the feed's window of closed bars at the bar that closed and the bar
//! before, so a clause needs nothing kept in memory and reads the same after a restart.

use chartgen::data::Bar;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{fires, indicator, Condition};

/// Most clauses an alert takes besides its own condition.
pub(super) const MAX_CLAUSES: usize = 3;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Clause {
    pub condition: Condition,
    /// The price level, or an indicator's level with `compare: level`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indicator: Option<indicator::Spec>,
}

/// The `and` clauses of `args`: none when absent.
pub(super) fn parse(args: &Value) -> Result<Vec<Clause>, String> {
    let list = match args.get("and") {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(list)) => list,
        Some(_) => return Err("and must be a list of conditions".into()),
    };
    if list.len() > MAX_CLAUSES {
        return Err(format!(
            "and takes at most {MAX_CLAUSES} conditions besides the alert's own"
        ));
    }
    list.iter()
        .enumerate()
        .map(|(i, c)| clause(c).map_err(|e| format!("and[{i}]: {e}")))
        .collect()
}

fn clause(v: &Value) -> Result<Clause, String> {
    let fields = v.as_object().ok_or("a condition must be an object")?;
    const TAKES: [&str; 6] = [
        "condition",
        "level",
        "indicator",
        "params",
        "line",
        "compare",
    ];
    if let Some(k) = fields.keys().find(|k| !TAKES.contains(&k.as_str())) {
        return Err(format!("a condition takes {}, not {k}", TAKES.join(", ")));
    }
    let condition: Condition =
        serde_json::from_value(v.get("condition").cloned().ok_or("condition is required")?)
            .map_err(|_| "condition has an unknown value")?;
    if condition == Condition::BarClose {
        return Err("bar_close is not a condition that holds or not".into());
    }
    // A futures-data indicator is read off the lock with the symbol (ADR-0080); a
    // condition is read from the bars alone, so it would never hold. Said before what
    // that indicator would ask for (oi's change_bars).
    if let Some(name) = indicator::futures_named(v) {
        return Err(format!(
            "{name} cannot be an and condition; make it the alert's own indicator"
        ));
    }
    let spec = indicator::parse(v)?;
    let level = match &spec {
        Some(_) => None,
        None => Some(
            v.get("level")
                .and_then(Value::as_f64)
                .filter(|l| l.is_finite() && *l > 0.0)
                .ok_or("level is required: a price greater than 0")?,
        ),
    };
    Ok(Clause {
        condition,
        level,
        indicator: spec,
    })
}

impl Clause {
    /// The comparison at the last of `bars` and the one before, and the value it read:
    /// the close, or the indicator's line.
    fn reading(&self, bars: &[Bar]) -> Option<(Option<f64>, f64, f64)> {
        match &self.indicator {
            Some(spec) => {
                let r = indicator::read(spec, bars)?;
                Some((r.prev, r.d, r.value))
            }
            None => {
                let level = self.level?;
                let last = bars.last()?.close;
                let prev = bars.len().checked_sub(2).map(|i| bars[i].close - level);
                Some((prev, last - level, last))
            }
        }
    }

    /// Whether it holds at the close of the last of `bars`, and its value there.
    /// A clause that cannot be read does not hold.
    pub(super) fn holds(&self, bars: &[Bar]) -> (bool, Option<f64>) {
        match self.reading(bars) {
            Some((prev, d, value)) => (fires(self.condition, prev, d), Some(value)),
            None => (false, None),
        }
    }

    /// As the alert's owner and its deliveries show it; `value` at the last close read.
    pub(super) fn view(&self, value: Option<f64>) -> Value {
        // An indicator's level is in its spec, as on the alert itself.
        let level = self
            .level
            .or_else(|| self.indicator.as_ref().and_then(|s| s.level));
        let mut v = json!({ "condition": self.condition, "level": level });
        match &self.indicator {
            Some(spec) => v["indicator"] = indicator::view(spec, value),
            None => v["price"] = json!(value),
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bars(closes: &[f64]) -> Vec<Bar> {
        closes
            .iter()
            .enumerate()
            .map(|(i, &c)| Bar {
                open: c,
                high: c,
                low: c,
                close: c,
                volume: 1.0,
                date: (i as i64 * 3600).to_string(),
            })
            .collect()
    }

    #[test]
    fn a_price_clause_reads_the_last_two_closes() {
        let c = parse(&json!({ "and": [{ "condition": "crossing_up", "level": 100 }] })).unwrap();
        assert_eq!(c[0].holds(&bars(&[99.0, 101.0])), (true, Some(101.0)));
        assert!(!c[0].holds(&bars(&[101.0, 102.0])).0);
        let c = parse(&json!({ "and": [{ "condition": "greater", "level": 100 }] })).unwrap();
        assert!(c[0].holds(&bars(&[101.0, 102.0])).0);
        assert!(!c[0].holds(&[]).0, "nothing read holds nothing");
    }

    #[test]
    fn what_a_clause_refuses() {
        let err = |v: Value| parse(&json!({ "and": [v] })).err().unwrap();
        assert_eq!(
            err(json!({ "condition": "greater" })),
            "and[0]: level is required: a price greater than 0"
        );
        assert_eq!(
            err(json!({ "condition": "bar_close" })),
            "and[0]: bar_close is not a condition that holds or not"
        );
        assert!(
            err(json!({ "condition": "less", "level": 1, "trigger": "once" }))
                .contains("not trigger")
        );
        assert!(
            err(json!({ "condition": "less", "indicator": "rsi" })).contains("level is required")
        );
        let four = json!({ "and": [
            { "condition": "less", "level": 1 }, { "condition": "less", "level": 1 },
            { "condition": "less", "level": 1 }, { "condition": "less", "level": 1 },
        ] });
        assert!(parse(&four).err().unwrap().contains("at most 3"));
        // Read off the lock with the symbol, so never from the bars alone.
        for name in ["funding", "funding_rate"] {
            assert_eq!(
                err(json!({ "indicator": name, "condition": "greater", "level": 0.01 })),
                "and[0]: funding cannot be an and condition; make it the alert's own indicator"
            );
        }
        // oi is refused as one too, before it would ask for change_bars.
        for name in ["oi", "open_interest"] {
            assert_eq!(
                err(json!({ "indicator": name, "condition": "greater", "level": 5 })),
                "and[0]: oi cannot be an and condition; make it the alert's own indicator"
            );
        }
        let rsi =
            parse(&json!({ "and": [{ "condition": "less", "indicator": "rsi", "level": 30 }] }))
                .unwrap();
        assert_eq!(rsi[0].indicator.as_ref().unwrap().line, "RSI");
        assert_eq!(
            rsi[0].view(Some(28.0))["level"],
            30.0,
            "shown as the alert's own is"
        );
        assert_eq!(rsi[0].view(Some(28.0))["indicator"]["value"], 28.0);
    }
}
