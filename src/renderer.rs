use crate::data::OhlcvData;
use crate::drawings::{parse_color, Drawing, View};
use crate::indicator::{DivLine, Indicator, PanelResult};
use plotters::prelude::*;

const BG: RGBColor = RGBColor(19, 23, 34);
const GRID: RGBAColor = RGBAColor(42, 46, 57, 0.5);
const TEXT: RGBColor = RGBColor(120, 123, 134);
const TITLE_COLOR: RGBColor = RGBColor(210, 210, 210);

/// Format epoch seconds to "YYYY-MM-DD HH:MM" (UTC, no chrono dep).
fn format_epoch(epoch: i64) -> String {
    let s = epoch;
    let days_since_epoch = s / 86400;
    let time_of_day = s % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;

    // Days to Y-M-D (simplified Gregorian, correct for 1970–2099)
    let mut y = 1970;
    let mut remaining = days_since_epoch;
    loop {
        let days_in_year = if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
            366
        } else {
            365
        };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        y += 1;
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let month_days: [i64; 12] = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut m = 0;
    for &md in &month_days {
        if remaining < md {
            break;
        }
        remaining -= md;
        m += 1;
    }
    let d = remaining + 1;
    format!("{:04}-{:02}-{:02} {:02}:{:02}", y, m + 1, d, hours, minutes)
}

/// Short date label depending on interval.
fn format_bar_label(epoch: i64, interval: &str) -> String {
    let full = format_epoch(epoch);
    if crate::interval::Interval::parse(interval).is_ok_and(|iv| iv.is_date_bar()) {
        full[..10].to_string() // YYYY-MM-DD
    } else {
        full // YYYY-MM-DD HH:MM
    }
}

pub fn render_chart(
    data: &OhlcvData,
    panels: &[Box<dyn Indicator>],
    output: &str,
    width: u32,
    height: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    // Compute all indicators
    let results: Vec<PanelResult> = panels.iter().map(|p| p.compute(data)).collect();
    let refs: Vec<&PanelResult> = results.iter().collect();
    render_results(data, &refs, output, width, height)
}

/// The chart's title: `SYMBOL · INTERVAL`, then how its bars are built, if they are
/// not plain candles of that interval: `BTCUSDT · 1H · P&F 3.4 x 3`.
pub fn chart_title(data: &OhlcvData, style: Option<&str>) -> String {
    let sym = data.symbol.as_deref().unwrap_or("Sample");
    let ivl = data.interval.as_deref().unwrap_or("");
    let mut title = if ivl.is_empty() {
        sym.to_string()
    } else {
        format!("{} · {}", sym, ivl.to_uppercase())
    };
    if let Some(style) = style {
        title.push_str(" · ");
        title.push_str(style);
    }
    title
}

/// `render_chart` for indicator results computed by the caller (so each
/// indicator runs once, and a failed one can be left out).
pub fn render_results(
    data: &OhlcvData,
    results: &[&PanelResult],
    output: &str,
    width: u32,
    height: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    render_labeled(data, results, output, width, height, None, &[])
}

