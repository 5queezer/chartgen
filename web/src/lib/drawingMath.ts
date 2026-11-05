// Pure geometry and label math of the measure, fib extension, pitchfork and
// long/short position drawings. No chart or DOM here, so it is unit-tested.
// Prices are chart values; "px" helpers take pixel coordinates.

export interface Px {
  x: number;
  y: number;
}

/** 1,234.50 — fixed decimals with thousands separators. */
export function formatAmount(v: number, precision: number): string {
  return v.toLocaleString("en-US", {
    minimumFractionDigits: precision,
    maximumFractionDigits: precision,
  });
}

const signed = (v: number, precision: number) =>
  `${v < 0 ? "-" : "+"}${formatAmount(Math.abs(v), precision)}`;

export interface PriceDelta {
  delta: number;
  /** Percent of the first price; 0 when that is 0. */
  pct: number;
}

export function priceDelta(from: number, to: number): PriceDelta {
  const delta = to - from;
  return { delta, pct: from === 0 ? 0 : (delta / Math.abs(from)) * 100 };
}

/** "+1,234.50 (+2.31%)" */
export function priceRangeLabel(from: number, to: number, precision: number): string {
  const { delta, pct } = priceDelta(from, to);
  return `${signed(delta, precision)} (${signed(pct, 2)}%)`;
}

/** "3d 4h", "5h 30m", "45m": the two largest non-zero units. */
export function formatDuration(ms: number): string {
  const total = Math.floor(Math.abs(ms) / 60_000);
  const d = Math.floor(total / 1440);
  const h = Math.floor((total % 1440) / 60);
  const m = total % 60;
  if (d > 0) return h > 0 ? `${d}d ${h}h` : `${d}d`;
  if (h > 0) return m > 0 ? `${h}h ${m}m` : `${h}h`;
  return `${m}m`;
}

/** Whole bars between two x pixels, given the chart's pixels per bar. */
export function barsBetween(x0: number, x1: number, barPx: number): number {
  return barPx > 0 ? Math.round(Math.abs(x1 - x0) / barPx) : 0;
}

/** "12 bars, 3d 4h" */
export function dateRangeLabel(bars: number, ms: number): string {
  return `${bars} bar${bars === 1 ? "" : "s"}, ${formatDuration(ms)}`;
}

/** Two short strokes making an arrow head at `to`, for a shaft from `from`. */
export function arrowHead(from: Px, to: Px, size = 7): [Px, Px][] {
  const a = Math.atan2(to.y - from.y, to.x - from.x);
  const wing = (s: number): Px => ({
    x: to.x - size * Math.cos(a + s),
    y: to.y - size * Math.sin(a + s),
  });
  return [
    [wing(Math.PI / 6), to],
    [wing(-Math.PI / 6), to],
  ];
}

/** A point of a drawing as the chart holds it. */
export interface AnglePoint {
  timestamp?: number;
  dataIndex?: number;
  value?: number;
}

/**
 * The moving point of a line, snapped to the nearest of horizontal, vertical
 * and 45° as seen on screen from `anchor` (`mouse` in the same pixels).
 * `moving` is the point as the chart placed it: on a bar, magnet applied.
 * Horizontal keeps its bar and takes the anchor's value, exactly, without going
 * through pixels. Vertical takes the anchor's bar, and its value from `mouse`
 * through `magnet`, because the chart's magnet read the bar under the mouse.
 * A diagonal keeps its bar and derives y from that bar's x, so `toValue` makes
 * it right on any axis (log, percent, inverted). Mouse on the anchor counts as
 * horizontal, which gives a zero-length line. `moving` comes back unchanged
 * when it has no bar (over the price axis) or the anchor has no pixel (a value
 * at or below zero on a log axis).
 */
export function snapToAngle(
  anchor: AnglePoint,
  moving: AnglePoint,
  mouse: Px,
  toPx: (p: AnglePoint) => Px,
  toValue: (y: number) => number | undefined,
  magnet: (value: number) => number = (value) => value,
): AnglePoint {
  if (moving.dataIndex === undefined && moving.timestamp === undefined) return moving;
  const a = toPx(anchor);
  const dx = mouse.x - a.x;
  const dy = mouse.y - a.y;
  if (!Number.isFinite(dx) || !Number.isFinite(dy)) return moving;
  const deg = (Math.atan2(Math.abs(dy), Math.abs(dx)) * 180) / Math.PI;
  if (deg < 22.5) return { ...moving, value: anchor.value };
  if (deg > 67.5) {
    const value = toValue(mouse.y);
    return {
      ...moving,
      timestamp: anchor.timestamp,
      dataIndex: anchor.dataIndex,
      value: value === undefined ? moving.value : magnet(value),
    };
  }
  const run = Math.abs(toPx(moving).x - a.x);
  return { ...moving, value: toValue(a.y + Math.sign(dy) * run) ?? moving.value };
}

/** TradingView's default levels of the trend-based fib extension. */
export const FIB_EXT_LEVELS = [
  0, 0.236, 0.382, 0.5, 0.618, 0.786, 1, 1.618, 2.618, 3.618, 4.236,
] as const;

/** Prices of each level, projected from C by the A to B move. */
export function fibExtensionLevels(
  a: number,
  b: number,
  c: number,
  levels: readonly number[] = FIB_EXT_LEVELS,
): { level: number; value: number }[] {
  return levels.map((level) => ({ level, value: c + (b - a) * level }));
}

/**
 * Where a ray from `origin` along `dir` leaves the box [0,w] x [0,h]
 * (the origin itself when it is already outside in that direction).
 */
