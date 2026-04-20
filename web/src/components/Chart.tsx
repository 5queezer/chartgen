// klinecharts wrapper. The interesting piece is the RSI indicator: we
// don't recompute RSI in the browser — chartgen already sends per-bar
// values. Instead, we register a custom `rsi_chartgen` indicator whose
// `calc` callback reads values stashed on each KLineData (via an index
// signature). Pre-warmup null values turn into gaps.
//
// The PR #66 review called out that tearing down the indicator on
// every data change causes flicker; instead we `applyNewData` and let
// klinecharts re-trigger the registered calc automatically, and only
// `createIndicator`/`removeIndicator` when the RSI pane should appear
// or disappear.
//
// Visual styling is driven by ADR-0006 (TradingView visual parity).
// The style keys set below live under the following hierarchy (see
// klinecharts `Styles` interface): grid.{horizontal,vertical}, candle.{
// bar, priceMark.last, tooltip}, indicator.{lines, tooltip}, xAxis.{
// axisLine, tickLine, tickText}, yAxis.{axisLine, tickLine, tickText},
// crosshair.{horizontal, vertical}, separator. `setCustomApi` provides
// `formatBigNumber` for K/M y-axis ticks.

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

import type { SeriesPayload } from "../types/responses";
import { PaneHeader, type PaneHeaderLine } from "./PaneHeader";

const RSI_INDICATOR_NAME = "rsi_chartgen";
const RSI_STASH_KEY = "chartgen_rsi";

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

// Default line color for the chartgen RSI series. Matches TradingView's
// RSI purple so the header-strip color chip reads correctly.
const RSI_LINE_COLOR = "#b388ff";

let registered = false;

/**
 * Register once at module load. `calc` reads the stashed value for each
 * bar; klinecharts handles the NaN-to-gap mapping.
 */
function ensureRsiRegistered(): void {
  if (registered) return;
  registered = true;
  registerIndicator<{ rsi: number | null }>({
    name: RSI_INDICATOR_NAME,
    shortName: "RSI",
    precision: 2,
    figures: [
      {
        key: "rsi",
        title: "RSI: ",
        type: "line",
        styles: () => ({ color: RSI_LINE_COLOR }),
      },
    ],
    calc: (dataList: KLineData[]) =>
      dataList.map((bar) => {
        const raw = bar[RSI_STASH_KEY];
        const v = typeof raw === "number" && Number.isFinite(raw) ? raw : null;
        return { rsi: v };
      }),
  });
}

function seriesToKLines(series: SeriesPayload): KLineData[] {
  const rsi = series.indicators["rsi"];
  const rsiValues =
    rsi?.lines?.[0]?.values ?? new Array<number | null>(series.bars.length);

  const klines: KLineData[] = [];
  for (let i = 0; i < series.bars.length; i++) {
    const bar = series.bars[i];
    if (!bar || bar.t === null) continue;
    const o = bar.o ?? 0;
    const h = bar.h ?? 0;
    const l = bar.l ?? 0;
    const c = bar.c ?? 0;
    const v = bar.v ?? 0;
    const stashed = rsiValues[i] ?? null;
    klines.push({
      timestamp: bar.t * 1000,
      open: o,
      high: h,
      low: l,
      close: c,
      volume: v,
      [RSI_STASH_KEY]: stashed,
    });
  }
  return klines;
}

function hasRsiData(series: SeriesPayload | null | undefined): boolean {
  if (!series) return false;
  const rsi = series.indicators["rsi"];
  if (!rsi) return false;
  const values = rsi.lines?.[0]?.values;
  return Array.isArray(values) && values.some((v) => v !== null);
}

function lastFinite(values: Array<number | null>): number | null {
  for (let i = values.length - 1; i >= 0; i--) {
    const v = values[i];
    if (typeof v === "number" && Number.isFinite(v)) return v;
  }
  return null;
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
      high: {
        show: false,
        color: PALETTE.fg,
        textFamily: FONT_FAMILY,
      },
      low: {
        show: false,
        color: PALETTE.fg,
        textFamily: FONT_FAMILY,
      },
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
    tooltip: {
      // Keep the crosshair tooltip visible but style it to match.
      showRule: "follow_cross",
      showType: "standard",
      text: {
        size: 12,
        family: FONT_FAMILY,
        color: PALETTE.fg,
        weight: "400",
        marginLeft: 8,
        marginTop: 6,
        marginRight: 8,
        marginBottom: 2,
      },
    },
  },
  indicator: {
    // Per-pane indicator values are rendered as Solid components in the
    // PaneHeader overlay. Suppress klinecharts's own canvas-drawn
    // indicator tooltip so the two don't stack.
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

interface HeaderState {
  name: string;
  params?: string;
  lines: PaneHeaderLine[];
  top: number;
}

export interface ChartProps {
  /** Current series payload; `null` before the first load. */
  series: SeriesPayload | null;
}

export function Chart(props: ChartProps): JSX.Element {
  let host!: HTMLDivElement;
  let container!: HTMLDivElement;
  const [chart, setChart] = createSignal<KChart | null>(null);
  const [rsiPaneId, setRsiPaneId] = createSignal<string | null>(null);
  const [headers, setHeaders] = createSignal<HeaderState[]>([]);

  onMount(() => {
    ensureRsiRegistered();
    const c = init(container, {
      // DeepPartial on purpose — we only override the leaves we care
      // about. klinecharts merges the rest from its default theme.
      styles: TV_STYLES as never,
      customApi: { formatBigNumber },
    });
    setChart(c);

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
    if (!c || !props.series) {
      setHeaders([]);
      return;
    }
    const next: HeaderState[] = [];

    // Price pane header: symbol + last close. Always pane id = "candle_pane".
    const series = props.series;
    const closes = series.bars.map((b) => b.c);
    const lastClose = lastFinite(closes);
    const candleTop = 4;
    next.push({
      name: series.symbol ?? "price",
      params: series.interval ? `· ${series.interval}` : undefined,
      lines: [
        {
          label: "close",
          value: lastClose,
          color: PALETTE.fg,
        },
      ],
      top: candleTop,
    });

    // RSI pane header, if the pane exists.
    const paneId = rsiPaneId();
    if (paneId !== null) {
      const rsiSize = c.getSize(paneId);
      const rsi = series.indicators["rsi"];
      const values = rsi?.lines?.[0]?.values ?? [];
      next.push({
        name: "RSI",
        params: "(14)",
        lines: [
          {
            label: "rsi",
            value: lastFinite(values),
            color: RSI_LINE_COLOR,
          },
        ],
        top: rsiSize ? rsiSize.top + 4 : 0,
      });
    }

    setHeaders(next);
  };

  // Feed new data as the prop changes.
  createEffect(() => {
    const c = chart();
    const series = props.series;
    if (!c || !series) return;

    const klines = seriesToKLines(series);
    c.applyNewData(klines);

    const shouldShowRsi = hasRsiData(series);
    const currentPane = rsiPaneId();
    if (shouldShowRsi && currentPane === null) {
      const paneId = c.createIndicator(RSI_INDICATOR_NAME, false, {
        height: 120,
      });
      setRsiPaneId(paneId);
    } else if (!shouldShowRsi && currentPane !== null) {
      c.removeIndicator(currentPane, RSI_INDICATOR_NAME);
      setRsiPaneId(null);
    }

    // applyNewData + createIndicator are synchronous; recompute header
    // positions on the next frame so klinecharts has laid out panes.
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
