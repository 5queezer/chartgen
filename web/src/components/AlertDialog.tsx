// Create / edit dialog for alerts, plus the entry points that open it:
// the header bell button, Alt+A (TradingView's shortcut: an alert at the
// last price) and `openAlertDialog`, which the Alerts tab "+", the chart
// context menu, a drawing's menu, an indicator legend or any other caller
// can use.
//
// Besides the price, an alert can watch a drawing or an indicator plot of
// the chart (ADR-0034): "Source" picks the left operand (the price, a
// drawing, a plot) and "Level" what it is compared with (a value, or a
// plot: of any indicator for the price, of the same indicator for a plot).
// An existing alert keeps its source; its condition, value, trigger and
// message can be edited.

import { Dialog } from "@kobalte/core";
import {
  For,
  Show,
  createMemo,
  createSignal,
  onCleanup,
  onMount,
  type JSX,
} from "solid-js";

import {
  CONDITIONS,
  CONDITION_LABELS,
  TRIGGERS,
  TRIGGER_LABELS,
  sourceOf,
  type AlertCondition,
  type AlertSource,
  type AlertTrigger,
} from "../lib/alerts";
import { keyName } from "../lib/instances";
import { isTypingContext } from "../lib/keys";
import { priceDecimals } from "../lib/quote";
import type { SeriesPayload } from "../types/responses";
import {
  MAX_ALERTS,
  addAlert,
  alerts,
  indicatorSignatures,
  lastPrice,
  updateAlert,
} from "../stores/alerts";
import { symbol as chartSymbol } from "../stores/layout";
import {
  requestNotificationPermission,
  showToast,
  unlockAudio,
} from "../services/alertEngine";

/** Value of the Source / Level select for a plot of an indicator instance. */
export const plotRef = (key: string, line: number) =>
  `plot:${JSON.stringify([key, line])}`;
/** Value of the Source select for a drawing. */
export const drawingRef = (id: string) => `drawing:${id}`;
const PRICE = "price";
const VALUE = "value";

interface Draft {
  /** Set when editing an existing alert. */
  id?: string;
  symbol: string;
  /** "price", a `drawingRef` or a `plotRef` (create only). */
  source: string;
  /** "value" or a `plotRef` (create only). */
  level: string;
  /** The alert's own source when editing; it cannot change. */
  fixed?: AlertSource;
  condition: AlertCondition;
  /** The level when it is a value: a price, or an indicator value. */
  price: string;
  trigger: AlertTrigger;
  message: string;
}

const [draft, setDraft] = createSignal<Draft | null>(null);

/** Plain decimal text of a price, at the precision the watchlist shows. */
const priceText = (p: number) => String(+p.toFixed(priceDecimals(p)));

export interface OpenAlertOptions {
  /** Edit this alert; the other fields are ignored. */
  id?: string;
  /** Defaults to the chart symbol. */
  symbol?: string;
  /** Prefilled level value; defaults to the symbol's last live price, if known. */
  price?: number;
  /** Watch this drawing (its record id). */
  drawingId?: string;
  /** Source select value (`plotRef`), for an alert on an indicator plot. */
  source?: string;
  /** Level select value (`plotRef`), for "price crossing this plot". */
  level?: string;
}

/** Open the dialog to create an alert (or edit one with `id`). */
export function openAlertDialog(opts: OpenAlertOptions = {}): void {
  const existing =
    opts.id === undefined ? undefined : alerts().find((a) => a.id === opts.id);
  if (existing) {
    const src = sourceOf(existing);
    const value =
      src.kind === "price"
        ? priceText(existing.price)
        : src.kind === "indicator" && src.against.kind === "value"
          ? String(src.against.value)
          : "";
    setDraft({
      id: existing.id,
      symbol: existing.symbol,
      source: PRICE,
      level: VALUE,
      fixed: src,
      condition: existing.condition,
      price: value,
      trigger: existing.trigger,
      message: existing.message ?? "",
    });
    return;
  }
  const sym = opts.symbol ?? chartSymbol();
  const price = opts.price ?? (opts.source ? null : lastPrice(sym));
  setDraft({
    symbol: sym,
    source:
      opts.drawingId !== undefined ? drawingRef(opts.drawingId) : (opts.source ?? PRICE),
    level: opts.level ?? VALUE,
    condition: "crossing",
    price: price === null ? "" : opts.source ? String(+price.toPrecision(6)) : priceText(price),
    trigger: "once",
    message: "",
  });
}

