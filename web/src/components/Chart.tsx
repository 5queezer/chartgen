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

import {
  createEffect,
  createSignal,
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

const RSI_INDICATOR_NAME = "rsi_chartgen";
const RSI_STASH_KEY = "chartgen_rsi";

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
    figures: [{ key: "rsi", title: "RSI: ", type: "line" }],
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

export interface ChartProps {
  /** Current series payload; `null` before the first load. */
  series: SeriesPayload | null;
}

export function Chart(props: ChartProps): JSX.Element {
  let container!: HTMLDivElement;
  const [chart, setChart] = createSignal<KChart | null>(null);
  const [rsiPaneId, setRsiPaneId] = createSignal<string | null>(null);

  onMount(() => {
    ensureRsiRegistered();
    const c = init(container, {
      styles: {
        // Minimal dark theme overrides. klinecharts's default dark theme
        // is shipped via the `dark` preset applied in its CSS-less
        // options; we set background + grid colors to align with our
        // tailwind tokens.
        grid: {
          horizontal: { color: "#2a2a2a" },
          vertical: { color: "#2a2a2a" },
        },
        candle: {
          bar: {
            upColor: "#26a69a",
            downColor: "#ef5350",
            noChangeColor: "#888888",
            upBorderColor: "#26a69a",
            downBorderColor: "#ef5350",
            noChangeBorderColor: "#888888",
            upWickColor: "#26a69a",
            downWickColor: "#ef5350",
            noChangeWickColor: "#888888",
          },
        },
      },
    });
    setChart(c);
  });

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
  });

  onCleanup(() => {
    if (container) dispose(container);
  });

  return <div ref={container} class="chart-container" />;
}
