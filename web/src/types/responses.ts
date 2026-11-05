// Hand-written Zod schemas for chartgen MCP tool RESPONSE payloads.
//
// Why hand-written: chartgen's `tools/list` schemas cover inputs only —
// the MCP spec does not yet require a formal `outputSchema`, so the wire
// shape for responses is documented in prose in
// `website/src/content/docs/reference/mcp.md` and produced by Rust
// serializers (`panel_result_to_series_json`, `panel_result_to_json`,
// `tool_get_indicators`, `tool_list_indicators`). This file is part of
// the wire contract: any change to those Rust functions ships in the
// same PR as the matching schema change here. See ADR-0004.
//
// Dependency note: this module imports `zod` at a source level. PR 3
// (the Solid frontend) adds it as a proper `package.json` dependency.
// There is no `web/package.json` yet, by design — this PR only lands
// the type-generation pipeline, not the frontend itself.

import { z } from "zod";

// A JSON number that the Rust side encodes as `null` when the underlying
// f64 is NaN or infinite (`finite_or_null` in `src/mcp.rs`).
const FiniteOrNull = z.number().nullable();

// ---------------------------------------------------------------------------
// generate_chart — format=series
//   Produced by `panel_result_to_series_json` + the `tool_generate_chart`
//   series branch. One indicator entry per requested indicator (key =
//   request name). Every `values[]` array is 1:1 aligned with `bars[]`.
// ---------------------------------------------------------------------------

export const SeriesBarSchema = z.object({
  t: z.number().nullable(), // unix seconds; null for synthetic sample_data
  o: FiniteOrNull,
  h: FiniteOrNull,
  l: FiniteOrNull,
  c: FiniteOrNull,
  v: FiniteOrNull,
});

// Plot colors (ADR-0026) are the PNG renderer's CSS "rgba(r,g,b,a)"
// strings; optional so payloads from older servers still parse.
export const SeriesLineSchema = z.object({
  index: z.number().int(),
  label: z.string().nullable(),
  values: z.array(FiniteOrNull),
  color: z.string().optional(),
  width: z.number().int().optional(),
  // Only on lines that break at trend flips (Supertrend, SAR): their legend
  // at rest reads the last bar, not the last finite value.
  breaks: z.boolean().optional(),
});

export const SeriesHistogramBucketSchema = z.object({
  index: z.number().int(),
  // Legend label, e.g. "Volume" or "Histogram".
  label: z.string().optional(),
  values: z.array(FiniteOrNull),
  // Per-bar colors: `color_idx[i]` indexes `palette` (each distinct color
  // once), 1:1 aligned with `values`; null where the bar has no color.
  palette: z.array(z.string()).optional(),
  color_idx: z.array(z.number().int().nullable()).optional(),
});

export const SeriesSignalSchema = z.object({
  x: z.number().int(),
  t: z.number().nullable(),
  y: z.number(),
  label: z.string(),
  color: z.string().optional(),
});

// `color` is a CSS "rgba(r,g,b,a)" string from the server's renderer
// (ADR-0012); optional so payloads from older servers still parse.
export const HLineSchema = z.object({
  y: z.number(),
  color: z.string().optional(),
});

export const SeriesFillSchema = z.object({
  y1: z.array(FiniteOrNull),
  y2: z.array(FiniteOrNull),
  color: z.string().optional(),
  // Used where y1 < y2 (Ichimoku's red cloud); `color` applies elsewhere.
  color_below: z.string().optional(),
});

export const SeriesHBarSchema = z.object({
  y: z.number(),
  height: z.number(),
  width: z.number(),
  offset: z.number(),
  left: z.boolean(),
  color: z.string().optional(),
});

export const SeriesDivLineSchema = z.object({
  x1: z.number().int(),
  t1: z.number().int().nullable(),
  y1: z.number(),
  x2: z.number().int(),
  t2: z.number().int().nullable(),
  y2: z.number(),
  dashed: z.boolean(),
  color: z.string().optional(),
});

export const YRangeSchema = z.tuple([FiniteOrNull, FiniteOrNull]);

export const SeriesIndicatorSchema = z.object({
  label: z.string(),
  is_overlay: z.boolean(),
  y_range: YRangeSchema.optional(),
  lines: z.array(SeriesLineSchema).optional(),
  histogram: z.array(SeriesHistogramBucketSchema).optional(),
  signals: z.array(SeriesSignalSchema).optional(),
  hlines: z.array(HLineSchema).optional(),
  fills: z.array(SeriesFillSchema).optional(),
  hbars: z.array(SeriesHBarSchema).optional(),
  divlines: z.array(SeriesDivLineSchema).optional(),
});

// Merging contract (ADR-0009): volume/time profiles (vpvr, hvn_lvn, tpo)
// are computed over the bars of *this* response. Keep `hbars` and the
// profile-derived `hlines` (POC/VAH/VAL, HVN/LVN nodes) from the initial
// full load; ignore them on tail-poll (`bars=2`) and history-page
// (`end_time`) responses, which only cover a slice of the chart.
// A failed user-script indicator (ADR-0025): `ScriptError::to_json`.
// `step` is the pipeline stage ("runtime" on a chart), `line` 1-based.
export const IndicatorErrorSchema = z.object({
  step: z.string(),
  line: z.number().int().nullable(),
  message: z.string(),
});
export type IndicatorError = z.infer<typeof IndicatorErrorSchema>;

