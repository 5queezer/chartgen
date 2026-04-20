// klinecharts wrapper. Every indicator in the SeriesPayload is rendered
// via a generic registration path: the server returns per-bar values in
// the payload, we stash them on each KLineData under per-indicator keys
// (`chartgen_<name>_line<i>` / `chartgen_<name>_hist<i>`), and we
// `registerIndicator` lazily for each name the first time we see it.
// The indicator's `calc` callback reads its stashed values back off the
// bars — no math runs in the browser.
//
// Pane lifecycle: we track created panes in a signal keyed by indicator
// name. When a new indicator shows up we `createIndicator`; when one
// drops out of the payload we `removeIndicator` and delete the entry.
//
// Tooltip: klinecharts's canvas-drawn candle tooltip is suppressed and
// the DOM PaneHeader becomes the single source of truth. When the
// crosshair is active the header shows the values at that bar index;
// otherwise it shows the last finite value. Handler is rAF-debounced.
//
// Visual styling is driven by ADR-0006 (TradingView visual parity).

import {
  createEffect,
  createSignal,
  For,
  onCleanup,
  onMount,
  type JSX,
} from "solid-js";
import {
  dispose,
  init,
  registerIndicator,
  type Chart as KChart,
  type KLineData,
} from "klinecharts";

import { indicatorName, type ActiveIndicator } from "../stores/indicators";
import type { SeriesPayload } from "../types/responses";
import { PaneHeader, type PaneHeaderLine } from "./PaneHeader";

// TradingView-aligned palette. Kept in JS as well as CSS because
// klinecharts draws to canvas and cannot resolve CSS variables.
const PALETTE = {
  bg: "#131722",
  surface: "#1e222d",
  border: "#2a2e39",
  fg: "#d1d4dc",
  fgDim: "#787b86",
  accent: "#2962ff",
  bull: "#26a69a",
  bear: "#ef5350",
} as const;

const FONT_FAMILY = "Inter, system-ui, sans-serif";

// TradingView-aligned rotating palette for indicator line/bar series.
// Color assignment is deterministic per (indicator name, series index)
// so a refresh doesn't reshuffle colors.
const TV_SERIES_PALETTE = [
  "#2962ff",
  "#ff6d00",
  "#00bcd4",
  "#e91e63",
  "#4caf50",
  "#ffeb3b",
  "#9c27b0",
  "#ff5252",
  "#607d8b",
  "#795548",
] as const;

function hashName(name: string): number {
  let sum = 0;
  for (let i = 0; i < name.length; i++) sum += name.charCodeAt(i);
  return sum;
}

function paletteColor(name: string, index: number): string {
  const base = hashName(name);
  return TV_SERIES_PALETTE[(base + index) % TV_SERIES_PALETTE.length]!;
}

function indicatorRegName(name: string): string {
  return `chartgen_${name}`;
}

function lineStashKey(name: string, i: number): string {
  return `chartgen_${name}_line${i}`;
}

function histStashKey(name: string, i: number): string {
  return `chartgen_${name}_hist${i}`;
}

const registeredIndicators = new Set<string>();

interface IndicatorShape {
  lines: number;
  hist: number;
}

/**
 * Register (once) a generic chartgen indicator for `name` with the
 * given figure shape. Subsequent calls with the same name are no-ops —
 * klinecharts's registry is global and re-registering throws.
 *
 * Each figure's `calc` just reads the stashed value back off the bar;
 * the Rust server has already done the math.
 */