/// `render_results`, naming in the title how the bars are built (ADR-0047), with
/// `drawings` on the price chart (ADR-0054).
pub fn render_labeled(
    data: &OhlcvData,
    results: &[&PanelResult],
    output: &str,
    width: u32,
    height: u32,
    style: Option<&str>,
    drawings: &[Drawing],
) -> Result<(), Box<dyn std::error::Error>> {
    let n = data.len();
    if n == 0 {
        // Nothing to render -- write a blank image
        let root = BitMapBackend::new(output, (width, height)).into_drawing_area();
        root.fill(&BG)?;
        root.present()?;
        return Ok(());
    }

    // Separate overlays from panels
    let overlays: Vec<&PanelResult> = results.iter().copied().filter(|r| r.is_overlay).collect();
    let panel_results: Vec<&PanelResult> =
        results.iter().copied().filter(|r| !r.is_overlay).collect();
    let n_panels = panel_results.len();

    // Price range
    let p_min = data.lows().iter().cloned().fold(f64::INFINITY, f64::min);
    let p_max = data
        .highs()
        .iter()
        .cloned()
        .fold(f64::NEG_INFINITY, f64::max);
    let p_margin = (p_max - p_min) * 0.05;
    // Room on the right for drawings past the last bar, as far as `View` allows.
    let future = drawings
        .iter()
        .flat_map(|d| &d.points)
        .map(|p| p.x - (n as i64 - 1))
        .max()
        .unwrap_or(0)
        .clamp(0, View::of(&data.bars).future);
    let x_max = (n as i64 + future) as f64;

    // Title bar height
    let title_h: u32 = 40;

    // Layout: title + candle 55% + panels
    let candle_h = ((height - title_h) as f64 * 0.55) as u32;
    let panel_h = if n_panels > 0 {
        (height - title_h - candle_h) / n_panels as u32
    } else {
        0
    };

    let root = BitMapBackend::new(output, (width, height)).into_drawing_area();
    root.fill(&BG)?;

    // Draw title: "SYMBOL · INTERVAL"
    {
        let title = chart_title(data, style);
        root.draw_text(
            &title,
            &("monospace", 24).into_font().color(&TITLE_COLOR),
            (16, 10),
        )?;
    }

    // Split vertically
    let mut areas = Vec::new();
    let mut y_start = title_h as i32;

    // Candle area
    let candle_area = root.clone().shrink((0, y_start), (width, candle_h));
    y_start += candle_h as i32;

    // Panel areas
    for _ in 0..n_panels {
        areas.push(root.clone().shrink((0, y_start), (width, panel_h)));
        y_start += panel_h as i32;
    }

    // ---- Draw candles + overlays ----
    {
        let mut chart = ChartBuilder::on(&candle_area)
            .margin(10)
            .x_label_area_size(0)
            .y_label_area_size(80)
            .build_cartesian_2d(0_f64..x_max, (p_min - p_margin)..(p_max + p_margin))?;

        chart
            .configure_mesh()
            .disable_x_mesh()
            .light_line_style(GRID)
            .bold_line_style(GRID)
            .axis_style(GRID)
            .label_style(("monospace", 14, &TEXT))
            .draw()?;

        for (i, bar) in data.bars.iter().enumerate() {
            let x = i as f64;
            let color = if bar.close >= bar.open {
                RGBColor(38, 166, 154)
            } else {
                RGBColor(239, 83, 80)
            };

            // Wick
            chart.draw_series(std::iter::once(PathElement::new(
                vec![(x, bar.low), (x, bar.high)],
                color.stroke_width(1),
            )))?;

            // Body
            let (bot, top) = if bar.close >= bar.open {
                (bar.open, bar.close)
            } else {
                (bar.close, bar.open)
            };
            chart.draw_series(std::iter::once(Rectangle::new(
                [(x - 0.35, bot), (x + 0.35, top)],
                color.filled(),
            )))?;
        }

        // Draw overlays on the candle chart
        for overlay in &overlays {
            // Fills
            for fill in &overlay.fills {
                let pairs: Vec<_> = fill
                    .y1
                    .iter()
                    .zip(&fill.y2)
                    .enumerate()
                    .filter(|(_, (a, b))| !a.is_nan() && !b.is_nan())
                    .map(|(i, (a, b))| {
                        let (lo, hi) = if a < b { (*a, *b) } else { (*b, *a) };
                        Rectangle::new(
                            [(i as f64 - 0.4, lo), (i as f64 + 0.4, hi)],
                            fill.color_for(*a, *b).filled(),
                        )
                    })
                    .collect();
                chart.draw_series(pairs)?;
            }

            // Lines, broken where a value is NaN (as on the web chart and with
            // Pine's plot.style_linebr), so a gap draws no connector.
            for line in &overlay.lines {
                let draw_width = if line.width <= 1 { 2 } else { line.width };
                for run in level_runs(line_runs(&line.y), overlay.level_lines) {
                    chart.draw_series(std::iter::once(PathElement::new(
                        run,
                        line.color.stroke_width(draw_width),
                    )))?;
                }
            }

            // Dots
            for dot in &overlay.dots {
                chart.draw_series(std::iter::once(Circle::new(
                    (dot.x as f64, dot.y),
                    dot.size,
                    dot.color.filled(),
                )))?;
            }

            // Pattern outlines (ADR-0046) draw the same way as segments.
            let strokes: Vec<DivLine> = overlay
                .patterns
                .iter()
                .flat_map(|p| {
                    p.strokes.iter().map(|s| DivLine {
                        x1: s.x1.max(0) as usize,
                        y1: s.y1,
                        x2: s.x2.max(0) as usize,
                        y2: s.y2,
                        color: p.bias.color(),
                        dashed: s.dashed,
                    })
                })
                .collect();

            // Pattern labels: past the bar's high or low, in the pattern's color.
            let text = |s: &str, x: f64, y: f64, above: bool, c: RGBAColor| {
                let off = if above { -16 } else { 4 };
                EmptyElement::at((x, y))
                    + Text::new(
                        s.to_string(),
                        (-4, off),
                        ("sans-serif", 12).into_font().color(&c),
                    )
            };
            for l in &overlay.labels {
                let c = l.bias.color();
                let c = if l.tentative { c.mix(0.5) } else { c };
                chart.draw_series(std::iter::once(text(&l.text, l.x as f64, l.y, l.above, c)))?;
            }
            for p in &overlay.patterns {
                let s = format!("{} ({})", p.name, p.status.as_str());
                let x = p.label_x.max(0) as f64;
                chart.draw_series(std::iter::once(text(
                    &s,
                    x,
                    p.label_y,
                    p.label_above,
                    p.bias.color(),
                )))?;
            }

            // Segments on overlay (e.g. zigzag)
            for d in overlay.divlines.iter().chain(&strokes) {
                let (x1, y1, x2, y2) = (d.x1 as f64, d.y1, d.x2 as f64, d.y2);
                // Dashed: 20 steps, every other one drawn (same as the panel path)
                let steps = if d.dashed { 20 } else { 1 };
                let (dx, dy) = ((x2 - x1) / steps as f64, (y2 - y1) / steps as f64);
                let stride = if d.dashed { 2 } else { 1 };
                for s in (0..steps).step_by(stride) {
                    chart.draw_series(LineSeries::new(
                        [
                            (x1 + dx * s as f64, y1 + dy * s as f64),
                            (x1 + dx * (s + 1) as f64, y1 + dy * (s + 1) as f64),
                        ],
                        d.color.stroke_width(2),
                    ))?;
                }
            }

            // HLines on overlay
            for h in &overlay.hlines {
                chart.draw_series(std::iter::once(PathElement::new(
                    vec![(0.0, h.y), (n as f64, h.y)],
                    h.color.stroke_width(1),
                )))?;
            }

            // HBars (VPVR horizontal histogram)
            for hbar in &overlay.hbars {
                let bar_width = n as f64 * hbar.width;
                let bar_offset = n as f64 * hbar.offset;
                let (x_left, x_right) = if hbar.left {
                    (bar_offset, bar_offset + bar_width)
                } else {
                    (n as f64 - bar_offset - bar_width, n as f64 - bar_offset)
                };
                let y_top = hbar.y + hbar.height / 2.0;
                let y_bot = hbar.y - hbar.height / 2.0;
                chart.draw_series(std::iter::once(Rectangle::new(
                    [(x_left, y_bot), (x_right, y_top)],
                    hbar.color.filled(),
                )))?;
            }
        }

        // Overlay labels and current value annotations on candle chart
        {
            let mut label_y = 5;
            for overlay in &overlays {
                if !overlay.label.is_empty() {
                    candle_area.draw_text(
                        &overlay.label,
                        &("monospace", 14)
                            .into_font()
                            .color(&RGBColor(200, 200, 210)),
                        (85, label_y),
                    )?;
                    label_y += 18;
                }
                for line in &overlay.lines {
                    if let Some(ref lbl) = line.label {
                        if let Some(val) = legend_value(&line.y, line.breaks) {
                            let text = format!("{}: {}", lbl, val);
                            let RGBAColor(r, g, b, _) = line.color;
                            candle_area.draw_text(
                                &text,
                                &("monospace", 12).into_font().color(&RGBColor(r, g, b)),
                                (85, label_y),
                            )?;
                            label_y += 15;
                        }
                    }
                }
            }
        }

        // Price label
        let lp = data.bars.last().unwrap().close;
        chart.draw_series(std::iter::once(Text::new(
            format!("{:.2}", lp),
            ((n - 1) as f64, lp),
            ("monospace", 14).into_font().color(&WHITE),
        )))?;

        if !drawings.is_empty() {
            let (xs, ys) = chart.plotting_area().get_pixel_range();
            let canvas = Canvas {
                area: &root,
                x: (xs.start as f64, xs.end as f64, x_max),
                y: (
                    ys.start as f64,
                    ys.end as f64,
                    p_min - p_margin,
                    p_max + p_margin,
                ),
            };
            let prec = decimals(lp);
            for d in drawings {
                canvas.drawing(d, prec)?;
            }
        }
    }

    // ---- Draw indicator panels ----
    for (area, result) in areas.iter().zip(&panel_results) {
        let (y_lo, y_hi) = result.y_range.unwrap_or_else(|| auto_range(result));

        let mut chart = ChartBuilder::on(area)
            .margin(5)
            .x_label_area_size(20)
            .y_label_area_size(80)
            .build_cartesian_2d(0_f64..x_max, y_lo..y_hi)?;

        chart
            .configure_mesh()
            .disable_x_mesh()
            .light_line_style(GRID)
            .bold_line_style(GRID)
            .axis_style(GRID)
            .label_style(("monospace", 13, &TEXT))
            .draw()?;

        // HLines
        for h in &result.hlines {
            chart.draw_series(std::iter::once(PathElement::new(
                vec![(0.0, h.y), (n as f64, h.y)],
                h.color.stroke_width(1),
            )))?;
        }

        // Fills (approximate with vertical bars for simplicity in plotters)
        for fill in &result.fills {
            let pairs: Vec<_> = fill
                .y1
                .iter()
                .zip(&fill.y2)
                .enumerate()
                .filter(|(_, (a, b))| !a.is_nan() && !b.is_nan())
                .map(|(i, (a, b))| {
                    let (lo, hi) = if a < b { (*a, *b) } else { (*b, *a) };
                    Rectangle::new(
                        [(i as f64 - 0.4, lo), (i as f64 + 0.4, hi)],
                        fill.color_for(*a, *b).filled(),
                    )
                })
                .collect();
            chart.draw_series(pairs)?;
        }

        // Bars (histogram)
        for bars in &result.bars {
            let rects: Vec<_> = bars
                .y
                .iter()
                .zip(&bars.colors)
                .enumerate()
                .filter(|(_, (y, _))| !y.is_nan())
                .map(|(i, (y, c))| {
                    Rectangle::new(
                        [
                            (i as f64 - 0.35, bars.bottom),
                            (i as f64 + 0.35, bars.bottom + y),
                        ],
                        c.filled(),
                    )
                })
                .collect();
            chart.draw_series(rects)?;
        }

        // Lines, broken where a value is NaN (as on the web chart and with
        // Pine's plot.style_linebr), so a gap draws no connector.
        for line in &result.lines {
            let draw_width = if line.width <= 1 { 2 } else { line.width };
            for run in level_runs(line_runs(&line.y), result.level_lines) {
                chart.draw_series(std::iter::once(PathElement::new(
                    run,
                    line.color.stroke_width(draw_width),
                )))?;
            }
        }

        // Dots
        for dot in &result.dots {
            chart.draw_series(std::iter::once(Circle::new(
                (dot.x as f64, dot.y),
                dot.size,
                dot.color.filled(),
            )))?;
        }

        // Divergence lines
        for divline in &result.divlines {
            let x1 = divline.x1 as f64;
            let y1 = divline.y1;
            let x2 = divline.x2 as f64;
            let y2 = divline.y2;

            if divline.dashed {
                let steps = 20;
                let dx = (x2 - x1) / steps as f64;
                let dy = (y2 - y1) / steps as f64;
                for s in (0..steps).step_by(2) {
                    let sx1 = x1 + dx * s as f64;
                    let sy1 = y1 + dy * s as f64;
                    let sx2 = x1 + dx * (s + 1) as f64;
                    let sy2 = y1 + dy * (s + 1) as f64;
                    chart.draw_series(LineSeries::new(
                        [(sx1, sy1), (sx2, sy2)],
                        ShapeStyle {
                            color: divline.color.to_rgba(),
                            filled: false,
                            stroke_width: 2,
                        },
                    ))?;
                }
            } else {
                chart.draw_series(LineSeries::new(
                    [(x1, y1), (x2, y2)],
                    ShapeStyle {
                        color: divline.color.to_rgba(),
                        filled: false,
                        stroke_width: 2,
                    },
                ))?;
            }
        }

        // Label
        if !result.label.is_empty() {
            area.draw_text(
                &result.label,
                &("monospace", 15)
                    .into_font()
                    .color(&RGBColor(200, 200, 210)),
                (85, 5),
            )?;
        }

        // Current value annotations
        {
            let mut ann_y = 22; // below the panel label
            for line in &result.lines {
                if let Some(ref lbl) = line.label {
                    // Value at the last bar
                    if let Some(val) = legend_value(&line.y, line.breaks) {
                        let text = format!("{}: {}", lbl, val);
                        let RGBAColor(r, g, b, _) = line.color;
                        area.draw_text(
                            &text,
                            &("monospace", 12).into_font().color(&RGBColor(r, g, b)),
                            (85, ann_y),
                        )?;
                        ann_y += 15;
                    }
                }
            }
        }
    }

    // ---- Draw X-axis datetime labels along the bottom ----
    {
        let interval = data.interval.as_deref().unwrap_or("4h");
        let y_pos = height as i32 - 4; // just above the bottom edge
        let x_margin = 80; // match Y-axis label width
        let chart_width = (width as i32 - x_margin - 10) as f64; // usable chart width
        let label_count = (width / 180).max(3) as usize; // ~1 label per 180px
        let step = (n / label_count).max(1);

        for idx in (0..n).step_by(step) {
            let epoch = data.bars[idx].date.parse::<i64>().unwrap_or(0);
            if epoch == 0 {
                continue;
            }
            let label = format_bar_label(epoch, interval);
            let x_frac = idx as f64 / n as f64;
            let x_px = x_margin + (x_frac * chart_width) as i32;
            root.draw_text(
                &label,
                &("monospace", 13).into_font().color(&TEXT),
                (x_px, y_pos - 10),
            )?;
        }
    }

    root.present()?;
    Ok(())
}

