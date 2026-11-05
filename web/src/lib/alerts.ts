// Alert model and evaluation (ADR-0020; sources: ADR-0034). Pure: no Solid,
// no storage.
//
// Semantics follow TradingView:
//   crossing        prev on one side of the level, now on the other or on it
//   crossing_up     prev below the level, now at or above it
//   crossing_down   prev above the level, now at or below it
//   greater / less  the price is above / below the level (state, not event)
//
// Re-arming (trigger "every"): a crossing alert needs no extra rule, because
// firing again requires the price to leave the level and cross it anew. The
// state conditions would fire on every tick while satisfied, so after firing
// they stay disarmed until the price is back on the other side (greater: at
// or below the level; less: at or above it). `rearmedReading` is that test.
//
// Sources (ADR-0034): an alert compares a left operand with a level. A price
// alert compares the price with `price`; a drawing alert the price with the
// drawing's line value at the current bar (two lines for a parallel
// channel); an indicator alert a plot with a value or another plot of the
// same indicator, or the price with a plot. Every source reduces to a
// `Reading`, the signed distances of the left operand from each level, and
// `fires` evaluates the condition on two consecutive readings. With one
// level that is exactly the rule above; with several, crossing any level
// fires, "greater" means above all of them and "less" below all of them.

import type { DrawingName, DrawingRecord, TimeAxis } from "../stores/drawings";

export const CONDITIONS = [
  "crossing",
  "crossing_up",
  "crossing_down",
  "greater",
  "less",
] as const;
export type AlertCondition = (typeof CONDITIONS)[number];

export const TRIGGERS = ["once", "every"] as const;
export type AlertTrigger = (typeof TRIGGERS)[number];

/** The right operand of an indicator alert. */
export type AlertAgainst =
  | { kind: "value"; value: number }
  | { kind: "plot"; line: number; name: string }
  /** The price is the left operand and the plot the level: "Close crossing EMA 200". */
  | { kind: "price" };

/** What an alert watches. A record without one is a price alert. */
export type AlertSource =
  | { kind: "price" }
  | {
      kind: "drawing";
      /** Record id of the drawing, on the alert's symbol. */
      drawingId: string;
      /** Display name at creation, e.g. "Trend line". */
      label: string;
      /**
       * Set on sloped lines: the chart interval they were created on. A
       * sloped line runs straight in bar-index space, so it is only
       * evaluated while the chart shows the symbol at this interval.
       * Horizontal lines have none and are evaluated anywhere.
       */
      interval?: string;
    }
  | {
      kind: "indicator";
      /** Evaluated only while the chart shows the symbol at this interval. */
      interval: string;
      /** Instance key in the series payload (ADR-0028). */
      instanceKey: string;
      /**
       * Name and params of the instance at creation (`indicatorSignature`):
       * a key reused by another instance, or changed params, do not match.
       */
      signature: string;
      /** Index into the indicator's `lines`. */
      line: number;
      /** Display name of the plot at creation, e.g. "RSI (14)" or "EMA 200". */
      name: string;
      against: AlertAgainst;
    };

export type DrawingAlertTool = Extract<
  DrawingName,
  | "segment"
  | "rayLine"
  | "straightLine"
  | "horizontalStraightLine"
  | "horizontalRayLine"
  | "parallelStraightLine"
>;

/** Drawings an alert can follow: the straight lines and the parallel channel. */
export const DRAWING_ALERT_TOOLS: readonly DrawingAlertTool[] = [
  "segment",
  "rayLine",
  "straightLine",
  "horizontalStraightLine",
  "horizontalRayLine",
  "parallelStraightLine",
];

export const isDrawingAlertTool = (name: string): name is DrawingAlertTool =>
  (DRAWING_ALERT_TOOLS as readonly string[]).includes(name);

export interface Alert {
  id: string;
  symbol: string;
  condition: AlertCondition;
  /** Level of a price alert; 0 and unused for the other sources. */
  price: number;
  /** Absent on price alerts (every record from before ADR-0034 is one). */
  source?: AlertSource;
  trigger: AlertTrigger;
  message?: string;
  active: boolean;
  /** Epoch ms. */
  createdAt: number;
  /** Epoch ms of the last time it fired. */
  lastTriggeredAt?: number;
  /**
   * A fired "every" greater/less alert waiting for `rearmed`. Persisted so
   * a reload does not fire it again while the condition still holds.
   */
  disarmed?: boolean;
  /** Why the alert was switched off by the app, e.g. "drawing deleted". */
  note?: string;
}