function ensureRegistered(name: string, shape: IndicatorShape): void {
  const regName = indicatorRegName(name);
  if (registeredIndicators.has(regName)) return;
  registeredIndicators.add(regName);

  type Row = Record<string, number | null>;
  const figures: Array<{
    key: string;
    title: string;
    type: "line" | "bar";
    baseValue?: number;
    styles?: () => { color: string };
  }> = [];

  for (let i = 0; i < shape.lines; i++) {
    figures.push({
      key: `line${i}`,
      title: "",
      type: "line",
      styles: () => ({ color: paletteColor(name, i) }),
    });
  }
  for (let i = 0; i < shape.hist; i++) {
    // Histograms are placed after lines in the palette rotation so a
    // MACD-style (2 lines + 1 hist) keeps line0/line1/hist0 distinct.
    const colorIdx = shape.lines + i;
    figures.push({
      key: `hist${i}`,
      title: "",
      type: "bar",
      baseValue: 0,
      styles: () => ({ color: paletteColor(name, colorIdx) }),
    });
  }

  registerIndicator<Row>({
    name: regName,
    shortName: name.toUpperCase(),
    precision: 2,
    figures: figures as never,
    calc: (dataList: KLineData[]) =>
      dataList.map((bar) => {
        const row: Row = {};
        for (let i = 0; i < shape.lines; i++) {
          const raw = bar[lineStashKey(name, i)];
          row[`line${i}`] =
            typeof raw === "number" && Number.isFinite(raw) ? raw : null;
        }
        for (let i = 0; i < shape.hist; i++) {
          const raw = bar[histStashKey(name, i)];
          row[`hist${i}`] =
            typeof raw === "number" && Number.isFinite(raw) ? raw : null;
        }
        return row;
      }),
  });
}

function shapeFromPayload(
  series: SeriesPayload,
  name: string,
): IndicatorShape {
  const ind = series.indicators[name];
  if (!ind) return { lines: 0, hist: 0 };
  const lines = ind.lines?.length ?? 0;
  const hist = ind.histogram?.length ?? 0;
  return { lines, hist };
}

function seriesToKLines(series: SeriesPayload): KLineData[] {
  const names = Object.keys(series.indicators);
  const klines: KLineData[] = [];
  for (let i = 0; i < series.bars.length; i++) {
    const bar = series.bars[i];
    if (!bar || bar.t === null) continue;
    const kline: KLineData = {
      timestamp: bar.t * 1000,
      open: bar.o ?? 0,
      high: bar.h ?? 0,
      low: bar.l ?? 0,
      close: bar.c ?? 0,
      volume: bar.v ?? 0,
    };
    for (const name of names) {
      const ind = series.indicators[name];
      if (!ind) continue;
      const lines = ind.lines ?? [];
      for (let li = 0; li < lines.length; li++) {
        const values = lines[li]?.values;
        if (!values) continue;
        kline[lineStashKey(name, li)] = values[i] ?? null;
      }
      const hist = ind.histogram ?? [];
      for (let hi = 0; hi < hist.length; hi++) {
        const values = hist[hi]?.values;
        if (!values) continue;
        kline[histStashKey(name, hi)] = values[i] ?? null;
      }
    }
    klines.push(kline);
  }
  return klines;
}

function lastFinite(values: Array<number | null>): number | null {
  for (let i = values.length - 1; i >= 0; i--) {
    const v = values[i];
    if (typeof v === "number" && Number.isFinite(v)) return v;
  }
  return null;
}

function valueAt(
  values: Array<number | null>,
  index: number,
): number | null {
  const v = values[index];
  return typeof v === "number" && Number.isFinite(v) ? v : null;
}

/**
 * Compact K/M/B formatter used for y-axis ticks and the right-axis
 * volume readout. Two decimals above 10k; unchanged below.
 */