// ---- Drawings (ADR-0054) ----
// In pixels of the whole image, clipped to the price chart's plotting area, with the
// web app's colors and labels on its dark theme.

const BLUE: &str = "#1677ff";
const RECT: (&str, &str) = ("rgba(41, 98, 255, 0.15)", "#2962ff");
const UP: (&str, &str) = ("rgba(41, 98, 255, 0.15)", "#2962ff");
const DOWN: (&str, &str) = ("rgba(242, 54, 69, 0.15)", "#f23645");
const GREEN: (&str, &str) = ("rgba(8, 153, 129, 0.22)", "#089981");
const RED: (&str, &str) = ("rgba(242, 54, 69, 0.22)", "#f23645");
const POSITION_LINE: &str = "#787b86";
const TEXT_FG: &str = "#d1d4dc";
const SURFACE: &str = "#1e222d";
const FIB_LEVELS: [f64; 7] = [1.0, 0.786, 0.618, 0.5, 0.382, 0.236, 0.0];
const FIB_EXT_LEVELS: [f64; 11] = [
    0.0, 0.236, 0.382, 0.5, 0.618, 0.786, 1.0, 1.618, 2.618, 3.618, 4.236,
];
const FIB_EXT_COLORS: [&str; 11] = [
    "#787b86", "#f23645", "#ff9800", "#4caf50", "#089981", "#00bcd4", "#5b8cff", "#ba68c8",
    "#e91e63", "#f23645", "#ff5252",
];
/// Height of a stacked measure label.
const LABEL_H: f64 = 22.0;