export const CONDITION_LABELS: Record<AlertCondition, string> = {
  crossing: "Crossing",
  crossing_up: "Crossing up",
  crossing_down: "Crossing down",
  greater: "Greater than",
  less: "Less than",
};

export const TRIGGER_LABELS: Record<AlertTrigger, string> = {
  once: "Only once",
  every: "Every time",
};

export type AlertStatus = "active" | "triggered" | "paused";

/** Inactive with a fire time: it ran its course; inactive without: paused. */
export function alertStatus(a: Alert): AlertStatus {
  return a.active
    ? "active"
    : a.lastTriggeredAt !== undefined
      ? "triggered"
      : "paused";
}

export const sourceOf = (a: Pick<Alert, "source">): AlertSource =>
  a.source ?? { kind: "price" };

/** A typed number as text, without float noise or trailing zeros. */
const num = (v: number) => String(+v.toPrecision(12));

/**
 * What the alert watches, in TradingView's words: "Crossing 65000",
 * "Close crossing Trend line", "RSI (14) crossing up 70",
 * "Close crossing EMA 200", "MACD crossing Signal". `formatLevel` formats
 * a price alert's level.
 */
export function describeAlert(
  a: Pick<Alert, "condition" | "price" | "source">,
  formatLevel: (price: number) => string = num,
): string {
  const cond = CONDITION_LABELS[a.condition];
  const src = sourceOf(a);
  switch (src.kind) {
    case "price":
      return `${cond} ${formatLevel(a.price)}`;
    case "drawing":
      return `Close ${cond.toLowerCase()} ${src.label}`;
    case "indicator": {
      const ag = src.against;
      if (ag.kind === "price") return `Close ${cond.toLowerCase()} ${src.name}`;
      const right = ag.kind === "value" ? num(ag.value) : ag.name;
      return `${src.name} ${cond.toLowerCase()} ${right}`;
    }
  }
}

/** Left operand minus each level; empty when the source has no value now. */
export type Reading = readonly number[];

/** Did the distance `prev` → `d` (left operand minus one level) cross it? */
function crossed(condition: AlertCondition, prev: number, d: number): boolean {
  switch (condition) {
    case "crossing_up":
      return prev < 0 && d >= 0;
    case "crossing_down":
      return prev > 0 && d <= 0;
    default:
      return (prev < 0 && d >= 0) || (prev > 0 && d <= 0);
  }
}

/**
 * Does going from reading `prev` to `cur` fire? `prev` is null before the
 * first reading, or when its levels do not line up with `cur`'s: events
 * cannot fire then, states can.
 */
export function fires(
  condition: AlertCondition,
  prev: Reading | null,
  cur: Reading,
): boolean {
  if (cur.length === 0) return false;
  if (condition === "greater") return cur.every((d) => d > 0);
  if (condition === "less") return cur.every((d) => d < 0);
  if (prev === null || prev.length !== cur.length) return false;
  return cur.some((d, i) => crossed(condition, prev[i]!, d));
}

/** True once a fired "every" alert may fire again (see the header). */
export function rearmedReading(condition: AlertCondition, cur: Reading): boolean {
  switch (condition) {
    case "greater":
      return cur.some((d) => d <= 0);
    case "less":
      return cur.some((d) => d >= 0);
    default:
      return true;
  }
}

/** True for conditions that need `rearmedReading` to hold before they fire again. */
export function needsRearm(condition: AlertCondition): boolean {
  return condition === "greater" || condition === "less";
}

/** The part of the chart's series an indicator alert reads. */
export interface ChartValues {
  interval: string;
  indicators: Readonly<
    Record<string, { lines?: readonly { values: readonly (number | null)[] }[] }>
  >;
}

type IndicatorSource = Extract<AlertSource, { kind: "indicator" }>;

/**
 * Reading of an indicator alert on the forming (last) bar of `chart`, with
 * the live `price`. Null when the chart is not its symbol's interval, the
 * indicator or plot is missing, or a plot has no value on that bar yet.
 * The caller makes sure `chart` is of the alert's symbol.
 */
