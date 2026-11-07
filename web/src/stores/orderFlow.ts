// Settings of the order-flow chart types (ADR-0048). One set for all charts, under
// `chartgen.orderflow`, validated on load; not part of a named layout.

import { createSignal } from "solid-js";

const KEY = "chartgen.orderflow";

export const FOOTPRINT_TYPES = ["buy_sell", "delta", "total", "ladder"] as const;
export const FOOTPRINT_DISPLAYS = ["cluster", "profile"] as const;
export const FOOTPRINT_SUMMARIES = ["box", "table", "none"] as const;
export const TPO_STYLES = ["letters", "blocks"] as const;
export const TPO_PERIODS = ["1D", "2D", "1W", "1M"] as const;
export const TPO_BLOCKS = ["5m", "15m", "30m", "1h", "2h", "4h"] as const;
export const VP_SIDES = ["left", "right"] as const;

export interface FootprintSettings {
  /** Ticks per row; 0 is auto. */
  rowTicks: number;
  atrLength: number;
  type: (typeof FOOTPRINT_TYPES)[number];
  display: (typeof FOOTPRINT_DISPLAYS)[number];
  gradient: boolean;
  gradientColors: [string, string, string, string];
  valueArea: number;
  showValueArea: boolean;
  showPoc: boolean;
  summary: (typeof FOOTPRINT_SUMMARIES)[number];
  imbalance: number;
  showImbalance: boolean;
  stacked: number;
  showStacked: boolean;
}

export interface TpoSettings {
  period: (typeof TPO_PERIODS)[number];
  block: (typeof TPO_BLOCKS)[number];
  /** Ticks per row; 0 is auto. */
  rowTicks: number;
  valueArea: number;
  style: (typeof TPO_STYLES)[number];
  splitByBlocks: boolean;
  /** Opacity of blocks outside the value area. */
  outsideOpacity: number;
  showPoc: boolean;
  showValueArea: boolean;
  showPoor: boolean;
  showSingles: boolean;
  showMidpoint: boolean;
  showOpenClose: boolean;
  showIb: boolean;
  ibBlocks: number;
  showProfile: boolean;
  profileSide: (typeof VP_SIDES)[number];
  summary: boolean;
  /** Up to 5 `HH:MM-HH:MM`, comma separated, in the exchange's time. */
  sessions: string;
}

export interface OrderFlowSettings {
  footprint: FootprintSettings;
  tpo: TpoSettings;
}

export function defaultOrderFlow(): OrderFlowSettings {
  return {
    footprint: {
      rowTicks: 0,
      atrLength: 14,
      type: "buy_sell",
      display: "cluster",
      gradient: false,
      gradientColors: ["#1e3a5f", "#26658c", "#2f97a8", "#5fd0b5"],
      valueArea: 70,
      showValueArea: true,
      showPoc: true,
      summary: "box",
      imbalance: 300,
      showImbalance: true,
      stacked: 3,
      showStacked: true,
    },
    tpo: {
      period: "1D",
      block: "30m",
      rowTicks: 0,
      valueArea: 70,
      style: "letters",
      splitByBlocks: false,
      outsideOpacity: 0.45,
      showPoc: true,
      showValueArea: true,
      showPoor: true,
      showSingles: true,
      showMidpoint: false,
      showOpenClose: true,
      showIb: true,
      ibBlocks: 2,
      showProfile: false,
      profileSide: "right",
      summary: false,
      sessions: "",
    },
  };
}

const COLOR = /^#[0-9a-f]{6}$/i;

const oneOf = <T extends string>(list: readonly T[], x: unknown, fallback: T): T =>
  list.includes(x as T) ? (x as T) : fallback;

const inRange = (x: unknown, min: number, max: number, fallback: number): number =>
  typeof x === "number" && Number.isFinite(x) && x >= min && x <= max ? x : fallback;

const bool = (x: unknown, fallback: boolean): boolean => (typeof x === "boolean" ? x : fallback);

