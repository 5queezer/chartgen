# Graph Report - .  (2026-04-10)

## Corpus Check
- Corpus is ~27,254 words - fits in a single context window. You may not need a graph.

## Summary
- 374 nodes · 464 edges · 31 communities detected
- Extraction: 98% EXTRACTED · 2% INFERRED · 0% AMBIGUOUS · INFERRED: 7 edges (avg confidence: 0.82)
- Token cost: 0 input · 0 output

## God Nodes (most connected - your core abstractions)
1. `Macd` - 7 edges
2. `WaveTrend` - 7 edges
3. `Rsi` - 7 edges
4. `make_data_item()` - 7 edges
5. `BollingerBandsInd` - 7 edges
6. `KeltnerChannels` - 7 edges
7. `Atr` - 7 edges
8. `Cci` - 7 edges
9. `Roc` - 7 edges
10. `Mfi` - 7 edges

## Surprising Connections (you probably didn't know these)
- None detected - all connections are within the same source files.

## Hyperedges (group relationships)
- **Graphify Workflow Rules** — claudemd_graph_report, claudemd_wiki_index, claudemd_rebuild_code_command [EXTRACTED 1.00]
- **Multi-Panel Technical Analysis Chart** — chart_ohlc_price_data, chart_oscillator_panel_1, chart_bollinger_band_indicator, chart_momentum_histogram, chart_volume_panel [EXTRACTED 1.00]

## Communities

### Community 0 - "TA-RS Bridge Indicators"
Cohesion: 0.04
Nodes (11): Atr, Cci, Cmf, Donchian, EmaStack, KeltnerChannels, make_data_item(), Mfi (+3 more)

### Community 1 - "Custom Indicators"
Cohesion: 0.04
Nodes (14): AdLine, HeikinAshi, HistVol, hma(), Ichimoku, KalmanVolume, make_data_item(), PivotPoints (+6 more)

### Community 2 - "E2E Test Suite"
Cohesion: 0.06
Nodes (4): assert_panel_valid(), test_all_indicators_1_bar(), test_all_indicators_200_bars(), test_all_indicators_5_bars()

### Community 3 - "External Data Indicators"
Cohesion: 0.11
Nodes (15): align_to_bars(), Cvd, FearGreed, fetch_fear_greed_data(), fetch_funding_data(), fetch_ls_data(), fetch_oi_data(), fng_color() (+7 more)

### Community 4 - "OAuth & HTTP Server"
Cohesion: 0.13
Nodes (18): AccessToken, AuthCode, AuthorizeQuery, base_url(), extract_bearer_token(), form_to_json(), mcp_handler(), oauth_authorize() (+10 more)

### Community 5 - "OHLCV Data Model"
Cohesion: 0.12
Nodes (9): Bar, OhlcvData, Rng, sample_data(), Cli, auto_range(), format_bar_label(), format_epoch() (+1 more)

### Community 6 - "Indicator Trait & Helpers"
Cohesion: 0.11
Nodes (14): Bars, Dot, Fill, HBar, highest(), HLine, Indicator, Line (+6 more)

### Community 7 - "Data Fetching & MCP Protocol"
Cohesion: 0.22
Nodes (11): aggregate_bars(), fetch_yahoo(), yahoo_interval_range(), handle_initialize(), handle_mcp_request(), handle_tools_call(), handle_tools_list(), parse_panels() (+3 more)

### Community 8 - "Cipher B Composite"
Cohesion: 0.27
Nodes (5): CipherB, CipherBConfig, compute_schaff(), DivResult, find_divs()

### Community 9 - "RSI Indicator"
Cohesion: 0.28
Nodes (2): compute_rsi(), Rsi

### Community 10 - "MACD Indicator"
Cohesion: 0.29
Nodes (1): Macd

### Community 11 - "WaveTrend Indicator"
Cohesion: 0.29
Nodes (1): WaveTrend

### Community 12 - "Graphify Config"
Cohesion: 0.25
Nodes (8): GRAPH_REPORT.md (God Nodes & Community Structure), graphify-out/ Output Directory, Graphify Knowledge Graph System, Rationale: Read GRAPH_REPORT.md Before Answering Architecture Questions, Rationale: Rebuild Graph After Code Modifications to Keep It Current, Rationale: Prefer Wiki Index Over Raw Files, graphify.watch._rebuild_code Graph Rebuild Command, graphify-out/wiki/index.md Navigation Entry

### Community 13 - "Bollinger Bands"
Cohesion: 0.29
Nodes (1): BollingerBandsInd

### Community 14 - "Williams %R"
Cohesion: 0.29
Nodes (1): WilliamsR

### Community 15 - "Parabolic SAR"
Cohesion: 0.29
Nodes (1): ParabolicSar

