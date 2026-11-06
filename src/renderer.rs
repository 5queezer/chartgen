use crate::data::OhlcvData;
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

/// `render_chart` for indicator results computed by the caller (so each
/// indicator runs once, and a failed one can be left out).
pub fn render_results(
    data: &OhlcvData,
    results: &[&PanelResult],
    output: &str,
    width: u32,
    height: u32,
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
        let sym = data.symbol.as_deref().unwrap_or("Sample");
        let ivl = data.interval.as_deref().unwrap_or("");
        let title = if ivl.is_empty() {
            sym.to_string()
        } else {
            format!("{} · {}", sym, ivl.to_uppercase())
        };
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
            .build_cartesian_2d(0_f64..(n as f64), (p_min - p_margin)..(p_max + p_margin))?;

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
                for run in line_runs(&line.y) {
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

            // Pattern outlines (ADR-0044) draw the same way as segments.
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
    }

    // ---- Draw indicator panels ----
    for (area, result) in areas.iter().zip(&panel_results) {
        let (y_lo, y_hi) = result.y_range.unwrap_or_else(|| auto_range(result));

        let mut chart = ChartBuilder::on(area)
            .margin(5)
            .x_label_area_size(20)
            .y_label_area_size(80)
            .build_cartesian_2d(0_f64..(n as f64), y_lo..y_hi)?;

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
            for run in line_runs(&line.y) {
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