const inputClass =
  "bg-[color:var(--color-bg-elev)] border border-[color:var(--color-border)] rounded-md px-3 py-1.5 text-sm text-[color:var(--color-fg)] focus:outline-none focus:border-[color:var(--color-accent)] w-44 text-right tabular-nums";
const selectClass =
  "bg-[color:var(--color-bg-elev)] border border-[color:var(--color-border)] rounded-md px-2 py-1.5 text-sm text-[color:var(--color-fg)] w-56";
const buttonClass =
  "border border-[color:var(--color-border)] rounded-md px-3 py-1.5 text-sm hover:border-[color:var(--color-accent)] cursor-pointer";

export function BellIcon(): JSX.Element {
  return (
    <svg
      width="18"
      height="18"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="1.6"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <path d="M6 16V11a6 6 0 0 1 12 0v5l2 2H4z" />
      <path d="M10 21a2 2 0 0 0 4 0" />
    </svg>
  );
}

/** Header button: an alert at the chart symbol's last price. */
export function AlertButton(): JSX.Element {
  return (
    <button
      type="button"
      aria-label="Add alert"
      title="Add alert (Alt+A)"
      onClick={() => openAlertDialog()}
      class="inline-flex items-center justify-center w-8 h-8 rounded-md cursor-pointer text-[color:var(--color-fg)] hover:bg-[color:var(--color-bg-elev)]"
    >
      <BellIcon />
    </button>
  );
}

/** A plot the dialog offers. */
interface PlotOption {
  ref: string;
  key: string;
  line: number;
  /** Option text, e.g. "EMA: EMA 200". */
  label: string;
  /** Name in descriptions, e.g. "EMA 200" or "RSI (14)". */
  name: string;
}

/** Last value of a plot in the series (its latest bar with one), or null. */
export function lastPlotValue(
  series: SeriesPayload | null,
  key: string,
  line: number,
): number | null {
  const values = series?.indicators[key]?.lines?.[line]?.values ?? [];
  for (let i = values.length - 1; i >= 0; i--) {
    const v = values[i];
    if (v != null) return v;
  }
  return null;
}

/** Plots of the series' indicators, in legend order. */
function plotOptions(
  series: SeriesPayload | null,
  params: ReadonlyMap<string, string | undefined>,
): PlotOption[] {
  return Object.entries(series?.indicators ?? {}).flatMap(([key, ind]) => {
    const p = params.get(key);
    const base = ind.label || keyName(key);
    const title = p ? `${base} ${p}` : base;
    const lines = ind.lines ?? [];
    return lines.map((l, line) => {
      const label = l.label || `Plot ${line + 1}`;
      // A plot named like its indicator (RSI's RSI) reads as the indicator.
      const name = lines.length === 1 || label === base ? title : label;
      return { ref: plotRef(key, line), key, line, label: `${title}: ${label}`, name };
    });
  });
}

/** Read-only text of a fixed source, for the edit dialog. */
function sourceText(src: AlertSource, symbol: string): string {
  switch (src.kind) {
    case "price":
      return symbol;
    case "drawing":
      return src.label;
    case "indicator": {
      const ag = src.against;
      if (ag.kind === "price") return `Close vs ${src.name}`;
      return ag.kind === "plot" ? `${src.name} vs ${ag.name}` : src.name;
    }
  }
}

export interface AlertDialogProps {
  /** The chart's live series: its indicators' plots can be alerted on. */
  series: SeriesPayload | null;
  /** Params label per instance key, e.g. "(14)" (see `paramLabels`). */
  params: ReadonlyMap<string, string | undefined>;
  /** Drawings of the chart symbol an alert can watch. */
  drawings: readonly { id: string; label: string; sloped: boolean }[];
}