fn rgba(c: &str) -> RGBAColor {
    let (r, g, b, a) = parse_color(c).unwrap_or((22, 119, 255, 1.0));
    RGBAColor(r, g, b, a)
}

type Px = (f64, f64);
type Drawn = Result<(), Box<dyn std::error::Error>>;

#[derive(Clone, Copy)]
struct Pen {
    color: RGBAColor,
    width: u32,
    /// Pixels on and off.
    dash: Option<(f64, f64)>,
}

#[derive(Clone, Copy, PartialEq)]
enum Baseline {
    Top,
    Middle,
    Bottom,
}

struct Label<'t> {
    text: &'t str,
    size: u32,
    fg: RGBAColor,
    bg: RGBAColor,
    border: Option<RGBAColor>,
    /// 0 left, 0.5 centred, 1 right of the anchor.
    align: f64,
    baseline: Baseline,
    pad: (f64, f64),
}

struct Canvas<'a, 'b> {
    area: &'a DrawingArea<BitMapBackend<'b>, plotters::coord::Shift>,
    /// Left and right pixel of the plot, and the bars they span from 0.
    x: (f64, f64, f64),
    /// Top and bottom pixel of the plot, and the prices at its bottom and top.
    y: (f64, f64, f64, f64),
}

impl Canvas<'_, '_> {
    fn px(&self, x: f64, price: f64) -> Px {
        let (l, r, span) = self.x;
        let (t, b, lo, hi) = self.y;
        (
            l + x / span * (r - l),
            b - (price - lo) / (hi - lo) * (b - t),
        )
    }

    fn inside(&self, (x, y): Px) -> bool {
        (self.x.0..=self.x.1).contains(&x) && (self.y.0..=self.y.1).contains(&y)
    }

    /// `a`-`b` cut to the plot (Liang-Barsky).
    fn clip(&self, a: Px, b: Px) -> Option<(Px, Px)> {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
        for (p, q) in [
            (-dx, a.0 - self.x.0),
            (dx, self.x.1 - a.0),
            (-dy, a.1 - self.y.0),
            (dy, self.y.1 - a.1),
        ] {
            if p == 0.0 {
                if q < 0.0 {
                    return None;
                }
            } else if p < 0.0 {
                t0 = t0.max(q / p);
            } else {
                t1 = t1.min(q / p);
            }
        }
        (t0 <= t1).then_some((
            (a.0 + t0 * dx, a.1 + t0 * dy),
            (a.0 + t1 * dx, a.1 + t1 * dy),
        ))
    }

