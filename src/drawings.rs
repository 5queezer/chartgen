//! Drawings on a chart (ADR-0054): the web app's drawing tools, as an MCP client hands
//! them to `generate_chart`. They are checked all together against the bars of the chart,
//! and any problem fails the whole call, so nothing is drawn by half.
//!
//! `TOOLS` is the one table of the tools. `list_drawing_tools` serves it, the renderer
//! draws by it, and `web/src/generated/drawing-tools.json` is a copy that the web app's
//! tests hold its own tools to (`the_web_app_has_the_same_tools`).

use crate::data::Bar;
use serde_json::{json, Map, Value};

/// One drawing tool of the web app.
#[derive(Debug, PartialEq)]
pub struct Tool {
    /// The web app's name for it (`DRAWING_NAMES`).
    pub name: &'static str,
    /// Its label in the drawing toolbar.
    pub label: &'static str,
    /// What each point is, in order; the tool takes exactly this many.
    pub points: &'static [&'static str],
    pub summary: &'static str,
}

/// The web app's tools, in its order.
pub const TOOLS: [Tool; 18] = [
    Tool {
        name: "segment",
        label: "Trend line",
        points: &["start", "end"],
        summary: "A line between two points.",
    },
    Tool {
        name: "rayLine",
        label: "Ray",
        points: &["origin", "a point it passes through, after which it runs on to the edge"],
        summary: "A line from the first point through the second, extended past it.",
    },
    Tool {
        name: "straightLine",
        label: "Extended line",
        points: &["a point on the line", "another point on the line"],
        summary: "A line through two points, extended both ways.",
    },
    Tool {
        name: "horizontalStraightLine",
        label: "Horizontal line",
        points: &["the price level (its time only places the handle)"],
        summary: "A horizontal line across the whole chart.",
    },
    Tool {
        name: "horizontalRayLine",
        label: "Horizontal ray",
        points: &["origin; the ray runs right from it at its price"],
        summary: "A horizontal line from a point to the right edge.",
    },
    Tool {
        name: "verticalStraightLine",
        label: "Vertical line",
        points: &["the bar (its price only places the handle)"],
        summary: "A vertical line at a bar.",
    },
    Tool {
        name: "parallelStraightLine",
        label: "Parallel channel",
        points: &[
            "first point of the base line",
            "second point of the base line",
            "a point the parallel line passes through",
        ],
        summary: "Two parallel lines across the whole chart.",
    },
    Tool {
        name: "fibonacciLine",
        label: "Fib retracement",
        points: &["the 100% level", "the 0% level"],
        summary: "Levels at 0, 23.6, 38.2, 50, 61.8, 78.6 and 100% between two prices, across the whole chart, each labelled with its price.",
    },
    Tool {
        name: "cgRect",
        label: "Rectangle",
        points: &["a corner", "the opposite corner"],
        summary: "A filled rectangle, e.g. a support or resistance zone.",
    },
    Tool {
        name: "cgText",
        label: "Text annotation",
        points: &["where the text starts (its left edge, vertically centred)"],
        summary: "A text label; `text` is required.",
    },
    Tool {
        name: "simpleTag",
        label: "Price label",
        points: &["the price (its time only places the handle)"],
        summary: "A dashed line across the chart with its price on the right.",
    },
    Tool {
        name: "cgFibExt",
        label: "Fib extension (trend-based)",
        points: &["A, where the move starts", "B, where it ends", "C, where the projection starts"],
        summary: "Levels C + (B - A) x 0, 0.236, 0.382, 0.5, 0.618, 0.786, 1, 1.618, 2.618, 3.618 and 4.236, from C to the right edge.",
    },
    Tool {
        name: "cgPitchfork",
        label: "Pitchfork",
        points: &["P1, the handle", "P2", "P3"],
        summary: "Andrews' pitchfork: a median line from P1 through the middle of P2-P3, and parallels through P2 and P3, to the edge.",
    },
    Tool {
        name: "cgPriceRange",
        label: "Price range",
        points: &["from", "to"],
        summary: "A band between two prices, labelled with the change and its percentage.",
    },
    Tool {
        name: "cgDateRange",
        label: "Date range",
        points: &["from", "to"],
        summary: "A band between two times, labelled with the bars and the time between them.",
    },
    Tool {
        name: "cgDatePriceRange",
        label: "Date and price range",
        points: &["from", "to"],
        summary: "A box between two points, labelled with the price change and the time between them.",
    },
    Tool {
        name: "cgLong",
        label: "Long position",
        points: &[
            "entry; its time is the box's left edge",
            "target, above the entry; its time is the box's right edge",
            "stop, below the entry; the same time as the target",
        ],
        summary: "A long position: a green target zone above the entry, a red stop zone below it, and the risk/reward ratio.",
    },
    Tool {
        name: "cgShort",
        label: "Short position",
        points: &[
            "entry; its time is the box's left edge",
            "target, below the entry; its time is the box's right edge",
            "stop, above the entry; the same time as the target",
        ],
        summary: "A short position: a green target zone below the entry, a red stop zone above it, and the risk/reward ratio.",
    },
];

/// Tools with a fill the user can recolor (the web app's `FILLED`).
pub const FILLED: [&str; 4] = ["cgRect", "cgPriceRange", "cgDateRange", "cgDatePriceRange"];
/// Most drawings in one call.
pub const MAX_DRAWINGS: usize = 50;
/// Longest text, as in the web app.
pub const MAX_TEXT: usize = 200;
/// Line widths in px, and the text sizes, the web app's style bar offers.
const MAX_WIDTH: u64 = 4;
pub const FONT_SIZES: [u64; 5] = [10, 12, 14, 18, 24];
pub const DASHES: [&str; 3] = ["solid", "dashed", "dotted"];

