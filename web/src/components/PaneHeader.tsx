// TradingView-style per-pane header strip (ADR-0006).
//
// Sits absolute-positioned over the top-left of each klinecharts pane
// and shows the indicator label, its parameter tuple, and the last
// non-null value for each line — colored to match the series color.
//
// We render this as Solid markup rather than wiring klinecharts's
// built-in indicator tooltip because the built-in tooltip renders on
// canvas and its typography cannot be styled to match the rest of the
// app (Inter + tabular-nums). Native tooltip is kept for crosshair
// hover; this header is the resting-state "current value" row.
//
// Inputs are plain data derived from SeriesPayload — no klinecharts
// imports here, which keeps this component cheap to re-render.

import { For, type JSX } from "solid-js";

export interface PaneHeaderLine {
  /** Series label, e.g. "Money Flow" or "RSI". */
  label: string;
  /** Last non-null value, or null when the line is not yet warmed up. */
  value: number | null;
  /** Display color matching the series stroke. */
  color: string;
}

export interface PaneHeaderProps {
  /** Indicator display name, e.g. "RSI" or "VMC Cipher_B_Divergences". */
  name: string;
  /** Parameter tuple stringified in the TV style, e.g. "(9, 12, hlc3)". */
  params?: string;
  /** Per-line label + last value + color. */
  lines: PaneHeaderLine[];
  /** Distance from the top of the chart host in pixels. */
  top: number;
}

function formatValue(v: number | null): string {
  if (v === null || !Number.isFinite(v)) return "—";
  const abs = Math.abs(v);
  // Four decimals under 1, two decimals between 1-10_000, K/M above.
  if (abs >= 1_000_000) return `${(v / 1_000_000).toFixed(2)}M`;
  if (abs >= 10_000) return `${(v / 1_000).toFixed(2)}K`;
  if (abs >= 1) return v.toFixed(2);
  return v.toFixed(4);
}

export function PaneHeader(props: PaneHeaderProps): JSX.Element {
  return (
    <div class="pane-header" style={{ top: `${props.top}px` }}>
      <span class="pane-header-label">{props.name}</span>
      {props.params ? (
        <span class="pane-header-params">{props.params}</span>
      ) : null}
      <For each={props.lines}>
        {(line) => (
          <span class="pane-header-value" style={{ color: line.color }}>
            {line.label} {formatValue(line.value)}
          </span>
        )}
      </For>
    </div>
  );
}
