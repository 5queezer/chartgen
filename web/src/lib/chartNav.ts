// View positioning for the bottom bar: fit a time range into the chart and
// center a bar. Drives a klinecharts instance; the loaded data is expected
// to cover the target already (liveFeed.loadUntil).

import type { Chart as KLineChart } from "klinecharts";

import { fitProjection, projectionBarSpace } from "./projection";

const CANDLE_PANE_ID = "candle_pane";
/** klinecharts clamps the bar width to this range (silently ignores others). */
const MIN_BAR_SPACE = 1;
const MAX_BAR_SPACE = 50;

const paneWidth = (chart: KLineChart) =>
  chart.getSize(CANDLE_PANE_ID, "main" as never)?.width ?? 0;

/**
 * Size the bars so everything from `startMs` on (null: all loaded bars)
 * fills the width, and scroll to the newest bar. A range with more bars
 * than pixels shows its newest part at the minimum bar width.
 */
export function fitRange(chart: KLineChart, startMs: number | null): void {
  const list = chart.getDataList();
  const first = startMs === null ? 0 : list.findIndex((d) => d.timestamp >= startMs);
  const count = first === -1 ? 0 : list.length - first;
  const usable = paneWidth(chart) - chart.getOffsetRightDistance();
  // With a projection (ADR-0033) its margin is part of the fit.
  const space = projectionBarSpace(chart, count, paneWidth(chart)) ?? (count > 0 ? usable / count : 0);
  if (count > 0 && space > 0) {
    chart.setBarSpace(Math.min(MAX_BAR_SPACE, Math.max(MIN_BAR_SPACE, space)));
  }
  fitProjection(chart);
  chart.scrollToRealTime();
}

/** Scroll so the bar nearest to `ms` sits in the middle of the chart. */
export function centerOnTime(chart: KLineChart, ms: number): void {
  chart.scrollToTimestamp(ms);
  // scrollToTimestamp puts the bar at the right edge, past the offset.
  chart.scrollByDistance(-(paneWidth(chart) / 2 - chart.getOffsetRightDistance()));
}
