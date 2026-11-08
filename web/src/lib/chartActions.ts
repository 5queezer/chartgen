// Chart-level conveniences: keyboard navigation, reset view, screenshot
// and fullscreen. The key mapping, file naming and screenshot caption are
// pure; the rest drive a klinecharts instance.

import { DomPosition, type Chart as KLineChart } from "klinecharts";

import {
  indicatorName,
  instanceKey,
  isHidden,
  paramLabels,
  type ActiveIndicator,
} from "../stores/indicators";
import { invertScale, setAutoScale, setInvertScale } from "../stores/layout";
import type { ParamDefaults } from "../stores/layouts";
import { chipName } from "./indicatorNames";
import { fitProjection } from "./projection";
import { FONT_FAMILY } from "./theme";
import { timeframeLabel, type Timeframe } from "./timeframe";

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

const SEPARATOR = " · ";

export interface ScreenshotCaption {
  /** `BTCUSDT · 1h`, the timeframe as the toolbar writes it. */
  title: string;
  /** Short names of the visible indicators, in legend order. */
  indicators: string[];
}

/** The header of a screenshot: symbol, timeframe and the short names of the visible indicators. */
export function screenshotCaption(
  symbol: string,
  timeframe: Timeframe,
  entries: readonly ActiveIndicator[],
  defaults: ParamDefaults,
): ScreenshotCaption {
  // Labelled over all entries, hidden ones too, so repeated instances read apart as in the picker and dialogs.
  const labels = paramLabels(entries, defaults, true);
  return {
    title: `${symbol}${SEPARATOR}${timeframeLabel(timeframe)}`,
    indicators: entries
      .filter((e) => !isHidden(e))
      .map((e) => chipName(indicatorName(e), labels.get(instanceKey(e)))),
  };
}

/** `items` joined by " · " into lines no wider than `maxWidth`; an item is never split. */
export function wrapJoined(
  items: readonly string[],
  maxWidth: number,
  measure: (text: string) => number,
): string[] {
  const lines: string[] = [];
  for (const item of items) {
    const last = lines.at(-1);
    const joined = `${last}${SEPARATOR}${item}`;
    if (last === undefined || measure(joined) > maxWidth) lines.push(item);
    else lines[lines.length - 1] = joined;
  }
  return lines;
}

/** Fill and text colors of a screenshot. */
export interface ScreenshotColors {
  background: string;
  fg: string;
  fgMuted: string;
}

// Header layout, in CSS px.
const PAD = 12;
const TITLE_LINE = 22;
const INDICATOR_LINE = 17;

/**
 * Download the chart (overlays included) as a PNG filled with
 * `colors.background`, under a header with the caption, and with the site's
 * host in the bottom-right corner of the plot area.
 */
export async function downloadChartImage(
  chart: KLineChart,
  filename: string,
  caption: ScreenshotCaption,
  colors: ScreenshotColors,
): Promise<void> {
  // klinecharts renders the picture at devicePixelRatio; the text is drawn at it too.
  const img = new Image();
  img.src = chart.getConvertPictureUrl(true, "png", colors.background);
  await img.decode();
  const dpr = window.devicePixelRatio || 1;
  const width = img.naturalWidth / dpr;
  const height = img.naturalHeight / dpr;

  const canvas = document.createElement("canvas");
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  const titleFont = `600 15px ${FONT_FAMILY}`;
  const indicatorFont = `12px ${FONT_FAMILY}`;
  ctx.font = indicatorFont;
  const lines = wrapJoined(caption.indicators, width - 2 * PAD, (t) => ctx.measureText(t).width);
  const header = PAD + TITLE_LINE + lines.length * INDICATOR_LINE + PAD / 2;

  // Resizing resets the context, so the scale and fonts are set after it.
  canvas.width = img.naturalWidth;
  canvas.height = img.naturalHeight + Math.round(header * dpr);
  ctx.scale(dpr, dpr);
  ctx.fillStyle = colors.background;
  ctx.fillRect(0, 0, width, header + height);
  ctx.drawImage(img, 0, header, width, height);

  ctx.textBaseline = "middle";
  ctx.fillStyle = colors.fg;
  ctx.font = titleFont;
  ctx.fillText(caption.title, PAD, PAD + TITLE_LINE / 2);
  ctx.fillStyle = colors.fgMuted;
  ctx.font = indicatorFont;
  lines.forEach((line, i) =>
    ctx.fillText(line, PAD, PAD + TITLE_LINE + (i + 0.5) * INDICATOR_LINE),
  );

  // Above the time axis, left of the price axis.
  const plot = chart.getSize("candle_pane", DomPosition.Main);
  const xAxis = chart.getSize("x_axis_pane");
  ctx.font = `11px ${FONT_FAMILY}`;
  ctx.textAlign = "right";
  ctx.textBaseline = "bottom";
  ctx.fillText(
    location.host,
    (plot ? plot.left + plot.width : width) - PAD / 2,
    header + (xAxis?.top ?? height) - PAD / 2,
  );

  const a = document.createElement("a");
  a.href = canvas.toDataURL("image/png");
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