// Instance key → error, for indicators left out of `indicators` because
// they failed. Absent when every indicator computed (always, for
// built-ins); servers that predate user scripts never send it.
export const IndicatorErrorsSchema = z
  .record(z.string(), IndicatorErrorSchema)
  .optional();

export const SeriesPayloadSchema = z.object({
  symbol: z.string().nullable(),
  interval: z.string().nullable(),
  // Price style the server applied to `bars` (and computed indicators on).
  // Absent from servers that predate it: raw candles.
  chart_style: z.enum(["candles", "heikin_ashi"]).optional(),
  bars: z.array(SeriesBarSchema),
  // Keyed by instance (ADR-0028): the entry's `id`, else its name; a
  // repeat of a name without an id is `name#2`, `name#3`...
  indicators: z.record(z.string(), SeriesIndicatorSchema),
  indicator_errors: IndicatorErrorsSchema,
  // True when older bars may exist before `bars[0]`: request them with
  // `end_time = bars[0].t * 1000`. Binance/sample: the full
  // `bars + warmup` count came back. Yahoo: the window was not cut by
  // Yahoo's lookback limit or the symbol's first trade date. Always false
  // for the no-ticker demo data (no timestamps) and exhausted history.
  has_more: z.boolean(),
});
export type SeriesPayload = z.infer<typeof SeriesPayloadSchema>;

// ---------------------------------------------------------------------------
// generate_chart — format=summary (also embedded in format=both)
//   Produced by `panel_result_to_json` + the summary branch of
//   `tool_generate_chart` + `ohlcv_summary`.
// ---------------------------------------------------------------------------

export const OhlcvSummaryFirstSchema = z.object({
  date: z.string(),
  open: z.number(),
  close: z.number(),
});

export const OhlcvSummaryLastSchema = z.object({
  date: z.string(),
  open: z.number(),
  high: z.number(),
  low: z.number(),
  close: z.number(),
  volume: z.number(),
});

export const OhlcvSummaryRangeSchema = z.object({
  high: z.number(),
  low: z.number(),
});

export const OhlcvSummarySchema = z.union([
  z.object({ bars: z.literal(0) }),
  z.object({
    bars: z.number().int(),
    first: OhlcvSummaryFirstSchema,
    last: OhlcvSummaryLastSchema,
    range: OhlcvSummaryRangeSchema,
    total_volume: z.number(),
    change_pct: z.number().nullable(),
  }),
]);

export const SummaryLineSchema = z.object({
  index: z.number().int(),
  label: z.string().nullable(),
  last_value: FiniteOrNull,
});

export const SummarySignalSchema = z.object({
  x: z.number().int(),
  y: z.number(),
  label: z.string(),
});

export const SummaryHistogramBucketSchema = z.object({
  index: z.number().int(),
  last_value: FiniteOrNull,
});

export const HBarSchema = z.object({
  y: z.number(),
  weight: z.number(),
});

export const DivergenceLineSchema = z.object({
  x1: z.number().int(),
  y1: z.number(),
  x2: z.number().int(),
  y2: z.number(),
  dashed: z.boolean(),
});

export const SummaryIndicatorSchema = z.object({
  label: z.string(),
  is_overlay: z.boolean(),
  y_range: YRangeSchema.optional(),
  lines: z.array(SummaryLineSchema).optional(),
  signals: z.array(SummarySignalSchema).optional(),
  decorative_dots: z.number().int().optional(),
  hlines: z.array(HLineSchema).optional(),
  histogram: z.array(SummaryHistogramBucketSchema).optional(),
  fills_count: z.number().int().optional(),
  hbars_top: z.array(HBarSchema).optional(),
  hbars_count: z.number().int().optional(),
  divergences: z.array(DivergenceLineSchema).optional(),
});

export const SummaryPayloadSchema = z.object({
  symbol: z.string().nullable(),
  interval: z.string().nullable(),
  price: OhlcvSummarySchema,
  // Same instance keys as the series payload (ADR-0028).
  indicators: z.record(z.string(), SummaryIndicatorSchema),
  indicator_errors: IndicatorErrorsSchema,
});
export type SummaryPayload = z.infer<typeof SummaryPayloadSchema>;

// ---------------------------------------------------------------------------
// list_indicators — `tool_list_indicators` return shape.
// ---------------------------------------------------------------------------

// One entry of `configurable_params` (`Indicator::params` in src/indicators/).
// `type` is "integer" | "number" | "boolean" | "string"; `default` may be null
// for optional params. Kept lenient so an unknown type never fails the parse.
export const ParamSpecSchema = z.object({
  name: z.string(),
  type: z.string(),
  default: z.unknown().optional(),
  description: z.string().optional(),
  min: z.number().optional(),
  max: z.number().optional(),
  enum: z.array(z.string()).optional(),
});
export type ParamSpec = z.infer<typeof ParamSpecSchema>;

