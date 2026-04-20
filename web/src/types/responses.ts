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

export const SeriesLineSchema = z.object({
  index: z.number().int(),
  label: z.string().nullable(),
  values: z.array(FiniteOrNull),
});

export const SeriesHistogramBucketSchema = z.object({
  index: z.number().int(),
  values: z.array(FiniteOrNull),
});

export const SeriesSignalSchema = z.object({
  x: z.number().int(),
  t: z.number().nullable(),
  y: z.number(),
  label: z.string(),
});

export const HLineSchema = z.object({
  y: z.number(),
});

export const SeriesFillSchema = z.object({
  y1: z.array(FiniteOrNull),
  y2: z.array(FiniteOrNull),
});

export const SeriesHBarSchema = z.object({
  y: z.number(),
  height: z.number(),
  width: z.number(),
  offset: z.number(),
  left: z.boolean(),
});

export const SeriesDivLineSchema = z.object({
  x1: z.number().int(),
  t1: z.number().int().nullable(),
  y1: z.number(),
  x2: z.number().int(),
  t2: z.number().int().nullable(),
  y2: z.number(),
  dashed: z.boolean(),
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

export const SeriesPayloadSchema = z.object({
  symbol: z.string().nullable(),
  interval: z.string().nullable(),
  bars: z.array(SeriesBarSchema),
  indicators: z.record(z.string(), SeriesIndicatorSchema),
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
  indicators: z.record(z.string(), SummaryIndicatorSchema),
});
export type SummaryPayload = z.infer<typeof SummaryPayloadSchema>;

// ---------------------------------------------------------------------------
// list_indicators — `tool_list_indicators` return shape.
// ---------------------------------------------------------------------------

export const IndicatorInfoSchema = z.object({
  name: z.string(),
  description: z.string(),
  type: z.string(), // category: "overlay" | "panel" | "external"
  overlay: z.boolean(),
  aliases: z.array(z.string()).optional(),
  configurable_params: z.unknown().optional(), // registry-defined shape
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
