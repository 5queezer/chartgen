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
// Indicator rows can also carry hover controls (hide, settings, remove) and
// open settings on double-click, like TradingView. Rows without callbacks
// (the price pane) stay non-interactive.
//
// Inputs are plain data derived from SeriesPayload — no klinecharts
// imports here, which keeps this component cheap to re-render.

import { For, Show, type JSX } from "solid-js";

import type { BarLegend } from "../lib/format";

export interface PaneHeaderLine {
  /** Series label, e.g. "Money Flow" or "RSI". */
  label: string;
  /** Last non-null value, or null when the line is not yet warmed up. */
  value: number | null;
  /** Display color matching the series stroke. */
  color: string;
  /** Shown instead of the formatted `value` (e.g. a percent change). */
  text?: string;
}

export interface PaneHeaderProps {
  /** Indicator display name, e.g. "RSI" or "VMC Cipher_B_Divergences". */
  name: string;
  /** Parameter tuple stringified in the TV style, e.g. "(9, 12, hlc3)". */
  params?: string;
  /** Per-line label + last value + color. */
  lines: PaneHeaderLine[];
  /**
   * Price-pane legend: O H L C, change vs previous close, and volume.
   * Rendered after the label/params, before `lines`.
   */
  legend?: BarLegend;
  /** Distance from the top of the chart host in pixels. */
  top: number;
  /** `data-testid` of the row. */
  testId?: string;
  /** Hide the indicator (client-side only). Omit for rows that cannot be hidden. */
  onToggleHidden?: () => void;
  /** Open the indicator's settings dialog. */
  onSettings?: () => void;
  /** Remove the indicator. Omit for the price pane. */
  onRemove?: () => void;
}

function Icon(props: { children: JSX.Element }): JSX.Element {
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 16 16"
      width="14"
      height="14"
      fill="none"
      stroke="currentColor"
      stroke-width="1.5"
      stroke-linecap="round"
      stroke-linejoin="round"
    >
      {props.children}
    </svg>
  );
}

function ControlButton(props: {
  label: string;
  onClick: () => void;
  children: JSX.Element;
}): JSX.Element {
  return (
    <button
      type="button"
      class="pane-header-btn"
      aria-label={props.label}
      title={props.label}
      onClick={(e) => {
        e.stopPropagation();
        props.onClick();
      }}
      onDblClick={(e) => e.stopPropagation()}
    >
      <Icon>{props.children}</Icon>
    </button>
  );
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
    <div
      class="pane-header"
      classList={{
        "pane-header-interactive": !!props.onSettings || !!props.onRemove,
      }}
      data-testid={props.testId}
      style={{ top: `${props.top}px` }}
      onDblClick={() => props.onSettings?.()}
    >
      <span class="pane-header-label">{props.name}</span>
      {props.params ? (
        <span class="pane-header-params">{props.params}</span>
      ) : null}
      <Show when={props.legend}>
        {(l) => (
          <>
            <Show when={l().time}>
              <span class="pane-header-vol" data-testid="legend-time">
                {l().time}
              </span>
            </Show>
            <span
              class={`pane-header-ohlc pane-header-${l().change?.direction ?? "flat"}`}
              data-testid="price-legend"
            >
              <span>O <b data-testid="legend-o">{l().o}</b></span>
              <span>H <b data-testid="legend-h">{l().h}</b></span>
              <span>L <b data-testid="legend-l">{l().l}</b></span>
              <span>C <b data-testid="legend-c">{l().c}</b></span>
              <Show when={l().change}>
                {(ch) => (
                  <span data-testid="legend-change">
                    {ch().abs} ({ch().pct})
                  </span>
                )}
              </Show>
            </span>
            <span class="pane-header-vol">
              Vol <span data-testid="legend-v">{l().v}</span>
            </span>
          </>
        )}
      </Show>
      <For each={props.lines}>
        {(line) => (
          <span class="pane-header-value" style={{ color: line.color }}>
            {line.label} {line.text ?? formatValue(line.value)}
          </span>
        )}
      </For>
      <Show when={props.onToggleHidden || props.onSettings || props.onRemove}>
        <span class="pane-header-controls">
          <Show when={props.onToggleHidden}>
            {(cb) => (
              <ControlButton label={`Hide ${props.name}`} onClick={cb()}>
                <path d="M1 8 C3 4.5 5.5 3 8 3 S13 4.5 15 8 C13 11.5 10.5 13 8 13 S3 11.5 1 8 Z" />
                <circle cx="8" cy="8" r="2" />
              </ControlButton>
            )}
          </Show>
          <Show when={props.onSettings}>
            {(cb) => (
              <ControlButton label={`Settings for ${props.name}`} onClick={cb()}>
                <circle cx="8" cy="8" r="2" />
                <path d="M8 1.5 V3.5 M8 12.5 V14.5 M1.5 8 H3.5 M12.5 8 H14.5 M3.4 3.4 L4.8 4.8 M11.2 11.2 L12.6 12.6 M3.4 12.6 L4.8 11.2 M11.2 4.8 L12.6 3.4" />
              </ControlButton>
            )}
          </Show>
          <Show when={props.onRemove}>
            {(cb) => (
              <ControlButton label={`Remove ${props.name}`} onClick={cb()}>
                <path d="M3.5 3.5 L12.5 12.5 M12.5 3.5 L3.5 12.5" />
              </ControlButton>
            )}
          </Show>
        </span>
      </Show>
    </div>
  );
}
