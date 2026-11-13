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
  label: z.string().optional(),
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

// Pattern channels (ADR-0046). A label belongs to its bar. `tentative` marks one
// on the forming bar, which can change or vanish until the bar closes.
export const BiasSchema = z.enum(["bullish", "bearish", "neutral"]);

export const SeriesLabelSchema = z.object({
  x: z.number().int(),
  t: z.number().nullable(),
  y: z.number(),
  above: z.boolean(),
  text: z.string(),
  key: z.string(),
  name: z.string(),
  bias: BiasSchema,
  color: z.string().optional(),
  tooltip: z.string(),
  tentative: z.boolean().optional(),
});

export const SeriesStrokeSchema = z.object({
  x1: z.number().int(),
  t1: z.number().nullable(),
  y1: z.number(),
  x2: z.number().int(),
  t2: z.number().nullable(),
  y2: z.number(),
  dashed: z.boolean(),
});

// A chart pattern's bar open times in ms (`null` until it confirms or fails) and its
// height, the measured move's. Absent on other patterns and on bars without times.
const PatternTimesShape = {
  start_time: z.number().int().optional(),
  end_time: z.number().int().optional(),
  known_time: z.number().int().optional(),
  confirmed_time: z.number().int().nullable().optional(),
  failed_time: z.number().int().nullable().optional(),
  height: z.number().optional(),
};

// A chart pattern. The newest response holds all of them, whatever bars it covers:
// an index before its first bar is negative, and `t` still names the bar.
export const SeriesPatternSchema = z.object({
  key: z.string(),
  name: z.string(),
  bias: BiasSchema,
  status: z.enum(["forming", "confirmed", "failed"]),
  color: z.string().optional(),
  strokes: z.array(SeriesStrokeSchema),
  label: z.object({
    x: z.number().int(),
    t: z.number().nullable(),
    y: z.number(),
    above: z.boolean(),
  }),
  tooltip: z.string(),
  target: z.number().optional(),
  invalidation: z.number().optional(),
  confirmed_t: z.number().nullable().optional(),
  ...PatternTimesShape,
});

// `lines[line]` is flat at `y` from bar `x` on. The newest response's levels
// rewrite that plot on every bar: they move as a whole when the swing does.
export const SeriesLevelSchema = z.object({
  line: z.number().int(),
  x: z.number().int(),
  t: z.number().nullable(),
  y: z.number(),
});

// What an alert can wait for: `t` is the bar of the newest occurrence on a closed bar.
export const SeriesEventSchema = z.object({
  key: z.string(),
  name: z.string(),
  t: z.number().nullable(),
});

export const YRangeSchema = z.tuple([FiniteOrNull, FiniteOrNull]);

// Values plotted on the `bars` bars after the last one (ADR-0033), e.g. the
// Ichimoku cloud's leading spans. `index` names the line / fill they
// continue; `t` are the future bar times (absent without bar timestamps).
export const SeriesFutureSchema = z.object({
  bars: z.number().int().nonnegative(),
  t: z.array(z.number().int()).optional(),
  lines: z.array(
    z.object({ index: z.number().int(), values: z.array(FiniteOrNull) }),
  ),
  fills: z.array(
    z.object({
      index: z.number().int(),
      y1: z.array(FiniteOrNull),
      y2: z.array(FiniteOrNull),
    }),
  ),
});