    fn line(&self, a: Px, b: Px, pen: Pen) -> Drawn {
        let Some((a, b)) = self.clip(a, b) else {
            return Ok(());
        };
        let style = pen.color.stroke_width(pen.width);
        let ip = |p: Px| (p.0.round() as i32, p.1.round() as i32);
        let Some((on, off)) = pen.dash else {
            self.area
                .draw(&PathElement::new(vec![ip(a), ip(b)], style))?;
            return Ok(());
        };
        let len = (b.0 - a.0).hypot(b.1 - a.1);
        let at = |d: f64| (a.0 + (b.0 - a.0) * d / len, a.1 + (b.1 - a.1) * d / len);
        let mut d = 0.0;
        while d < len {
            let end = (d + on).min(len);
            self.area
                .draw(&PathElement::new(vec![ip(at(d)), ip(at(end))], style))?;
            d += on + off;
        }
        Ok(())
    }

    /// A line from `from` through `through` and on past the plot.
    fn ray(&self, from: Px, through: Px, pen: Pen) -> Drawn {
        let (dx, dy) = (through.0 - from.0, through.1 - from.1);
        let len = dx.hypot(dy);
        if len == 0.0 {
            return Ok(());
        }
        let far = 1e5 / len;
        self.line(from, (from.0 + dx * far, from.1 + dy * far), pen)
    }

    fn rect(&self, a: Px, b: Px, fill: RGBAColor, pen: Pen) -> Drawn {
        let x0 = a.0.min(b.0).max(self.x.0);
        let x1 = a.0.max(b.0).min(self.x.1);
        let y0 = a.1.min(b.1).max(self.y.0);
        let y1 = a.1.max(b.1).min(self.y.1);
        if x0 < x1 && y0 < y1 {
            self.area.draw(&Rectangle::new(
                [(x0 as i32, y0 as i32), (x1 as i32, y1 as i32)],
                fill.filled(),
            ))?;
        }
        let c = [a, (b.0, a.1), b, (a.0, b.1), a];
        for w in c.windows(2) {
            self.line(w[0], w[1], pen)?;
        }
        Ok(())
    }

    fn text_width(&self, text: &str, size: u32) -> f64 {
        self.area
            .estimate_text_size(text, &("sans-serif", size).into_text_style(self.area))
            .map_or(0.0, |(w, _)| w as f64)
    }

    /// The centre of a label on the part of [left, right] inside the plot, kept inside it.
    fn label_x(&self, left: f64, right: f64, text: &str, size: u32) -> f64 {
        let (lo, hi) = (left.max(self.x.0), right.min(self.x.1));
        let mid = if lo <= hi {
            (lo + hi) / 2.0
        } else {
            (left + right) / 2.0
        };
        let half = (self.text_width(text, size) + 12.0) / 2.0;
        mid.max(self.x.0 + half)
            .min((self.x.1 - half).max(self.x.0 + half))
    }

    fn label(&self, (x, y): Px, l: Label) -> Drawn {
        if !self.inside((x, y)) {
            return Ok(());
        }
        let font = ("sans-serif", l.size).into_font();
        let (w, h) = self
            .area
            .estimate_text_size(l.text, &font.clone().into_text_style(self.area))?;
        let (bw, bh) = (w as f64 + 2.0 * l.pad.0, h as f64 + 2.0 * l.pad.1);
        let x0 = x - l.align * bw;
        let y0 = match l.baseline {
            Baseline::Top => y,
            Baseline::Middle => y - bh / 2.0,
            Baseline::Bottom => y - bh,
        };
        let corner = |dx: f64, dy: f64| ((x0 + dx) as i32, (y0 + dy) as i32);
        self.area.draw(&Rectangle::new(
            [corner(0.0, 0.0), corner(bw, bh)],
            l.bg.filled(),
        ))?;
        if let Some(c) = l.border {
            self.area.draw(&Rectangle::new(
                [corner(0.0, 0.0), corner(bw, bh)],
                c.stroke_width(1),
            ))?;
        }
        self.area
            .draw_text(l.text, &font.color(&l.fg), corner(l.pad.0, l.pad.1))?;
        Ok(())
    }

    /// A white label on `bg`, as the measure and position tools have.
    fn tag(&self, at: Px, text: &str, bg: RGBAColor, baseline: Baseline) -> Drawn {
        self.label(
            at,
            Label {
                text,
                size: 12,
                fg: rgba("#ffffff"),
                bg,
                border: None,
                align: 0.5,
                baseline,
                pad: (6.0, 3.0),
            },
        )
    }

    fn arrow(&self, from: Px, to: Px, pen: Pen) -> Drawn {
        self.line(from, to, pen)?;
        let a = (to.1 - from.1).atan2(to.0 - from.0);
        for s in [std::f64::consts::FRAC_PI_6, -std::f64::consts::FRAC_PI_6] {
            let wing = (to.0 - 7.0 * (a + s).cos(), to.1 - 7.0 * (a + s).sin());
            self.line(wing, to, Pen { dash: None, ..pen })?;
        }
        Ok(())
    }

