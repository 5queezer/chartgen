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
//
// A price alert on a value can run on the server instead (ADR-0042), with no
// browser open and an optional webhook. Drawing and indicator alerts cannot:
// the dialog says so rather than offering it.

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
  createServerAlert,
  serverAlertError,
  serverAlerts,
  serverAlertsCanCreate,
  serverAlertsEnabled,
  serverWebhooksAllowed,
  updateServerAlert,
} from "../stores/serverAlerts";
import {
  requestNotificationPermission,
  showToast,
  unlockAudio,
} from "../services/alertEngine";

/** Value of the Source / Level select for a plot of an indicator instance. */
export const plotRef = (key: string, line: number) =>
  `plot:${JSON.stringify([key, line])}`;
/** Value of the Source select for a detection of an indicator (ADR-0046). */
export const eventRef = (key: string, event: string) =>
  `event:${JSON.stringify([key, event])}`;
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
  /** Run on the server (create only). */
  onServer: boolean;
  /** Set when editing a server alert. */
  serverId?: string;
  evaluateOn: "live" | "close";
  /** New webhook URL; empty keeps the current one when editing. */
  webhook: string;
  /** The webhook's host when editing a server alert that has one. */
  webhookHost?: string;
  sign: boolean;
}

const SERVER_DEFAULTS = {
  onServer: false,
  evaluateOn: "live",
  webhook: "",
  sign: false,
} as const;

const [draft, setDraft] = createSignal<Draft | null>(null);

/** Plain decimal text of a price, at the precision the watchlist shows. */
const priceText = (p: number) => String(+p.toFixed(priceDecimals(p)));