export const SeriesIndicatorSchema = z.object({
  label: z.string(),
  is_overlay: z.boolean(),
  /** A profile read from these lower-timeframe bars rather than the chart's (ADR-0048). */
  profile_interval: z.string().optional(),
  /** Its lines hold levels: nothing is drawn across a change of value. */
  level_lines: z.boolean().optional(),
  steps: z.boolean().optional(),
  point_lines: z.boolean().optional(),
  line_breaks: z.array(z.number().int().nonnegative()).optional(),
  y_range: YRangeSchema.optional(),
  lines: z.array(SeriesLineSchema).optional(),
  histogram: z.array(SeriesHistogramBucketSchema).optional(),
  signals: z.array(SeriesSignalSchema).optional(),
  hlines: z.array(HLineSchema).optional(),
  fills: z.array(SeriesFillSchema).optional(),
  hbars: z.array(SeriesHBarSchema).optional(),
  divlines: z.array(SeriesDivLineSchema).optional(),
  labels: z.array(SeriesLabelSchema).optional(),
  patterns: z.array(SeriesPatternSchema).optional(),
  levels: z.array(SeriesLevelSchema).optional(),
  events: z.array(SeriesEventSchema).optional(),
  future: SeriesFutureSchema.optional(),
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
  chart_style: z
    .enum(["candles", "heikin_ashi", "renko", "kagi", "point_figure", "line_break", "range"])
    .optional(),
  // Inputs of a price-based style with the box its bars were built on (ADR-0047).
  chart_params: z
    .object({
      box: z.enum(["atr", "fixed", "percent"]).optional(),
      atr_length: z.number().optional(),
      percent: z.number().optional(),
      box_size: z.number().optional(),
      reversal: z.number().optional(),
      lines: z.number().optional(),
    })
    .optional(),
  // First source bar a price-based build started from (ADR-0047). A later one is
  // a new build, whose bars need not line up with this one's.
  anchor: z.number().nullable().optional(),
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

// The newest pattern labels (ADR-0046); `labels_count` has them all.
export const SummaryLabelSchema = z.object({
  x: z.number().int(),
  key: z.string(),
  name: z.string(),
  bias: BiasSchema,
  tentative: z.boolean().optional(),
});

export const SummaryPatternSchema = z.object({
  key: z.string(),
  name: z.string(),
  bias: BiasSchema,
  status: z.enum(["forming", "confirmed", "failed"]),
  tooltip: z.string(),
  target: z.number().optional(),
  invalidation: z.number().optional(),
  confirmed_x: z.number().int().optional(),
  ...PatternTimesShape,
});

export const SummaryIndicatorSchema = z.object({
  label: z.string(),
  is_overlay: z.boolean(),
  y_range: YRangeSchema.optional(),
  lines: z.array(SummaryLineSchema).optional(),
  signals: z.array(SummarySignalSchema).optional(),
  labels: z.array(SummaryLabelSchema).optional(),
  labels_count: z.number().int().optional(),
  patterns: z.array(SummaryPatternSchema).optional(),
  levels: z
    .array(z.object({ line: z.number().int(), x: z.number().int(), y: z.number() }))
    .optional(),
  events: z
    .array(z.object({ key: z.string(), name: z.string(), x: z.number().int().nullable() }))
    .optional(),
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
// Scripted indicators (ADR-0025, ADR-0041): `tool_list_my_indicators`,
// `tool_get_my_indicator`, `tool_register_indicator`, `tool_delete_indicator`.
// ---------------------------------------------------------------------------

export const MyIndicatorSchema = z.object({
  name: z.string(),
  title: z.string(),
  description: z.string(),
  overlay: z.boolean(),
  updated_at: z.number().int(),
});
export type MyIndicator = z.infer<typeof MyIndicatorSchema>;

export const MyIndicatorsPayloadSchema = z.object({
  total: z.number().int(),
  indicators: z.array(MyIndicatorSchema),
});
export type MyIndicatorsPayload = z.infer<typeof MyIndicatorsPayloadSchema>;

export const MyIndicatorSourcePayloadSchema = z.object({
  name: z.string(),
  source: z.string(),
});
export type MyIndicatorSourcePayload = z.infer<typeof MyIndicatorSourcePayloadSchema>;

// The `isError` body of `get_my_indicator` for a script you do not have. Any
// other error is a failed read, which says nothing about whether it exists.
export const IndicatorNotFoundSchema = z.object({ error: z.literal("not_found") });

export const RegisteredIndicatorPayloadSchema = z.object({
  ok: z.literal(true),
  name: z.string(),
  title: z.string(),
  overlay: z.boolean(),
  replaced: z.boolean(),
  shadows_builtin: z.boolean(),
});
export type RegisteredIndicatorPayload = z.infer<typeof RegisteredIndicatorPayloadSchema>;

// The `isError` body of a script that failed validation. Other rejections
// (bad name, script limit) are plain text.
export const ScriptRejectionSchema = z.object({
  ok: z.literal(false),
  errors: z.array(IndicatorErrorSchema),
});
export type ScriptRejection = z.infer<typeof ScriptRejectionSchema>;

export const DeleteIndicatorPayloadSchema = z.object({
  ok: z.literal(true),
  name: z.string(),
});
export type DeleteIndicatorPayload = z.infer<typeof DeleteIndicatorPayloadSchema>;

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

// ---------------------------------------------------------------------------
// Server alerts (ADR-0042)
//   Produced by `ServerAlerts::view` and `ServerAlerts::list` in
//   `src/server_alerts.rs`. A webhook shows its host and whether it signs, never
//   its URL or secret. `webhook_secret` only appears in the response that made it.
// ---------------------------------------------------------------------------

/**
 * An indicator alert's indicator (ADR-0067), as `indicator::view` in
 * `src/server_alerts/indicator.rs` writes it: in a list, `value` is the line at the last
 * close read; in a fire's event or wake-up, the line at that close.
 */
export const IndicatorViewSchema = z.object({
  name: z.string(),
  params: z.record(z.string(), z.unknown()),
  line: z.string(),
  compare: z.enum(["level", "close"]),
  value: z.number().nullable(),
});

export const ServerAlertSchema = z.object({
  id: z.string(),
  symbol: z.string(),
  interval: z.string(),
  evaluate_on: z.enum(["live", "close"]),
  condition: z.enum(["crossing", "crossing_up", "crossing_down", "greater", "less"]),
  /** Null only on an indicator alert that compares the close with the line (ADR-0067). */
  level: z.number().nullable(),
  trigger: z.enum(["once", "every"]),
  message: z.string().nullable(),
  active: z.boolean(),
  disarmed: z.boolean(),
  created_at: z.number(),
  last_triggered_at: z.number().nullable(),
  fire_count: z.number().int(),
  webhook: z.object({ host: z.string(), signed: z.boolean() }).nullable(),
  last_delivery_at: z.number().nullable(),
  last_status: z.number().int().nullable(),
  last_error: z.string().nullable(),
  push: z.boolean(),
  last_push_at: z.number().nullable(),
  last_push_status: z.number().int().nullable(),
  last_push_error: z.string().nullable(),
  /**
   * The devices the last fire went to, each with its outcome once known. Each tab reads
   * its own entry: one user's browsers can fare differently (ADR-0044).
   */
  last_push_devices: z
    .record(
      z.string(),
      z
        .object({
          at: z.number(),
          status: z.number().int().nullable(),
          error: z.string().nullable(),
        })
        .nullable(),
    )
    .default({}),
  feed: z.enum(["live", "waiting"]),
  last_price: z.number().nullable(),
  /** Present, and true, only on an alert that wakes the owner's agent (ADR-0061). */
  notify_agent: z.boolean().optional(),
  /** Present only on an alert that is about one of the owner's setups. */
  setup_id: z.string().optional(),
  /**
   * Present only on an alert on an indicator's line at bar closes (ADR-0067). `value` is
   * the line at the last close it read, null until it has read one.
   */
  indicator: IndicatorViewSchema.optional(),
});
export type ServerAlert = z.infer<typeof ServerAlertSchema>;

export const ServerAlertsPayloadSchema = z.object({
  enabled: z.boolean(),
  // What this client may do; absent when the server has them off.
  can_create: z.boolean().optional(),
  webhooks: z.boolean().optional(),
  // Null where this client's alerts cannot notify a device.
  push: z.object({ public_key: z.string(), devices: z.number().int() }).nullable().optional(),
  alerts: z.array(ServerAlertSchema),
});
export type ServerAlertsPayload = z.infer<typeof ServerAlertsPayloadSchema>;

export const ServerAlertChangePayloadSchema = ServerAlertSchema.extend({
  webhook_secret: z.string().optional(),
});
export type ServerAlertChangePayload = z.infer<typeof ServerAlertChangePayloadSchema>;

export const DeleteServerAlertPayloadSchema = z.object({ deleted: z.boolean() });
export type DeleteServerAlertPayload = z.infer<typeof DeleteServerAlertPayloadSchema>;

export const SubscribePushPayloadSchema = z.object({
  devices: z.number().int(),
  device: z.string(),
});
export type SubscribePushPayload = z.infer<typeof SubscribePushPayloadSchema>;

export const UnsubscribePushPayloadSchema = z.object({
  removed: z.boolean(),
  devices: z.number().int(),
});
export type UnsubscribePushPayload = z.infer<typeof UnsubscribePushPayloadSchema>;

// --- Order flow (ADR-0048) ---------------------------------------------------

export const FootprintBarSchema = z.object({
  t: z.number(),
  /** Height of this bar's rows: a reply that had to widen them has its own. */
  row: z.number(),
  buy: z.number(),
  sell: z.number(),
  min_delta: z.number(),
  max_delta: z.number(),
  poc: z.number().nullable(),
  vah: z.number().nullable(),
  val: z.number().nullable(),
  /** `[price, buy, sell]`, ascending, the price being the row's bottom. */
  rows: z.array(z.tuple([z.number(), z.number(), z.number()])),
  buy_imbalances: z.array(z.number()),
  sell_imbalances: z.array(z.number()),
  stacked: z.array(
    z.object({
      side: z.enum(["buy", "sell"]),
      low: z.number(),
      high: z.number(),
      count: z.number(),
    }),
  ),
});
export type FootprintBar = z.infer<typeof FootprintBarSchema>;

export const FootprintPayloadSchema = z.object({
  symbol: z.string(),
  interval: z.string(),
  intrabar_interval: z.string().nullable().default(null),
  side: z.string().default("close_vs_open"),
  row_size: z.number().default(1),
  row_auto: z.boolean().default(true),
  row_widened: z.boolean().default(false),
  tick: z.number().default(0.01),
  bars_without_data: z.number().default(0),
  footprints: z.array(FootprintBarSchema),
  has_more: z.boolean(),
});
export type FootprintPayload = z.infer<typeof FootprintPayloadSchema>;
export type FootprintMeta = Omit<FootprintPayload, "footprints" | "has_more"> & {
  /** A page of older bars was read from coarser data than the load. */
  coarser_older?: boolean;
};

export const TpoProfileSchema = z.object({
  /** The period's start, or the session's: what identifies it between replies. */
  t: z.number(),
  end: z.number(),
  /** Height of this profile's rows, as a footprint bar's. */
  row: z.number(),
  block_times: z.array(z.number()),
  open: z.number(),
  close: z.number(),
  high: z.number(),
  low: z.number(),
  mid: z.number(),
  poc: z.number(),
  vah: z.number(),
  val: z.number(),
  poor_high: z.boolean(),
  poor_low: z.boolean(),
  single_prints: z.array(z.tuple([z.number(), z.number()])),
  ib_high: z.number(),
  ib_low: z.number(),
  total_blocks: z.number(),
  blocks_above_poc: z.number(),
  blocks_below_poc: z.number(),
  rotation_factor: z.number(),
  volume: z.number(),
  rows: z.array(
    z.object({
      p: z.number(),
      letters: z.string(),
      blocks: z.array(z.number()),
      v: z.number(),
    }),
  ),
});
export type TpoProfile = z.infer<typeof TpoProfileSchema>;

export const TpoPayloadSchema = z.object({
  symbol: z.string(),
  period: z.string(),
  block: z.string(),
  /** The block asked for could not cover a whole period: `block` is a longer one. */
  block_widened: z.boolean(),
  row_size: z.number(),
  row_auto: z.boolean(),
  row_widened: z.boolean(),
  tick: z.number(),
  timezone: z.string(),
  periods_cut: z.boolean(),
  profiles: z.array(TpoProfileSchema),
  has_more: z.boolean(),
});
export type TpoPayload = z.infer<typeof TpoPayloadSchema>;
export type TpoMeta = Omit<TpoPayload, "profiles" | "has_more">;

// ---------------------------------------------------------------------------
// Setups (ADR-0059)
//   Produced by `Record::view` in `src/setups.rs` and by `Setups::view` and
//   `Setups::list` in `src/setup_service.rs`. The server sets status, fill, exit,
//   result_r and shadow_results; an agent never does. `tests/fixtures/setup_responses.json` holds
//   real answers, pinned by a Rust test and parsed here in `responses.test.ts`.
// ---------------------------------------------------------------------------

export const SetupStatusSchema = z.enum([
  "watching",
  "triggered",
  "won",
  "invalidated",
  "expired",
  "cancelled",
]);
export type SetupStatus = z.infer<typeof SetupStatusSchema>;

const SetupZoneSchema = z.object({ low: z.number(), high: z.number() });

/** A price and the 1m bar it happened in (its open time, ms). */
const SetupMarkSchema = z.object({ price: z.number(), bar_time: z.number() });

const SetupLevelsSchema = z.object({
  entry: SetupZoneSchema,
  invalidation: z.number(),
  // Nearest first; the first decides a win.
  targets: z.array(z.number()).min(1),
  expires_at: z.number(),
});

/** A change the agent sent that no bar has reached yet. */
export const SetupPendingSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("levels"), at: z.number(), levels: SetupLevelsSchema }),
  z.object({ kind: z.literal("cancel"), at: z.number() }),
]);
export type SetupPending = z.infer<typeof SetupPendingSchema>;