    fn drawing(&self, d: &Drawing, prec: usize) -> Drawn {
        let p: Vec<Px> = d
            .points
            .iter()
            .map(|q| self.px(q.x as f64, q.price))
            .collect();
        let v: Vec<f64> = d.points.iter().map(|q| q.price).collect();
        let s = &d.style;
        let color = |dflt: &str| rgba(s.color.as_deref().unwrap_or(dflt));
        let dash = match s.dash {
            Some("dashed") => Some((6.0, 4.0)),
            Some("dotted") => Some((1.0, 3.0)),
            _ => None,
        };
        let pen = |c: RGBAColor| Pen {
            color: c,
            width: s.width.unwrap_or(1),
            dash,
        };
        let (l, r, t, b) = (self.x.0, self.x.1, self.y.0, self.y.1);
        let y_of = |price: f64| self.px(0.0, price).1;
        match d.tool.name {
            "segment" => self.line(p[0], p[1], pen(color(BLUE))),
            "rayLine" => self.ray(p[0], p[1], pen(color(BLUE))),
            "straightLine" => {
                self.ray(p[0], p[1], pen(color(BLUE)))?;
                self.ray(p[1], p[0], pen(color(BLUE)))
            }
            "horizontalStraightLine" => self.line((l, p[0].1), (r, p[0].1), pen(color(BLUE))),
            "horizontalRayLine" => self.line(p[0], (r, p[0].1), pen(color(BLUE))),
            "verticalStraightLine" => self.line((p[0].0, t), (p[0].0, b), pen(color(BLUE))),
            "parallelStraightLine" => {
                let shift = if p[0].0 == p[1].0 {
                    (p[2].0 - p[0].0, 0.0)
                } else {
                    let k = (p[1].1 - p[0].1) / (p[1].0 - p[0].0);
                    (0.0, p[2].1 - (p[0].1 + k * (p[2].0 - p[0].0)))
                };
                for (a, c) in [
                    (p[0], p[1]),
                    (
                        (p[0].0 + shift.0, p[0].1 + shift.1),
                        (p[1].0 + shift.0, p[1].1 + shift.1),
                    ),
                ] {
                    self.ray(a, c, pen(color(BLUE)))?;
                    self.ray(c, a, pen(color(BLUE)))?;
                }
                Ok(())
            }
            "fibonacciLine" => {
                for pct in FIB_LEVELS {
                    let price = v[1] + (v[0] - v[1]) * pct;
                    let y = y_of(price);
                    self.line((l, y), (r, y), pen(color(BLUE)))?;
                    let text = format!("{} ({:.1}%)", amount(price, prec), pct * 100.0);
                    self.label(
                        (l, y),
                        Label {
                            text: &text,
                            size: 12,
                            fg: color("#ffffff"),
                            bg: rgba(BLUE),
                            border: None,
                            align: 0.0,
                            baseline: Baseline::Bottom,
                            pad: (4.0, 2.0),
                        },
                    )?;
                }
                Ok(())
            }
            "cgRect" => self.rect(
                p[0],
                p[1],
                rgba(s.fill.as_deref().unwrap_or(RECT.0)),
                pen(color(RECT.1)),
            ),
            "cgText" => self.label(
                p[0],
                Label {
                    text: d.text.as_deref().unwrap_or(""),
                    size: s.font_size.unwrap_or(12),
                    fg: color(TEXT_FG),
                    bg: rgba(SURFACE),
                    border: Some(rgba(RECT.1)),
                    align: 0.0,
                    baseline: Baseline::Middle,
                    pad: (6.0, 3.0),
                },
            ),
            "simpleTag" => {
                let y = p[0].1;
                let c = color(BLUE);
                self.line(
                    (l, y),
                    (r, y),
                    Pen {
                        dash: dash.or(Some((2.0, 2.0))),
                        ..pen(c)
                    },
                )?;
                let text = format!("{:.*}", prec, v[0]);
                self.label(
                    (r, y),
                    Label {
                        text: &text,
                        size: 12,
                        fg: rgba("#ffffff"),
                        bg: c,
                        border: None,
                        align: 1.0,
                        baseline: Baseline::Middle,
                        pad: (4.0, 2.0),
                    },
                )
            }
            "cgFibExt" => {
                let guide = Pen {
                    dash: Some((4.0, 4.0)),
                    ..pen(color(BLUE))
                };
                self.line(p[0], p[1], guide)?;
                self.line(p[1], p[2], guide)?;
                for (level, dflt) in FIB_EXT_LEVELS.iter().zip(FIB_EXT_COLORS) {
                    let price = v[2] + (v[1] - v[0]) * level;
                    let y = y_of(price);
                    let c = color(dflt);
                    self.line((p[2].0, y), (r.max(p[2].0), y), pen(c))?;
                    let text = format!("{level} ({})", amount(price, prec));
                    self.label(
                        (p[2].0 + 4.0, y),
                        Label {
                            text: &text,
                            size: 12,
                            fg: c,
                            bg: BG.mix(0.85),
                            border: None,
                            align: 0.0,
                            baseline: Baseline::Bottom,
                            pad: (2.0, 2.0),
                        },
                    )?;
                }
                Ok(())
            }
            "cgPitchfork" => {
                let mid = ((p[1].0 + p[2].0) / 2.0, (p[1].1 + p[2].1) / 2.0);
                let dir = (mid.0 - p[0].0, mid.1 - p[0].1);
                for o in [p[0], p[1], p[2]] {
                    self.ray(o, (o.0 + dir.0, o.1 + dir.1), pen(color(BLUE)))?;
                }
                self.line(
                    p[1],
                    p[2],
                    Pen {
                        dash: Some((4.0, 4.0)),
                        ..pen(color(BLUE))
                    },
                )
            }
            "cgPriceRange" => self.measure(d, &p, true, false, prec, pen),
            "cgDateRange" => self.measure(d, &p, false, true, prec, pen),
            "cgDatePriceRange" => self.measure(d, &p, true, true, prec, pen),
            _ => self.position(d, &p, prec, pen),
        }
    }

