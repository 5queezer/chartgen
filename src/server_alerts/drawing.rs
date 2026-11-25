//! Server alerts on a drawing of a setup, at bar closes (ADR-0081).
//!
//! The alert copies the drawing's points when it is made: a later revision of the
//! setup's drawings does not move it. At each close of its interval the close is
//! compared with the drawing's price at that bar: a line's, a level's, or a
//! rectangle's top or bottom.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::Condition;

/// Lines through two points. A trend line (`segment`) is read past its end, as its
/// slope runs on; a ray from its origin; an extended line both ways.
const LINES: &[&str] = &["segment", "rayLine", "straightLine"];
/// A price, across the chart or from a point to the right.
const LEVELS: &[&str] = &["horizontalStraightLine", "horizontalRayLine", "simpleTag"];
/// A zone between two prices, from its left edge on.
const BOXES: &[&str] = &["cgRect"];

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Spec {
    pub setup_id: String,
    /// Its place in the setup's `drawings` when the alert was made.
    pub index: usize,
    pub tool: String,
    /// The points as the setup held them: open time of their bar in ms, and price.
    pub points: Vec<(i64, f64)>,
}

/// The drawing `args` name, or `None`. `drawings` reads the owner's setup.
pub(super) fn parse(
    args: &Value,
    condition: Condition,
    drawings: impl FnOnce(&str) -> Result<Vec<Value>, String>,
) -> Result<Option<Spec>, String> {
    let given = match args.get("drawing") {
        None | Some(Value::Null) => return Ok(None),
        Some(v) => v.as_object().ok_or("drawing must be {setup_id, index}")?,
    };
    if let Some(k) = given
        .keys()
        .find(|k| !["setup_id", "index"].contains(&k.as_str()))
    {
        return Err(format!("drawing takes setup_id and index, not {k}"));
    }
    let setup_id = given
        .get("setup_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or("drawing.setup_id is required")?;
    let index =
        given.get("index").and_then(Value::as_u64).ok_or(
            "drawing.index is required: the drawing's place in the setup's drawings, from 0",
        )? as usize;
    let all = drawings(setup_id)?;
    let d = all.get(index).ok_or_else(|| {
        format!(
            "Setup {setup_id} has {} drawing(s); index {index} is not one",
            all.len()
        )
    })?;
    let tool = d["tool"].as_str().unwrap_or_default().to_string();
    let points: Vec<(i64, f64)> = d["points"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| Some((p["timestamp"].as_i64()?, p["price"].as_f64()?)))
        .collect();
    let all_tools: Vec<&str> = LINES.iter().chain(LEVELS).chain(BOXES).copied().collect();
    if !all_tools.contains(&tool.as_str()) {
        return Err(format!(
            "A server alert cannot watch a {tool} drawing; it watches {}",
            all_tools.join(", ")
        ));
    }
    let spec = Spec {
        setup_id: setup_id.to_string(),
        index,
        tool,
        points,
    };
    spec.check(condition)?;
    Ok(Some(spec))
}

impl Spec {
    fn kind(&self) -> &'static str {
        match self.tool.as_str() {
            t if LINES.contains(&t) => "line",
            t if BOXES.contains(&t) => "box",
            _ => "level",
        }
    }

    /// What the drawing and `condition` cannot do together.
    pub(super) fn check(&self, condition: Condition) -> Result<(), String> {
        let need = match self.kind() {
            "level" => 1,
            _ => 2,
        };
        if self.points.len() < need {
            return Err(format!("This {} drawing has too few points", self.tool));
        }
        match self.kind() {
            "line" => {
                let ((t0, _), (t1, _)) = (self.points[0], self.points[1]);
                if t0 == t1 {
                    return Err("A line through two points of one bar is vertical: it has no price at a close".into());
                }
                if self.tool == "rayLine" && t1 < t0 {
                    return Err("This ray runs to the left: no close comes after it".into());
                }
            }
            "box" if condition == Condition::Crossing => {
                return Err(
                    "On a rectangle, send crossing_up (a close above its top) or crossing_down (below its bottom)"
                        .into(),
                );
            }
            _ => {}
        }
        if condition == Condition::BarClose {
            return Err("A bar_close alert takes no drawing".into());
        }
        Ok(())
    }

    /// The price a close on the bar opening at `t_ms` is compared with: a line's at
    /// that time, or a rectangle's top for an upward condition and its bottom for a
    /// downward one. `None` before the drawing starts.
    pub(super) fn level(&self, condition: Condition, t_ms: i64) -> Option<f64> {
        let (t0, p0) = *self.points.first()?;
        match self.kind() {
            "line" => {
                let (t1, p1) = *self.points.get(1)?;
                let start = match self.tool.as_str() {
                    "segment" => t0.min(t1),
                    "rayLine" => t0,
                    _ => i64::MIN,
                };
                (t_ms >= start)
                    .then(|| p0 + (p1 - p0) * (t_ms - t0) as f64 / (t1 - t0) as f64)
                    .filter(|p| p.is_finite())
            }
            "box" => {
                let (t1, p1) = *self.points.get(1)?;
                if t_ms < t0.min(t1) {
                    return None;
                }
                let up = matches!(condition, Condition::Greater | Condition::CrossingUp);
                Some(if up { p0.max(p1) } else { p0.min(p1) })
            }
            _ => (self.tool != "horizontalRayLine" || t_ms >= t0).then_some(p0),
        }
    }
}

