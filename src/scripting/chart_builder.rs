//! The `chart` object passed to `compute(ohlcv, params, chart)`, mapping 1:1
//! onto `PanelResult`, and the read-only `ohlcv` object.

use allocative::Allocative;
use plotters::style::RGBAColor;
use starlark::environment::{Methods, MethodsBuilder};
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::float::UnpackFloat;
use starlark::values::list::UnpackList;
use starlark::values::none::NoneType;
use starlark::values::{
    starlark_value, Heap, NoSerialize, ProvidesStaticType, StarlarkPagablePanic, StarlarkValue,
    Value,
};
use std::fmt;
use std::sync::{Arc, Mutex};

use super::runtime::ScriptInfo;
use super::series::{err, operand, Operand, Series};
use crate::data::OhlcvData;
use crate::indicator::{rgba, Bars, DivLine, Dot, Fill, HBar, HLine, Line, PanelResult};

/// Most plot objects of each kind one script may draw.
const MAX_LINES: usize = 64;
const MAX_MARKS: usize = 10_000;

// ---- ohlcv ----

pub const COLUMNS: &[&str] = &[
    "open", "high", "low", "close", "volume", "time", "hl2", "hlc3", "ohlc4", "hlcc4",
];

/// `ohlcv.open` ... `ohlcv.time` (unix seconds, na when unknown) and the
/// `hl2` / `hlc3` / `ohlc4` / `hlcc4` sources.
#[derive(ProvidesStaticType, NoSerialize, Allocative, StarlarkPagablePanic)]
pub struct Ohlcv {
    #[allocative(skip)]
    pub data: OhlcvData,
    #[allocative(skip)]
    cols: Vec<Arc<[f64]>>,
}
starlark_simple_value!(Ohlcv);

impl Ohlcv {
    pub fn new(data: &OhlcvData) -> Self {
        let cols = COLUMNS
            .iter()
            .map(|c| match *c {
                "volume" => crate::indicators::pine::volumes(data).into(),
                "time" => data
                    .bars
                    .iter()
                    .map(|b| b.date.parse::<f64>().unwrap_or(f64::NAN))
                    .collect(),
                src => crate::indicators::pine::source_series(data, src).into(),
            })
            .collect();
        Self {
            data: data.clone(),
            cols,
        }
    }
}

impl fmt::Debug for Ohlcv {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Ohlcv({} bars)", self.data.len())
    }
}

impl fmt::Display for Ohlcv {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<ohlcv {} bars>", self.data.len())
    }
}

#[starlark_value(type = "ohlcv")]
impl<'v> StarlarkValue<'v> for Ohlcv {
    fn length(&self) -> starlark::Result<i32> {
        Ok(self.data.len() as i32)
    }

    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        let i = COLUMNS.iter().position(|c| *c == attribute)?;
        Some(heap.alloc(Series {
            v: self.cols[i].clone(),
            boolean: false,
        }))
    }

    fn dir_attr(&self) -> Vec<String> {
        COLUMNS.iter().map(|c| c.to_string()).collect()
    }
}

// ---- colors ----

/// Pine v5 `color.*` constants.
const NAMED: &[(&str, u32)] = &[
    ("aqua", 0x00bcd4),
    ("black", 0x363a45),
    ("blue", 0x2196f3),
    ("fuchsia", 0xe040fb),
    ("gray", 0x787b86),
    ("grey", 0x787b86),
    ("green", 0x4caf50),
    ("lime", 0x00e676),
    ("maroon", 0x880e4f),
    ("navy", 0x311b92),
    ("olive", 0x808000),
    ("orange", 0xff9800),
    ("purple", 0x9c27b0),
    ("red", 0xff5252),
    ("silver", 0xb2b5be),
    ("teal", 0x00897b),
    ("white", 0xffffff),
    ("yellow", 0xffeb3b),
];

