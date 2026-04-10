/// Multi-timeframe aggregation utilities.
///
/// Aggregate lower-timeframe bars into higher-timeframe bars and map
/// higher-TF indicator values back to the original bar grid.
use crate::data::{Bar, OhlcvData};

/// Aggregate lower-timeframe bars into higher-timeframe bars.
///
/// Each bar's `date` field is parsed as epoch seconds (i64). Bars are grouped
/// by period boundary (`time / period_secs * period_secs`).
///
/// Returns `(aggregated_data, mapping)` where `mapping[agg_idx]` contains the
/// original bar indices that were merged into that aggregated bar. Returns
/// `None` if `target_tf` is unrecognised or no bars can be parsed.
pub fn aggregate_bars(data: &OhlcvData, target_tf: &str) -> Option<(OhlcvData, Vec<Vec<usize>>)> {
    let period_secs = tf_to_seconds(target_tf)?;

    if data.bars.is_empty() {
        return Some((
            OhlcvData {
                bars: vec![],
                symbol: data.symbol.clone(),
                interval: Some(target_tf.to_string()),
            },
            vec![],
        ));
    }

    let mut agg_bars: Vec<Bar> = Vec::new();
    let mut mapping: Vec<Vec<usize>> = Vec::new();

    let mut current_boundary: Option<i64> = None;
    let mut group_indices: Vec<usize> = Vec::new();

    for (i, bar) in data.bars.iter().enumerate() {
        let ts: i64 = bar.date.parse().ok()?;
        let boundary = ts / period_secs * period_secs;

        if current_boundary == Some(boundary) {
            group_indices.push(i);
        } else {
            // Flush previous group.
            if !group_indices.is_empty() {
                let agg = build_agg_bar(&data.bars, &group_indices);
                agg_bars.push(agg);
                mapping.push(std::mem::take(&mut group_indices));
            }
            current_boundary = Some(boundary);
            group_indices.push(i);
        }
    }

    // Flush final group.
    if !group_indices.is_empty() {
        let agg = build_agg_bar(&data.bars, &group_indices);
        agg_bars.push(agg);
        mapping.push(group_indices);
    }

    Some((
        OhlcvData {
            bars: agg_bars,
            symbol: data.symbol.clone(),
            interval: Some(target_tf.to_string()),
        },
        mapping,
    ))
}

/// Map higher-TF indicator values back to lower-TF bars.
///
/// Each lower-TF bar receives the value of its parent higher-TF bar. Indices
/// not covered by the mapping are set to `NaN`.
pub fn map_to_lower_tf(
    htf_values: &[f64],
    mapping: &[Vec<usize>],
    original_len: usize,
) -> Vec<f64> {
    let mut out = vec![f64::NAN; original_len];
    for (agg_idx, indices) in mapping.iter().enumerate() {
        if let Some(&val) = htf_values.get(agg_idx) {
            for &orig_idx in indices {
                if orig_idx < original_len {
                    out[orig_idx] = val;
                }
            }
        }
    }
    out
}

fn build_agg_bar(bars: &[Bar], indices: &[usize]) -> Bar {
    let first = &bars[indices[0]];
    let last = &bars[*indices.last().unwrap()];
    let high = indices
        .iter()
        .map(|&i| bars[i].high)
        .fold(f64::NEG_INFINITY, f64::max);
    let low = indices
        .iter()
        .map(|&i| bars[i].low)
        .fold(f64::INFINITY, f64::min);
    let volume: f64 = indices.iter().map(|&i| bars[i].volume).sum();

    Bar {
        open: first.open,
        high,
        low,
        close: last.close,
        volume,
        date: first.date.clone(),
    }
}