/** A drawing with its points on the bars, as generate_chart reports it (ADR-0054). */
export const SetupDrawingSchema = z.object({
  tool: z.string(),
  points: z.array(
    z.object({
      time: z.string(),
      timestamp: z.number(),
      price: z.number(),
      bar: z.number().int(),
      snap: z.enum(["open", "high", "low", "close"]).optional(),
    }),
  ),
  style: z.record(z.string(), z.unknown()),
  text: z.string().optional(),
});
export type SetupDrawing = z.infer<typeof SetupDrawingSchema>;

/** How a shadow exit variant ended, or that it has not (ADR-0066). */
export const SetupShadowStatusSchema = z.enum([
  "open",
  "won",
  "stopped",
  "breakeven",
  "timed_out",
  "expired",
  "cancelled",
]);
export type SetupShadowStatus = z.infer<typeof SetupShadowStatusSchema>;

/** One fixed exit variant scored on the setup's own 1m bars from the fill on. */
export const SetupShadowSchema = z
  .object({
    status: SetupShadowStatusSchema,
    // The parts' R, each net of its own costs, weighted by share. Null while open, and
    // when it expired while nobody scored it.
    result_r: z.number().nullable(),
    // The stop in force from the next bar on: the invalidation, or break-even after
    // costs once v1 took T1 (T1 where that is nearer) or v2 reached +1R. A stop exit at
    // or through the invalidation is "stopped"; one at a moved stop short of it is
    // "breakeven", near 0 R.
    stop: z.number(),
    // Each part closed: price, share of the position, 1m bar open time (ms).
    exits: z.array(
      z.object({ price: z.number(), fraction: z.number().positive(), bar_time: z.number() }),
    ),
    exited_at: z.number().nullable(),
    // v3: close time (ms) of the bar its time stop judges; null once judged, and for v1/v2.
    time_stop_at: z.number().nullable(),
  })
  .strict();