    fn measure(
        &self,
        d: &Drawing,
        p: &[Px],
        price: bool,
        date: bool,
        prec: usize,
        pen: impl Fn(RGBAColor) -> Pen,
    ) -> Drawn {
        let (a, b) = (p[0], p[1]);
        let (pa, pb) = (&d.points[0], &d.points[1]);
        let up = if date && !price {
            b.0 >= a.0
        } else {
            pb.price >= pa.price
        };
        let look = if up { UP } else { DOWN };
        let line = rgba(d.style.color.as_deref().unwrap_or(look.1));
        self.rect(
            a,
            b,
            rgba(d.style.fill.as_deref().unwrap_or(look.0)),
            pen(line),
        )?;
        let (xm, ym) = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
        if price {
            self.arrow((xm, a.1), (xm, b.1), pen(line))?;
        }
        if date {
            self.arrow((a.0, ym), (b.0, ym), pen(line))?;
        }
        let mut texts = vec![];
        if price {
            texts.push(price_range_label(pa.price, pb.price, prec));
        }
        if date {
            let bars = (pb.x - pa.x).abs();
            let ms = (pb.time_ms - pa.time_ms).abs();
            texts.push(format!(
                "{bars} bar{}, {}",
                if bars == 1 { "" } else { "s" },
                duration(ms)
            ));
        }
        let (top, bottom) = (a.1.min(b.1), a.1.max(b.1));
        // Stacked above the band, the last nearest; below it when there is no room above.
        let below = top - 4.0 - texts.len() as f64 * LABEL_H < self.y.0;
        for (i, text) in texts.iter().enumerate() {
            let x = self.label_x(a.0.min(b.0), a.0.max(b.0), text, 12);
            let y = if below {
                bottom + 4.0 + i as f64 * LABEL_H
            } else {
                top - 4.0 - (texts.len() - 1 - i) as f64 * LABEL_H
            };
            let baseline = if below {
                Baseline::Top
            } else {
                Baseline::Bottom
            };
            self.tag((x, y), text, line, baseline)?;
        }
        Ok(())
    }

    fn position(
        &self,
        d: &Drawing,
        p: &[Px],
        prec: usize,
        pen: impl Fn(RGBAColor) -> Pen,
    ) -> Drawn {
        let (e, t, s) = (p[0], p[1], p[2]);
        let (left, right) = (e.0.min(t.0), e.0.max(t.0));
        let thin = |c: &str| Pen {
            color: rgba(c),
            width: 1,
            dash: None,
        };
        self.rect((left, e.1), (right, t.1), rgba(GREEN.0), thin(GREEN.1))?;
        self.rect((left, e.1), (right, s.1), rgba(RED.0), thin(RED.1))?;
        let line = rgba(d.style.color.as_deref().unwrap_or(POSITION_LINE));
        self.line((left, e.1), (right, e.1), pen(line))?;
        let long = d.tool.name == "cgLong";
        let [pe, pt, ps] = [0, 1, 2].map(|i| d.points[i].price);
        let dir = if long { 1.0 } else { -1.0 };
        let (reward, risk) = ((pt - pe) * dir, (pe - ps) * dir);
        let pct = |v: f64| if pe == 0.0 { 0.0 } else { v / pe.abs() * 100.0 };
        let target = format!("Target: {} ({}%)", amount(pt, prec), amount(pct(reward), 2));
        let stop = format!("Stop: {} ({}%)", amount(ps, prec), amount(pct(risk), 2));
        let ratio = format!(
            "Risk/Reward Ratio: {}",
            if risk > 0.0 {
                format!("{:.2}", reward / risk)
            } else {
                "—".into()
            }
        );
        let side = |up: bool| if up { Baseline::Bottom } else { Baseline::Top };
        self.tag(
            (self.label_x(left, right, &target, 12), t.1),
            &target,
            rgba(GREEN.1),
            side(t.1 <= e.1),
        )?;
        self.tag(
            (self.label_x(left, right, &stop, 12), s.1),
            &stop,
            rgba(RED.1),
            side(s.1 < e.1),
        )?;
        self.tag(
            (self.label_x(left, right, &ratio, 12), e.1),
            &ratio,
            line,
            Baseline::Middle,
        )
    }
}

/// Decimals for prices near `p`: 2 from 1 up, more below so they keep 4 digits.
fn decimals(p: f64) -> usize {
    let a = p.abs();
    if a >= 1.0 || a == 0.0 || !a.is_finite() {
        2
    } else {
        ((-a.log10()).ceil() as usize + 3).min(8)
    }
}

/// `1,234.50`: fixed decimals with thousands separators, as the web app's `formatAmount`.
fn amount(v: f64, prec: usize) -> String {
    let text = format!("{:.*}", prec, v.abs());
    let (int, frac) = text
        .split_once('.')
        .map_or((text.as_str(), ""), |(i, f)| (i, f));
    let mut grouped = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    let sign = if v < 0.0 && text.bytes().any(|b| b.is_ascii_digit() && b != b'0') {
        "-"
    } else {
        ""
    };
    if frac.is_empty() {
        format!("{sign}{grouped}")
    } else {
        format!("{sign}{grouped}.{frac}")
    }
}

/// `+1,234.50 (+2.31%)`, as the web app's `priceRangeLabel`.
fn price_range_label(from: f64, to: f64, prec: usize) -> String {
    let delta = to - from;
    let pct = if from == 0.0 {
        0.0
    } else {
        delta / from.abs() * 100.0
    };
    let signed =
        |v: f64, p: usize| format!("{}{}", if v < 0.0 { "-" } else { "+" }, amount(v.abs(), p));
    format!("{} ({}%)", signed(delta, prec), signed(pct, 2))
}

/// `3d 4h`, `5h 30m`, `45m`: the two largest units, as the web app's `formatDuration`.
fn duration(ms: i64) -> String {
    let total = ms.abs() / 60_000;
    let (d, h, m) = (total / 1440, (total % 1440) / 60, total % 60);
    match (d, h, m) {
        (d, 0, _) if d > 0 => format!("{d}d"),
        (d, h, _) if d > 0 => format!("{d}d {h}h"),
        (_, h, 0) if h > 0 => format!("{h}h"),
        (_, h, m) if h > 0 => format!("{h}h {m}m"),
        _ => format!("{m}m"),
    }
}