export function extendRay(origin: Px, dir: Px, w: number, h: number): Px {
  const reach = (o: number, d: number, max: number) =>
    d > 0 ? (max - o) / d : d < 0 ? -o / d : Infinity;
  const t = Math.max(0, Math.min(reach(origin.x, dir.x, w), reach(origin.y, dir.y, h)));
  return Number.isFinite(t) ? { x: origin.x + dir.x * t, y: origin.y + dir.y * t } : origin;
}

/**
 * Andrews' pitchfork: the median line runs from P1 through the midpoint of
 * P2-P3, and two parallels through P2 and P3, all extended to the box edge.
 */
export function pitchforkLines(
  p1: Px,
  p2: Px,
  p3: Px,
  w: number,
  h: number,
): { median: [Px, Px]; upper: [Px, Px]; lower: [Px, Px]; handle: [Px, Px] } {
  const mid = { x: (p2.x + p3.x) / 2, y: (p2.y + p3.y) / 2 };
  const dir = { x: mid.x - p1.x, y: mid.y - p1.y };
  const ray = (o: Px): [Px, Px] => [o, extendRay(o, dir, w, h)];
  return { median: ray(p1), upper: ray(p2), lower: ray(p3), handle: [p2, p3] };
}

export type PositionKind = "long" | "short";

export interface PositionStats {
  /** Signed so that profit is positive for either side. */
  reward: number;
  risk: number;
  rewardPct: number;
  riskPct: number;
  /** reward / risk; null when risk is 0. */
  ratio: number | null;
}

export function positionStats(
  kind: PositionKind,
  entry: number,
  target: number,
  stop: number,
): PositionStats {
  const dir = kind === "long" ? 1 : -1;
  const reward = (target - entry) * dir;
  const risk = (entry - stop) * dir;
  const pct = (v: number) => (entry === 0 ? 0 : (v / Math.abs(entry)) * 100);
  return {
    reward,
    risk,
    rewardPct: pct(reward),
    riskPct: pct(risk),
    ratio: risk > 0 ? reward / risk : null,
  };
}

export interface PositionLabels {
  target: string;
  stop: string;
  ratio: string;
}

/** "Target: 112.44 (2.00%)", "Stop: 108.00 (1.00%)", "Risk/Reward Ratio: 2.00" */
export function positionLabels(
  kind: PositionKind,
  entry: number,
  target: number,
  stop: number,
  precision: number,
): PositionLabels {
  const s = positionStats(kind, entry, target, stop);
  return {
    target: `Target: ${formatAmount(target, precision)} (${formatAmount(s.rewardPct, 2)}%)`,
    stop: `Stop: ${formatAmount(stop, precision)} (${formatAmount(s.riskPct, 2)}%)`,
    ratio: `Risk/Reward Ratio: ${s.ratio === null ? "—" : s.ratio.toFixed(2)}`,
  };
}

/** Default target/stop of a fresh position: +2% / -1% for a long, mirrored for a short. */
export const DEFAULT_TARGET_PCT = 2;
export const DEFAULT_STOP_PCT = 1;
/** Minimum width of a position, in bars. */
export const DEFAULT_POSITION_BARS = 20;
/** Share of the visible price span a fresh position's reward covers. */
export const POSITION_SPAN_SHARE = 0.1;
/** Share of the visible bars a fresh position spans (at least DEFAULT_POSITION_BARS). */
export const POSITION_BARS_SHARE = 0.1;

/** Width of a fresh position: 10% of the visible bars, at least 20. */
export function positionBars(visibleBars: number | null): number {
  const share =
    visibleBars !== null && Number.isFinite(visibleBars) ? visibleBars * POSITION_BARS_SHARE : 0;
  return Math.max(DEFAULT_POSITION_BARS, Math.round(share));
}

/** Reward of a fresh position: 10% of the visible price span, or null without a usable span. */
export function positionReward(span: number | null): number | null {
  return span !== null && Number.isFinite(span) && span > 0 ? span * POSITION_SPAN_SHARE : null;
}

/**
 * Default target and stop: reward = `reward` (see positionReward) and
 * stop = half of it, for a 2:1 ratio. Without a reward, or when the stop
 * would fall to zero or below, +2% / -1% of the entry (mirrored for a short).
 */
export function defaultPosition(
  kind: PositionKind,
  entry: number,
  reward: number | null = null,
): { target: number; stop: number } {
  const dir = kind === "long" ? 1 : -1;
  if (reward !== null && reward > 0) {
    const target = entry + dir * reward;
    const stop = entry - (dir * reward) / 2;
    if (target > 0 && stop > 0) return { target, stop };
  }
  return {
    target: entry * (1 + (dir * DEFAULT_TARGET_PCT) / 100),
    stop: entry * (1 - (dir * DEFAULT_STOP_PCT) / 100),
  };
}

/**
 * Keep a position consistent while a handle is dragged: the target lies on
 * the profit side of the entry, the stop on the loss side, and the entry
 * between them. `moved` is the index of the dragged value (0 entry, 1
 * target, 2 stop); values are [entry, target, stop].
 */
export function clampPosition(
  kind: PositionKind,
  v: readonly [number, number, number],
  moved: 0 | 1 | 2,
): [number, number, number] {
  const long = kind === "long";
  let [entry, target, stop] = v;
  if (moved === 0) {
    const [lo, hi] = long ? [stop, target] : [target, stop];
    entry = Math.min(Math.max(entry, lo), hi);
  } else if (moved === 1) {
    target = long ? Math.max(target, entry) : Math.min(target, entry);
  } else {
    stop = long ? Math.min(stop, entry) : Math.max(stop, entry);
  }
  return [entry, target, stop];
}