impl Tool {
    pub fn by_name(name: &str) -> Option<&'static Tool> {
        TOOLS.iter().find(|t| t.name == name)
    }

    pub fn takes_text(&self) -> bool {
        self.name == "cgText"
    }

    /// The style fields the web app's style bar sets on this tool.
    pub fn style_fields(&self) -> Vec<&'static str> {
        if self.takes_text() {
            return vec!["color", "fontSize"];
        }
        let mut f = vec!["color", "width", "dash"];
        if FILLED.contains(&self.name) {
            f.push("fill");
        }
        f
    }
}

/// `list_drawing_tools`' answer, and the web app's copy of the table.
pub fn tools_json() -> Value {
    let tools: Vec<Value> = TOOLS
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "label": t.label,
                "summary": t.summary,
                "points": t.points,
                "style": t.style_fields(),
                "text": t.takes_text(),
            })
        })
        .collect();
    json!({
        "tools": tools,
        "max_drawings": MAX_DRAWINGS,
        "max_text": MAX_TEXT,
        "style": {
            "color": "#rgb, #rrggbb, #rrggbbaa or rgba(r, g, b, a)",
            "width": "line width in px, 1 to 4",
            "dash": DASHES,
            "fill": "fill color, as color; may carry alpha",
            "fontSize": FONT_SIZES,
        },
        "point": "{\"time\": T, \"price\": P} or {\"time\": T, \"snap\": \"open\"|\"high\"|\"low\"|\"close\"}. T is unix seconds, unix milliseconds or an ISO 8601 date or date-time (UTC unless it has an offset); it resolves to the bar containing it. snap takes that bar's value.",
    })
}

/// A bar value a point can be anchored to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Snap {
    Open,
    High,
    Low,
    Close,
}

impl Snap {
    fn parse(s: &str) -> Option<Snap> {
        match s {
            "open" => Some(Snap::Open),
            "high" => Some(Snap::High),
            "low" => Some(Snap::Low),
            "close" => Some(Snap::Close),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Snap::Open => "open",
            Snap::High => "high",
            Snap::Low => "low",
            Snap::Close => "close",
        }
    }

    fn of(self, bar: &Bar) -> f64 {
        match self {
            Snap::Open => bar.open,
            Snap::High => bar.high,
            Snap::Low => bar.low,
            Snap::Close => bar.close,
        }
    }
}

/// A drawing's look; absent fields take the tool's own, as in the web app.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Style {
    pub color: Option<String>,
    pub width: Option<u32>,
    pub dash: Option<&'static str>,
    pub fill: Option<String>,
    pub font_size: Option<u32>,
}

impl Style {
    fn to_json(&self) -> Value {
        let mut m = Map::new();
        if let Some(c) = &self.color {
            m.insert("color".into(), json!(c));
        }
        if let Some(w) = self.width {
            m.insert("width".into(), json!(w));
        }
        if let Some(d) = self.dash {
            m.insert("dash".into(), json!(d));
        }
        if let Some(f) = &self.fill {
            m.insert("fill".into(), json!(f));
        }
        if let Some(s) = self.font_size {
            m.insert("fontSize".into(), json!(s));
        }
        Value::Object(m)
    }

    fn pairs(&self) -> Vec<(&'static str, String)> {
        let mut out = vec![];
        if let Some(c) = &self.color {
            out.push(("color", c.clone()));
        }
        if let Some(f) = &self.fill {
            out.push(("fill", f.clone()));
        }
        if let Some(w) = self.width {
            out.push(("width", w.to_string()));
        }
        if let Some(d) = self.dash {
            out.push(("dash", d.to_string()));
        }
        if let Some(s) = self.font_size {
            out.push(("fontSize", s.to_string()));
        }
        out
    }
}

enum PriceOf {
    Price(f64),
    Snap(Snap),
}

/// A drawing as asked for, before its times meet the bars.
pub struct Request {
    tool: &'static Tool,
    points: Vec<(i64, PriceOf)>,
    style: Style,
    text: Option<String>,
}

/// A point on the chart.
#[derive(Clone, Debug, PartialEq)]
pub struct Point {
    /// The open time of its bar, in ms; past the last bar, on that bar's grid.
    pub time_ms: i64,
    /// Bars from the first bar of the chart.
    pub x: i64,
    pub price: f64,
    pub snap: Option<Snap>,
}

/// A drawing whose points are on the chart's bars.
#[derive(Clone, Debug, PartialEq)]
pub struct Drawing {
    pub tool: &'static Tool,
    pub points: Vec<Point>,
    pub style: Style,
    pub text: Option<String>,
}

/// What is wrong with one drawing (`index`), or with the list (`index` None).
#[derive(Debug, PartialEq)]
pub struct Problem {
    pub index: Option<usize>,
    pub tool: Option<String>,
    pub message: String,
    pub hint: Option<String>,
}

impl Problem {
    fn new(index: Option<usize>, tool: Option<&str>, message: String) -> Problem {
        Problem {
            index,
            tool: tool.map(str::to_string),
            message,
            hint: None,
        }
    }

    fn hint(mut self, hint: impl Into<String>) -> Problem {
        self.hint = Some(hint.into());
        self
    }