/** The valid parts of a stored value, defaults for the rest. */
export function sanitizeOrderFlow(x: unknown): OrderFlowSettings {
  const d = defaultOrderFlow();
  const raw = (x && typeof x === "object" ? x : {}) as Record<string, unknown>;
  const f = (raw.footprint && typeof raw.footprint === "object" ? raw.footprint : {}) as Record<string, unknown>;
  const t = (raw.tpo && typeof raw.tpo === "object" ? raw.tpo : {}) as Record<string, unknown>;
  const colors =
    Array.isArray(f.gradientColors) &&
    f.gradientColors.length === 4 &&
    f.gradientColors.every((c) => typeof c === "string" && COLOR.test(c))
      ? (f.gradientColors as FootprintSettings["gradientColors"])
      : d.footprint.gradientColors;
  return {
    footprint: {
      rowTicks: inRange(f.rowTicks, 0, 1_000_000, d.footprint.rowTicks),
      atrLength: inRange(f.atrLength, 1, 500, d.footprint.atrLength),
      type: oneOf(FOOTPRINT_TYPES, f.type, d.footprint.type),
      display: oneOf(FOOTPRINT_DISPLAYS, f.display, d.footprint.display),
      gradient: bool(f.gradient, d.footprint.gradient),
      gradientColors: colors,
      valueArea: inRange(f.valueArea, 1, 100, d.footprint.valueArea),
      showValueArea: bool(f.showValueArea, d.footprint.showValueArea),
      showPoc: bool(f.showPoc, d.footprint.showPoc),
      summary: oneOf(FOOTPRINT_SUMMARIES, f.summary, d.footprint.summary),
      imbalance: inRange(f.imbalance, 100, 100_000, d.footprint.imbalance),
      showImbalance: bool(f.showImbalance, d.footprint.showImbalance),
      stacked: inRange(f.stacked, 1, 100, d.footprint.stacked),
      showStacked: bool(f.showStacked, d.footprint.showStacked),
    },
    tpo: {
      period: oneOf(TPO_PERIODS, t.period, d.tpo.period),
      block: oneOf(TPO_BLOCKS, t.block, d.tpo.block),
      rowTicks: inRange(t.rowTicks, 0, 1_000_000, d.tpo.rowTicks),
      valueArea: inRange(t.valueArea, 1, 100, d.tpo.valueArea),
      style: oneOf(TPO_STYLES, t.style, d.tpo.style),
      splitByBlocks: bool(t.splitByBlocks, d.tpo.splitByBlocks),
      outsideOpacity: inRange(t.outsideOpacity, 0, 1, d.tpo.outsideOpacity),
      showPoc: bool(t.showPoc, d.tpo.showPoc),
      showValueArea: bool(t.showValueArea, d.tpo.showValueArea),
      showPoor: bool(t.showPoor, d.tpo.showPoor),
      showSingles: bool(t.showSingles, d.tpo.showSingles),
      showMidpoint: bool(t.showMidpoint, d.tpo.showMidpoint),
      showOpenClose: bool(t.showOpenClose, d.tpo.showOpenClose),
      showIb: bool(t.showIb, d.tpo.showIb),
      ibBlocks: inRange(t.ibBlocks, 1, 100, d.tpo.ibBlocks),
      showProfile: bool(t.showProfile, d.tpo.showProfile),
      profileSide: oneOf(VP_SIDES, t.profileSide, d.tpo.profileSide),
      summary: bool(t.summary, d.tpo.summary),
      sessions: typeof t.sessions === "string" ? t.sessions.slice(0, 80) : d.tpo.sessions,
    },
  };
}

function load(): OrderFlowSettings {
  try {
    const raw = typeof localStorage === "undefined" ? null : localStorage.getItem(KEY);
    return sanitizeOrderFlow(raw === null ? null : JSON.parse(raw));
  } catch {
    return defaultOrderFlow();
  }
}

const [orderFlowSettings, setSignal] = createSignal<OrderFlowSettings>(load());
export { orderFlowSettings };

export function saveOrderFlow(next: OrderFlowSettings): void {
  setSignal(next);
  try {
    localStorage.setItem(KEY, JSON.stringify(next));
  } catch {
    // Best-effort only.
  }
}

/** The sessions of a TPO setting, as `get_tpo` takes them. */
export function tpoSessions(text: string): string[] {
  return text
    .split(",")
    .map((s) => s.trim())
    .filter((s) => s.length > 0)
    .slice(0, 5);
}