export function indicatorReading(
  src: IndicatorSource,
  price: number,
  chart: ChartValues | undefined,
): Reading | null {
  if (!chart || chart.interval !== src.interval) return null;
  const lines = chart.indicators[src.instanceKey]?.lines;
  const last = (i: number) => {
    const v = lines?.[i]?.values.at(-1);
    return typeof v === "number" && Number.isFinite(v) ? v : null;
  };
  const plot = last(src.line);
  if (plot === null) return null;
  const ag = src.against;
  if (ag.kind === "price") return [price - plot];
  const level = ag.kind === "value" ? ag.value : last(ag.line);
  return level === null ? null : [plot - level];
}

/**
 * Both anchors on one bar: `drawingLevels` has no value for it, so it cannot be
 * alerted on. A point on a future bar may carry only its `dataIndex`.
 */
export function isVerticalLine(rec: {
  points: readonly { timestamp?: number; dataIndex?: number }[];
}): boolean {
  const [a, b] = rec.points;
  if (!a || !b) return false;
  if (a.timestamp !== undefined && b.timestamp !== undefined) return a.timestamp === b.timestamp;
  return a.dataIndex !== undefined && a.dataIndex === b.dataIndex;
}

/** Horizontal tools: their level needs no bars, so they are evaluated off the chart too. */
export const isHorizontalTool = (name: string): boolean =>
  name === "horizontalStraightLine" || name === "horizontalRayLine";

/**
 * Bar index of a time on the chart's bars, as klinecharts places a drawing
 * point: the nearest bar inside the data, whole bar intervals beyond it
 * (`toChartPoint` in stores/drawings maps stored points the same way).
 */
export function barIndexOf(axis: TimeAxis, ts: number): number {
  const times = axis.timestamps;
  if (ts < axis.firstTs) return Math.round((ts - axis.firstTs) / axis.intervalMs);
  if (ts > axis.lastTs) {
    return axis.count - 1 + Math.round((ts - axis.lastTs) / axis.intervalMs);
  }
  let lo = 0;
  let hi = times.length - 1;
  while (hi - lo > 1) {
    const mid = (lo + hi) >> 1;
    if (times[mid]! <= ts) lo = mid;
    else hi = mid;
  }
  return ts - times[lo]! <= times[hi]! - ts ? lo : hi;
}

/** How the chart draws: its bars, and whether the price axis is logarithmic. */
export interface ChartView {
  axis: TimeAxis;
  log: boolean;
}

/**
 * The line values a drawing alert compares the price with at bar time `t`
 * (epoch ms): one per line, two for a parallel channel, none where the
 * drawing has no value (a ray before its start, a vertical line) or cannot
 * be alerted on. Horizontal lines are constant. A sloped line is read the
 * way klinecharts draws it: straight in bar-index space (gaps such as
 * nights and weekends take no room), in log(value) on a log axis, extended
 * past its points (a trend line both ways, like TradingView's). Without
 * `view` sloped lines have no value.
 */
export function drawingLevels(
  rec: Pick<DrawingRecord, "name" | "points">,
  t: number,
  view?: ChartView,
): number[] {
  const [a, b, c] = rec.points;
  if (!a) return [];
  if (rec.name === "horizontalStraightLine") return [a.value];
  // One point, extending right (DrawingToolbar's override).
  if (rec.name === "horizontalRayLine") return t >= a.timestamp ? [a.value] : [];
  const sloped =
    rec.name === "segment" ||
    rec.name === "straightLine" ||
    rec.name === "rayLine" ||
    rec.name === "parallelStraightLine";
  if (!sloped || !b || !view) return [];
  const { axis, log } = view;
  if (log && rec.points.some((p) => p.value <= 0)) return [];
  const x = (ts: number) => barIndexOf(axis, ts);
  const y = (v: number) => (log ? Math.log(v) : v);
  const [xa, xb, xt] = [x(a.timestamp), x(b.timestamp), x(t)];
  if (xa === xb) return [];
  // A ray starts at its first point and runs towards its second.
  if (rec.name === "rayLine" && (xb > xa ? xt < xa : xt > xa)) return [];
  const at = (i: number) => y(a.value) + ((y(b.value) - y(a.value)) * (i - xa)) / (xb - xa);
  const out = [at(xt)];
  if (rec.name === "parallelStraightLine") {
    if (!c) return [];
    out.push(at(xt) + y(c.value) - at(x(c.timestamp)));
  }
  return log ? out.map(Math.exp) : out;
}
