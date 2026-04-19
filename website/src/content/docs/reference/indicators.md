---
title: Indicators
description: All 33 indicators, grouped by category.
---

33 indicators across 5 categories. All names and aliases are accepted by the
CLI (`-p <name>`) and the `generate_chart` MCP tool.

## Overlays (12)

Drawn on the price chart.

| Name | Aliases |
|------|---------|
| `ema_stack` | `ema` |
| `bbands` | `bb`, `bollinger` |
| `keltner` | `kc` |
| `donchian` | `dc` |
| `supertrend` | `st` |
| `sar` | `psar` |
| `ichimoku` | `ichi` |
| `heikin_ashi` | `ha` |
| `vwap` | |
| `vwap_bands` | |
| `pivot` | `pivot_points` |
| `vpvr` | `vp`, `volume_profile` |

## Momentum (9)

| Name | Aliases |
|------|---------|
| `rsi` | |
| `macd` | |
| `stoch` | `stochastic` |
| `cci` | |
| `williams_r` | `willr` |
| `roc` | |
| `wavetrend` | `wt` |
| `cipher_b` | `cipher`, `cb` |
| `adx` | |

## Volatility (3) · Volume (5) · Crypto (4)

| Name | Aliases | Category |
|------|---------|----------|
| `atr` | | Volatility |
| `histvol` | `hv` | Volatility |
| `kalman_volume` | `kv` | Volatility |
| `obv` | | Volume |
| `mfi` | | Volume |
| `cmf` | | Volume |
| `ad` | `adline` | Volume |
| `cvd` | | Volume |
| `funding` | `funding_rate` | Crypto |
| `oi` | `open_interest` | Crypto |
| `long_short` | `ls_ratio` | Crypto |
| `fear_greed` | `fng` | Crypto |

## Parameter examples (MCP)

```json
{"name": "rsi", "length": 21}
{"name": "bbands", "length": 30, "mult": 2.5}
{"name": "vpvr", "bins": 32, "side": "right"}
```

Use the `list_indicators` MCP tool to discover the exact parameters each
indicator accepts.