fn tf_to_seconds(tf: &str) -> Option<i64> {
    match tf {
        "1m" => Some(60),
        "5m" => Some(300),
        "15m" => Some(900),
        "30m" => Some(1800),
        "1h" => Some(3600),
        "4h" => Some(14_400),
        "1d" => Some(86_400),
        "1wk" => Some(604_800),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aggregate_1h_to_4h() {
        let bars: Vec<Bar> = (0..8)
            .map(|i| Bar {
                date: format!("{}", 1_700_000_000 + i * 3600),
                open: 100.0 + i as f64,
                high: 105.0 + i as f64,
                low: 95.0 + i as f64,
                close: 102.0 + i as f64,
                volume: 1000.0,
            })
            .collect();
        let data = OhlcvData {
            bars,
            interval: Some("1h".into()),
            symbol: Some("TEST".into()),
        };
        let (agg, mapping) = aggregate_bars(&data, "4h").unwrap();

        // Should produce 2 or 3 4h bars depending on alignment.
        assert!(agg.bars.len() >= 2);
        // Each mapping group should have at most 4 bars and be non-empty.
        for group in &mapping {
            assert!(group.len() <= 4);
            assert!(!group.is_empty());
        }
        // Total original indices covered must equal 8.
        let total: usize = mapping.iter().map(|g| g.len()).sum();
        assert_eq!(total, 8);
        // Aggregated bar OHLCV sanity: high >= open, low <= open.
        for bar in &agg.bars {
            assert!(bar.high >= bar.open);
            assert!(bar.low <= bar.close);
        }
    }

    #[test]
    fn test_aggregate_preserves_ohlcv() {
        // Two bars that should land in the same 1h bucket.
        let bars = vec![
            Bar {
                date: "1700000000".into(),
                open: 100.0,
                high: 110.0,
                low: 90.0,
                close: 105.0,
                volume: 500.0,
            },
            Bar {
                date: "1700000060".into(), // +60s, same hour
                open: 105.0,
                high: 115.0,
                low: 92.0,
                close: 108.0,
                volume: 700.0,
            },
        ];
        let data = OhlcvData {
            bars,
            interval: Some("1m".into()),
            symbol: None,
        };
        let (agg, mapping) = aggregate_bars(&data, "1h").unwrap();
        assert_eq!(agg.bars.len(), 1);
        assert_eq!(mapping.len(), 1);
        assert_eq!(mapping[0], vec![0, 1]);

        let b = &agg.bars[0];
        assert!((b.open - 100.0).abs() < f64::EPSILON);
        assert!((b.high - 115.0).abs() < f64::EPSILON);
        assert!((b.low - 90.0).abs() < f64::EPSILON);
        assert!((b.close - 108.0).abs() < f64::EPSILON);
        assert!((b.volume - 1200.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_aggregate_empty() {
        let data = OhlcvData {
            bars: vec![],
            interval: Some("1m".into()),
            symbol: None,
        };
        let (agg, mapping) = aggregate_bars(&data, "1h").unwrap();
        assert!(agg.bars.is_empty());
        assert!(mapping.is_empty());
    }

    #[test]
    fn test_aggregate_unknown_tf() {
        let data = OhlcvData {
            bars: vec![],
            interval: None,
            symbol: None,
        };
        assert!(aggregate_bars(&data, "3h").is_none());
    }

    #[test]
    fn test_map_to_lower_tf() {
        let mapping = vec![vec![0, 1, 2, 3], vec![4, 5, 6, 7]];
        let htf_values = vec![50.0, 60.0];
        let result = map_to_lower_tf(&htf_values, &mapping, 8);
        assert_eq!(result[0], 50.0);
        assert_eq!(result[3], 50.0);
        assert_eq!(result[4], 60.0);
        assert_eq!(result[7], 60.0);
    }

    #[test]
    fn test_map_to_lower_tf_nan_gaps() {
        // Mapping doesn't cover index 2.
        let mapping = vec![vec![0, 1], vec![3, 4]];
        let htf_values = vec![10.0, 20.0];
        let result = map_to_lower_tf(&htf_values, &mapping, 5);
        assert_eq!(result[0], 10.0);
        assert_eq!(result[1], 10.0);
        assert!(result[2].is_nan());
        assert_eq!(result[3], 20.0);
        assert_eq!(result[4], 20.0);
    }

    #[test]
    fn test_tf_to_seconds() {
        assert_eq!(tf_to_seconds("1m"), Some(60));
        assert_eq!(tf_to_seconds("5m"), Some(300));
        assert_eq!(tf_to_seconds("15m"), Some(900));
        assert_eq!(tf_to_seconds("30m"), Some(1800));
        assert_eq!(tf_to_seconds("1h"), Some(3600));
        assert_eq!(tf_to_seconds("4h"), Some(14_400));
        assert_eq!(tf_to_seconds("1d"), Some(86_400));
        assert_eq!(tf_to_seconds("1wk"), Some(604_800));
        assert_eq!(tf_to_seconds("banana"), None);
    }
}
