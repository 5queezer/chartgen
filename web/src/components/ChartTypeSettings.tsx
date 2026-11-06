// Inputs of a price-based chart type (ADR-0047): how its box is set, a Point &
// Figure reversal, a Line Break line count. OK applies them to the active chart,
// which reloads; anything else that closes the dialog leaves them as they were.

import { Dialog } from "@kobalte/core";
import { createEffect, createMemo, createSignal, Show, type JSX } from "solid-js";

import {
  MAX_ATR_LENGTH,
  MAX_LINES,
  MAX_REVERSAL,
  type BoxMode,
  type ChartParams,
} from "../lib/priceBars";
import { chartParams, chartType, setChartParams } from "../stores/layout";

export interface ChartTypeSettingsProps {
  open: boolean;
  /** The box the chart is on now: where a fixed box starts from. */
  currentBox: number | undefined;
  onClose: () => void;
}

const inputClass =
  "w-28 bg-[color:var(--color-bg-elev)] border border-[color:var(--color-border)] rounded px-2 py-1 text-sm text-right";
const rowClass = "flex items-center justify-between gap-4 text-sm";

const TITLES: Partial<Record<string, string>> = {
  renko: "Renko",
  kagi: "Kagi",
  point_figure: "Point & figure",
  line_break: "Line break",
  range: "Range",
};

/** What the box is called on each type. */
const BOX_LABELS: Partial<Record<string, string>> = {
  renko: "Box size",
  kagi: "Reversal amount",
  point_figure: "Box size",
  range: "Range size",
};

const number = (text: string): number => (text.trim() === "" ? NaN : Number(text));

const wholeIn = (text: string, max: number): number | null => {
  const n = number(text);
  return Number.isInteger(n) && n >= 1 && n <= max ? n : null;
};