/// `"#rrggbb"`, `"#rrggbbaa"` (its alpha wins over `alpha`) or a Pine color
/// name.
pub fn parse_color(s: &str, alpha: f64) -> starlark::Result<RGBAColor> {
    if !(0.0..=1.0).contains(&alpha) {
        return err(format!("alpha must be in 0..1, got {alpha}"));
    }
    if let Some(&(_, hex)) = NAMED.iter().find(|(n, _)| n.eq_ignore_ascii_case(s)) {
        return Ok(rgba(hex, alpha));
    }
    let hex = s.strip_prefix('#').unwrap_or("");
    match (hex.len(), u32::from_str_radix(hex, 16)) {
        (6, Ok(v)) => Ok(rgba(v, alpha)),
        (8, Ok(v)) => Ok(rgba(v >> 8, (v & 0xff) as f64 / 255.0)),
        _ => err(format!(
            "unknown color {s:?}: use \"#rrggbb\", \"#rrggbbaa\" or a name like \"red\""
        )),
    }
}

// ---- chart ----

/// Accumulates what `compute` draws.
#[derive(ProvidesStaticType, NoSerialize, Allocative, StarlarkPagablePanic)]
pub struct Chart {
    #[allocative(skip)]
    acc: Mutex<PanelResult>,
    bars: usize,
}
starlark_simple_value!(Chart);

impl Chart {
    pub fn new(bars: usize, info: &ScriptInfo) -> Self {
        Self {
            acc: Mutex::new(PanelResult {
                label: info.title.clone(),
                is_overlay: info.overlay,
                ..Default::default()
            }),
            bars,
        }
    }

    pub fn take(&self) -> PanelResult {
        std::mem::take(&mut *self.acc.lock().unwrap_or_else(|e| e.into_inner()))
    }

    fn with<R>(
        &self,
        f: impl FnOnce(&mut PanelResult) -> starlark::Result<R>,
    ) -> starlark::Result<R> {
        f(&mut self.acc.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// A series or number as one value per bar.
    fn per_bar(&self, v: Value, what: &str) -> starlark::Result<Vec<f64>> {
        match operand(v, what)? {
            Operand::S(s) if s.len() == self.bars => Ok(s.to_vec()),
            Operand::S(s) => err(format!(
                "{what}: series has {} values, the chart has {} bars",
                s.len(),
                self.bars
            )),
            Operand::N(n) => Ok(vec![n; self.bars]),
        }
    }
}

fn room<T>(items: &[T], max: usize, what: &str) -> starlark::Result<()> {
    if items.len() >= max {
        return err(format!("too many {what} (limit {max})"));
    }
    Ok(())
}

impl fmt::Debug for Chart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Chart({} bars)", self.bars)
    }
}

impl fmt::Display for Chart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<chart>")
    }
}

#[starlark_value(type = "chart")]
impl<'v> StarlarkValue<'v> for Chart {
    fn get_methods() -> Option<&'static Methods> {
        starlark::methods_static!(METHODS = chart_methods);
        Some(METHODS.methods())
    }
}

