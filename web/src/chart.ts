/**
 * klinecharts wiring.
 *
 * Maps chartgen's `format=series` payload onto klinecharts' KLineData and a
 * secondary RSI pane rendered from chartgen's own values (not klinecharts'
 * built-in calc). Pre-warmup nulls in the RSI array are emitted as `{}`
 * so klinecharts leaves a gap.
 */

import {
  init,
  dispose,
  registerIndicator,
  type Chart,
  type KLineData,
} from 'klinecharts';
import type { SeriesPayload } from './types.js';

const RSI_INDICATOR_NAME = 'ChartgenRSI';
const RSI_PANE_ID = 'chartgen-rsi';

let rsiRegistered = false;

/**
 * Register a custom indicator that pulls RSI from a `rsi` field stashed on
 * each KLineData bar. Idempotent — klinecharts registers indicators on a
 * global registry, so we only do this once per process.
 */
function ensureRsiIndicator(): void {
  if (rsiRegistered) return;
  registerIndicator<{ rsi?: number }>({
    name: RSI_INDICATOR_NAME,
    shortName: 'RSI',
    figures: [
      { key: 'rsi', title: 'RSI: ', type: 'line' },
    ],
    minValue: 0,
    maxValue: 100,
    calc: (dataList) =>
      dataList.map((bar) => {
        const raw = (bar as unknown as { rsi?: number | null }).rsi;
        if (raw === null || raw === undefined || !Number.isFinite(raw)) {
          return {};
        }
        return { rsi: raw };
      }),
  });
  rsiRegistered = true;
}

export interface KlineController {
  render(payload: SeriesPayload): void;
  dispose(): void;
}

export function createKlineChart(container: HTMLElement): KlineController {
  ensureRsiIndicator();

  const chart: Chart | null = init(container, {
    styles: darkStyles(),
  });
  if (!chart) {
    throw new Error('klinecharts failed to initialize');
  }

  let rsiPaneId: string | null = null;

  return {
    render(payload) {
      const bars = payload.bars;
      const rsiValues: Array<number | null> | undefined =
        payload.indicators?.['rsi']?.lines?.[0]?.values;

      const klines: KLineData[] = bars.map((b, i) => {
        const k: KLineData = {
          timestamp: b.t * 1000, // chartgen emits unix seconds
          open: b.o ?? NaN,
          high: b.h ?? NaN,
          low: b.l ?? NaN,
          close: b.c ?? NaN,
          volume: b.v ?? 0,
        };
        if (rsiValues) {
          const v = rsiValues[i];
          if (v !== null && v !== undefined) {
            (k as unknown as Record<string, unknown>).rsi = v;
          }
        }
        return k;
      });

      chart.applyNewData(klines);

      if (rsiPaneId) {
        chart.removeIndicator(rsiPaneId);
        rsiPaneId = null;
      }
      if (rsiValues) {
        const id = chart.createIndicator(
          { name: RSI_INDICATOR_NAME },
          false,
          { id: RSI_PANE_ID, height: 120 },
        );
        rsiPaneId = id ?? RSI_PANE_ID;
      }
    },

    dispose() {
      dispose(container);
    },
  };
}

/** Minimal dark palette — avoids pulling a full theming layer for v0. */
function darkStyles() {
  return {
    grid: {
      horizontal: { color: '#1e2431' },
      vertical: { color: '#1e2431' },
    },
    candle: {
      bar: {
        upColor: '#26a69a',
        downColor: '#ef5350',
        noChangeColor: '#8a93a6',
      },
      tooltip: { text: { color: '#e6e9ef' } },
      priceMark: {
        last: {
          upColor: '#26a69a',
          downColor: '#ef5350',
          noChangeColor: '#8a93a6',
          text: { color: '#ffffff' },
        },
      },
    },
    xAxis: {
      axisLine: { color: '#1e2431' },
      tickLine: { color: '#1e2431' },
      tickText: { color: '#8a93a6' },
    },
    yAxis: {
      axisLine: { color: '#1e2431' },
      tickLine: { color: '#1e2431' },
      tickText: { color: '#8a93a6' },
    },
    crosshair: {
      horizontal: {
        line: { color: '#4f8cff' },
        text: { backgroundColor: '#4f8cff' },
      },
      vertical: {
        line: { color: '#4f8cff' },
        text: { backgroundColor: '#4f8cff' },
      },
    },
    separator: { color: '#1e2431' },
  } as const;
}