export function AlertDialog(props: AlertDialogProps): JSX.Element {
  const [error, setError] = createSignal<string | null>(null);

  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.code !== "KeyA" || !e.altKey || e.ctrlKey || e.metaKey) return;
      if (isTypingContext(e)) return;
      e.preventDefault();
      openAlertDialog();
    };
    window.addEventListener("keydown", onKey);
    onCleanup(() => window.removeEventListener("keydown", onKey));
  });

  // Drawings and plots belong to the chart: offered only for its symbol.
  const onChart = () => draft()?.symbol === chartSymbol();
  // Live ticks replace the series every second: the options only change when
  // the plots do, so an open select is left alone. (The options also mark
  // themselves `selected`, in case they are re-created under a selection.)
  const plots = createMemo(
    () => (onChart() ? plotOptions(props.series, props.params) : []),
    [],
    {
      equals: (a, b) =>
        a.length === b.length && a.every((p, i) => p.ref === b[i]!.ref && p.label === b[i]!.label),
    },
  );
  const drawings = createMemo(() => {
    if (!onChart()) return [];
    // Same-named drawings are numbered, so they read apart.
    const seen = new Map<string, number>();
    return props.drawings.map((d) => {
      const n = (seen.get(d.label) ?? 0) + 1;
      seen.set(d.label, n);
      return { ...d, ref: drawingRef(d.id), label: n > 1 ? `${d.label} ${n}` : d.label };
    });
  });
  const sourcePlot = () => plots().find((p) => p.ref === draft()?.source);
  const levels = createMemo(() => {
    const d = draft();
    if (!d || d.source.startsWith("drawing:")) return [];
    const own = sourcePlot();
    if (d.source !== PRICE && !own) return [];
    return plots().filter((p) => (own ? p.key === own.key && p.line !== own.line : true));
  });
  /** What the value field is, or null when the level is not a value. */
  const valueLabel = () => {
    const d = draft();
    if (!d) return null;
    if (d.fixed) {
      const f = d.fixed;
      if (f.kind === "price") return "Price";
      return f.kind === "indicator" && f.against.kind === "value" ? "Value" : null;
    }
    if (d.level !== VALUE || d.source.startsWith("drawing:")) return null;
    return d.source === PRICE ? "Price" : "Value";
  };

  const close = () => {
    setDraft(null);
    setError(null);
  };

  const patch = (p: Partial<Draft>) => {
    setDraft((d) => (d ? { ...d, ...p } : d));
    setError(null);
  };

  const pickSource = (source: string) => {
    const plot = plots().find((p) => p.ref === source);
    // The value now, not when the options were built.
    const value = plot
      ? lastPlotValue(props.series, plot.key, plot.line)
      : lastPrice(draft()?.symbol ?? "");
    patch({
      source,
      level: VALUE,
      price: value === null ? "" : plot ? String(+value.toPrecision(6)) : priceText(value),
    });
  };

  /** The new alert's source from the selects; a string is an error. */
  const buildSource = (d: Draft, value: number): AlertSource | string => {
    const interval = props.series?.interval ?? "";
    if (d.source.startsWith("drawing:")) {
      const dr = drawings().find((x) => x.ref === d.source);
      if (!dr) return "That drawing is gone.";
      if (!dr.sloped) return { kind: "drawing", drawingId: dr.id, label: dr.label };
      // A sloped line is read on the bars of this interval (ADR-0034).
      if (interval === "") return "The chart has no interval to read the line on.";
      return { kind: "drawing", drawingId: dr.id, label: dr.label, interval };
    }
    const level = plots().find((p) => p.ref === d.level);
    const plot = d.source === PRICE ? level : sourcePlot();
    if (d.source === PRICE && !level) return { kind: "price" };
    const signature = plot && indicatorSignatures()[plot.key];
    if (!plot || !signature || interval === "") {
      return "That indicator is no longer on the chart.";
    }
    return {
      kind: "indicator",
      interval,
      instanceKey: plot.key,
      signature,
      line: plot.line,
      name: plot.name,
      against:
        d.source === PRICE
          ? { kind: "price" }
          : level
            ? { kind: "plot", line: level.line, name: level.name }
            : { kind: "value", value },
    };
  };

  const submit = (e: SubmitEvent) => {
    e.preventDefault();
    unlockAudio();
    const d = draft();
    if (!d) return;
    const label = valueLabel();
    const value = Number(d.price);
    if (label !== null && (d.price.trim() === "" || !Number.isFinite(value))) {
      setError(`Enter a ${label.toLowerCase()}.`);
      return;
    }
    if (label === "Price" && value <= 0) {
      setError("Enter a price greater than zero.");
      return;
    }
    if (d.id !== undefined) {
      updateAlert(d.id, {
        condition: d.condition,
        trigger: d.trigger,
        message: d.message,
        ...(label === "Price" ? { price: value } : label === "Value" ? { value } : {}),
      });
      close();
      return;
    }
    const source = buildSource(d, value);
    if (typeof source === "string") {
      setError(source);
      return;
    }
    const created = addAlert({
      symbol: d.symbol,
      condition: d.condition,
      trigger: d.trigger,
      message: d.message,
      ...(source.kind === "price" ? { price: value } : { source }),
    });
    if (created === null) {
      setError(`Alert limit reached (${MAX_ALERTS}).`);
      return;
    }
    requestNotificationPermission();
    showToast(`Alert created for ${d.symbol}`);
    close();
  };

  return (
    <Dialog.Root
      open={draft() !== null}
      onOpenChange={(open) => {
        if (!open) close();
      }}
      modal
    >
      <Dialog.Portal>
        <Dialog.Overlay class="fixed inset-0 z-40 bg-black/60" />
        <div class="fixed inset-0 z-50 flex items-start justify-center pt-[10vh] px-4">
          <Dialog.Content class="w-full max-w-md bg-[color:var(--color-bg)] border border-[color:var(--color-border)] rounded-lg shadow-xl">
            <Show when={draft()}>
              {(d) => (
                <form class="flex flex-col gap-3 p-4" onSubmit={submit}>
                  <Dialog.Title class="text-sm font-semibold">
                    {d().id === undefined ? "Create alert" : "Edit alert"} on{" "}
                    {d().symbol}
                  </Dialog.Title>
                  <Show
                    when={d().fixed}
                    fallback={
                      <label class="flex items-center justify-between gap-3 text-sm">
                        Source
                        <select
                          class={selectClass}
                          value={d().source}
                          onChange={(e) => pickSource(e.currentTarget.value)}
                        >
                          <option value={PRICE}>{d().symbol}</option>
                          <Show when={drawings().length > 0}>
                            <optgroup label="Drawings">
                              <For each={drawings()}>
                                {(o) => (
                                  <option value={o.ref} selected={o.ref === d().source}>
                                    {o.label}
                                  </option>
                                )}
                              </For>
                            </optgroup>
                          </Show>
                          <Show when={plots().length > 0}>
                            <optgroup label="Indicators">
                              <For each={plots()}>
                                {(o) => (
                                  <option value={o.ref} selected={o.ref === d().source}>
                                    {o.label}
                                  </option>
                                )}
                              </For>
                            </optgroup>
                          </Show>
                        </select>
                      </label>
                    }
                  >
                    {(f) => (
                      <div class="flex items-center justify-between gap-3 text-sm">
                        Source
                        <span data-role="alert-source" class="w-56 truncate text-right">
                          {sourceText(f(), d().symbol)}
                        </span>
                      </div>
                    )}
                  </Show>
                  <label class="flex items-center justify-between gap-3 text-sm">
                    Condition
                    <select
                      class={selectClass}
                      value={d().condition}
                      onChange={(e) =>
                        patch({ condition: e.currentTarget.value as AlertCondition })
                      }
                    >
                      <For each={CONDITIONS}>
                        {(c) => <option value={c}>{CONDITION_LABELS[c]}</option>}
                      </For>
                    </select>
                  </label>
                  <Show when={!d().fixed && levels().length > 0}>
                    <label class="flex items-center justify-between gap-3 text-sm">
                      Level
                      <select
                        class={selectClass}
                        value={d().level}
                        onChange={(e) => patch({ level: e.currentTarget.value })}
                      >
                        <option value={VALUE}>Value</option>
                        <For each={levels()}>
                          {(o) => (
                            <option value={o.ref} selected={o.ref === d().level}>
                              {o.label}
                            </option>
                          )}
                        </For>
                      </select>
                    </label>
                  </Show>
                  <Show when={valueLabel()}>
                    {(label) => (
                      <label class="flex items-center justify-between gap-3 text-sm">
                        {label()}
                        <input
                          class={inputClass}
                          inputmode="decimal"
                          autofocus
                          value={d().price}
                          onInput={(e) => patch({ price: e.currentTarget.value })}
                        />
                      </label>
                    )}
                  </Show>
                  <label class="flex items-center justify-between gap-3 text-sm">
                    Trigger
                    <select
                      class={selectClass}
                      value={d().trigger}
                      onChange={(e) =>
                        patch({ trigger: e.currentTarget.value as AlertTrigger })
                      }
                    >
                      <For each={TRIGGERS}>
                        {(t) => <option value={t}>{TRIGGER_LABELS[t]}</option>}
                      </For>
                    </select>
                  </label>
                  <label class="flex items-center justify-between gap-3 text-sm">
                    Message
                    <input
                      class={`${inputClass} text-left`}
                      maxlength="200"
                      placeholder="Optional"
                      value={d().message}
                      onInput={(e) => patch({ message: e.currentTarget.value })}
                    />
                  </label>
                  <Show when={error()}>
                    <p role="alert" class="text-xs text-[color:var(--color-danger)]">
                      {error()}
                    </p>
                  </Show>
                  <div class="flex justify-end gap-2">
                    <button type="button" class={buttonClass} onClick={close}>
                      Cancel
                    </button>
                    <button type="submit" class={buttonClass}>
                      {d().id === undefined ? "Create" : "Save"}
                    </button>
                  </div>
                </form>
              )}
            </Show>
          </Dialog.Content>
        </div>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
