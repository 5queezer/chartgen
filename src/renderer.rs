use crate::data::OhlcvData;
use crate::drawings::{parse_color, Drawing, View};
use crate::indicator::{Indicator, PanelResult, Projection};
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
    let plain: Vec<_> = results.iter().map(|r| (*r, None)).collect();
    render_labeled(data, &plain, output, width, height, None, &[])
}

/// `render_results`, naming in the title how the bars are built (ADR-0047), with
/// `drawings` on the price chart (ADR-0054). A result may come with what it plots past
/// the last bar (ADR-0033): the image then reaches that far.
pub fn render_labeled(
    data: &OhlcvData,
    results: &[(&PanelResult, Option<&Projection>)],
    output: &str,
    width: u32,
    height: u32,
    style: Option<&str>,
    drawings: &[Drawing],
) -> Result<(), Box<dyn std::error::Error>> {
    let root = BitMapBackend::new(output, (width, height)).into_drawing_area();
    draw_chart(&root, data, results, style, drawings)?;
    root.present()?;
    Ok(())
}

type Root<'b> = DrawingArea<BitMapBackend<'b>, plotters::coord::Shift>;
type Chart<'a, 'b> = ChartContext<
    'a,
    BitMapBackend<'b>,
    Cartesian2d<plotters::coord::types::RangedCoordf64, plotters::coord::types::RangedCoordf64>,
>;

const BULL: RGBColor = RGBColor(38, 166, 154);
const BEAR: RGBColor = RGBColor(239, 83, 80);
/// Padding around the last price in its tag.
const TAG_PAD: u32 = 4;