export type SetupShadow = z.infer<typeof SetupShadowSchema>;

export const SetupShadowsSchema = z
  .object({
    v1_half_t1_be: SetupShadowSchema,
    v2_be_at_1r: SetupShadowSchema,
    v3_time_stop: SetupShadowSchema,
  })
  .strict();
export type SetupShadows = z.infer<typeof SetupShadowsSchema>;

export const SetupSchema = SetupLevelsSchema.extend({
  id: z.string(),
  symbol: z.string(),
  interval: z.string(),
  direction: z.enum(["long", "short"]),
  decision: z.enum(["take", "skip"]),
  thesis: z.string(),
  drawings: z.array(SetupDrawingSchema),
  // Which rule set the levels: the agent, or for a skip the server's bracket from the
  // ATR, or at the least risk a setup may have where the ATR was less.
  bracket: z.enum(["agent", "atr", "floor"]),
  status: SetupStatusSchema,
  created_at: z.number(),
  // The assumption the result is scored with, in basis points per side.
  costs: z.object({ fee_bps_per_side: z.number(), slippage_bps_per_side: z.number() }),
  fill: SetupMarkSchema.nullable(),
  exit: SetupMarkSchema.nullable(),
  // In R. Null while the setup is open, and for one that expired filled while nobody
  // scored it (`gap` is then true): no price was read, so none is invented.
  result_r: z.number().nullable(),
  // Open time of the newest 1m bar applied, ms.
  scored_through: z.number().nullable(),
  // True when the server missed bars it could not read afterwards: the result does not
  // cover them. The log's `gap` entries say which stretch.
  gap: z.boolean(),
  pending: z.array(SetupPendingSchema),
  revisions: z.number().int(),
  agent_revisions: z.number().int(),
  // The agent's labels, such as strategy:retest-short. Empty on setups stored before
  // tags existed.
  tags: z.array(z.string()),
  // From the fill on, in R of the risk at the fill and before costs: the furthest price
  // went against the position (0 or less) and with it (0 or more). The fill bar counts
  // only against it; nothing beyond the exit counts. Null before a fill, and on a setup
  // filled before the server kept them.
  mae_r: z.number().nonpositive().nullable(),
  mfe_r: z.number().nonnegative().nullable(),
  // Open time (ms) of the 1m bar that set each extreme; null when its value is.
  mae_at: z.number().nullable(),
  mfe_at: z.number().nullable(),
  // While watching, the least distance from a bar's high or low to the entry zone, in R
  // of the risk from the zone's edge nearest the price to the invalidation; 0 once
  // filled. Null before the first bar, and on setups stored before the server kept it.
  closest_approach_r: z.number().nonnegative().nullable(),
  // Fixed exit variants scored on the same bars, for comparison only: they never change
  // status or result_r, and one may stay open after the setup closed (ADR-0066). Null
  // before the fill, and on setups filled before the server kept them.
  shadow_results: SetupShadowsSchema.nullable(),
  // In bars of the setup's own interval, to two places; null before a fill.
  // bars_to_exit runs to the newest scored bar while filled, and is null on a setup
  // that expired filled while nobody scored it. bars_to_mae/mfe are null when
  // mae_at/mfe_at are.
  bars_to_fill: z.number().nonnegative().nullable(),
  bars_to_exit: z.number().nonnegative().nullable(),
  bars_to_mae: z.number().nonnegative().nullable(),
  bars_to_mfe: z.number().nonnegative().nullable(),
  // What the agent stated when it made the setup (ADR-0064): fixed from then on, and
  // absent when it stated none. The server gives them no meaning.
  features: z.record(z.string(), z.union([z.number(), z.boolean(), z.string()])).optional(),
  probability: z.number().min(0).max(1).optional(),
  playbook_version: z.string().optional(),
});
export type Setup = z.infer<typeof SetupSchema>;