export interface OpenAlertOptions {
  /** Edit this alert; the other fields are ignored. */
  id?: string;
  /** Edit this server alert; the other fields are ignored. */
  serverId?: string;
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
  const onServer = serverAlerts().find((a) => a.id === opts.serverId);
  if (onServer) {
    setDraft({
      ...SERVER_DEFAULTS,
      serverId: onServer.id,
      symbol: onServer.symbol,
      source: PRICE,
      level: VALUE,
      fixed: { kind: "price" },
      condition: onServer.condition,
      price: priceText(onServer.level),
      trigger: onServer.trigger,
      message: onServer.message ?? "",
      evaluateOn: onServer.evaluate_on,
      webhookHost: onServer.webhook?.host,
    });
    return;
  }
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
      ...SERVER_DEFAULTS,
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
    ...SERVER_DEFAULTS,
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

/** A detection the dialog offers. */
interface EventOption {
  ref: string;
  key: string;
  event: string;
  /** Option text and name in descriptions, e.g. "Chart Patterns: Bullish pattern". */
  label: string;
}

/** Detections of the series' indicators, in legend order. */
function eventOptions(
  series: SeriesPayload | null,
  params: ReadonlyMap<string, string | undefined>,
): EventOption[] {
  return Object.entries(series?.indicators ?? {}).flatMap(([key, ind]) => {
    const p = params.get(key);
    const base = ind.label || keyName(key);
    const title = p ? `${base} ${p}` : base;
    return (ind.events ?? []).map((e) => ({
      ref: eventRef(key, e.key),
      key,
      event: e.key,
      label: `${title}: ${e.name}`,
    }));
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
      if (ag.kind === "event") return src.name;
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
  const [pending, setPending] = createSignal(false);
  /** A new webhook secret, shown once before the dialog closes. */
  const [secret, setSecret] = createSignal<string | null>(null);

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
  const events = createMemo(
    () => (onChart() ? eventOptions(props.series, props.params) : []),
    [],
    {
      equals: (a, b) =>
        a.length === b.length && a.every((e, i) => e.ref === b[i]!.ref && e.label === b[i]!.label),
    },
  );
  /** The draft watches a detection: no condition, level or value to choose. */
  const onEvent = () => {
    const d = draft();
    if (!d) return false;
    return d.fixed
      ? d.fixed.kind === "indicator" && d.fixed.against.kind === "event"
      : d.source.startsWith("event:");
  };
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
    if (!d || d.source.startsWith("drawing:") || d.source.startsWith("event:")) return [];
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
    if (d.level !== VALUE || d.source.startsWith("drawing:") || d.source.startsWith("event:")) {
      return null;
    }
    return d.source === PRICE ? "Price" : "Value";
  };

  const close = () => {
    setDraft(null);
    setError(null);
    setSecret(null);
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

  /** A plain price level: all a server alert can watch. */
  const canRunOnServer = () => {
    const d = draft();
    return d !== null && d.id === undefined && d.source === PRICE && d.level === VALUE;
  };
  const runsOnServer = () => {
    const d = draft();
    return d !== null && (d.serverId !== undefined || (d.onServer && canRunOnServer()));
  };

  /** Create or change the server alert in the draft; shows a new secret once. */
  const submitServer = async (d: Draft, level: number) => {
    const webhook = d.webhook.trim();
    const args = {
      condition: d.condition,
      level,
      trigger: d.trigger,
      message: d.message,
      ...(webhook !== "" ? { webhook_url: webhook } : {}),
      ...(d.sign ? { sign: true } : {}),
    };
    setPending(true);
    try {
      const saved =
        d.serverId !== undefined
          ? await updateServerAlert({ alert_id: d.serverId, ...args })
          : await createServerAlert({
              ...args,
              symbol: d.symbol,
              interval: props.series?.interval ?? "1m",
              evaluate_on: d.evaluateOn,
            });
      if (d.serverId === undefined) {
        requestNotificationPermission();
        showToast(`Server alert created for ${d.symbol}`);
      }
      if (saved.webhook_secret !== undefined) setSecret(saved.webhook_secret);
      else close();
    } catch (err) {
      setError(serverAlertError(err));
    } finally {
      setPending(false);
    }
  };

  /** The new alert's source from the selects; a string is an error. */
  const buildSource = (d: Draft, value: number): AlertSource | string => {
    const interval = props.series?.interval ?? "";
    if (d.source.startsWith("event:")) {
      const ev = events().find((x) => x.ref === d.source);
      const signature = ev && indicatorSignatures()[ev.key];
      if (!ev || !signature || interval === "") return "That indicator is no longer on the chart.";
      return {
        kind: "indicator",
        interval,
        instanceKey: ev.key,
        signature,
        line: 0,
        name: ev.label,
        against: { kind: "event", key: ev.event },
      };
    }
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
    if (runsOnServer()) {
      void submitServer(d, value);
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
      // A detection only has one way to fire (see `AlertAgainst`).
      condition: onEvent() ? "crossing_up" : d.condition,
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
            <Show when={secret()}>
              {(s) => (
                <div class="flex flex-col gap-3 p-4">
                  <Dialog.Title class="text-sm font-semibold">Webhook secret</Dialog.Title>
                  <p class="text-xs text-[color:var(--color-fg-muted)]">
                    Copy it now. It signs each delivery and is not shown again.
                  </p>
                  <code data-role="webhook-secret" class="text-xs break-all select-all">
                    {s()}
                  </code>
                  <div class="flex justify-end">
                    <button type="button" class={buttonClass} onClick={close}>
                      Done
                    </button>
                  </div>
                </div>
              )}
            </Show>
            <Show when={!secret() && draft()}>
              {(d) => (
                <form class="flex flex-col gap-3 p-4" onSubmit={submit}>
                  <Dialog.Title class="text-sm font-semibold">
                    {d().id === undefined && d().serverId === undefined
                      ? "Create alert"
                      : "Edit alert"}{" "}
                    on {d().symbol}
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
                          <Show when={events().length > 0}>
                            <optgroup label="Patterns">
                              <For each={events()}>
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
                  <Show
                    when={!onEvent()}
                    fallback={
                      <p
                        data-role="alert-event-note"
                        class="text-xs text-[color:var(--color-fg-muted)]"
                      >
                        Fires when a bar closes with a new detection. Nothing on the forming bar
                        fires it, and a pattern shown as forming has not fired.
                      </p>
                    }
                  >
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
                  </Show>
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
                  <Show when={d().id === undefined && d().serverId === undefined}>
                    <Show
                      when={canRunOnServer()}
                      fallback={
                        <p
                          data-role="alert-browser-only"
                          class="text-xs text-[color:var(--color-fg-muted)]"
                        >
                          Drawing and indicator alerts run in this browser, only while a tab is
                          open.
                        </p>
                      }
                    >
                      <Show
                        when={serverAlertsEnabled() && serverAlertsCanCreate()}
                        fallback={
                          <p
                            data-role="alert-browser-only"
                            class="text-xs text-[color:var(--color-fg-muted)]"
                          >
                            {serverAlertsEnabled()
                              ? "This server only runs alerts for sign-ins its operator allowed"
                              : "This server does not run alerts"}
                            : this one runs in this browser, only while a tab is open.
                          </p>
                        }
                      >
                        <label class="flex items-center justify-between gap-3 text-sm">
                          Run on server
                          <input
                            type="checkbox"
                            checked={d().onServer}
                            onChange={(e) => patch({ onServer: e.currentTarget.checked })}
                          />
                        </label>
                      </Show>
                    </Show>
                  </Show>
                  <Show when={runsOnServer()}>
                    <Show when={d().serverId === undefined}>
                      <label class="flex items-center justify-between gap-3 text-sm">
                        Check on
                        <select
                          class={selectClass}
                          value={d().evaluateOn}
                          onChange={(e) =>
                            patch({ evaluateOn: e.currentTarget.value as Draft["evaluateOn"] })
                          }
                        >
                          <option value="live">Every tick</option>
                          <option value="close">
                            Bar close ({props.series?.interval ?? "1m"})
                          </option>
                        </select>
                      </label>
                    </Show>
                    <Show
                      when={serverWebhooksAllowed()}
                      fallback={
                        <p
                          data-role="webhooks-off"
                          class="text-xs text-[color:var(--color-fg-muted)]"
                        >
                          This server does not send webhooks for this sign-in. The alert fires
                          here, in the Alerts tab.
                        </p>
                      }
                    >
                      <label class="flex items-center justify-between gap-3 text-sm">
                        Webhook
                        <input
                          class={`${inputClass} text-left`}
                          type="url"
                          maxlength="512"
                          autocomplete="off"
                          placeholder={
                            d().webhookHost ? `Keep ${d().webhookHost}` : "https:// (optional)"
                          }
                          value={d().webhook}
                          onInput={(e) => patch({ webhook: e.currentTarget.value })}
                        />
                      </label>
                      <label
                        class="flex items-center justify-between gap-3 text-sm"
                        title="A new URL does not keep the old secret: sign again to get a new one."
                      >
                        Sign webhook
                        <input
                          type="checkbox"
                          checked={d().sign}
                          onChange={(e) => patch({ sign: e.currentTarget.checked })}
                        />
                      </label>
                    </Show>
                  </Show>
                  <Show when={error()}>
                    <p role="alert" class="text-xs text-[color:var(--color-danger)]">
                      {error()}
                    </p>
                  </Show>
                  <div class="flex justify-end gap-2">
                    <button type="button" class={buttonClass} onClick={close}>
                      Cancel
                    </button>
                    <button type="submit" class={buttonClass} disabled={pending()}>
                      {d().id === undefined && d().serverId === undefined ? "Create" : "Save"}
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