    pub fn to_json(&self) -> Value {
        json!({ "index": self.index, "tool": self.tool, "message": self.message, "hint": self.hint })
    }
}

/// The error text of a call whose drawings did not all pass.
pub fn problems_text(problems: &[Problem]) -> String {
    let all: Vec<Value> = problems.iter().map(Problem::to_json).collect();
    serde_json::to_string_pretty(&json!({
        "error": format!(
            "{} problem(s) with drawings; nothing was drawn. Fix them and send the call again.",
            problems.len()
        ),
        "drawing_errors": all,
    }))
    .unwrap_or_default()
}

/// Oldest and newest time a point may name: 1900-01-01, and a year from `now_ms`.
const MIN_TIME_MS: i64 = -2_208_988_800_000;
const AHEAD_MS: i64 = 366 * 86_400_000;

/// A point's time in ms: unix seconds (below 1e11 in magnitude) or ms, or an ISO 8601
/// date or date-time, UTC unless it carries an offset.
pub fn parse_time(v: &Value, now_ms: i64) -> Result<i64, String> {
    let ms = match v {
        Value::Number(n) => {
            let f = n
                .as_f64()
                .filter(|f| f.is_finite())
                .ok_or("time is not a finite number")?;
            if f.abs() < 1e11 {
                (f * 1000.0).round() as i64
            } else {
                f.round() as i64
            }
        }
        Value::String(s) => {
            let s = s.trim();
            if let Ok(d) = chrono::DateTime::parse_from_rfc3339(s) {
                d.timestamp_millis()
            } else if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
                d.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp_millis()
            } else if let Some(d) = [
                "%Y-%m-%dT%H:%M:%S%.f",
                "%Y-%m-%dT%H:%M",
                "%Y-%m-%d %H:%M:%S%.f",
                "%Y-%m-%d %H:%M",
            ]
            .iter()
            .find_map(|f| chrono::NaiveDateTime::parse_from_str(s, f).ok())
            {
                d.and_utc().timestamp_millis()
            } else {
                return Err(format!(
                    "time {s:?} is neither unix seconds, unix ms nor an ISO 8601 date (2026-10-12) or date-time (2026-10-12T14:00:00Z)"
                ));
            }
        }
        _ => return Err("time must be a number (unix s or ms) or an ISO 8601 string".into()),
    };
    if !(MIN_TIME_MS..=now_ms.saturating_add(AHEAD_MS)).contains(&ms) {
        return Err(format!(
            "time {} is out of range: 1900-01-01 to a year from now",
            iso(ms)
        ));
    }
    Ok(ms)
}

/// The colors the web app's stores accept: `#rgb`, `#rrggbb`, `#rrggbbaa`, `rgba(r, g, b, a)`.
pub fn parse_color(s: &str) -> Option<(u8, u8, u8, f64)> {
    let hex = |h: &str| u8::from_str_radix(h, 16).ok();
    if let Some(h) = s.strip_prefix('#') {
        if !h.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let d = |i: usize| hex(&h[i..i + 1]).map(|v| v * 17);
        return match h.len() {
            3 => Some((d(0)?, d(1)?, d(2)?, 1.0)),
            6 | 8 => Some((
                hex(&h[0..2])?,
                hex(&h[2..4])?,
                hex(&h[4..6])?,
                if h.len() == 8 {
                    hex(&h[6..8])? as f64 / 255.0
                } else {
                    1.0
                },
            )),
            _ => None,
        };
    }
    let lower = s.to_ascii_lowercase();
    let inner = lower.strip_prefix("rgba(")?.strip_suffix(')')?;
    let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
    let [r, g, b, a] = parts[..] else { return None };
    // As the web app's pattern: no sign, no leading zeros, no exponent.
    let channel = |c: &str| {
        let ok = !c.is_empty()
            && c.bytes().all(|b| b.is_ascii_digit())
            && (c == "0" || !c.starts_with('0'));
        ok.then(|| c.parse::<u8>().ok()).flatten()
    };
    let digits = |t: &str| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit());
    let alpha_ok = a == "0"
        || a == "1"
        || a.strip_prefix("0.")
            .or_else(|| a.strip_prefix('.'))
            .is_some_and(digits)
        || a.strip_prefix("1.")
            .is_some_and(|t| !t.is_empty() && t.bytes().all(|b| b == b'0'));
    if !alpha_ok {
        return None;
    }
    Some((channel(r)?, channel(g)?, channel(b)?, a.parse().ok()?))
}

fn parse_style(v: Option<&Value>, tool: &Tool) -> Result<Style, String> {
    let mut style = Style::default();
    let Some(v) = v.filter(|v| !v.is_null()) else {
        return Ok(style);
    };
    let obj = v.as_object().ok_or("style must be an object")?;
    let fields = tool.style_fields();
    for (k, v) in obj {
        if !fields.contains(&k.as_str()) {
            return Err(format!(
                "style.{k} does not apply to {}; it takes {}",
                tool.name,
                fields.join(", ")
            ));
        }
        match k.as_str() {
            "color" | "fill" => {
                let c = v
                    .as_str()
                    .filter(|c| parse_color(c).is_some())
                    .ok_or(format!(
                        "style.{k} must be #rgb, #rrggbb, #rrggbbaa or rgba(r, g, b, a)"
                    ))?;
                if k == "color" {
                    style.color = Some(c.to_string());
                } else {
                    style.fill = Some(c.to_string());
                }
            }
            "width" => {
                let w = v.as_u64().filter(|w| (1..=MAX_WIDTH).contains(w));
                style.width = Some(w.ok_or("style.width must be an integer from 1 to 4")? as u32);
            }
            "dash" => {
                let d = v.as_str().and_then(|d| DASHES.iter().find(|x| **x == d));
                style.dash = Some(d.ok_or("style.dash must be solid, dashed or dotted")?);
            }
            _ => {
                let s = v.as_u64().filter(|s| FONT_SIZES.contains(s));
                style.font_size =
                    Some(s.ok_or("style.fontSize must be one of 10, 12, 14, 18, 24")? as u32);
            }
        }
    }
    Ok(style)
}

