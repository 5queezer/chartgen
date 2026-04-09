use crate::data::OhlcvData;
use crate::indicator::{Indicator, PanelResult};
use plotters::prelude::*;

const BG: RGBColor = RGBColor(19, 23, 34);
const GRID: RGBAColor = RGBAColor(42, 46, 57, 0.5);
const TEXT: RGBColor = RGBColor(120, 123, 134);

pub fn render_chart(
    data: &OhlcvData,
    panels: &[Box<dyn Indicator>],
    output: &str,
    width: u32,
    height: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let n = data.len();

    // Compute all indicators
    let results: Vec<PanelResult> = panels.iter().map(|p| p.compute(data)).collect();

    // Separate overlays from panels
    let overlays: Vec<&PanelResult> = results.iter().filter(|r| r.is_overlay).collect();
    let panel_results: Vec<&PanelResult> = results.iter().filter(|r| !r.is_overlay).collect();
    let n_panels = panel_results.len();

    // Price range
    let p_min = data.lows().iter().cloned().fold(f64::INFINITY, f64::min);
    let p_max = data
        .highs()
        .iter()
        .cloned()
        .fold(f64::NEG_INFINITY, f64::max);
    let p_margin = (p_max - p_min) * 0.05;

    // Layout: candle panel takes 55%, rest shared equally
    let candle_h = (height as f64 * 0.55) as u32;
    let panel_h = if n_panels > 0 {
        (height - candle_h) / n_panels as u32
    } else {
        0
    };

    let root = BitMapBackend::new(output, (width, height)).into_drawing_area();
    root.fill(&BG)?;

    // Split vertically
    let mut areas = Vec::new();
    let mut y_start = 0;

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
            .y_label_area_size(60)
            .build_cartesian_2d(0_f64..(n as f64), (p_min - p_margin)..(p_max + p_margin))?;

        chart
            .configure_mesh()
            .disable_x_mesh()
            .light_line_style(GRID)
            .bold_line_style(GRID)
            .axis_style(GRID)
            .label_style(("monospace", 12, &TEXT))
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
                            fill.color.filled(),
                        )
                    })
                    .collect();
                chart.draw_series(pairs)?;
            }

            // Lines
            for line in &overlay.lines {
                let points: Vec<_> = line
                    .y
                    .iter()
                    .enumerate()
                    .filter(|(_, y)| !y.is_nan())
                    .map(|(i, y)| (i as f64, *y))
                    .collect();
                if !points.is_empty() {
                    chart.draw_series(std::iter::once(PathElement::new(
                        points,
                        line.color.stroke_width(line.width),
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
        }

        // Price label
        let lp = data.bars.last().unwrap().close;
        chart.draw_series(std::iter::once(Text::new(
            format!("{:.2}", lp),
            ((n - 1) as f64, lp),
            ("monospace", 12).into_font().color(&WHITE),
        )))?;
    }

    // ---- Draw indicator panels ----
    for (area, result) in areas.iter().zip(&panel_results) {
        let (y_lo, y_hi) = result.y_range.unwrap_or_else(|| auto_range(result));

        let mut chart = ChartBuilder::on(area)
            .margin(5)
            .x_label_area_size(20)
            .y_label_area_size(60)
            .build_cartesian_2d(0_f64..(n as f64), y_lo..y_hi)?;

        chart
            .configure_mesh()
            .disable_x_mesh()
            .light_line_style(GRID)
            .bold_line_style(GRID)
            .axis_style(GRID)
            .label_style(("monospace", 10, &TEXT))
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
                        fill.color.filled(),
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

        // Lines
        for line in &result.lines {
            let points: Vec<_> = line
                .y
                .iter()
                .enumerate()
                .filter(|(_, y)| !y.is_nan())
                .map(|(i, y)| (i as f64, *y))
                .collect();
            if !points.is_empty() {
                chart.draw_series(std::iter::once(PathElement::new(
                    points,
                    line.color.stroke_width(line.width),
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

        // Label
        if !result.label.is_empty() {
            area.draw_text(
                &result.label,
                &("monospace", 12).into_font().color(&TEXT),
                (width as i32 - 80, 5),
            )?;
        }
    }

    root.present()?;
    Ok(())
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
    let margin = (hi - lo).max(1.0) * 0.1;
    (lo - margin, hi + margin)
}