### Community 16 - "ADX Indicator"
Cohesion: 0.29
Nodes (1): Adx

### Community 17 - "Chart Visualization (Image)"
Cohesion: 0.48
Nodes (7): Bollinger Band / Envelope Indicator Panel, Momentum / MACD Histogram Panel, OHLC Price Data (Candlestick Panel), Oscillator Indicator Panel 1 (Multi-Line, Likely Stochastic or MACD-Variant), Trading Candlestick Chart with Technical Indicators, Trend: Price Rise Followed by Sharp Decline, Volume / Activity Bar Panel

### Community 18 - "Graph Report Cross-Refs"
Cohesion: 0.33
Nodes (6): Community 0: TA-RS Bridge Indicators, Community 1: Custom Indicators, Community 7: Data Fetching & MCP Protocol, Macd (God Node, 7 edges), make_data_item() (God Node, 7 edges), Rationale: Adx as Cross-Community Bridge

### Community 19 - "Cipher B / RSI Link"
Cohesion: 1.0
Nodes (2): Community 8: Cipher B Composite, Rsi (God Node, 7 edges)

### Community 20 - "Library Entrypoint"
Cohesion: 1.0
Nodes (0): 

### Community 21 - "Report Summary"
Cohesion: 1.0
Nodes (1): Graph Report Summary

### Community 22 - "Report God Nodes"
Cohesion: 1.0
Nodes (1): God Nodes (Most Connected Core Abstractions)

### Community 23 - "WaveTrend (God Node)"
Cohesion: 1.0
Nodes (1): WaveTrend (God Node, 7 edges)

### Community 24 - "Bollinger (God Node)"
Cohesion: 1.0
Nodes (1): BollingerBandsInd (God Node, 7 edges)

### Community 25 - "Keltner (God Node)"
Cohesion: 1.0
Nodes (1): KeltnerChannels (God Node, 7 edges)

### Community 26 - "ATR (God Node)"
Cohesion: 1.0
Nodes (1): Atr (God Node, 7 edges)

### Community 27 - "CCI (God Node)"
Cohesion: 1.0
Nodes (1): Cci (God Node, 7 edges)

### Community 28 - "ROC (God Node)"
Cohesion: 1.0
Nodes (1): Roc (God Node, 7 edges)

### Community 29 - "MFI (God Node)"
Cohesion: 1.0
Nodes (1): Mfi (God Node, 7 edges)

### Community 30 - "Knowledge Gaps"
Cohesion: 1.0
Nodes (1): Knowledge Gap: 21 Isolated Nodes

## Knowledge Gaps
- **37 isolated node(s):** `Cli`, `Line`, `Fill`, `Bars`, `Dot` (+32 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **Thin community `Cipher B / RSI Link`** (2 nodes): `Community 8: Cipher B Composite`, `Rsi (God Node, 7 edges)`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Library Entrypoint`** (1 nodes): `lib.rs`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Report Summary`** (1 nodes): `Graph Report Summary`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Report God Nodes`** (1 nodes): `God Nodes (Most Connected Core Abstractions)`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `WaveTrend (God Node)`** (1 nodes): `WaveTrend (God Node, 7 edges)`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Bollinger (God Node)`** (1 nodes): `BollingerBandsInd (God Node, 7 edges)`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Keltner (God Node)`** (1 nodes): `KeltnerChannels (God Node, 7 edges)`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `ATR (God Node)`** (1 nodes): `Atr (God Node, 7 edges)`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `CCI (God Node)`** (1 nodes): `Cci (God Node, 7 edges)`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `ROC (God Node)`** (1 nodes): `Roc (God Node, 7 edges)`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `MFI (God Node)`** (1 nodes): `Mfi (God Node, 7 edges)`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Knowledge Gaps`** (1 nodes): `Knowledge Gap: 21 Isolated Nodes`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Adx` connect `ADX Indicator` to `Custom Indicators`?**
  _High betweenness centrality (0.026) - this node is a cross-community bridge._
- **What connects `Cli`, `Line`, `Fill` to the rest of the system?**
  _37 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `TA-RS Bridge Indicators` be split into smaller, more focused modules?**
  _Cohesion score 0.04 - nodes in this community are weakly interconnected._
- **Should `Custom Indicators` be split into smaller, more focused modules?**
  _Cohesion score 0.04 - nodes in this community are weakly interconnected._
- **Should `E2E Test Suite` be split into smaller, more focused modules?**
  _Cohesion score 0.06 - nodes in this community are weakly interconnected._
- **Should `External Data Indicators` be split into smaller, more focused modules?**
  _Cohesion score 0.11 - nodes in this community are weakly interconnected._
- **Should `OAuth & HTTP Server` be split into smaller, more focused modules?**
  _Cohesion score 0.13 - nodes in this community are weakly interconnected._