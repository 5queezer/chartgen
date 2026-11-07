// Chart-level conveniences: keyboard navigation, reset view, screenshot
// and fullscreen. The key mapping and file naming are pure; the rest
// drive a klinecharts instance.

import type { Chart as KLineChart } from "klinecharts";

import { invertScale, setAutoScale, setInvertScale } from "../stores/layout";
import { fitProjection } from "./projection";

export type ChartAction =
  | "scrollLeft"
  | "scrollRight"
  | "zoomIn"
  | "zoomOut"
  | "home"
  | "end"
  | "reset"
  | "invertScale";

/** klinecharts' default bar width in px; "reset chart" restores it. */
export const DEFAULT_BAR_SPACE = 8;
const ZOOM_STEP = 1.2;

/** Map a keystroke to a chart action; modifiers other than Alt+R and Alt+I disqualify. */
export function chartKeyAction(e: KeyboardEvent): ChartAction | null {
  if (e.altKey && !e.ctrlKey && !e.metaKey && e.code === "KeyR") return "reset";
  if (e.altKey && !e.ctrlKey && !e.metaKey && e.code === "KeyI") {
    return e.repeat ? null : "invertScale";
  }
  if (e.altKey || e.ctrlKey || e.metaKey) return null;
  switch (e.key) {
    case "ArrowLeft":
      return "scrollLeft";
    case "ArrowRight":
      return "scrollRight";
    case "ArrowUp":
    case "+":
    case "=":
      return "zoomIn";
    case "ArrowDown":
    case "-":
      return "zoomOut";
    case "Home":
      return "home";
    case "End":
      return "end";
    default:
      return null;
  }
}

export function applyChartAction(chart: KLineChart, action: ChartAction): void {
  const space = chart.getBarSpace();
  switch (action) {
    case "scrollLeft":
      chart.scrollByDistance(space);
      break;
    case "scrollRight":
      chart.scrollByDistance(-space);
      break;
    case "zoomIn":
      chart.setBarSpace(space * ZOOM_STEP);
      break;
    case "zoomOut":
      chart.setBarSpace(space / ZOOM_STEP);
      break;
    case "home": {
      // scrollToDataIndex puts the bar at the right edge; offset by the
      // visible count so bar 0 lands at the left edge instead.
      const { from, to } = chart.getVisibleRange();
      chart.scrollToDataIndex(Math.max(0, to - from - 1));
      break;
    }
    case "end":
      fitProjection(chart);
      chart.scrollToRealTime();
      break;
    case "reset":
      resetChart(chart);
      break;
    case "invertScale":
      setInvertScale(!invertScale());
      break;
  }
}

/** Time axis back to real time at default zoom. */
export function resetTimeScale(chart: KLineChart): void {
  chart.setBarSpace(DEFAULT_BAR_SPACE);
  fitProjection(chart);
  chart.scrollToRealTime();
}

/** Time axis back to real time at default zoom, and the price axis back to auto-fit. */
export function resetChart(chart: KLineChart, setAuto: (next: boolean) => void = setAutoScale): void {
  setAuto(true);
  resetTimeScale(chart);
}

const pad = (n: number) => String(n).padStart(2, "0");

/** `<SYMBOL>_<TF>_<yyyy-mm-dd_HHMM>.png`, local time, filesystem-safe. */
export function screenshotFilename(
  symbol: string,
  timeframe: string,
  at: Date,
): string {
  const safe = (s: string) => s.replace(/[^A-Za-z0-9.-]/g, "-");
  const date = `${at.getFullYear()}-${pad(at.getMonth() + 1)}-${pad(at.getDate())}`;
  const time = `${pad(at.getHours())}${pad(at.getMinutes())}`;
  return `${safe(symbol)}_${safe(timeframe)}_${date}_${time}.png`;
}

/** Download the chart (overlays included) as a PNG filled with `background`. */
export function downloadChartImage(
  chart: KLineChart,
  filename: string,
  background: string,
): void {
  const a = document.createElement("a");
  a.href = chart.getConvertPictureUrl(true, "png", background);
  a.download = filename;
  a.click();
}

/** Enter fullscreen on the app root, or leave it if already there. */
export function toggleFullscreen(): void {
  const request = document.fullscreenElement != null
    ? document.exitFullscreen()
    : document.documentElement.requestFullscreen();
  // Rejects when the browser or an embedding iframe refuses; nothing to do.
  request.catch(() => {});
}