/// The points of a line as runs between NaN values, each drawn as its own
/// path: a gap breaks the line like Pine's plot.style_linebr and the web
/// chart, instead of a connector across it.
fn line_runs(y: &[f64]) -> Vec<Vec<(f64, f64)>> {
    let points: Vec<(f64, f64)> = y.iter().enumerate().map(|(i, v)| (i as f64, *v)).collect();
    points
        .split(|(_, v)| v.is_nan())
        .filter(|run| !run.is_empty())
        .map(<[_]>::to_vec)
        .collect()
}

/// `runs`, split again wherever a value changes if the line holds levels, so a jump
/// between two levels draws no connector (`PanelResult::level_lines`).
fn level_runs(runs: Vec<Vec<(f64, f64)>>, levels: bool) -> Vec<Vec<(f64, f64)>> {
    if !levels {
        return runs;
    }
    runs.iter()
        .flat_map(|run| run.chunk_by(|a, b| a.1 == b.1).map(<[_]>::to_vec))
        .collect()
}

/// Legend text of a line: its last finite value. A line that breaks at trend
/// flips (`breaks`) reads the last bar instead, "n/a" when it has none there
/// (the inactive Supertrend line). None when it has no value at all.
fn legend_value(y: &[f64], breaks: bool) -> Option<String> {
    if y.iter().all(|v| v.is_nan()) {
        return None;
    }
    let v = if breaks {
        y.last().copied().filter(|v| !v.is_nan())
    } else {
        y.iter().rev().find(|v| !v.is_nan()).copied()
    };
    Some(v.map_or_else(|| "n/a".into(), |v| format!("{v:.2}")))
}

fn auto_range(r: &PanelResult) -> (f64, f64) {
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for line in &r.lines {
        for v in &line.y {
            if v.is_nan() {
                continue;
            }
            lo = lo.min(*v);
            hi = hi.max(*v);
        }
    }
    for bars in &r.bars {
        for v in &bars.y {
            if v.is_nan() {
                continue;
            }
            lo = lo.min(bars.bottom.min(bars.bottom + *v));
            hi = hi.max(bars.bottom.max(bars.bottom + *v));
        }
    }
    for hbar in &r.hbars {
        lo = lo.min(hbar.y - hbar.height / 2.0);
        hi = hi.max(hbar.y + hbar.height / 2.0);
    }
    for divline in &r.divlines {
        if !divline.y1.is_nan() {
            lo = lo.min(divline.y1);
            hi = hi.max(divline.y1);
        }
        if !divline.y2.is_nan() {
            lo = lo.min(divline.y2);
            hi = hi.max(divline.y2);
        }
    }
    // Fallback for empty panels
    if lo.is_infinite() || hi.is_infinite() || (hi - lo).abs() < 1e-10 {
        return (-1.0, 1.0);
    }
    let margin = (hi - lo).max(1.0) * 0.1;
    (lo - margin, hi + margin)
}

#[cfg(test)]
mod tests {
    use super::*;
    const NAN: f64 = f64::NAN;

    #[test]
    fn the_title_names_how_the_bars_are_built() {
        let data = OhlcvData {
            bars: Vec::new(),
            symbol: Some("BTCUSDT".into()),
            interval: Some("1h".into()),
            session: None,
        };
        assert_eq!(chart_title(&data, None), "BTCUSDT · 1H");
        assert_eq!(
            chart_title(&data, Some("P&F 3.4 x 3")),
            "BTCUSDT · 1H · P&F 3.4 x 3"
        );
    }

    #[test]
    fn lines_break_at_nan() {
        // A flip of linebr lines: no point joins the runs across the gap.
        let runs = line_runs(&[NAN, 1.0, 2.0, NAN, NAN, 3.0, NAN, 4.0, 5.0]);
        assert_eq!(
            runs,
            vec![
                vec![(1.0, 1.0), (2.0, 2.0)],
                vec![(5.0, 3.0)],
                vec![(7.0, 4.0), (8.0, 5.0)],
            ]
        );
        assert!(line_runs(&[NAN, NAN]).is_empty());
        let levels = line_runs(&[1.0, 1.0, 2.0, 2.0, NAN, 3.0]);
        assert_eq!(level_runs(levels.clone(), false).len(), 2);
        assert_eq!(
            level_runs(levels, true),
            vec![
                vec![(0.0, 1.0), (1.0, 1.0)],
                vec![(2.0, 2.0), (3.0, 2.0)],
                vec![(5.0, 3.0)]
            ]
        );
        assert_eq!(line_runs(&[1.0, 2.0]), vec![vec![(0.0, 1.0), (1.0, 2.0)]]);
    }

    #[test]
    fn legend_shows_the_last_bar() {
        assert_eq!(legend_value(&[1.0, 2.5], false).as_deref(), Some("2.50"));
        // The inactive trend line: no stale value from an earlier bar.
        assert_eq!(legend_value(&[1.0, NAN], true).as_deref(), Some("n/a"));
        assert_eq!(legend_value(&[1.0, NAN], false).as_deref(), Some("1.00"));
        assert_eq!(legend_value(&[NAN, NAN], true), None);
        assert_eq!(legend_value(&[], false), None);
    }
}

#[cfg(test)]
mod label_tests {
    use super::*;

    #[test]
    fn date_bars_are_labelled_by_date_alone() {
        let t = 1_793_802_600; // 2026-11-04 14:30 UTC
        for tf in ["1d", "2D", "3d", "1wk", "2W", "1mo", "3M"] {
            assert_eq!(format_bar_label(t, tf), "2026-11-04", "{tf}");
        }
        for tf in ["1m", "7m", "45m", "8h", "36h"] {
            assert_eq!(format_bar_label(t, tf), "2026-11-04 14:30", "{tf}");
        }
    }
}