fn parse_point(v: &Value, now_ms: i64) -> Result<(i64, PriceOf), String> {
    let obj = v
        .as_object()
        .ok_or("must be an object {\"time\", \"price\"} or {\"time\", \"snap\"}")?;
    if let Some(k) = obj
        .keys()
        .find(|k| !["time", "price", "snap"].contains(&k.as_str()))
    {
        return Err(format!(
            "unknown field {k:?}; a point has time and price, or time and snap"
        ));
    }
    let time = parse_time(obj.get("time").ok_or("time is missing")?, now_ms)?;
    let value = match (obj.get("price"), obj.get("snap")) {
        (Some(_), Some(_)) => return Err("give price or snap, not both".into()),
        (Some(p), None) => PriceOf::Price(
            p.as_f64()
                .filter(|p| p.is_finite())
                .ok_or("price must be a finite number")?,
        ),
        (None, Some(s)) => PriceOf::Snap(
            s.as_str()
                .and_then(Snap::parse)
                .ok_or("snap must be open, high, low or close")?,
        ),
        (None, None) => return Err("price (or snap) is missing".into()),
    };
    Ok((time, value))
}

/// Check the shape of every drawing; all problems at once.
pub fn parse(v: &Value, now_ms: i64) -> Result<Vec<Request>, Vec<Problem>> {
    let Some(list) = v.as_array() else {
        return Err(vec![Problem::new(
            None,
            None,
            "drawings must be an array".into(),
        )]);
    };
    if list.len() > MAX_DRAWINGS {
        return Err(vec![Problem::new(
            None,
            None,
            format!(
                "at most {MAX_DRAWINGS} drawings per call, got {}",
                list.len()
            ),
        )]);
    }
    let mut out = vec![];
    let mut problems = vec![];
    for (i, d) in list.iter().enumerate() {
        match parse_one(d, now_ms) {
            Ok(r) => out.push(r),
            Err(p) => problems.push(Problem {
                index: Some(i),
                ..p
            }),
        }
    }
    if problems.is_empty() {
        Ok(out)
    } else {
        Err(problems)
    }
}

fn parse_one(d: &Value, now_ms: i64) -> Result<Request, Problem> {
    let fail = |tool: Option<&str>, m: String| Problem::new(None, tool, m);
    let obj = d.as_object().ok_or_else(|| {
        fail(
            None,
            "a drawing must be an object {tool, points, style?, text?}".into(),
        )
    })?;
    let name = obj.get("tool").and_then(Value::as_str).unwrap_or("");
    let tool = Tool::by_name(name).ok_or_else(|| {
        fail(None, format!("unknown tool {name:?}"))
            .hint("call list_drawing_tools for the 18 tools")
    })?;
    let fail = |m: String| Problem::new(None, Some(tool.name), m);
    if let Some(k) = obj
        .keys()
        .find(|k| !["tool", "points", "style", "text"].contains(&k.as_str()))
    {
        return Err(fail(format!(
            "unknown field {k:?}; a drawing has tool, points, style and text"
        )));
    }
    let points = obj
        .get("points")
        .and_then(Value::as_array)
        .ok_or_else(|| fail("points must be an array".into()))?;
    let want = tool.points.len();
    if points.len() != want {
        return Err(fail(format!(
            "{} takes exactly {want} point(s), got {}",
            tool.name,
            points.len()
        ))
        .hint(format!("its points are: {}", tool.points.join("; "))));
    }
    let points = points
        .iter()
        .enumerate()
        .map(|(j, p)| parse_point(p, now_ms).map_err(|m| fail(format!("points[{j}]: {m}"))))
        .collect::<Result<Vec<_>, _>>()?;
    let style = parse_style(obj.get("style"), tool).map_err(fail)?;
    let text = match obj.get("text").filter(|t| !t.is_null()) {
        None if tool.takes_text() => return Err(fail("cgText needs text".into())),
        None => None,
        Some(_) if !tool.takes_text() => {
            return Err(fail(format!(
                "{} takes no text; only cgText does",
                tool.name
            )))
        }
        Some(t) => {
            let t = t
                .as_str()
                .ok_or_else(|| fail("text must be a string".into()))?;
            if t.trim().is_empty()
                // In UTF-16 units, as the web app's stores count it.
                || t.encode_utf16().count() > MAX_TEXT
                || t.chars().any(char::is_control)
            {
                return Err(fail(format!(
                    "text must be 1 to {MAX_TEXT} characters on one line"
                )));
            }
            Some(t.to_string())
        }
    };
    Ok(Request {
        tool,
        points,
        style,
        text,
    })
}

/// `2026-10-12T14:00:00Z`.
pub fn iso(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| d.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_else(|| ms.to_string())
}

/// Open times of `bars` in ms; None for the demo series, which has none.
pub fn bar_times(bars: &[Bar]) -> Option<Vec<i64>> {
    bars.iter()
        .map(|b| b.date.parse::<i64>().ok().map(|s| s * 1000))
        .collect()
}

