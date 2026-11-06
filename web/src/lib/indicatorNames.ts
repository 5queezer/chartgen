// Display names for indicator ids. The registry has no display name; the
// chart legend uses the `label` of each series payload entry, so those
// labels are remembered here and used by the chips and the picker. Until
// an indicator has been seen in a payload, a prettified id stands in.

import { createSignal } from "solid-js";

const [labels, setLabels] = createSignal<Record<string, string>>({});

const ACRONYMS = new Set([
  "adx", "atr", "bb", "cci", "cvd", "ema", "hvn", "lvn", "macd", "mfi",
  "obv", "roc", "rsi", "sar", "sma", "svp", "vp", "vwap", "wma",
]);
const OVERRIDES: Record<string, string> = {
  bbands: "Bollinger Bands",
  sar: "Parabolic SAR",
};

/**
 * What can change on the chart after it was drawn, for indicators that detect
 * patterns (ADR-0046). Shown in their settings so no repaint is a surprise.
 */
const REPAINT_NOTES: Record<string, string> = {
  candlestick_patterns:
    "A label on the forming bar is outlined and can change or vanish until the bar closes. Labels on closed bars never change, and alerts wait for the close.",
  chart_patterns:
    "Read from closed bars: the forming bar changes nothing. A swing is known only after Depth more bars, so a pattern appears that much later. Forming patterns can reshape or vanish as swings confirm. A pattern is confirmed only after a later bar closes past its line, and can still fail after that.",
  auto_fib:
    "Read from closed bars: the forming bar moves nothing. The levels are redrawn when a newer swing is confirmed, Depth bars after it, or when a close breaks the start of the swing. Older bars then show the new levels too.",
};
const NOTE_ALIASES: Record<string, string> = {
  candle_patterns: "candlestick_patterns",
  auto_chart_patterns: "chart_patterns",
  auto_fib_retracement: "auto_fib",
};

/** The repaint note of an indicator id, if it has one. */
export const repaintNote = (id: string): string | undefined =>
  REPAINT_NOTES[NOTE_ALIASES[id] ?? id];

/** `volume_profile` -> `Volume Profile`, `ema_stack` -> `EMA Stack`. */
export function prettifyId(id: string): string {
  return (
    OVERRIDES[id] ??
    id
      .split("_")
      .map((w) =>
        ACRONYMS.has(w) ? w.toUpperCase() : w.charAt(0).toUpperCase() + w.slice(1),
      )
      .join(" ")
  );
}

/** Remember the legend labels of a series payload's indicators. */
export function rememberLabels(
  indicators: Record<string, { label?: string | null }> | undefined,
): void {
  if (!indicators) return;
  const cur = labels();
  const next: Record<string, string> = {};
  for (const [name, ind] of Object.entries(indicators)) {
    if (ind.label && cur[name] !== ind.label) next[name] = ind.label;
  }
  if (Object.keys(next).length > 0) setLabels({ ...cur, ...next });
}

/** The name shown for an indicator id (reactive). */
export function displayName(id: string): string {
  return labels()[id] ?? prettifyId(id);
}

/**
 * A server label like `SMA 20` carries the length of the instance it came
 * from; when the instance's own params `(50)` are shown next to it, drop the
 * number so it is neither said twice nor wrong for another instance.
 */
export function withoutParamNumber(label: string, params?: string): string {
  return params ? label.replace(/\s+\d+(?:\.\d+)?$/, "") : label;
}

/** Chip text: the display name of the registry id, then the param label. */
export function chipName(id: string, params?: string): string {
  return [withoutParamNumber(displayName(id), params), params]
    .filter(Boolean)
    .join(" ");
}