export const IndicatorInfoSchema = z.object({
  name: z.string(),
  description: z.string(),
  type: z.string(), // category: "overlay" | "panel" | "external"
  overlay: z.boolean(),
  aliases: z.array(z.string()).optional(),
  configurable_params: z.array(ParamSpecSchema).optional(),
  // "user" for the caller's own scripted indicators (ADR-0025); absent
  // for built-ins.
  source: z.literal("user").optional(),
});

export const ListIndicatorsPayloadSchema = z.object({
  total: z.number().int(),
  indicators: z.array(IndicatorInfoSchema),
});
export type ListIndicatorsPayload = z.infer<typeof ListIndicatorsPayloadSchema>;

// ---------------------------------------------------------------------------
// get_indicators — `tool_get_indicators` return shape.
//   Per-indicator values are the same `panel_result_to_json` shape used
//   by the summary payload; the top-level envelope also reports the data
//   source ("engine_cache" or "fetched") and, when fetched, a bar count.
// ---------------------------------------------------------------------------

export const GetIndicatorsPayloadSchema = z.object({
  symbol: z.string(),
  interval: z.string(),
  source: z.enum(["engine_cache", "fetched"]),
  bars: z.number().int().optional(),
  indicators: z.record(
    z.string(),
    z.union([SummaryIndicatorSchema, z.object({ error: z.string() })]),
  ),
});
export type GetIndicatorsPayload = z.infer<typeof GetIndicatorsPayloadSchema>;

// ---------------------------------------------------------------------------
// get_quotes — `get_quotes` in `src/mcp.rs` (ADR-0011).
//   One entry per distinct requested symbol, in request order: a quote,
//   or `{symbol, error}` when that symbol failed. Numbers go through
//   `finite_or_null`; high/low/volume are null when upstream omits them.
// ---------------------------------------------------------------------------

export const QuoteSchema = z.object({
  symbol: z.string(),
  last: FiniteOrNull,
  change: FiniteOrNull,
  change_pct: FiniteOrNull,
  prev_close: FiniteOrNull,
  high: FiniteOrNull,
  low: FiniteOrNull,
  volume: FiniteOrNull,
  source: z.enum(["binance", "yahoo", "sample"]),
});
export type Quote = z.infer<typeof QuoteSchema>;

export const QuoteErrorSchema = z.object({
  symbol: z.string(),
  error: z.string(),
});
export type QuoteError = z.infer<typeof QuoteErrorSchema>;

export const QuotesPayloadSchema = z.object({
  quotes: z.array(z.union([QuoteSchema, QuoteErrorSchema])),
});
export type QuotesPayload = z.infer<typeof QuotesPayloadSchema>;

// ---------------------------------------------------------------------------
// search_symbols — `search_symbols` in `src/mcp.rs` / `src/search.rs`.
//   Ranked results (exact symbol, prefix, contains, name match). `warnings`
//   is present only when one source failed and its results are missing.
// ---------------------------------------------------------------------------

export const SEARCH_RESULT_TYPES = [
  "crypto",
  "stock",
  "etf",
  "fund",
  "index",
  "forex",
  "future",
] as const;

export const SearchResultSchema = z.object({
  // The charset every chartgen symbol uses (`normalize_quote_symbol`).
  symbol: z.string().regex(/^[A-Z0-9.^=\-_]{1,32}$/),
  name: z.string(),
  exchange: z.string(),
  type: z.enum(SEARCH_RESULT_TYPES),
  source: z.enum(["binance", "yahoo", "sample"]),
});
export type SearchResult = z.infer<typeof SearchResultSchema>;

export const SearchSymbolsPayloadSchema = z.object({
  results: z.array(SearchResultSchema),
  warnings: z.array(z.string()).optional(),
});
export type SearchSymbolsPayload = z.infer<typeof SearchSymbolsPayloadSchema>;

// ---------------------------------------------------------------------------
// subscribe_chart / unsubscribe_chart — `tool_subscribe_chart` and
//   `tool_unsubscribe_chart` in `src/mcp.rs`; pushes come from
//   `src/chart_push.rs` (ADR-0021).
// ---------------------------------------------------------------------------

export const SubscribeChartPayloadSchema = z.object({
  subscription_id: z.string(),
});
export type SubscribeChartPayload = z.infer<typeof SubscribeChartPayloadSchema>;

export const UnsubscribeChartPayloadSchema = z.object({
  // False when the subscription was already gone (e.g. its SSE stream closed).
  unsubscribed: z.boolean(),
});
export type UnsubscribeChartPayload = z.infer<typeof UnsubscribeChartPayloadSchema>;

/** Method of a pushed chart tail on the SSE stream. */
export const CHART_TICK_METHOD = "notifications/chart_tick";

// `params` of `notifications/chart_tick`: `series` is exactly the
// generate_chart(format=series, bars=2, warmup) tail of the subscribed chart.
export const ChartTickParamsSchema = z.object({
  subscription_id: z.string(),
  series: SeriesPayloadSchema,
});
export type ChartTickParams = z.infer<typeof ChartTickParamsSchema>;