/** One entry of a setup's log. Entries are only ever added. */
export const SetupRevisionSchema = z.object({
  seq: z.number().int(),
  at: z.number(),
  by: z.enum(["agent", "server"]),
  kind: z.enum(["created", "revised", "cancelled", "status", "voided", "gap"]),
  reason: z.string().nullable(),
  changes: z.record(z.string(), z.object({ from: z.unknown(), to: z.unknown() })),
});
export type SetupRevision = z.infer<typeof SetupRevisionSchema>;

// create_setup, revise_setup, get_setup and cancel_setup: the setup with its whole log.
export const SetupPayloadSchema = SetupSchema.extend({ log: z.array(SetupRevisionSchema) });
export type SetupPayload = z.infer<typeof SetupPayloadSchema>;

export const SetupsPayloadSchema = z.object({
  enabled: z.boolean(),
  // The rest is absent when the server has setups off.
  can_create: z.boolean().optional(),
  replay: z.boolean().optional(),
  // How many matched; `setups` holds at most the call's limit, newest first.
  total: z.number().int().optional(),
  limits: z
    .object({
      open_setups: z.number().int(),
      setups: z.number().int(),
      symbols: z.number().int(),
      revisions_per_setup: z.number().int(),
      changes_per_minute: z.number().int(),
      bytes: z.number().int(),
    })
    .optional(),
  setups: z.array(SetupSchema),
});
export type SetupsPayload = z.infer<typeof SetupsPayloadSchema>;

