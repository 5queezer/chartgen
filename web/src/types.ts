/**
 * Hand-written response shapes for chartgen's MCP `generate_chart` tool when
 * called with `format=series`.
 *
 * Authoritative spec: `website/src/content/docs/reference/mcp.md` §"Series
 * payload shape" and `src/mcp.rs::panel_result_to_series_json`.
 *
 * The JSON-RPC envelope is the standard MCP `tools/call` result:
 *   { jsonrpc, id, result: { content: [{ type: "text", text: "<stringified JSON>"}] } }
 *
 * The series payload lives inside `result.content[0].text` as a JSON string.
 */

/** A single OHLCV bar. `t` is unix **seconds**, not milliseconds. */
export interface SeriesBar {
  t: number;
  o: number | null;
  h: number | null;
  l: number | null;
  c: number | null;
  v: number | null;
}

export interface SeriesLine {
  index: number;
  label: string;
  /** Pre-warmup values are JSON null; array length equals bars.length. */
  values: Array<number | null>;
}

export interface SeriesHistogram {
  index: number;
  label: string;
  values: Array<number | null>;
}

export interface SeriesSignal {
  /** Bar index into the `bars` array. */
  x: number;
  /** Unix seconds matching bars[x].t; may be null for synthetic data. */
  t: number | null;
  y: number;
  label: string;
}

export interface SeriesHLine {
  y: number;
}

export interface SeriesIndicator {
  label: string;
  is_overlay: boolean;
  y_range?: [number, number];
  lines: SeriesLine[];
  histogram?: SeriesHistogram[];
  signals?: SeriesSignal[];
  hlines?: SeriesHLine[];
}

export interface SeriesPayload {
  symbol: string | null;
  interval: string | null;
  bars: SeriesBar[];
  /** Keyed by the name each indicator was requested with. */
  indicators: Record<string, SeriesIndicator>;
}

/**
 * Push-notification wire format (JSON-RPC 2.0 notification) delivered over SSE.
 * Spec: `website/src/content/docs/guides/notifications.md`.
 */
export interface AlertTriggeredNotification {
  jsonrpc: '2.0';
  method: 'notifications/alert_triggered';
  params: {
    alert_id: string;
    symbol: string;
    interval: string;
    condition:
      | { type: 'price_above' | 'price_below'; threshold: number }
      | { type: 'indicator_signal'; indicator: string; signal: string };
    value: number;
    triggered_at: string;
  };
}