/// The usual spacing of bars, in ms: the most common of the last 100 gaps, so weekends and
/// sessions do not count (as the web app's `timeAxisOf`). 0 with fewer than two bars.
fn spacing(times: &[i64]) -> i64 {
    let mut counts: Vec<(i64, usize)> = vec![];
    for w in times[times.len().saturating_sub(101)..].windows(2) {
        let d = w[1] - w[0];
        if d <= 0 {
            continue;
        }
        match counts.iter_mut().find(|(g, _)| *g == d) {
            Some((_, c)) => *c += 1,
            None => counts.push((d, 1)),
        }
    }
    counts.iter().max_by_key(|(_, c)| *c).map_or(0, |(g, _)| *g)
}

/// What the PNG shows of the bars, for warnings: its price range and how far right it goes.
pub struct View {
    pub price_lo: f64,
    pub price_hi: f64,
    /// Most bars past the last one the image extends to for a drawing.
    pub future: i64,
}

impl View {
    pub fn of(bars: &[Bar]) -> View {
        let lo = bars.iter().map(|b| b.low).fold(f64::INFINITY, f64::min);
        let hi = bars
            .iter()
            .map(|b| b.high)
            .fold(f64::NEG_INFINITY, f64::max);
        let margin = (hi - lo) * 0.05;
        View {
            price_lo: lo - margin,
            price_hi: hi + margin,
            future: (bars.len() as i64 / 2).max(1),
        }
    }
}

/// Drawings on the bars, and what an agent should know about how they came out.
pub struct Resolved {
    pub drawings: Vec<Drawing>,
    pub warnings: Vec<String>,
}

/// Put every drawing on `bars`, the bars the chart shows; all problems at once.
pub fn resolve(requests: Vec<Request>, bars: &[Bar]) -> Result<Resolved, Vec<Problem>> {
    let Some(times) = bar_times(bars).filter(|t| !t.is_empty()) else {
        return Err(vec![Problem::new(
            None,
            None,
            "this chart has no bar times to put drawings on".into(),
        )
        .hint("pass a ticker; the demo series without one has no times")]);
    };
    let gap = spacing(&times);
    let view = View::of(bars);
    let mut drawings = vec![];
    let mut problems = vec![];
    let mut warnings = vec![];
    for (i, r) in requests.into_iter().enumerate() {
        let at = |j: usize| format!("drawings[{i}].points[{j}]");
        let mut points = vec![];
        let mut failed = None;
        for (j, (t, v)) in r.points.iter().enumerate() {
            match place(*t, v, bars, &times, gap) {
                Ok(p) => points.push(p),
                Err((m, h)) => {
                    failed = Some(
                        Problem::new(Some(i), Some(r.tool.name), format!("points[{j}]: {m}"))
                            .hint(h),
                    );
                    break;
                }
            }
        }
        if let Some(p) = failed.or_else(|| check(i, r.tool, &points)) {
            problems.push(p);
            continue;
        }
        for (j, p) in points.iter().enumerate() {
            let past = p.x - (times.len() as i64 - 1);
            if past > view.future {
                warnings.push(format!(
                    "{}: {} bars past the last bar, beyond the image's right edge ({} bars past it); that part is cut off",
                    at(j), past, view.future
                ));
            }
            if r.tool.name != "verticalStraightLine"
                && !(view.price_lo..=view.price_hi).contains(&p.price)
            {
                warnings.push(format!(
                    "{}: price {} is outside the visible price range {:.6} to {:.6}; that part is cut off",
                    at(j), p.price, view.price_lo, view.price_hi
                ));
            }
        }
        drawings.push(Drawing {
            tool: r.tool,
            points,
            style: r.style,
            text: r.text,
        });
    }
    if problems.is_empty() {
        Ok(Resolved { drawings, warnings })
    } else {
        Err(problems)
    }
}

/// A point on the bar containing `t`, or past the last bar on its grid (rounded to a bar, as
/// the web app places it).
fn place(
    t: i64,
    v: &PriceOf,
    bars: &[Bar],
    times: &[i64],
    gap: i64,
) -> Result<Point, (String, String)> {
    let first = times[0];
    let last = *times.last().unwrap();
    if t < first {
        return Err((
            format!(
                "time {} is before the first bar of the chart, {}",
                iso(t),
                iso(first)
            ),
            format!(
                "load more history (a larger bars, up to 5000) or pick a time at or after {}",
                iso(first)
            ),
        ));
    }
    let i = times.partition_point(|&x| x <= t) - 1;
    let (time_ms, x, bar) = if i + 1 < times.len() || gap == 0 || t < last + gap {
        (times[i], i as i64, Some(&bars[i]))
    } else {
        let k = ((t - last) as f64 / gap as f64).round() as i64;
        (last + k * gap, (times.len() - 1) as i64 + k, None)
    };
    let (price, snap) = match (v, bar) {
        (PriceOf::Price(p), _) => (*p, None),
        (PriceOf::Snap(s), Some(b)) => (s.of(b), Some(*s)),
        (PriceOf::Snap(_), None) => {
            return Err((
                format!(
                    "snap needs a bar, and time {} is past the last bar, {}",
                    iso(t),
                    iso(last)
                ),
                "give a price for a point in the future".into(),
            ))
        }
    };
    Ok(Point {
        time_ms,
        x,
        price,
        snap,
    })
}