fn draw_chart(
    root: &Root,
    data: &OhlcvData,
    results: &[(&PanelResult, Option<&Projection>)],
    style: Option<&str>,
    drawings: &[Drawing],
) -> Drawn {
    let (width, height) = root.dim_in_pixel();
    root.fill(&BG)?;
    let n = data.len();
    if n == 0 {
        // Nothing to render -- a blank image
        return Ok(());
    }

    // Separate overlays from panels
    let overlays: Vec<_> = results
        .iter()
        .copied()
        .filter(|(r, _)| r.is_overlay)
        .collect();
    let panel_results: Vec<_> = results
        .iter()
        .copied()
        .filter(|(r, _)| !r.is_overlay)
        .collect();
    let n_panels = panel_results.len();

    // Price range: the candles', as the drawings' `View`. Overlays past it are cut off,
    // as on the web chart, instead of squeezing the candles.
    let view = View::of(&data.bars);
    let price = (view.price_lo, view.price_hi);
    // Room on the right for drawings and projections past the last bar, as far as `View`
    // allows.
    let drawn = drawings
        .iter()
        .flat_map(|d| &d.points)
        .map(|p| p.x - (n as i64 - 1))
        .max()
        .unwrap_or(0);
    let projected = results
        .iter()
        .filter_map(|(_, f)| *f)
        .map(|f| f.bars as i64)
        .max()
        .unwrap_or(0);
    let x = x_span(n, drawn.max(projected).clamp(0, view.future));

    // The last price, tagged in a gutter right of every pane, so their bars line up.
    let last = &data.bars[n - 1];
    let tag = format!("{:.*}", decimals(last.close), last.close);
    let tag_font = ("monospace", 14).into_font().color(&WHITE);
    let (tag_w, tag_h) = root.estimate_text_size(&tag, &tag_font)?;
    let gutter = tag_w + 2 * TAG_PAD + 6;

    let title_h = TITLE_H;
    let (candle_h, panel_h) = layout(height, n_panels);

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

    // The price pane's plot, left and right pixel: the date labels follow its x range.
    let price_x: (f64, f64);

    // ---- Draw candles + overlays ----
    {
        let mut chart = ChartBuilder::on(&candle_area)
            .margin(10)
            .margin_right(gutter)
            .x_label_area_size(0)
            .y_label_area_size(80)
            .build_cartesian_2d(x.0..x.1, price.0..price.1)?;
        let (xs, ys) = chart.plotting_area().get_pixel_range();
        price_x = (xs.start as f64, xs.end as f64);

        chart
            .configure_mesh()
            .disable_x_mesh()
            .x_labels(0)
            .y_labels(tick_count(candle_h.saturating_sub(20)))
            .y_label_formatter(&|v| axis_number(*v, price, tick_count(candle_h.saturating_sub(20))))
            .max_light_lines(0)
            .light_line_style(GRID)
            .bold_line_style(GRID)
            .axis_style(GRID)
            .label_style(("monospace", 14, &TEXT))
            .draw()?;

        for (overlay, future) in &overlays {
            draw_background(&mut chart, overlay, *future, n, x, price)?;
        }

        for (i, bar) in data.bars.iter().enumerate() {
            let x = i as f64;
            let color = if bar.close >= bar.open { BULL } else { BEAR };

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
        for (overlay, future) in &overlays {
            draw_foreground(&mut chart, overlay, *future, n, x, price)?;
        }

        draw_profile_labels(&mut chart, &overlays, x, price)?;

        // Opaque, wrapped legend rows keep candles from crossing the text.
        let mut label_y = 5;
        for (overlay, _) in &overlays {
            label_y = draw_legend(&candle_area, overlay, label_y, gutter)?;
        }

        // Price label: in the gutter at the last close, in its candle's color.
        {
            let (gx, gy) = chart.backend_coord(&(x.1, last.close));
            let (l, t) = (gx + 2, gy - (tag_h / 2 + TAG_PAD) as i32);
            let (w, h) = ((tag_w + 2 * TAG_PAD) as i32, (tag_h + 2 * TAG_PAD) as i32);
            let color = if last.close >= last.open { BULL } else { BEAR };
            root.draw(&Rectangle::new([(l, t), (l + w, t + h)], color.filled()))?;
            root.draw_text(&tag, &tag_font, (l + TAG_PAD as i32, t + TAG_PAD as i32))?;
        }

        if !drawings.is_empty() {
            let canvas = Canvas {
                area: root,
                x: price_x,
                bars: x,
                y: (ys.start as f64, ys.end as f64, price.0, price.1),
            };
            let prec = decimals(last.close);
            for d in drawings {
                canvas.drawing(d, prec)?;
            }
        }
    }

    // ---- Draw indicator panels ----
    for (area, (result, future)) in areas.iter().zip(&panel_results) {
        let y = panel_range(result, *future);

        // The price pane's margins left and right, so a bar sits under its candle.
        let mut chart = ChartBuilder::on(area)
            .margin(5)
            .margin_left(10)
            .margin_right(gutter)
            .x_label_area_size(20)
            .y_label_area_size(80)
            .build_cartesian_2d(x.0..x.1, y.0..y.1)?;

        // The label area stays for the spacing, empty: the dates under the last pane are
        // the image's only x labels.
        chart
            .configure_mesh()
            .disable_x_mesh()
            .x_labels(0)
            .y_labels(tick_count(panel_h.saturating_sub(30)))
            .y_label_formatter(&|v| axis_number(*v, y, tick_count(panel_h.saturating_sub(30))))
            .max_light_lines(0)
            .light_line_style(GRID)
            .bold_line_style(GRID)
            .axis_style(GRID)
            .label_style(("monospace", 13, &TEXT))
            .draw()?;

        draw_result(&mut chart, result, *future, n, x, y)?;

        draw_legend(area, result, 5, gutter)?;
    }

    // ---- Draw X-axis datetime labels along the bottom ----
    {
        let interval = data.interval.as_deref().unwrap_or("4h");
        let y_pos = height as i32 - 4; // just above the bottom edge
        let font = ("monospace", 13).into_font().color(&TEXT);
        let label_count = (width / 180).max(3) as usize; // ~1 label per 180px
        let step = (n / label_count).max(1);

        for idx in (0..n).step_by(step) {
            let epoch = data.bars[idx].date.parse::<i64>().unwrap_or(0);
            if epoch == 0 {
                continue;
            }
            let label = format_bar_label(epoch, interval);
            // Centred under its bar, kept inside the image.
            let w = root.estimate_text_size(&label, &font)?.0 as f64;
            let x = bar_x(idx as f64, x, price_x) - w / 2.0;
            let x = x.min(width as f64 - w).max(0.0);
            root.draw_text(&label, &font, (x as i32, y_pos - 10))?;
        }
    }

    Ok(())
}

/// Draws `r` on `chart`, cut to the bars `x` and the values `y` the pane shows. A value
/// past them is not drawn: plotters would pin it to the edge, where it reads as a level.
/// `future` goes on past the last of the `n` bars (ADR-0033).
fn draw_result(
    chart: &mut Chart,
    r: &PanelResult,
    future: Option<&Projection>,
    n: usize,
    x: (f64, f64),
    y: (f64, f64),
) -> Drawn {
    draw_background(chart, r, future, n, x, y)?;
    draw_foreground(chart, r, future, n, x, y)
}

fn draw_background(
    chart: &mut Chart,
    r: &PanelResult,
    future: Option<&Projection>,
    n: usize,
    x: (f64, f64),
    y: (f64, f64),
) -> Drawn {
    let rect = |chart: &mut Chart, (x0, x1): (f64, f64), a: f64, b: f64, c: RGBAColor| -> Drawn {
        let (lo, hi) = (a.min(b).max(y.0), a.max(b).min(y.1));
        let (x0, x1) = (x0.max(x.0), x1.min(x.1));
        if lo <= hi && x0 < x1 {
            chart.draw_series(std::iter::once(Rectangle::new(
                [(x0, lo), (x1, hi)],
                c.filled(),
            )))?;
        }
        Ok(())
    };
    // Fills (approximate with vertical bars for simplicity in plotters), and on past the
    // last bar.
    let projected = future
        .into_iter()
        .flat_map(|p| &p.fills)
        .filter_map(|(i, y1, y2)| Some((r.fills.get(*i)?, n, y1, y2)));
    for (fill, from, y1, y2) in r
        .fills
        .iter()
        .map(|f| (f, 0, &f.y1, &f.y2))
        .chain(projected)
    {
        for (i, (a, b)) in y1.iter().zip(y2).enumerate() {
            if a.is_nan() || b.is_nan() {
                continue;
            }
            let at = (from + i) as f64;
            rect(chart, (at - 0.4, at + 0.4), *a, *b, fill.color_for(*a, *b))?;
        }
    }

    // HBars (VPVR horizontal histogram) and the levels with them: over a range profile's
    // bars (`span`), its edges marked, or over the whole pane.
    let (s0, s1) = r.span.map_or(x, |(a, b)| (a as f64 - 0.5, b as f64 + 0.5));
    for hbar in &r.hbars {
        let (w, off) = ((s1 - s0) * hbar.width, (s1 - s0) * hbar.offset);
        let edges = if hbar.left {
            (s0 + off, s0 + off + w)
        } else {
            (s1 - off - w, s1 - off)
        };
        let half = hbar.height / 2.0;
        rect(chart, edges, hbar.y - half, hbar.y + half, hbar.color)?;
    }
    Ok(())
}

fn draw_foreground(
    chart: &mut Chart,
    r: &PanelResult,
    future: Option<&Projection>,
    n: usize,
    x: (f64, f64),
    y: (f64, f64),
) -> Drawn {
    let path = |chart: &mut Chart, run: &[Pt], style: ShapeStyle| -> Drawn {
        for piece in clip_run(run, x, y) {
            chart.draw_series(std::iter::once(PathElement::new(piece, style)))?;
        }
        Ok(())
    };
    let rect = |chart: &mut Chart, (x0, x1): (f64, f64), a: f64, b: f64, c: RGBAColor| -> Drawn {
        let (lo, hi) = (a.min(b).max(y.0), a.max(b).min(y.1));
        let (x0, x1) = (x0.max(x.0), x1.min(x.1));
        if lo <= hi && x0 < x1 {
            chart.draw_series(std::iter::once(Rectangle::new(
                [(x0, lo), (x1, hi)],
                c.filled(),
            )))?;
        }
        Ok(())
    };
    let inside = |(px, py): Pt| (x.0..=x.1).contains(&px) && (y.0..=y.1).contains(&py);

    let (s0, s1) = r.span.map_or(x, |(a, b)| (a as f64 - 0.5, b as f64 + 0.5));
    if let Some((_, last)) = r.span {
        let edge = TEXT.mix(0.6).stroke_width(1);
        for at in [s0].into_iter().chain((last + 1 < n).then_some(s1)) {
            for (a, b) in dashes((at, y.0), (at, y.1)) {
                path(chart, &[a, b], edge)?;
            }
        }
    }
    for h in &r.hlines {
        path(chart, &[(s0, h.y), (s1, h.y)], h.color.stroke_width(1))?;
    }

    // Bars (histogram)
    for bars in &r.bars {
        for (i, (v, c)) in bars.y.iter().zip(&bars.colors).enumerate() {
            if !v.is_nan() {
                let at = i as f64;
                rect(
                    chart,
                    (at - 0.35, at + 0.35),
                    bars.bottom,
                    bars.bottom + v,
                    *c,
                )?;
            }
        }
    }

    // Lines, broken where a value is NaN (as on the web chart and with
    // Pine's plot.style_linebr), so a gap draws no connector.
    for line in &r.lines {
        let draw_width = if line.width <= 1 { 2 } else { line.width };
        for run in level_runs(
            drawing_runs(&line.y, &r.line_breaks, r.steps),
            r.level_lines,
        ) {
            if r.point_lines || (run.len() == 1 && !r.line_breaks.is_empty()) {
                for &point in run.iter().filter(|p| inside(**p)) {
                    chart.draw_series(std::iter::once(Circle::new(
                        point,
                        2,
                        line.color.filled(),
                    )))?;
                }
            } else {
                path(chart, &run, line.color.stroke_width(draw_width))?;
            }
        }
    }
    // On past the last bar, from its value there.
    for (i, values) in future.iter().flat_map(|p| &p.lines) {
        let Some(line) = r.lines.get(*i) else {
            continue;
        };
        let joined: Vec<f64> = line.y.last().into_iter().chain(values).copied().collect();
        for run in drawing_runs(&joined, &[], r.steps) {
            let run: Vec<Pt> = run.iter().map(|(k, v)| (k + (n - 1) as f64, *v)).collect();
            let draw_width = if line.width <= 1 { 2 } else { line.width };
            path(chart, &run, line.color.stroke_width(draw_width))?;
        }
    }

    // Dots
    for dot in r.dots.iter().filter(|d| inside((d.x as f64, d.y))) {
        chart.draw_series(std::iter::once(Circle::new(
            (dot.x as f64, dot.y),
            dot.size,
            dot.color.filled(),
        )))?;
    }

    // Segments (divergences, zigzag) and pattern outlines (ADR-0046), which draw the same
    // way, dashed or not.
    let strokes = r.patterns.iter().flat_map(|p| {
        p.strokes.iter().map(|s| {
            let ends = ((s.x1 as f64, s.y1), (s.x2 as f64, s.y2));
            (ends, p.bias.color(), s.dashed)
        })
    });
    let segments = r
        .divlines
        .iter()
        .map(|d| {
            (
                ((d.x1 as f64, d.y1), (d.x2 as f64, d.y2)),
                d.color,
                d.dashed,
            )
        })
        .chain(strokes);
    for ((a, b), color, dashed) in segments {
        let Some((a, b)) = clip(a, b, x, y) else {
            continue;
        };
        let pieces = if dashed { dashes(a, b) } else { vec![(a, b)] };
        for (a, b) in pieces {
            chart.draw_series(LineSeries::new([a, b], color.stroke_width(2)))?;
        }
    }

    // Pattern labels: past the bar's high or low, in the pattern's color, moved left to
    // end at the plot's right edge rather than run past it.
    let right = chart.plotting_area().get_pixel_range().0.end;
    let text = |chart: &mut Chart, s: &str, (x, y): Pt, above: bool, c: RGBAColor| -> Drawn {
        let font = ("sans-serif", 12).into_font().color(&c);
        let w = chart.plotting_area().estimate_text_size(s, &font)?.0 as i32;
        let dx = (-4).min(right - chart.backend_coord(&(x, y)).0 - w);
        let dy = if above { -16 } else { 4 };
        chart.draw_series(std::iter::once(
            EmptyElement::at((x, y)) + Text::new(s.to_string(), (dx, dy), font),
        ))?;
        Ok(())
    };
    for l in r.labels.iter().filter(|l| inside((l.x as f64, l.y))) {
        let c = l.bias.color();
        let c = if l.tentative { c.mix(0.5) } else { c };
        text(chart, &l.text, (l.x as f64, l.y), l.above, c)?;
    }
    for p in &r.patterns {
        let at = (p.label_x as f64, p.label_y);
        if inside(at) {
            let s = format!("{} ({})", p.name, p.status.as_str());
            text(chart, &s, at, p.label_above, p.bias.color())?;
        }
    }
    Ok(())
}

/// Bars a pane spans for `n` bars: one more each side, so the first and last candles
/// show whole, and `future` more past the last for what is drawn there.
fn x_span(n: usize, future: i64) -> (f64, f64) {
    (-1.0, n as f64 + future as f64)
}

/// Pixel of bar `x` (from 0) on a plot from `left` to `right` that spans the bars `span`,
/// the extension past the last bar included.
fn bar_x(x: f64, (first, last): (f64, f64), (left, right): (f64, f64)) -> f64 {
    left + (x - first) / (last - first) * (right - left)
}

/// Height of the title bar.
const TITLE_H: u32 = 40;
/// Strip under the last pane for the date labels, as a panel's x label area.
const DATE_LABELS_H: u32 = 20;

/// Heights of the price pane and of each panel below it: the price pane takes 55% under
/// the title and the panels share the rest, or with no panels all of it but the date labels.
fn layout(height: u32, n_panels: usize) -> (u32, u32) {
    let below_title = height.saturating_sub(TITLE_H);
    if n_panels == 0 {
        return (below_title.saturating_sub(DATE_LABELS_H), 0);
    }
    let candle_h = (below_title as f64 * 0.55) as u32;
    (candle_h, (below_title - candle_h) / n_panels as u32)
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
/// A point in a chart's own units: bar and value.
type Pt = (f64, f64);
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
    area: &'a Root<'b>,
    /// Left and right pixel of the plot.
    x: (f64, f64),
    /// The bars at its left and right edge.
    bars: (f64, f64),
    /// Top and bottom pixel of the plot, and the prices at its bottom and top.
    y: (f64, f64, f64, f64),
}

impl Canvas<'_, '_> {
    fn px(&self, x: f64, price: f64) -> Px {
        let (t, b, lo, hi) = self.y;
        (
            bar_x(x, self.bars, self.x),
            b - (price - lo) / (hi - lo) * (b - t),
        )
    }

    fn inside(&self, (x, y): Px) -> bool {
        (self.x.0..=self.x.1).contains(&x) && (self.y.0..=self.y.1).contains(&y)
    }

    /// `a`-`b` cut to the plot.
    fn clip(&self, a: Px, b: Px) -> Option<(Px, Px)> {
        clip(a, b, self.x, (self.y.0, self.y.1))
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

/// Preserve every sample, splitting BEFORE session boundaries. Steps hold the old
/// value until the new bar, then jump vertically at that bar.
fn drawing_runs(values: &[f64], breaks: &[usize], steps: bool) -> Vec<Vec<Pt>> {
    if breaks.is_empty() && !steps {
        return line_runs(values);
    }
    let mut runs = Vec::new();
    let mut run = Vec::new();
    for (i, &value) in values.iter().enumerate() {
        if (!value.is_finite() || breaks.contains(&i)) && !run.is_empty() {
            runs.push(std::mem::take(&mut run));
        }
        if !value.is_finite() {
            continue;
        }
        if steps {
            if let Some(&(_, previous)) = run.last() {
                run.push((i as f64, previous));
            }
        }
        run.push((i as f64, value));
    }
    if !run.is_empty() {
        runs.push(run);
    }
    runs
}

/// Price tags for profile levels, placed together so adjacent profiles cannot
/// overwrite each other's names. Numeric prices retain price precision.
fn draw_profile_labels(
    chart: &mut Chart,
    overlays: &[(&PanelResult, Option<&Projection>)],
    x: (f64, f64),
    y: (f64, f64),
) -> Drawn {
    let mut labels = Vec::new();
    for (r, _) in overlays {
        let end = r.span.map_or(x.1, |(_, last)| last as f64 + 0.5).min(x.1);
        for (i, h) in r.hlines.iter().enumerate() {
            if let Some(Some(name)) = r.hline_labels.get(i) {
                if h.y.is_finite() && (y.0..=y.1).contains(&h.y) {
                    labels.push((
                        end,
                        h.y,
                        format!("{name}: {}", readable_number(h.y, decimals(h.y), false)),
                        h.color,
                    ));
                }
            }
        }
        for line in &r.lines {
            let Some(name) = line.label.as_deref() else {
                continue;
            };
            if !["POC", "VAH", "VAL", "HVN", "LVN"]
                .iter()
                .any(|level| name.contains(level))
            {
                continue;
            }
            let Some(&value) = line.y.iter().rev().find(|v| v.is_finite()) else {
                continue;
            };
            if (y.0..=y.1).contains(&value) {
                labels.push((
                    end,
                    value,
                    format!("{name}: {}", readable_number(value, decimals(value), false)),
                    line.color,
                ));
            }
        }
    }
    labels.sort_by(|a, b| b.1.total_cmp(&a.1));
    let (px, py) = chart.plotting_area().get_pixel_range();
    let height = py.end - py.start;
    let spacing = 18.min(height / labels.len().max(1) as i32);
    let mut previous = py.start - spacing;
    let count = labels.len();
    for (i, (end, value, text, color)) in labels.into_iter().enumerate() {
        let (gx, gy) = chart.backend_coord(&(end, value));
        let color = legend_color(RGBColor(color.0, color.1, color.2));
        let font = ("monospace", 12).into_font().color(&color);
        let width = chart.plotting_area().estimate_text_size(&text, &font)?.0 as i32;
        let top = (gy - 7)
            .max(previous + spacing)
            .min(py.end - 16 - (count - i - 1) as i32 * spacing)
            .max(py.start);
        previous = top;
        let dx = (-width - 5).max(px.start - gx);
        let dy = top - gy;
        chart.draw_series(std::iter::once(
            EmptyElement::at((end, value))
                + Rectangle::new([(dx - 3, dy - 2), (dx + width + 3, dy + 14)], BG.filled())
                + Text::new(text, (dx, dy), font),
        ))?;
    }
    Ok(())
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

/// `a`-`b` cut to the box from `x.0` to `x.1` by `y.0` to `y.1` (Liang-Barsky); None
/// when none of it is inside. An end inside comes back as it was.
fn clip(a: Pt, b: Pt, x: (f64, f64), y: (f64, f64)) -> Option<(Pt, Pt)> {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
    for (p, q) in [
        (-dx, a.0 - x.0),
        (dx, x.1 - a.0),
        (-dy, a.1 - y.0),
        (dy, y.1 - a.1),
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
    let at = |t: f64| match t {
        0.0 => a,
        1.0 => b,
        t => (a.0 + t * dx, a.1 + t * dy),
    };
    (t0 <= t1).then(|| (at(t0), at(t1)))
}

/// The pieces of the line through `run` inside the box `x` by `y`, each ending where the
/// line leaves it.
fn clip_run(run: &[Pt], x: (f64, f64), y: (f64, f64)) -> Vec<Vec<Pt>> {
    if let [p] = run {
        let inside = (x.0..=x.1).contains(&p.0) && (y.0..=y.1).contains(&p.1);
        return if inside { vec![vec![*p]] } else { vec![] };
    }
    let mut pieces = Vec::new();
    let mut piece: Vec<Pt> = Vec::new();
    for w in run.windows(2) {
        let Some((a, b)) = clip(w[0], w[1], x, y) else {
            continue;
        };
        if piece.last() != Some(&a) {
            if !piece.is_empty() {
                pieces.push(std::mem::take(&mut piece));
            }
            piece.push(a);
        }
        piece.push(b);
        if b != w[1] {
            pieces.push(std::mem::take(&mut piece));
        }
    }
    if !piece.is_empty() {
        pieces.push(piece);
    }
    pieces
}

/// `a`-`b` dashed: 19 steps, every other one drawn, so a dash ends at each end.
fn dashes(a: Pt, b: Pt) -> Vec<(Pt, Pt)> {
    const STEPS: usize = 19;
    let (dx, dy) = ((b.0 - a.0) / STEPS as f64, (b.1 - a.1) / STEPS as f64);
    let at = |s: usize| match s {
        0 => a,
        STEPS => b,
        s => (a.0 + dx * s as f64, a.1 + dy * s as f64),
    };
    (0..STEPS).step_by(2).map(|s| (at(s), at(s + 1))).collect()
}

/// Lift dim plot colors for small text on the dark, opaque legend surface.
fn legend_color(c: RGBColor) -> RGBColor {
    let light = (f64::from(c.0) * 299.0 + f64::from(c.1) * 587.0 + f64::from(c.2) * 114.0) / 1000.0;
    let mix = ((145.0 - light) / (255.0 - light)).clamp(0.0, 1.0);
    let lift = |v| (f64::from(v) + (255.0 - f64::from(v)) * mix).round() as u8;
    RGBColor(lift(c.0), lift(c.1), lift(c.2))
}

/// Legend rows of a result: each labelled line with its value, or with none, its
/// histogram's under the result's name (Funding, AO).
fn legend(r: &PanelResult) -> Vec<(String, RGBColor)> {
    let solid = |c: &RGBAColor| legend_color(RGBColor(c.0, c.1, c.2));
    let mut rows: Vec<_> = r
        .lines
        .iter()
        .filter_map(|l| {
            let text = format!("{}: {}", l.label.as_ref()?, legend_value(&l.y, l.breaks)?);
            Some((text, solid(&l.color)))
        })
        .collect();
    if let (true, Some(bars)) = (rows.is_empty(), r.bars.first()) {
        if let Some(v) = legend_value(&bars.y, false) {
            let name = if r.label.is_empty() {
                "Histogram"
            } else {
                &r.label
            };
            let color = bars
                .y
                .iter()
                .rposition(|v| !v.is_nan())
                .and_then(|i| bars.colors.get(i))
                .map_or(TEXT, solid);
            rows.push((format!("{name}: {v}"), color));
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    for label in &r.labels {
        if !label.name.is_empty() && seen.insert((label.text.clone(), label.key.clone())) {
            rows.push((
                format!("{} = {} ({})", label.text, label.name, label.bias.as_str()),
                solid(&label.bias.color()),
            ));
        }
    }
    rows
}

/// Legend text of a line: its last finite value. A line that breaks at trend
/// flips (`breaks`) reads the last bar instead, "n/a" when it has none there
/// (the inactive Supertrend line). None when it has no value at all, and "—" when it
/// has one alone: nothing of it is drawn, and its value is not warmed up.
fn legend_value(y: &[f64], breaks: bool) -> Option<String> {
    let values = y.iter().filter(|v| !v.is_nan()).count();
    if values == 0 {
        return None;
    }
    if values == 1 && !breaks {
        return Some("—".into());
    }
    let v = if breaks {
        y.last().copied().filter(|v| !v.is_nan())
    } else {
        y.iter().rev().find(|v| !v.is_nan()).copied()
    };
    Some(v.map_or_else(|| "n/a".into(), legend_number))
}

/// Two decimals, more below 0.1 so a small value keeps three digits (funding at 0.0100).
fn legend_number(v: f64) -> String {
    let a = v.abs();
    let prec = if a == 0.0 || a >= 0.1 || !a.is_finite() {
        2
    } else {
        ((-a.log10()).ceil() as usize + 2).min(10)
    };
    readable_number(v, prec, true)
}

fn readable_number(v: f64, precision: usize, compact: bool) -> String {
    if !v.is_finite() {
        return "n/a".into();
    }
    let (scale, suffix) = number_scale(v, compact);
    let mut text = format!("{:.*}", precision, v / scale);
    if text.contains('.') {
        text = text.trim_end_matches('0').trim_end_matches('.').into();
    }
    if text == "-0" {
        text = "0".into();
    }
    format!("{text}{suffix}")
}

fn number_scale(v: f64, compact: bool) -> (f64, &'static str) {
    if compact && v.abs() >= 1e9 {
        (1e9, "B")
    } else if compact && v.abs() >= 1e6 {
        (1e6, "M")
    } else if compact && v.abs() >= 1e3 {
        (1e3, "K")
    } else {
        (1.0, "")
    }
}

fn tick_count(plot_height: u32) -> usize {
    (plot_height / 36).clamp(3, 8) as usize
}

fn axis_number(v: f64, range: (f64, f64), ticks: usize) -> String {
    // Precision is measured AFTER scaling: neighboring ticks must not collapse
    // to the same K/M/B string when the absolute values are close together.
    let scale = number_scale(v, true).0;
    let step = (range.1 - range.0).abs() / ticks.max(1) as f64 / scale;
    let prec = if step > 0.0 && step < 1.0 {
        (-step.log10()).ceil() as usize + 1
    } else {
        1
    };
    readable_number(v, prec.min(16), true)
}

/// One indicator per wrapped row, on an opaque surface.
fn draw_legend(
    area: &Root,
    r: &PanelResult,
    top: i32,
    gutter: u32,
) -> Result<i32, Box<dyn std::error::Error>> {
    let mut entries = Vec::new();
    if !r.label.is_empty() {
        entries.push((r.label.clone(), TITLE_COLOR));
    }
    entries.extend(legend(r));
    let right = area.dim_in_pixel().0.saturating_sub(gutter + 12) as i32;
    let mut x = 90;
    let mut y = top + 4;
    let mut positioned = Vec::new();
    let mut max_x = x;
    for (text, color) in entries {
        let font = ("monospace", 12).into_font().color(&color);
        let w = area.estimate_text_size(&text, &font)?.0 as i32;
        if x > 90 && x + w > right {
            x = 90;
            y += 18;
        }
        max_x = max_x.max(x + w);
        positioned.push((text, color, x, y));
        x += w + 18;
    }
    if positioned.is_empty() {
        return Ok(top);
    }
    area.draw(&Rectangle::new(
        [(85, top), (max_x + 5, y + 16)],
        BG.filled(),
    ))?;
    for (text, color, x, y) in positioned {
        area.draw_text(&text, &("monospace", 12).into_font().color(&color), (x, y))?;
    }
    Ok(y + 18)
}

/// Lowest and highest value `r` draws, its projection's included; None when it draws none.
fn extent(r: &PanelResult, future: Option<&Projection>) -> Option<(f64, f64)> {
    let lines = r.lines.iter().flat_map(|l| &l.y).copied();
    let fills = r
        .fills
        .iter()
        .flat_map(|f| f.y1.iter().chain(&f.y2))
        .copied();
    let bars = r.bars.iter().flat_map(|b| {
        let drawn = b.y.iter().filter(|v| !v.is_nan());
        drawn.flat_map(move |v| [b.bottom, b.bottom + v])
    });
    let hbars = r
        .hbars
        .iter()
        .flat_map(|h| [h.y - h.height / 2.0, h.y + h.height / 2.0]);
    let segments = r.divlines.iter().flat_map(|d| [d.y1, d.y2]);
    let dots = r.dots.iter().map(|d| d.y);
    let projected = future.into_iter().flat_map(|p| {
        let lines = p.lines.iter().flat_map(|(_, v)| v);
        lines
            .chain(p.fills.iter().flat_map(|(_, a, b)| a.iter().chain(b)))
            .copied()
    });
    lines
        .chain(fills)
        .chain(bars)
        .chain(hbars)
        .chain(segments)
        .chain(dots)
        .chain(projected)
        .chain(r.hlines.iter().map(|h| h.y))
        .filter(|v| v.is_finite())
        .fold(None, |acc, v| {
            Some(acc.map_or((v, v), |(lo, hi): (f64, f64)| (lo.min(v), hi.max(v))))
        })
}

/// A panel's value range: its own (`y_range`, which places its reference lines), grown
/// past a side its values reach beyond, or with none, its values with a tenth more each
/// side. Nothing it draws is cut off.
fn panel_range(r: &PanelResult, future: Option<&Projection>) -> (f64, f64) {
    let Some((lo, hi)) = extent(r, future) else {
        return r.y_range.unwrap_or((-1.0, 1.0));
    };
    match r.y_range {
        Some((a, b)) => {
            let pad = (hi.max(b) - lo.min(a)) * 0.05;
            (
                if lo < a { lo - pad } else { a },
                if hi > b { hi + pad } else { b },
            )
        }
        None if hi > lo => {
            let pad = (hi - lo) * 0.1;
            (lo - pad, hi + pad)
        }
        None => {
            let pad = if lo == 0.0 { 1.0 } else { lo.abs() * 0.1 };
            (lo - pad, hi + pad)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const NAN: f64 = f64::NAN;

    #[test]
    fn readable_values_drop_negative_zero_and_compact_large_numbers() {
        assert_eq!(legend_number(-0.0), "0");
        assert_eq!(legend_number(8_400_000_000.0), "8.4B");
        assert_eq!(legend_number(-1_250_000.0), "-1.25M");
        assert_eq!(legend_number(1200.0), "1.2K");
        assert_eq!(legend_number(20.0), "20");
    }

    #[test]
    fn compact_axis_ticks_keep_narrow_range_differences() {
        for (a, b, range) in [
            (80_000.0, 80_002.0, (80_000.0, 80_010.0)),
            (
                8_400_000_000.0,
                8_400_004_000.0,
                (8_400_000_000.0, 8_400_020_000.0),
            ),
        ] {
            assert_ne!(axis_number(a, range, 5), axis_number(b, range, 5));
        }
        assert_eq!(axis_number(-1e-16, (-0.05, 0.05), 3), "0");
    }

    #[test]
    fn profile_histograms_preserve_candle_body_colors() {
        let (w, h) = (800, 400);
        let data = crate::data::sample_data(40);
        let paint = |results: &[(&PanelResult, Option<&Projection>)]| {
            let mut buf = vec![0; (w * h * 3) as usize];
            {
                let root = BitMapBackend::with_buffer(&mut buf, (w, h)).into_drawing_area();
                draw_chart(&root, &data, results, None, &[]).unwrap();
            }
            buf
        };
        let base = paint(&[]);
        let profile = PanelResult {
            is_overlay: true,
            hbars: vec![crate::indicator::HBar {
                y: (data.bars[0].low + data.bars[0].high) / 2.0,
                height: 1000.0,
                width: 1.0,
                offset: 0.0,
                color: RGBAColor(0, 0, 255, 0.5),
                left: true,
            }],
            ..Default::default()
        };
        let actual = paint(&[(&profile, None)]);
        let mut candles = 0;
        for (before, after) in base
            .as_chunks::<3>()
            .0
            .iter()
            .zip(actual.as_chunks::<3>().0)
        {
            if *before == [BULL.0, BULL.1, BULL.2] || *before == [BEAR.0, BEAR.1, BEAR.2] {
                candles += 1;
                assert_eq!(before, after, "histogram must stay behind the candle");
            }
        }
        assert!(candles > 100);
    }

    #[test]
    fn bars_sit_on_the_price_panes_x_range() {
        // 100 bars and 49 more for drawings past the last one, on a plot from 90 to 1390.
        let plot = (90.0, 1390.0);
        let span = x_span(100, 49);
        assert_eq!(span, (-1.0, 149.0));
        assert_eq!(bar_x(-1.0, span, plot), 90.0);
        assert_eq!(bar_x(149.0, span, plot), 1390.0);
        assert_eq!(
            bar_x(99.0, span, plot),
            90.0 + 1300.0 * 100.0 / 150.0,
            "the last bar, 100 of 150 in"
        );
        // The first and last bars are a bar in from the edges.
        assert_eq!(bar_x(0.0, x_span(10, 0), plot), 90.0 + 1300.0 / 11.0);
        assert_eq!(bar_x(9.0, x_span(10, 0), plot), 1390.0 - 1300.0 / 11.0);
    }

    #[test]
    fn without_panels_the_price_pane_takes_the_height() {
        // Title, then the price pane down to the date labels.
        assert_eq!(layout(1080, 0), (1020, 0));
        assert_eq!(layout(400, 0), (340, 0));
        // With panels, as before: 55% for the price pane, the rest shared.
        assert_eq!(layout(1080, 1), (572, 468));
        assert_eq!(layout(1080, 2), (572, 234));
        assert_eq!(layout(800, 3), (418, 114));
    }

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
        assert_eq!(legend_value(&[1.0, 2.5], false).as_deref(), Some("2.5"));
        // The inactive trend line: no stale value from an earlier bar.
        assert_eq!(legend_value(&[1.0, NAN], true).as_deref(), Some("n/a"));
        assert_eq!(legend_value(&[1.0, 2.0, NAN], false).as_deref(), Some("2"));
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

#[cfg(test)]
mod clip_tests {
    use super::*;
    use crate::indicator::{Bars, Fill, HBar, HLine, Line};
    const NAN: f64 = f64::NAN;

    fn line(y: Vec<f64>) -> Line {
        Line {
            y,
            color: RGBAColor(255, 0, 255, 1.0),
            width: 1,
            label: Some("L".into()),
            breaks: false,
        }
    }

    #[test]
    fn a_line_is_cut_where_it_leaves_the_pane_not_pinned_to_its_edge() {
        // #128: up out of the pane and back: two pieces, each ending on the edge.
        let run = [(0.0, 1.0), (1.0, 5.0), (2.0, 1.0)];
        assert_eq!(
            clip_run(&run, (-1.0, 3.0), (0.0, 3.0)),
            vec![vec![(0.0, 1.0), (0.5, 3.0)], vec![(1.5, 3.0), (2.0, 1.0)]]
        );
        // Wholly past it: nothing, where plotters drew a flat line on the edge.
        assert!(clip_run(&[(0.0, 9.0), (1.0, 9.0)], (-1.0, 3.0), (0.0, 3.0)).is_empty());
        // Inside: as it was, every point the same.
        let inside = [(0.0, 0.1), (1.0, 0.7), (2.0, 0.3)];
        assert_eq!(
            clip_run(&inside, (-1.0, 3.0), (0.0, 1.0)),
            vec![inside.to_vec()]
        );
        // Before the first bar (a re-based pattern stroke): cut at the pane's left edge.
        assert_eq!(
            clip((-5.0, 0.0), (5.0, 10.0), (-1.0, 10.0), (0.0, 100.0)),
            Some(((-1.0, 4.0), (5.0, 10.0)))
        );
    }

    #[test]
    fn dashes_reach_both_ends() {
        let d = dashes((0.0, 0.0), (19.0, 38.0));
        assert_eq!(d.first().unwrap().0, (0.0, 0.0));
        assert_eq!(d.last().unwrap().1, (19.0, 38.0));
        assert_eq!(d.len(), 10);
    }

    #[test]
    fn a_panels_own_range_grows_to_what_it_draws() {
        // #126: CCI at -420 on its ±250 axis: the low side grows, the ±100 lines stay.
        let cci = PanelResult {
            lines: vec![line(vec![-419.9, 0.0, 120.0])],
            y_range: Some((-250.0, 250.0)),
            ..Default::default()
        };
        let (lo, hi) = panel_range(&cci, None);
        assert!(lo < -419.9 && lo > -460.0, "{lo}");
        assert_eq!(hi, 250.0);
        // Inside it: the range as it was.
        let calm = PanelResult {
            lines: vec![line(vec![-80.0, 90.0])],
            ..cci
        };
        assert_eq!(panel_range(&calm, None), (-250.0, 250.0));
    }

    #[test]
    fn small_values_fill_their_panel() {
        // #132: funding near 0.01 %: the range is its values' and the zero line, not ±0.1.
        let funding = PanelResult {
            bars: vec![Bars {
                y: vec![0.003, NAN, 0.01],
                colors: vec![RGBAColor(0, 0, 0, 1.0); 3],
                bottom: 0.0,
            }],
            ..Default::default()
        };
        let (lo, hi) = panel_range(&funding, None);
        assert!(
            (lo - -0.001).abs() < 1e-12 && (hi - 0.011).abs() < 1e-12,
            "{lo} {hi}"
        );
        // A flat line keeps a range around it.
        let flat = PanelResult {
            lines: vec![line(vec![5000.0, 5000.0])],
            ..Default::default()
        };
        let (lo, hi) = panel_range(&flat, None);
        assert!(lo < 5000.0 && hi > 5000.0 && hi - lo < 2000.0);
        // What is drawn past the last bar counts too.
        let future = Projection {
            bars: 1,
            lines: vec![(0, vec![9000.0])],
            fills: vec![],
        };
        assert!(panel_range(&flat, Some(&future)).1 > 9000.0);
    }

    #[test]
    fn a_histogram_alone_has_a_legend_value() {
        // #132: Funding shows its last value, small ones with three digits.
        let funding = PanelResult {
            label: "Funding".into(),
            bars: vec![Bars {
                y: vec![0.0057, 0.01, NAN],
                colors: vec![RGBAColor(38, 166, 154, 0.7); 3],
                bottom: 0.0,
            }],
            ..Default::default()
        };
        assert_eq!(
            legend(&funding),
            vec![(
                "Funding: 0.01".to_string(),
                legend_color(RGBColor(38, 166, 154))
            )]
        );
        // With a labelled line, the line's value alone, as before.
        let macd = PanelResult {
            lines: vec![line(vec![1.0, 2.0])],
            ..funding
        };
        assert_eq!(legend(&macd)[0].0, "L: 2");
        assert_eq!(legend_number(0.0057), "0.0057");
        assert_eq!(legend_number(-0.5), "-0.5");
        assert_eq!(legend_number(80395.55), "80.4K");
    }

    #[test]
    fn a_line_not_warmed_up_has_no_legend_number() {
        // #131: EMA 200 over 200 bars has one value, which draws nothing.
        let mut y = vec![NAN; 199];
        y.push(80395.55);
        assert_eq!(legend_value(&y, false).as_deref(), Some("—"));
    }

    /// The pixels painted pure magenta when `r` is drawn on a 520 x 200 plot of bars -1 to 51
    /// (10 pixels a bar) by values 0 to 100.
    fn painted(r: &PanelResult, future: Option<&Projection>) -> Vec<(u32, u32)> {
        painted_where(r, future, |p| p == [255, 0, 255])
    }

    fn painted_where(
        r: &PanelResult,
        future: Option<&Projection>,
        hit: impl Fn(&[u8]) -> bool,
    ) -> Vec<(u32, u32)> {
        const W: u32 = 520;
        let mut buf = vec![0u8; (W * 200 * 3) as usize];
        {
            let root = BitMapBackend::with_buffer(&mut buf, (W, 200)).into_drawing_area();
            let mut chart = ChartBuilder::on(&root)
                .build_cartesian_2d(-1.0..51.0, 0.0..100.0)
                .unwrap();
            draw_result(&mut chart, r, future, 50, (-1.0, 51.0), (0.0, 100.0)).unwrap();
            root.present().unwrap();
        }
        (0..W * 200)
            .filter(|i| hit(&buf[(*i * 3) as usize..][..3]))
            .map(|i| (i % W, i / W))
            .collect()
    }

    #[test]
    fn values_past_the_pane_draw_nothing() {
        // #128: a pivot level above the price range draws nothing, not a line on the edge.
        let above = PanelResult {
            lines: vec![line(vec![500.0; 50])],
            hlines: vec![HLine {
                y: -20.0,
                color: RGBAColor(255, 0, 255, 1.0),
            }],
            ..Default::default()
        };
        assert!(painted(&above, None).is_empty());
        // A line that leaves the pane at bar 25 stops there.
        let mut y = vec![50.0; 25];
        y.extend([500.0; 25]);
        let leaving = painted(
            &PanelResult {
                lines: vec![line(y)],
                ..Default::default()
            },
            None,
        );
        assert!(!leaving.is_empty());
        assert!(leaving.iter().all(|(x, _)| *x <= 262), "{leaving:?}");
    }

    #[test]
    fn stairs_points_and_session_breaks_do_not_draw_diagonal_connectors() {
        let base = PanelResult {
            lines: vec![line(vec![20.0, 80.0])],
            ..Default::default()
        };
        let stairs = painted(
            &PanelResult {
                steps: true,
                ..base.clone()
            },
            None,
        );
        assert!(stairs.iter().any(|(x, y)| *x == 15 && *y > 155));
        assert!(!stairs
            .iter()
            .any(|(x, y)| *x == 15 && (90..110).contains(y)));
        for r in [
            PanelResult {
                point_lines: true,
                ..base.clone()
            },
            PanelResult {
                line_breaks: vec![1],
                ..base
            },
        ] {
            let dots = painted(&r, None);
            assert!(!dots.is_empty(), "singleton session values remain visible");
            assert!(!dots.iter().any(|(x, _)| *x == 15));
        }
    }

    #[test]
    fn a_label_at_the_last_bar_moves_left_to_stay_inside() {
        // auto_fib's "Fib swing (forming)" on the last bar (pixel 510) ran off the image.
        let label = crate::indicator::BarLabel {
            x: 49,
            y: 50.0,
            above: false,
            text: "Fib swing (forming)".into(),
            key: String::new(),
            name: String::new(),
            bias: crate::indicator::Bias::Bullish,
            tooltip: String::new(),
            tentative: false,
        };
        let r = PanelResult {
            labels: vec![label],
            ..Default::default()
        };
        // The label's blue, smoothed at its edges.
        let px = painted_where(&r, None, |p| p[2] > 120 && p[2] > p[0] + 60);
        let first = px.iter().map(|p| p.0).min();
        assert!(
            first < Some(470),
            "the text starts well left of its bar: {first:?}"
        );
    }

    #[test]
    fn a_range_profile_sits_over_its_bars() {
        // #129: bars 10 to 20 of 50: the histogram and its levels span them alone.
        let magenta = RGBAColor(255, 0, 255, 1.0);
        let frvp = PanelResult {
            hbars: vec![HBar {
                y: 50.0,
                height: 10.0,
                width: 1.0,
                offset: 0.0,
                color: magenta,
                left: true,
            }],
            hlines: vec![HLine {
                y: 80.0,
                color: magenta,
            }],
            span: Some((10, 20)),
            ..Default::default()
        };
        let px = painted(&frvp, None);
        assert!(!px.is_empty());
        // Bars 9.5 and 20.5 lie at pixels 105 and 215.
        assert!(px.iter().all(|(x, _)| (104..=216).contains(x)), "{px:?}");
    }

    #[test]
    fn a_projection_is_drawn_past_the_last_bar() {
        // #127: the cloud past bar 49, nothing before it.
        let cloud = PanelResult {
            fills: vec![Fill {
                y1: vec![NAN; 50],
                y2: vec![NAN; 50],
                color: RGBAColor(255, 0, 255, 1.0),
                color_below: None,
            }],
            ..Default::default()
        };
        let future = Projection {
            bars: 5,
            lines: vec![],
            fills: vec![(0, vec![40.0; 5], vec![60.0; 5])],
        };
        assert!(painted(&cloud, None).is_empty());
        let px = painted(&cloud, Some(&future));
        assert!(!px.is_empty());
        // Bar 49.6, the first projected rect's left edge, lies at pixel 506.
        assert!(px.iter().all(|(x, _)| *x >= 505), "{px:?}");
    }

    #[test]
    fn the_last_price_and_the_first_candle_show_whole() {
        // #133: on an 800 x 400 image, the tag ends inside the image and no candle reaches
        // the plot's left edge (pixel 90).
        let (w, h) = (800, 400);
        let data = crate::data::sample_data(40);
        let mut buf = vec![0u8; (w * h * 3) as usize];
        {
            let root = BitMapBackend::with_buffer(&mut buf, (w, h)).into_drawing_area();
            draw_chart(&root, &data, &[], None, &[]).unwrap();
            root.present().unwrap();
        }
        let candle = |i: usize| {
            let p = &buf[i * 3..][..3];
            p == [BULL.0, BULL.1, BULL.2] || p == [BEAR.0, BEAR.1, BEAR.2]
        };
        let cols: Vec<u32> = (0..(w * h) as usize)
            .filter(|i| candle(*i))
            .map(|i| i as u32 % w)
            .collect();
        assert!(
            cols.iter().all(|x| *x > 90 && *x < w - 2),
            "{:?}",
            (cols.iter().min(), cols.iter().max())
        );
        assert!(cols.iter().any(|x| *x > w - 40), "the tag is in the gutter");
    }
}