// ---------------------------------------------------------------------------
// Track record (ADR-0064)
//   Produced by `Setups::track_record` in `src/setup_service.rs` and `track_record`
//   in `src/setups.rs`: counts and sums over the caller's closed setups. It judges
//   nothing; a group of a few setups says little.
// ---------------------------------------------------------------------------

const TrackRecordCountsSchema = z.object({
  count: z.number().int(),
  won: z.number().int(),
  invalidated: z.number().int(),
  expired: z.number().int(),
  cancelled: z.number().int(),
  /** Setups with a result in R. One that expired filled while nobody scored it has none. */
  with_result: z.number().int(),
  total_r: z.number(),
  /** Over `with_result`; null when that is 0. A setup that never traded is in it with 0 R. */
  mean_r: z.number().nullable(),
  /** Setups whose entry was reached, and the mean over those of them with a result. */
  entered: z.number().int(),
  mean_r_entered: z.number().nullable(),
  /** Results that do not cover bars the server missed. */
  gaps: z.number().int(),
  /** Of the setups that stated a probability: its mean, and how many of them won. */
  probability: z.object({
    stated: z.number().int(),
    mean: z.number().nullable(),
    won: z.number().int(),
    observed: z.number().nullable(),
  }),
});

export const TrackRecordPayloadSchema = z.object({
  replay: z.boolean(),
  /** Closed setups counted, and open ones that match the filters and are not. */
  closed: z.number().int(),
  open: z.number().int(),
  /** True when older closed setups were left out. */
  truncated: z.boolean(),
  group_by: z.enum(["playbook_version", "feature", "probability", "symbol", "direction"]).nullable(),
  feature: z.string().nullable(),
  total: TrackRecordCountsSchema,
  /** In the order of the keys; null is the group of setups that stated none. */
  groups: z.array(
    TrackRecordCountsSchema.extend({
      // A number is written as a decimal: 1 and 1.0 are the one group `1.0`.
      key: z.union([z.string(), z.number(), z.boolean()]).nullable(),
    }),
  ),
});
export type TrackRecordPayload = z.infer<typeof TrackRecordPayloadSchema>;