function formatBigNumber(value: string | number): string {
  const n = typeof value === "string" ? Number(value) : value;
  if (!Number.isFinite(n)) return String(value);
  const abs = Math.abs(n);
  if (abs >= 1_000_000_000) return `${(n / 1_000_000_000).toFixed(2)}B`;
  if (abs >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  if (abs >= 10_000) return `${(n / 1_000).toFixed(2)}K`;
  return String(value);
}

/**
 * Format an indicator's parameter object in the TradingView style,
 * e.g. `{ length: 14 }` → `(14)`. Only scalar values are included
 * (numbers, strings, booleans) — objects/arrays are skipped. The
 * `name` key itself is filtered out.
 */
function formatIndicatorParams(entry: ActiveIndicator): string | undefined {
  if (typeof entry === "string") return undefined;
  const parts: string[] = [];
  for (const k of Object.keys(entry)) {
    if (k === "name") continue;
    const v = entry[k];
    if (typeof v === "number" || typeof v === "string" || typeof v === "boolean") {
      parts.push(String(v));
    }
  }
  return parts.length > 0 ? `(${parts.join(", ")})` : undefined;
}

/** TradingView-style setStyles payload. Kept as a constant so future
 *  readers can see the whole styled surface in one place. */
const TV_STYLES = {
  grid: {
    show: true,
    horizontal: { show: true, color: PALETTE.border, size: 1 },
    vertical: { show: true, color: PALETTE.border, size: 1 },
  },
  candle: {
    bar: {
      upColor: PALETTE.bull,
      downColor: PALETTE.bear,
      noChangeColor: PALETTE.fgDim,
      upBorderColor: PALETTE.bull,
      downBorderColor: PALETTE.bear,
      noChangeBorderColor: PALETTE.fgDim,
      upWickColor: PALETTE.bull,
      downWickColor: PALETTE.bear,
      noChangeWickColor: PALETTE.fgDim,
    },
    priceMark: {
      show: true,
      high: { show: false, color: PALETTE.fg, textFamily: FONT_FAMILY },
      low: { show: false, color: PALETTE.fg, textFamily: FONT_FAMILY },
      last: {
        show: true,
        upColor: PALETTE.bull,
        downColor: PALETTE.bear,
        noChangeColor: PALETTE.accent,
        line: {
          show: true,
          style: "dashed",
          dashedValue: [4, 4],
          size: 1,
        },
        text: {
          show: true,
          size: 12,
          paddingLeft: 4,
          paddingRight: 4,
          paddingTop: 2,
          paddingBottom: 2,
          color: "#ffffff",
          family: FONT_FAMILY,
          weight: "500",
          borderSize: 0,
          borderColor: "transparent",
          borderRadius: 2,
        },
      },
    },
    // Suppress the canvas-drawn candle tooltip — the DOM PaneHeader
    // renders the readouts in a styled, consistent format.
    tooltip: {
      showRule: "none",
      showType: "standard",
    },
  },
  indicator: {
    // Same story on the indicator side: the PaneHeader overlay is the
    // single source of truth, so the canvas tooltip is silenced.
    tooltip: {
      showRule: "none",
      showName: false,
      showParams: false,
    },
    lastValueMark: {
      show: false,
    },
  },
  xAxis: {
    axisLine: { show: true, color: PALETTE.border, size: 1 },
    tickLine: { show: true, color: PALETTE.border, size: 1, length: 3 },
    tickText: {
      show: true,
      color: PALETTE.fgDim,
      family: FONT_FAMILY,
      weight: "400",
      size: 11,
    },
  },
  yAxis: {
    axisLine: { show: true, color: PALETTE.border, size: 1 },
    tickLine: { show: true, color: PALETTE.border, size: 1, length: 3 },
    tickText: {
      show: true,
      color: PALETTE.fgDim,
      family: FONT_FAMILY,
      weight: "400",
      size: 11,
    },
  },
  crosshair: {
    show: true,
    horizontal: {
      show: true,
      line: {
        show: true,
        style: "dashed",
        dashedValue: [4, 2],
        size: 1,
        color: PALETTE.fgDim,
      },
      text: {
        show: true,
        color: "#ffffff",
        size: 11,
        family: FONT_FAMILY,
        weight: "400",
        backgroundColor: PALETTE.surface,
        borderColor: PALETTE.border,
      },
    },
    vertical: {
      show: true,
      line: {
        show: true,
        style: "dashed",
        dashedValue: [4, 2],
        size: 1,
        color: PALETTE.fgDim,
      },
      text: {
        show: true,
        color: "#ffffff",
        size: 11,
        family: FONT_FAMILY,
        weight: "400",
        backgroundColor: PALETTE.surface,
        borderColor: PALETTE.border,
      },
    },
  },
  separator: {
    size: 1,
    color: PALETTE.border,
    fill: true,
    activeBackgroundColor: PALETTE.border,
  },
} as const;

const CANDLE_PANE_ID = "candle_pane";

interface HeaderState {
  name: string;
  params?: string;
  lines: PaneHeaderLine[];
  top: number;
}

export interface ChartProps {
  /** Current series payload; `null` before the first load. */
  series: SeriesPayload | null;
  /**
   * The active indicator registry from the store. Used to source
   * per-indicator parameter tuples for the header; the server's
   * payload only carries the indicator's `label`, not the params the
   * client asked for.
   */
  activeIndicators: ActiveIndicator[];
}

export function Chart(props: ChartProps): JSX.Element {
  let host!: HTMLDivElement;
  let container!: HTMLDivElement;
  const [chart, setChart] = createSignal<KChart | null>(null);
  // Map of indicator name → klinecharts pane id. Drives create/remove
  // decisions across effect runs.
  const [paneIds, setPaneIds] = createSignal<Record<string, string>>({});
  const [headers, setHeaders] = createSignal<HeaderState[]>([]);
  // Crosshair state drives the header readout. null = no crosshair
  // (resting state → show last finite values).
  const [crosshairIndex, setCrosshairIndex] = createSignal<number | null>(
    null,
  );
  let crosshairFrame: number | null = null;

  onMount(() => {
    const c = init(container, {
      // DeepPartial on purpose — we only override the leaves we care
      // about. klinecharts merges the rest from its default theme.
      styles: TV_STYLES as never,
      customApi: { formatBigNumber },
    });
    setChart(c);

    // Wire the crosshair subscription. klinecharts fires an event on
    // every mouse move; we debounce into a single rAF tick so Solid
    // doesn't re-render 60+ times a second for a pixel-level change.
    //
    // klinecharts only executes the onCrosshairChange action when the
    // crosshair is over a pane (it checks `isString(crosshair.paneId)`),
    // which means we never see an event when the mouse leaves the
    // chart. A mouseleave listener on the host clears the state.
    const onCrosshair = (data?: unknown) => {
      if (crosshairFrame !== null) return;
      const payload = data as
        | { dataIndex?: number; paneId?: string }
        | undefined;
      crosshairFrame = requestAnimationFrame(() => {
        crosshairFrame = null;
        if (!payload || typeof payload.dataIndex !== "number") {
          setCrosshairIndex(null);
        } else {
          setCrosshairIndex(payload.dataIndex);
        }
      });
    };
    // Use the string form of the action type to avoid pulling in the
    // runtime enum (TS-only import-elision friendly).
    c!.subscribeAction("onCrosshairChange" as never, onCrosshair);
    const onLeave = () => setCrosshairIndex(null);
    host.addEventListener("mouseleave", onLeave);
    onCleanup(() => {
      c!.unsubscribeAction("onCrosshairChange" as never, onCrosshair);
      host.removeEventListener("mouseleave", onLeave);
      if (crosshairFrame !== null) cancelAnimationFrame(crosshairFrame);
    });

    // Re-layout pane headers when the chart resizes. A ResizeObserver
    // on the host catches both viewport changes and indicator-pane
    // toggles.
    if (host && typeof ResizeObserver !== "undefined") {
      const ro = new ResizeObserver(() => recomputeHeaders());
      ro.observe(host);
      onCleanup(() => ro.disconnect());
    }
  });

  const recomputeHeaders = () => {
    const c = chart();
    const series = props.series;
    if (!c || !series) {
      setHeaders([]);
      return;
    }
    const next: HeaderState[] = [];
    const xi = crosshairIndex();

    // Price pane header. The rest-state shows symbol + last close;
    // when the crosshair is active, show OHLCV at that bar.
    const bars = series.bars;
    const closes = bars.map((b) => b.c);
    let priceLines: PaneHeaderLine[];
    if (xi !== null && xi >= 0 && xi < bars.length) {
      const bar = bars[xi]!;
      priceLines = [
        { label: "O", value: bar.o, color: PALETTE.fg },
        { label: "H", value: bar.h, color: PALETTE.fg },
        { label: "L", value: bar.l, color: PALETTE.fg },
        { label: "C", value: bar.c, color: PALETTE.fg },
        { label: "V", value: bar.v, color: PALETTE.fgDim },
      ];
    } else {
      priceLines = [
        { label: "close", value: lastFinite(closes), color: PALETTE.fg },
      ];
    }
    next.push({
      name: series.symbol ?? "price",
      params: series.interval ? `· ${series.interval}` : undefined,
      lines: priceLines,
      top: 4,
    });

    // Per-indicator headers. Overlays stack below the price-pane row
    // on the candle pane; oscillators anchor at their pane's top.
    const paneMap = paneIds();
    const paramsByName = new Map<string, string | undefined>();
    for (const entry of props.activeIndicators) {
      paramsByName.set(indicatorName(entry), formatIndicatorParams(entry));
    }
    let overlayRow = 0;

    for (const name of Object.keys(series.indicators)) {
      const ind = series.indicators[name]!;
      if (!paneMap[name]) continue; // pane not yet created (or skipped)

      const lineEntries = ind.lines ?? [];
      const histEntries = ind.histogram ?? [];
      const headerLines: PaneHeaderLine[] = [];
      for (let i = 0; i < lineEntries.length; i++) {
        const line = lineEntries[i]!;
        const value =
          xi !== null ? valueAt(line.values, xi) : lastFinite(line.values);
        headerLines.push({
          label: line.label ?? `line${i}`,
          value,
          color: paletteColor(name, i),
        });
      }
      for (let i = 0; i < histEntries.length; i++) {
        const hist = histEntries[i]!;
        const value =
          xi !== null ? valueAt(hist.values, xi) : lastFinite(hist.values);
        headerLines.push({
          label: `hist${i}`,
          value,
          color: paletteColor(name, lineEntries.length + i),
        });
      }
      if (headerLines.length === 0) continue;

      const displayName = ind.label && ind.label.length > 0 ? ind.label : name;
      const params = paramsByName.get(name);

      if (ind.is_overlay) {
        overlayRow += 1;
        // 20px per row below the 4px price-header row — keeps overlays
        // readable without a DOM-measured layout pass.
        const top = 4 + 20 * overlayRow;
        next.push({ name: displayName, params, lines: headerLines, top });
      } else {
        const size = c.getSize(paneMap[name]!);
        const top = size ? size.top + 4 : 0;
        next.push({ name: displayName, params, lines: headerLines, top });
      }
    }

    setHeaders(next);
  };

  // React to crosshair changes by recomputing header values only.
  createEffect(() => {
    crosshairIndex();
    recomputeHeaders();
  });

  // Feed new data and manage the indicator pane lifecycle.
  createEffect(() => {
    const c = chart();
    const series = props.series;
    if (!c || !series) return;

    // 1. Register any indicators we haven't seen before. Must happen
    //    before applyNewData so klinecharts's calc pipeline has the
    //    figure config to draw against.
    const names = Object.keys(series.indicators);
    for (const name of names) {
      const shape = shapeFromPayload(series, name);
      if (shape.lines === 0 && shape.hist === 0) continue;
      ensureRegistered(name, shape);
      const ind = series.indicators[name]!;
      if (
        (ind.fills && ind.fills.length > 0) ||
        (ind.hbars && ind.hbars.length > 0) ||
        (ind.divlines && ind.divlines.length > 0)
      ) {
        console.debug(
          `[chartgen] indicator "${name}" has fills/hbars/divlines; ` +
            "not rendered in this PR (tracked for PR γ).",
        );
      }
    }

    // 2. Push bar data with all per-indicator stashes applied.
    const klines = seriesToKLines(series);
    c.applyNewData(klines);

    // 3. Pane lifecycle. Create for new indicators, remove for dropped.
    const currentPanes = paneIds();
    const nextPanes: Record<string, string> = {};
    const activeNames = new Set<string>();
    for (const name of names) {
      const shape = shapeFromPayload(series, name);
      if (shape.lines === 0 && shape.hist === 0) continue; // skip empty
      activeNames.add(name);
      const regName = indicatorRegName(name);
      if (currentPanes[name]) {
        nextPanes[name] = currentPanes[name]!;
        continue;
      }
      const ind = series.indicators[name]!;
      // Overlays attach to the candle pane; oscillators get their own
      // pane (isStack=true stacks panes bottom-up).
      const paneOptions = ind.is_overlay
        ? { id: CANDLE_PANE_ID }
        : { height: 120 };
      const newPaneId = c.createIndicator(regName, true, paneOptions);
      if (newPaneId !== null) nextPanes[name] = newPaneId;
    }
    for (const name of Object.keys(currentPanes)) {
      if (!activeNames.has(name)) {
        c.removeIndicator(currentPanes[name]!, indicatorRegName(name));
      }
    }
    setPaneIds(nextPanes);

    // klinecharts lays out panes synchronously inside createIndicator,
    // but getSize reads through the renderer — one microtask is enough
    // for pane heights to settle before we position the DOM headers.
    queueMicrotask(recomputeHeaders);
  });

  onCleanup(() => {
    if (container) dispose(container);
  });

  return (
    <div ref={host} class="chart-host">
      <div ref={container} class="chart-container" />
      <For each={headers()}>
        {(h) => (
          <PaneHeader
            name={h.name}
            params={h.params}
            lines={h.lines}
            top={h.top}
          />
        )}
      </For>
    </div>
  );
}