export function ChartTypeSettings(props: ChartTypeSettingsProps): JSX.Element {
  const [box, setBox] = createSignal<BoxMode>("atr");
  const [length, setLength] = createSignal("");
  const [size, setSize] = createSignal("");
  const [percent, setPercent] = createSignal("");
  const [reversal, setReversal] = createSignal("");
  const [lines, setLines] = createSignal("");

  createEffect(() => {
    if (!props.open) return;
    const p = chartParams();
    setBox(p.box);
    setLength(String(p.atrLength));
    setSize(String(p.boxSize ?? props.currentBox ?? ""));
    setPercent(String(p.percent));
    setReversal(String(p.reversal));
    setLines(String(p.lines));
  });

  const usesBox = () => chartType() !== "line_break";

  /** The inputs as they stand, or what is wrong with them. Only what this type reads counts. */
  const result = createMemo((): ChartParams | string => {
    const p = chartParams();
    const out: ChartParams = { ...p, box: box() };
    if (usesBox()) {
      if (box() === "atr") {
        const n = wholeIn(length(), MAX_ATR_LENGTH);
        if (n === null) return `ATR length must be a whole number from 1 to ${MAX_ATR_LENGTH}.`;
        out.atrLength = n;
      } else if (box() === "percent") {
        const n = number(percent());
        if (!(n > 0 && n <= 100)) return "The percentage must be above 0 and at most 100.";
        out.percent = n;
      } else {
        const n = number(size());
        if (!(Number.isFinite(n) && n > 0)) return "The size must be a number above 0.";
        out.boxSize = n;
      }
    }
    if (chartType() === "point_figure") {
      const n = wholeIn(reversal(), MAX_REVERSAL);
      if (n === null) return `Reversal must be a whole number from 1 to ${MAX_REVERSAL}.`;
      out.reversal = n;
    }
    if (chartType() === "line_break") {
      const n = wholeIn(lines(), MAX_LINES);
      if (n === null) return `Line count must be a whole number from 1 to ${MAX_LINES}.`;
      out.lines = n;
    }
    return out;
  });

  const submit = (e: Event) => {
    e.preventDefault();
    const r = result();
    if (typeof r === "string") return;
    setChartParams(r);
    props.onClose();
  };

  return (
    <Dialog.Root
      open={props.open}
      onOpenChange={(open) => {
        if (!open) props.onClose();
      }}
      modal
    >
      <Dialog.Portal>
        <Dialog.Overlay class="fixed inset-0 z-40 bg-black/60" />
        <div class="fixed inset-0 z-50 flex items-start justify-center pt-[10vh] px-4">
          <Dialog.Content
            class="w-full max-w-sm bg-[color:var(--color-bg)] border border-[color:var(--color-border)] rounded-lg shadow-xl"
            data-testid="chart-type-settings"
          >
            <form onSubmit={submit} noValidate class="flex flex-col">
              <div class="px-4 py-3 border-b border-[color:var(--color-border)]">
                <Dialog.Title class="text-sm font-semibold">
                  {TITLES[chartType()] ?? "Chart type"} settings
                </Dialog.Title>
              </div>
              <div class="flex flex-col gap-3 px-4 py-3">
                <Show when={usesBox()}>
                  <label class={rowClass}>
                    {BOX_LABELS[chartType()] ?? "Box size"}
                    <select
                      class={inputClass}
                      value={box()}
                      aria-label="Box assignment"
                      onChange={(e) => setBox(e.currentTarget.value as BoxMode)}
                    >
                      <option value="atr">ATR</option>
                      <option value="fixed">Traditional</option>
                      <option value="percent">Percentage LTP</option>
                    </select>
                  </label>
                  <Show when={box() === "atr"}>
                    <label class={rowClass}>
                      ATR length
                      <input
                        class={inputClass}
                        inputmode="numeric"
                        value={length()}
                        onInput={(e) => setLength(e.currentTarget.value)}
                      />
                    </label>
                  </Show>
                  <Show when={box() === "fixed"}>
                    <label class={rowClass}>
                      Size
                      <input
                        class={inputClass}
                        inputmode="decimal"
                        value={size()}
                        onInput={(e) => setSize(e.currentTarget.value)}
                      />
                    </label>
                  </Show>
                  <Show when={box() === "percent"}>
                    <label class={rowClass}>
                      Percent of last price
                      <input
                        class={inputClass}
                        inputmode="decimal"
                        value={percent()}
                        onInput={(e) => setPercent(e.currentTarget.value)}
                      />
                    </label>
                  </Show>
                  <Show when={box() !== "fixed"}>
                    <p class="text-xs text-[color:var(--color-fg-muted)]">
                      Set when the chart loads and rounded to two significant digits, then kept
                      until the next load.
                    </p>
                  </Show>
                </Show>
                <Show when={chartType() === "point_figure"}>
                  <label class={rowClass}>
                    Reversal, in boxes
                    <input
                      class={inputClass}
                      inputmode="numeric"
                      value={reversal()}
                      onInput={(e) => setReversal(e.currentTarget.value)}
                    />
                  </label>
                </Show>
                <Show when={chartType() === "line_break"}>
                  <label class={rowClass}>
                    Line count
                    <input
                      class={inputClass}
                      inputmode="numeric"
                      value={lines()}
                      onInput={(e) => setLines(e.currentTarget.value)}
                    />
                  </label>
                </Show>
                <Show when={typeof result() === "string"}>
                  <p role="alert" class="text-xs text-[color:var(--color-danger,#ef5350)]">
                    {result() as string}
                  </p>
                </Show>
              </div>
              <div class="flex justify-end gap-2 px-4 py-3 border-t border-[color:var(--color-border)]">
                <button
                  type="button"
                  class="px-3 py-1.5 rounded-md text-sm cursor-pointer hover:bg-[color:var(--color-bg-elev)]"
                  onClick={props.onClose}
                >
                  Cancel
                </button>
                <button
                  type="submit"
                  disabled={typeof result() === "string"}
                  class="px-3 py-1.5 rounded-md text-sm cursor-pointer bg-[color:var(--color-accent)] text-white disabled:opacity-40 disabled:cursor-not-allowed"
                >
                  OK
                </button>
              </div>
            </form>
          </Dialog.Content>
        </div>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