// ---------------------------------------------------------------------------
// GET /admin/overview — `src/server/admin.rs` (ADR-0063). The operator page's
//   one read: counts, times, ids and names. Times are unix seconds. A client's
//   id is its first 8 characters.
// ---------------------------------------------------------------------------

export const OperatorUserSchema = z.object({
  id: z.string(),
  name: z.string(),
  provider: z.string(),
  created_at: z.number().int(),
  last_login_at: z.number().int(),
  last_token_at: z.number().int().nullable(),
  workspace_docs: z.number().int(),
  workspace_bytes: z.number().int(),
  setups_open: z.number().int(),
  setups_closed: z.number().int(),
  server_alerts: z.number().int(),
  authorisations: z.number().int(),
  push_devices: z.number().int(),
});

export const OperatorClientSchema = z.object({
  id: z.string(),
  name: z.string().nullable(),
  created_at: z.number().int(),
  last_token_at: z.number().int().nullable(),
  has_authorisation: z.boolean(),
  owns_setups: z.boolean(),
});

export const OperatorOverviewSchema = z.object({
  server: z.object({
    version: z.string(),
    started_at: z.number().int().nullable(),
    schema_version: z.number().int(),
    pending_deliveries: z.number().int(),
    store_bytes: z.number().int().nullable(),
    setups_total: z.number().int(),
    features: z.record(z.boolean()),
    feeds: z.object({
      watched_pairs: z.number().int(),
      watched_pairs_limit: z.number().int(),
      push_subscriptions: z.number().int(),
      push_feeds: z.number().int(),
    }),
  }),
  users: z.object({ total: z.number().int(), shown: z.array(OperatorUserSchema) }),
  clients: z.object({
    total: z.number().int(),
    limit: z.number().int(),
    shown: z.array(OperatorClientSchema),
  }),
});
export type OperatorOverview = z.infer<typeof OperatorOverviewSchema>;