/// What a tool needs of its placed points.
fn check(i: usize, tool: &'static Tool, p: &[Point]) -> Option<Problem> {
    let fail = |m: &str, h: &str| Some(Problem::new(Some(i), Some(tool.name), m.into()).hint(h));
    let same = |a: &Point, b: &Point| a.x == b.x && a.price == b.price;
    match tool.name {
        "segment" | "rayLine" | "straightLine" | "parallelStraightLine" if same(&p[0], &p[1]) => {
            fail(
                "the line's two points are the same point",
                "give the second point another bar or price",
            )
        }
        "cgLong" | "cgShort" => {
            let long = tool.name == "cgLong";
            let (e, t, s) = (&p[0], &p[1], &p[2]);
            if t.time_ms != s.time_ms {
                return fail(
                    "the target and the stop must have the same time: both are the box's right edge",
                    "give the stop the target's time",
                );
            }
            let ok = if long {
                t.price > e.price && s.price < e.price
            } else {
                t.price < e.price && s.price > e.price
            };
            (!ok).then(|| {
                Problem::new(
                    Some(i),
                    Some(tool.name),
                    format!(
                        "a {} needs the target {} the entry and the stop {} it",
                        if long { "long" } else { "short" },
                        if long { "above" } else { "below" },
                        if long { "below" } else { "above" },
                    ),
                )
                .hint("points are entry, target, stop")
            })
        }
        _ => None,
    }
}

impl Drawing {
    pub fn to_json(&self) -> Value {
        let points: Vec<Value> = self
            .points
            .iter()
            .map(|p| {
                let mut v = json!({ "time": iso(p.time_ms), "timestamp": p.time_ms, "price": p.price, "bar": p.x });
                if let Some(s) = p.snap {
                    v["snap"] = json!(s.as_str());
                }
                v
            })
            .collect();
        let mut v =
            json!({ "tool": self.tool.name, "points": points, "style": self.style.to_json() });
        if let Some(t) = &self.text {
            v["text"] = json!(t);
        }
        v
    }
}

/// Longest `draw=` value a share link carries, as the web app reads it.
pub const MAX_SHARE_LENGTH: usize = 8192;

/// The `draw=` value of a share link for `drawings` (ADR-0054), or None when it would be
/// longer than `MAX_SHARE_LENGTH`. Items `tool(t:p;t:p;key:value)` joined by commas: a point is
/// unix seconds and a price, values are percent-encoded outside `[A-Za-z0-9._-]`.
pub fn share_param(drawings: &[Drawing]) -> Option<String> {
    let items: Vec<String> = drawings
        .iter()
        .map(|d| {
            let mut parts: Vec<String> = d
                .points
                .iter()
                .map(|p| format!("{}:{}", seconds(p.time_ms), p.price))
                .collect();
            let mut pairs = d.style.pairs();
            if let Some(t) = &d.text {
                pairs.push(("text", t.clone()));
            }
            parts.extend(
                pairs
                    .into_iter()
                    .map(|(k, v)| format!("{k}:{}", escape(&v))),
            );
            format!("{}({})", d.tool.name, parts.join(";"))
        })
        .collect();
    let value = items.join(",");
    (value.len() <= MAX_SHARE_LENGTH).then_some(value)
}

fn seconds(ms: i64) -> String {
    if ms % 1000 == 0 {
        (ms / 1000).to_string()
    } else {
        format!("{}", ms as f64 / 1000.0)
    }
}