#[starlark_module]
fn chart_methods(builder: &mut MethodsBuilder) {
    /// A line (`PanelResult.lines`).
    fn line<'v>(
        this: &Chart,
        #[starlark(require = pos)] series: Value<'v>,
        #[starlark(require = named, default = "")] title: &str,
        #[starlark(require = named, default = "#2962ff")] color: &str,
        #[starlark(require = named, default = 2)] width: i32,
        #[starlark(require = named, default = UnpackFloat(0.9))] alpha: UnpackFloat,
    ) -> starlark::Result<NoneType> {
        let alpha = alpha.0;
        let y = this.per_bar(series, "chart.line")?;
        let color = parse_color(color, alpha)?;
        this.with(|pr| {
            room(&pr.lines, MAX_LINES, "lines")?;
            pr.lines.push(Line {
                breaks: false,
                y,
                color,
                width: width.clamp(1, 10) as u32,
                label: (!title.is_empty()).then(|| title.to_owned()),
            });
            Ok(NoneType)
        })
    }

    /// Fill between two series or levels. `color_below` (optional) is used
    /// where `a < b`.
    fn fill<'v>(
        this: &Chart,
        #[starlark(require = pos)] a: Value<'v>,
        #[starlark(require = pos)] b: Value<'v>,
        #[starlark(require = named, default = "#2962ff")] color: &str,
        #[starlark(require = named, default = UnpackFloat(0.2))] alpha: UnpackFloat,
        #[starlark(require = named)] color_below: Option<&str>,
    ) -> starlark::Result<NoneType> {
        let alpha = alpha.0;
        let (y1, y2) = (
            this.per_bar(a, "chart.fill")?,
            this.per_bar(b, "chart.fill")?,
        );
        let color = parse_color(color, alpha)?;
        let color_below = color_below.map(|c| parse_color(c, alpha)).transpose()?;
        this.with(|pr| {
            room(&pr.fills, MAX_LINES, "fills")?;
            pr.fills.push(Fill {
                y1,
                y2,
                color,
                color_below,
            });
            Ok(NoneType)
        })
    }

    /// Histogram bars from `bottom`. `color` is one color for every bar, or
    /// `colors` a list with one color per bar.
    fn bars<'v>(
        this: &Chart,
        #[starlark(require = pos)] series: Value<'v>,
        #[starlark(require = named, default = "#26a69a")] color: &str,
        #[starlark(require = named)] colors: Option<UnpackList<String>>,
        #[starlark(require = named, default = UnpackFloat(0.9))] alpha: UnpackFloat,
        #[starlark(require = named, default = UnpackFloat(0.0))] bottom: UnpackFloat,
    ) -> starlark::Result<NoneType> {
        let alpha = alpha.0;
        let bottom = bottom.0;
        let y = this.per_bar(series, "chart.bars")?;
        let colors = match colors {
            Some(list) if list.items.len() == y.len() => list
                .items
                .iter()
                .map(|c| parse_color(c, alpha))
                .collect::<starlark::Result<Vec<_>>>()?,
            Some(list) => {
                return err(format!(
                    "chart.bars: {} colors for {} bars",
                    list.items.len(),
                    y.len()
                ))
            }
            None => vec![parse_color(color, alpha)?; y.len()],
        };
        this.with(|pr| {
            room(&pr.bars, MAX_LINES, "bar series")?;
            pr.bars.push(Bars { y, colors, bottom });
            Ok(NoneType)
        })
    }

    /// A dot at bar `x` (int) and value `y`; or, with `x` a bool series, a
    /// dot on every bar where it is true, at `y` (a series or a number).
    fn dot<'v>(
        this: &Chart,
        #[starlark(require = pos)] x: Value<'v>,
        #[starlark(require = pos)] y: Value<'v>,
        #[starlark(require = named, default = "#2962ff")] color: &str,
        #[starlark(require = named, default = 4)] size: i32,
        #[starlark(require = named)] label: Option<&str>,
        #[starlark(require = named, default = UnpackFloat(1.0))] alpha: UnpackFloat,
    ) -> starlark::Result<NoneType> {
        let alpha = alpha.0;
        let color = parse_color(color, alpha)?;
        let points: Vec<(usize, f64)> = if let Some(cond) = Series::from_value(x) {
            let ys = this.per_bar(y, "chart.dot")?;
            if cond.v.len() != this.bars {
                return err("chart.dot: condition series length differs from the bar count");
            }
            (0..this.bars)
                .filter(|&i| cond.v[i] != 0.0 && !cond.v[i].is_nan())
                .map(|i| (i, ys[i]))
                .collect()
        } else {
            let i = x.unpack_i32().filter(|i| (0..this.bars as i32).contains(i));
            let Some(i) = i else {
                return err("chart.dot: x must be a bar index (int) or a bool series");
            };
            match operand(y, "chart.dot")? {
                Operand::N(v) => vec![(i as usize, v)],
                Operand::S(s) => vec![(i as usize, s.get(i as usize).copied().unwrap_or(f64::NAN))],
            }
        };
        this.with(|pr| {
            if pr.dots.len() + points.len() > MAX_MARKS {
                return err(format!("too many dots (limit {MAX_MARKS})"));
            }
            pr.dots.extend(
                points
                    .into_iter()
                    .filter(|p| p.1.is_finite())
                    .map(|(x, y)| Dot {
                        x,
                        y,
                        color,
                        size: size.clamp(1, 20) as u32,
                        label: label.map(str::to_owned),
                    }),
            );
            Ok(NoneType)
        })
    }

    /// Horizontal reference line.
    fn hline(
        this: &Chart,
        #[starlark(require = pos)] y: UnpackFloat,
        #[starlark(require = named, default = "#787b86")] color: &str,
        #[starlark(require = named, default = UnpackFloat(0.5))] alpha: UnpackFloat,
    ) -> starlark::Result<NoneType> {
        let y = y.0;
        let alpha = alpha.0;
        let color = parse_color(color, alpha)?;
        this.with(|pr| {
            room(&pr.hlines, MAX_LINES, "hlines")?;
            pr.hlines.push(HLine { y, color });
            Ok(NoneType)
        })
    }

    /// Horizontal bar at a price level (volume-profile style). `width` and
    /// `offset` are fractions of the chart width.
    // Keyword arguments mirror the PanelResult fields one to one.
    #[allow(clippy::too_many_arguments)]
    fn hbar(
        this: &Chart,
        #[starlark(require = pos)] y: UnpackFloat,
        #[starlark(require = pos)] height: UnpackFloat,
        #[starlark(require = pos)] width: UnpackFloat,
        #[starlark(require = named, default = UnpackFloat(0.0))] offset: UnpackFloat,
        #[starlark(require = named, default = "#2962ff")] color: &str,
        #[starlark(require = named, default = UnpackFloat(0.5))] alpha: UnpackFloat,
        #[starlark(require = named, default = false)] left: bool,
    ) -> starlark::Result<NoneType> {
        let y = y.0;
        let height = height.0;
        let width = width.0;
        let offset = offset.0;
        let alpha = alpha.0;
        let color = parse_color(color, alpha)?;
        this.with(|pr| {
            room(&pr.hbars, MAX_MARKS, "hbars")?;
            pr.hbars.push(HBar {
                y,
                height,
                width: width.clamp(0.0, 1.0),
                offset: offset.clamp(0.0, 1.0),
                color,
                left,
            });
            Ok(NoneType)
        })
    }

    /// Straight line from bar `x1` at `y1` to bar `x2` at `y2`.
    // Keyword arguments mirror the PanelResult fields one to one.
    #[allow(clippy::too_many_arguments)]
    fn divline(
        this: &Chart,
        #[starlark(require = pos)] x1: i32,
        #[starlark(require = pos)] y1: UnpackFloat,
        #[starlark(require = pos)] x2: i32,
        #[starlark(require = pos)] y2: UnpackFloat,
        #[starlark(require = named, default = "#787b86")] color: &str,
        #[starlark(require = named, default = UnpackFloat(0.9))] alpha: UnpackFloat,
        #[starlark(require = named, default = false)] dashed: bool,
    ) -> starlark::Result<NoneType> {
        let y1 = y1.0;
        let y2 = y2.0;
        let alpha = alpha.0;
        let in_range = |x: i32| (0..this.bars as i32).contains(&x);
        if !in_range(x1) || !in_range(x2) || x1 == x2 {
            return err("chart.divline: x1 and x2 must be distinct bar indexes");
        }
        let color = parse_color(color, alpha)?;
        this.with(|pr| {
            room(&pr.divlines, MAX_MARKS, "divlines")?;
            pr.divlines.push(DivLine {
                x1: x1 as usize,
                y1,
                x2: x2 as usize,
                y2,
                color,
                dashed,
            });
            Ok(NoneType)
        })
    }

    /// Fixed y-axis range for the panel.
    fn set_y_range(
        this: &Chart,
        #[starlark(require = pos)] lo: UnpackFloat,
        #[starlark(require = pos)] hi: UnpackFloat,
    ) -> starlark::Result<NoneType> {
        let lo = lo.0;
        let hi = hi.0;
        if !(lo.is_finite() && hi.is_finite() && lo < hi) {
            return err("chart.set_y_range: need finite lo < hi");
        }
        this.with(|pr| {
            pr.y_range = Some((lo, hi));
            Ok(NoneType)
        })
    }

    /// Panel label (defaults to `META["title"]`).
    fn set_label(
        this: &Chart,
        #[starlark(require = pos)] label: &str,
    ) -> starlark::Result<NoneType> {
        this.with(|pr| {
            pr.label = label.chars().take(80).collect();
            Ok(NoneType)
        })
    }
}