// ---------------------------------------------------------------------------
// Event log (ADR-0061)
//   Produced by `answer` and `view` in `src/event_log.rs`. `what` is the event's
//   stored body: written by `fire_event` in `src/server_alerts.rs` for
//   `alert.fired` and by `status_events` in `src/setup_service.rs` for
//   `setup.status`. It never holds a thesis or a note.
// ---------------------------------------------------------------------------

/** `none`: logged only. `dropped`: too many were waiting. `failed`: given up. */
export const EventDeliverySchema = z.enum([
  "none",
  "waiting",
  "delivered",
  "failed",
  "dropped",
]);
export type EventDelivery = z.infer<typeof EventDeliverySchema>;

const LoggedEventBaseSchema = z.object({
  /** The owner's own counter: it only grows, and a number is never used twice. */
  seq: z.number().int(),
  /** The `id` of the wake-up, where one was sent. */
  id: z.string(),
  /** When it happened, in ms: a setup's status at its bar's time. */
  at: z.number(),
  alert_id: z.string().optional(),
  setup_id: z.string().optional(),
  delivery: EventDeliverySchema,
});

export const LoggedEventSchema = z.discriminatedUnion("type", [
  LoggedEventBaseSchema.extend({
    type: z.literal("alert.fired"),
    what: z.object({
      /** The alert as it was at the fire, so a wake-up can say it once the alert is gone. */
      symbol: z.string(),
      interval: z.string(),
      evaluate_on: z.enum(["live", "close"]),
      trigger: z.enum(["once", "every"]),
      price: z.number(),
      /**
       * The level the fire compared with; with compare: close, the line's value; null for
       * an alert that has no level, as one that fires at a bar's close.
       */
      level: z.number().nullable(),
      condition: z.string(),
      bar_time: z.number(),
      fire_count: z.number().int(),
      /** Only on an indicator alert's fire (ADR-0067). */
      indicator: IndicatorViewSchema.optional(),
    }),
  }),
  LoggedEventBaseSchema.extend({
    type: z.literal("setup.status"),
    what: z.object({
      from: SetupStatusSchema,
      to: SetupStatusSchema,
      gap: z.boolean(),
      result_r: z.number().nullable().optional(),
    }),
  }),
]);
export type LoggedEvent = z.infer<typeof LoggedEventSchema>;

export const EventsPayloadSchema = z.object({
  events: z.array(LoggedEventSchema),
  /** Pass as `after` to read on. */
  cursor: z.number().int(),
  more: z.boolean(),
  /** True when events after the caller's cursor are no longer kept. */
  truncated: z.boolean(),
});
export type EventsPayload = z.infer<typeof EventsPayloadSchema>;