/// Every UTF-8 byte outside `[A-Za-z0-9._-]` as `%XX`, as the web app's `textValue`.
pub fn escape(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"._-".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// Swing highs and lows of `bars`: zigzag pivots with `depth` bars on each side, alternating,
/// newest last, at most `max`. As (bar index, price, is_high).
pub fn swings(bars: &[Bar], depth: usize, max: usize) -> Vec<(usize, f64, bool)> {
    let all = crate::indicators::trend::zigzag_pivots(bars, depth, |_| 0.0);
    all[all.len().saturating_sub(max)..].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_790_000_000_000;

    /// Bars an hour apart from 2026-10-12T00:00Z, rising by one from 100.
    fn bars(n: usize) -> Vec<Bar> {
        let start = 1_760_227_200; // 2025-10-12T00:00:00Z
        (0..n)
            .map(|i| {
                let o = 100.0 + i as f64;
                Bar {
                    open: o,
                    high: o + 2.0,
                    low: o - 1.0,
                    close: o + 1.0,
                    volume: 1.0,
                    date: (start + i as i64 * 3600).to_string(),
                }
            })
            .collect()
    }

    fn resolve_json(v: Value) -> Result<Resolved, Vec<Problem>> {
        resolve(parse(&v, NOW)?, &bars(50))
    }

    #[test]
    fn times_are_seconds_ms_or_iso() {
        let s = 1_760_227_200_000;
        for v in [
            json!(1_760_227_200),
            json!(1_760_227_200_000i64),
            json!(1_760_227_200.0),
            json!("2025-10-12"),
            json!("2025-10-12T00:00:00Z"),
            json!("2025-10-12T02:00:00+02:00"),
            json!("2025-10-12T00:00"),
            json!("2025-10-12 00:00:00"),
        ] {
            assert_eq!(parse_time(&v, NOW), Ok(s), "{v}");
        }
        assert!(parse_time(&json!("yesterday"), NOW).is_err());
        assert!(parse_time(&json!(true), NOW).is_err());
        assert!(parse_time(&json!("1800-01-01"), NOW).is_err());
    }

    #[test]
    fn a_time_lands_on_the_bar_containing_it() {
        let r = resolve_json(json!([
            { "tool": "segment", "points": [
                { "time": "2025-10-12T01:30:00Z", "price": 5 },
                { "time": "2025-10-12T03:00:00Z", "snap": "high" }
            ]}
        ]))
        .unwrap();
        let p = &r.drawings[0].points;
        assert_eq!((p[0].x, p[0].time_ms), (1, 1_760_227_200_000 + 3_600_000));
        assert_eq!(
            (p[1].x, p[1].price, p[1].snap),
            (3, 105.0, Some(Snap::High))
        );
    }

    #[test]
    fn a_time_past_the_last_bar_is_on_its_grid() {
        // Last bar 49 at +49h; +60h20m rounds to 11 bars past it.
        let t = 1_760_227_200 + 60 * 3600 + 20 * 60;
        let r = resolve_json(
            json!([{ "tool": "horizontalRayLine", "points": [{ "time": t, "price": 120 }] }]),
        )
        .unwrap();
        let p = &r.drawings[0].points[0];
        assert_eq!(p.x, 60);
        assert_eq!(p.time_ms, (1_760_227_200 + 60 * 3600) * 1000);
        // Within the last bar's hour it is the last bar.
        let r = resolve_json(json!([{ "tool": "horizontalRayLine", "points": [{ "time": 1_760_227_200 + 49 * 3600 + 59 * 60, "price": 120 }] }]))
            .unwrap();
        assert_eq!(r.drawings[0].points[0].x, 49);
    }

    #[test]
    fn before_the_first_bar_fails_with_a_hint() {
        let e = resolve_json(json!([{ "tool": "segment", "points": [
            { "time": "2025-10-01", "price": 1 }, { "time": "2025-10-12", "price": 2 }
        ]}]))
        .err()
        .unwrap();
        assert_eq!(e[0].index, Some(0));
        assert!(
            e[0].message
                .contains("before the first bar of the chart, 2025-10-12T00:00:00Z"),
            "{}",
            e[0].message
        );
        assert!(e[0].hint.as_deref().unwrap().contains("load more history"));
    }

    #[test]
    fn snap_needs_a_loaded_bar() {
        let e = resolve_json(json!([{ "tool": "horizontalStraightLine", "points": [{ "time": "2026-01-01", "snap": "close" }] }]))
            .err()
            .unwrap();
        assert!(e[0].message.contains("snap needs a bar"));
    }

    #[test]
    fn one_bad_drawing_fails_them_all_and_each_is_named() {
        let e = resolve_json(json!([
            { "tool": "segment", "points": [{ "time": "2025-10-12", "price": 1 }, { "time": "2025-10-12T05:00:00Z", "price": 2 }] },
            { "tool": "cgRect", "points": [{ "time": "2025-10-12", "price": 1 }] },
            { "tool": "nope", "points": [] },
            { "tool": "cgText", "points": [{ "time": "2025-10-12", "price": 1 }] },
            { "tool": "segment", "points": [{ "time": "2025-10-12", "price": 1 }, { "time": "2025-10-12", "price": 1 }], "style": { "fill": "#fff" } }
        ]))
        .err()
        .unwrap();
        let idx: Vec<_> = e.iter().map(|p| p.index).collect();
        assert_eq!(idx, vec![Some(1), Some(2), Some(3), Some(4)]);
        assert!(e[0].message.contains("exactly 2 point(s)"));
        assert!(e[1].message.contains("unknown tool"));
        assert!(e[2].message.contains("needs text"));
        assert!(e[3].message.contains("style.fill does not apply"));
        let text = problems_text(&e);
        assert!(text.contains("nothing was drawn"));
    }

    #[test]
    fn each_tool_takes_its_points() {
        for t in &TOOLS {
            let pts = |n: usize| -> Vec<Value> {
                (0..n)
                    .map(|k| json!({ "time": 1_760_227_200 + (k as i64 + 1) * 3600, "price": 100 + 10 * k as i64 }))
                    .collect()
            };
            let mk = |n| json!([{ "tool": t.name, "points": pts(n), "text": t.takes_text().then_some("x") }]);
            assert!(parse(&mk(t.points.len() + 1), NOW).is_err(), "{}", t.name);
            if t.points.len() > 1 {
                assert!(parse(&mk(t.points.len() - 1), NOW).is_err(), "{}", t.name);
            }
            assert!(parse(&mk(t.points.len()), NOW).is_ok(), "{}", t.name);
        }
    }

    #[test]
    fn text_is_counted_as_the_web_app_counts_it() {
        let text = |t: String| {
            let v = json!([{ "tool": "cgText", "points": [{ "time": 1_760_227_200, "price": 1 }], "text": t }]);
            parse(&v, NOW).is_ok()
        };
        assert!(text("x".repeat(200)));
        assert!(!text("x".repeat(201)));
        // Outside the BMP a character is two UTF-16 units.
        assert!(text("📈".repeat(100)));
        assert!(!text("📈".repeat(101)));
    }

    #[test]
    fn positions_check_their_sides() {
        let long = |t: f64, s: f64, ts: i64| {
            json!([{ "tool": "cgLong", "points": [
                { "time": 1_760_227_200, "price": 110 },
                { "time": 1_760_227_200 + 10 * 3600, "price": t },
                { "time": ts, "price": s }
            ]}])
        };
        let edge = 1_760_227_200 + 10 * 3600;
        assert!(resolve_json(long(120.0, 105.0, edge)).is_ok());
        assert!(resolve_json(long(105.0, 120.0, edge)).is_err());
        assert!(resolve_json(long(120.0, 105.0, edge + 3600)).is_err());
    }

    #[test]
    fn styles_are_the_web_apps() {
        let style = |s: Value, tool: &str| parse_style(Some(&s), Tool::by_name(tool).unwrap());
        assert!(style(
            json!({ "color": "#2962ff", "width": 2, "dash": "dotted" }),
            "segment"
        )
        .is_ok());
        assert!(style(json!({ "fill": "rgba(41, 98, 255, 0.15)" }), "cgRect").is_ok());
        assert!(style(json!({ "color": "#d1d4dc", "fontSize": 14 }), "cgText").is_ok());
        assert!(style(json!({ "width": 5 }), "segment").is_err());
        assert!(style(json!({ "fontSize": 13 }), "cgText").is_err());
        assert!(style(json!({ "width": 2 }), "cgText").is_err());
        assert!(style(json!({ "color": "red" }), "segment").is_err());
        assert!(style(json!({ "color": "rgba(256, 0, 0, 1)" }), "segment").is_err());
        assert!(style(json!({ "color": "rgba(1, 2, 3, 1.5)" }), "segment").is_err());
    }

    #[test]
    fn warnings_say_what_is_cut_off() {
        let r = resolve_json(json!([
            { "tool": "horizontalStraightLine", "points": [{ "time": 1_760_227_200, "price": 1000 }] },
            { "tool": "horizontalRayLine", "points": [{ "time": 1_760_227_200 + 200 * 3600, "price": 120 }] }
        ]))
        .unwrap();
        assert_eq!(r.warnings.len(), 2, "{:?}", r.warnings);
        assert!(r.warnings[0].contains("outside the visible price range"));
        assert!(r.warnings[1].contains("beyond the image's right edge"));
    }

    #[test]
    fn the_share_param_is_compact() {
        let r = resolve_json(json!([
            { "tool": "segment", "points": [{ "time": 1_760_227_200, "price": 100.5 }, { "time": 1_760_230_800, "snap": "high" }],
              "style": { "color": "#2962ff", "width": 2 } },
            { "tool": "cgText", "points": [{ "time": 1_760_227_200, "price": 110 }], "text": "Break, out; 100%" }
        ]))
        .unwrap();
        // The web app's shareLink.test.ts decodes this very string.
        assert_eq!(
            share_param(&r.drawings).unwrap(),
            "segment(1760227200:100.5;1760230800:103;color:%232962ff;width:2),cgText(1760227200:110;text:Break%2C%20out%3B%20100%25)"
        );
    }

    #[test]
    fn swings_alternate() {
        let mut b = bars(40);
        for (i, bar) in b.iter_mut().enumerate() {
            let v = if (i / 5) % 2 == 0 {
                (i % 5) as f64
            } else {
                5.0 - (i % 5) as f64
            };
            bar.high = 100.0 + v + 1.0;
            bar.low = 100.0 + v - 1.0;
        }
        let s = swings(&b, 2, 4);
        assert!(!s.is_empty() && s.len() <= 4);
        assert!(s.windows(2).all(|w| w[0].2 != w[1].2));
    }

    /// The web app's tools are these, in this order, with these labels and point counts.
    /// Regenerate the copy with `UPDATE_DRAWING_TOOLS=1 cargo test the_web_app_has`.
    #[test]
    fn the_web_app_has_the_same_tools() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let path = root.join("web/src/generated/drawing-tools.json");
        let want = serde_json::to_string_pretty(&tools_json()).unwrap() + "\n";
        if std::env::var_os("UPDATE_DRAWING_TOOLS").is_some() {
            std::fs::write(&path, &want).unwrap();
        }
        let have = std::fs::read_to_string(&path).unwrap_or_default();
        assert_eq!(
            have,
            want,
            "{} is stale: run UPDATE_DRAWING_TOOLS=1 cargo test the_web_app_has",
            path.display()
        );

        // And the web app's own source names the same tools, labels and point counts.
        let store = std::fs::read_to_string(root.join("web/src/stores/drawings.ts")).unwrap();
        let names_block = store
            .split("DRAWING_NAMES = [")
            .nth(1)
            .unwrap()
            .split(']')
            .next()
            .unwrap();
        let names: Vec<&str> = names_block.split('"').skip(1).step_by(2).collect();
        assert_eq!(names, TOOLS.iter().map(|t| t.name).collect::<Vec<_>>());
        let exact = store
            .split("EXACT_POINTS")
            .nth(1)
            .unwrap()
            .split("};")
            .next()
            .unwrap();
        for line in exact
            .lines()
            .filter(|l| l.contains(':') && !l.contains("Record"))
        {
            let (name, n) = line.trim().trim_end_matches(',').split_once(':').unwrap();
            let tool = Tool::by_name(name.trim()).unwrap();
            assert_eq!(
                tool.points.len(),
                n.trim().parse::<usize>().unwrap(),
                "{name}"
            );
        }
        let filled = store
            .split("FILLED: readonly DrawingName[] = [")
            .nth(1)
            .unwrap()
            .split(']')
            .next()
            .unwrap();
        let filled: Vec<&str> = filled.split('"').skip(1).step_by(2).collect();
        assert_eq!(filled, FILLED);
        let toolbar =
            std::fs::read_to_string(root.join("web/src/components/DrawingToolbar.tsx")).unwrap();
        for t in &TOOLS {
            let def = format!("name: \"{}\",\n    label: \"{}\",", t.name, t.label);
            assert!(toolbar.contains(&def), "label of {}", t.name);
        }
    }
}
