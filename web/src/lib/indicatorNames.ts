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