/// The alert's drawing as its owner and its deliveries show it, with the price it
/// had at the last close read.
pub(super) fn view(spec: &Spec, level: Option<f64>) -> Value {
    json!({
        "setup_id": spec.setup_id,
        "index": spec.index,
        "tool": spec.tool,
        "points": spec.points.iter().map(|(t, p)| json!({ "timestamp": t, "price": p })).collect::<Vec<_>>(),
        "level": level,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(tool: &str, points: &[(i64, f64)]) -> Spec {
        Spec {
            setup_id: "s".into(),
            index: 0,
            tool: tool.into(),
            points: points.to_vec(),
        }
    }

    const H: i64 = 3_600_000;

    #[test]
    fn a_trend_line_runs_on_past_its_end_and_not_before_its_start() {
        let s = spec("segment", &[(0, 100.0), (10 * H, 110.0)]);
        assert_eq!(s.level(Condition::CrossingUp, 5 * H), Some(105.0));
        assert_eq!(s.level(Condition::CrossingUp, 20 * H), Some(120.0));
        assert_eq!(s.level(Condition::CrossingUp, -H), None);
        let ext = spec("straightLine", &[(0, 100.0), (10 * H, 110.0)]);
        assert_eq!(ext.level(Condition::Less, -10 * H), Some(90.0));
    }

    #[test]
    fn a_rectangle_is_its_top_upward_and_its_bottom_downward() {
        let s = spec("cgRect", &[(0, 90.0), (10 * H, 80.0)]);
        assert_eq!(s.level(Condition::CrossingUp, 50 * H), Some(90.0));
        assert_eq!(s.level(Condition::Greater, 50 * H), Some(90.0));
        assert_eq!(s.level(Condition::CrossingDown, 50 * H), Some(80.0));
        assert_eq!(s.level(Condition::Less, 50 * H), Some(80.0));
        assert!(s.check(Condition::Crossing).is_err());
    }

    #[test]
    fn levels_and_what_cannot_be_watched() {
        let h = spec("horizontalRayLine", &[(10 * H, 70.0)]);
        assert_eq!(h.level(Condition::Crossing, 9 * H), None);
        assert_eq!(h.level(Condition::Crossing, 11 * H), Some(70.0));
        assert!(spec("segment", &[(H, 1.0), (H, 2.0)])
            .check(Condition::Crossing)
            .is_err());
        assert!(spec("rayLine", &[(2 * H, 1.0), (H, 2.0)])
            .check(Condition::Crossing)
            .is_err());
        let err = parse(
            &json!({ "drawing": { "setup_id": "s", "index": 0 } }),
            Condition::Crossing,
            |_| Ok(vec![json!({ "tool": "cgText", "points": [] })]),
        )
        .err()
        .unwrap();
        assert!(
            err.starts_with("A server alert cannot watch a cgText drawing"),
            "{err}"
        );
        let err = parse(
            &json!({ "drawing": { "setup_id": "s", "index": 2 } }),
            Condition::Crossing,
            |_| Ok(vec![]),
        )
        .err()
        .unwrap();
        assert_eq!(err, "Setup s has 0 drawing(s); index 2 is not one");
    }
}
