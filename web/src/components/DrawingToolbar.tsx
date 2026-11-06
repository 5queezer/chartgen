// Vertical drawing toolbar + drawing controller for the klinecharts
// instance. Picking a tool arms it for exactly one drawing (as in
// TradingView); Esc cancels. Drawings are persisted per symbol through
// stores/drawings.ts as (timestamp, value) points, so they survive
// timeframe changes and paging.

import {
  createEffect,
  createSignal,
  type Accessor,
  For,
  on,
  onCleanup,
  Show,
  untrack,
  type JSX,
} from "solid-js";
import { Dialog } from "@kobalte/core";
import { createStore, reconcile } from "solid-js/store";
import {
  ActionType,
  registerOverlay,
  type Chart as KChart,
  type OverlayCreate,
  type Overlay,
  type OverlayEvent,
  type Point,
} from "klinecharts";

import { isDrawingAlertTool, isVerticalLine, pointBarIndex } from "../lib/alerts";
import { isRelease, isTypingContext, undoRedoKey } from "../lib/keys";
import { effectiveMagnet, magnetKeyHeld, magnetValue, type Magnet } from "../lib/magnet";
import { syncDrawingAlerts } from "../stores/alerts";
import { createHistory } from "../stores/drawingHistory";
import {
  loadDrawings,
  MAX_DRAWINGS,
  overlayStyles,
  saveDrawings,
  serialize,
  timeAxisOf,
  toChartPoint,
  type ChartPoint,
  type DrawingName,
  type DrawingRecord,
  type DrawingStyle,
  type TimeAxis,
} from "../stores/drawings";
import { openAlertDialog } from "./AlertDialog";
import { DrawingStyleBar } from "./DrawingStyleBar";
import { overlayLabels, setOverlayAxis, setPositionSize } from "./drawingOverlays";
import { ToolGroup } from "./ToolGroup";
import { positionReward, snapToAngle, type Px } from "../lib/drawingMath";
import { PALETTES } from "../lib/theme";
import { theme } from "../stores/layout";

// klinecharts has no rectangle; two corner points, drawn as a polygon
// whose colors come from the overlay's `styles.polygon`.
registerOverlay({
  name: "cgRect",
  totalStep: 3,
  needDefaultPointFigure: true,
  needDefaultXAxisFigure: true,
  needDefaultYAxisFigure: true,
  createPointFigures: ({ coordinates }) => {
    const [a, b] = coordinates;
    if (!a || !b) return [];
    return {
      type: "polygon",
      attrs: {
        coordinates: [
          { x: a.x, y: a.y },
          { x: b.x, y: a.y },
          { x: b.x, y: b.y },
          { x: a.x, y: b.y },
        ],
      },
    };
  },
});

// klinecharts' horizontalRayLine takes two clicks and points toward the
// second one. TradingView places it with one click, extending right.
registerOverlay({
  name: "horizontalRayLine",
  totalStep: 2,
  needDefaultPointFigure: true,
  needDefaultXAxisFigure: true,
  needDefaultYAxisFigure: true,
  createPointFigures: ({ coordinates, bounding }) => {
    const a = coordinates[0];
    if (!a) return [];
    return {
      type: "line",
      attrs: { coordinates: [a, { x: bounding.width, y: a.y }] },
    };
  },
});

// klinecharts' simpleAnnotation ignores all mouse events, so it could be
// neither selected nor edited. This label can; its text is `extendData`.
registerOverlay({
  name: "cgText",
  totalStep: 2,
  needDefaultPointFigure: true,
  needDefaultXAxisFigure: false,
  needDefaultYAxisFigure: false,
  createPointFigures: ({ overlay, coordinates }) => {
    const c = coordinates[0];
    if (!c) return [];
    const own = (overlay.styles as { text?: { color?: string; size?: number } } | null)?.text;
    return {
      type: "text",
      attrs: {
        x: c.x,
        y: c.y,
        text: typeof overlay.extendData === "string" ? overlay.extendData : "",
        baseline: "middle",
      },
      styles: {
        style: "stroke_fill",
        color: own?.color ?? PALETTES[theme()].fg,
        backgroundColor: PALETTES[theme()].surface,
        borderColor: "#2962ff",
        size: own?.size ?? 12,
        paddingLeft: 6,
        paddingRight: 6,
        paddingTop: 3,
        paddingBottom: 3,
      },
    };
  },
});

interface ToolDef {
  name: DrawingName;
  label: string;
  /** Alt+<code> shortcut, using TradingView's key where one exists. */
  code?: string;
  icon: () => JSX.Element;
}

const RECT_STYLES = {
  polygon: {
    style: "stroke_fill",
    color: "rgba(41, 98, 255, 0.15)",
    borderColor: "#2962ff",
  },
};

const DEFAULT_TEXT = "Text";
/** Two-point lines that Shift snaps to horizontal, vertical or 45°. */
const SNAP_TOOLS: ReadonlySet<string> = new Set(["segment", "rayLine", "straightLine"]);

const svgProps = {
  width: 20,
  height: 20,
  viewBox: "0 0 24 24",
  fill: "none",
  stroke: "currentColor",
  "stroke-width": 1.6,
  "stroke-linecap": "round",
  "stroke-linejoin": "round",
  "aria-hidden": true,
} as const;

const Dot = (p: { x: number; y: number }) => (
  <circle cx={p.x} cy={p.y} r="2" fill="currentColor" stroke="none" />
);

const TOOLS: readonly ToolDef[] = [
  {
    name: "segment",
    label: "Trend line",
    code: "KeyT",
    icon: () => (
      <svg {...svgProps}>
        <path d="M5 19L19 5" />
        <Dot x={5} y={19} />
        <Dot x={19} y={5} />
      </svg>
    ),
  },
  {
    name: "rayLine",
    label: "Ray",
    icon: () => (
      <svg {...svgProps}>
        <path d="M5 19L22 2" />
        <Dot x={5} y={19} />
      </svg>
    ),
  },
  {
    name: "straightLine",
    label: "Extended line",
    icon: () => (
      <svg {...svgProps}>
        <path d="M2 22L22 2" />
        <Dot x={8} y={16} />
        <Dot x={16} y={8} />
      </svg>
    ),
  },
  {
    name: "horizontalStraightLine",
    label: "Horizontal line",
    code: "KeyH",
    icon: () => (
      <svg {...svgProps}>
        <path d="M2 12h20" />
        <Dot x={12} y={12} />
      </svg>
    ),
  },
  {
    name: "horizontalRayLine",
    label: "Horizontal ray",
    code: "KeyJ",
    icon: () => (
      <svg {...svgProps}>
        <path d="M5 12h17" />
        <Dot x={5} y={12} />
      </svg>
    ),
  },
  {
    name: "verticalStraightLine",
    label: "Vertical line",
    code: "KeyV",
    icon: () => (
      <svg {...svgProps}>
        <path d="M12 2v20" />
        <Dot x={12} y={12} />
      </svg>
    ),
  },
  {
    name: "parallelStraightLine",
    label: "Parallel channel",
    icon: () => (
      <svg {...svgProps}>
        <path d="M3 15L19 5M5 21L21 11" />
        <path d="M3 15L19 5L21 11L5 21z" fill="currentColor" fill-opacity="0.15" stroke="none" />
      </svg>
    ),
  },
  {
    name: "fibonacciLine",
    label: "Fib retracement",
    code: "KeyF",
    icon: () => (
      <svg {...svgProps}>
        <path d="M3 5h18M3 10h18M3 15h18M3 20h18" />
        <path d="M6 20L18 5" stroke-dasharray="2 2" />
      </svg>
    ),
  },
  {
    name: "cgFibExt",
    label: "Fib extension (trend-based)",
    icon: () => (
      <svg {...svgProps}>
        <path d="M3 4h18M3 9h18M3 14h18M3 20h18" />
        <path d="M4 20L9 12L12 16" stroke-dasharray="2 2" />
      </svg>
    ),
  },
  {
    name: "cgRect",
    label: "Rectangle",
    icon: () => (
      <svg {...svgProps}>
        <rect x="4" y="6" width="16" height="12" />
      </svg>
    ),
  },
  {
    name: "cgText",
    label: "Text annotation",
    icon: () => (
      <svg {...svgProps}>
        <path d="M6 6h12M12 6v13" />
      </svg>
    ),
  },
  {
    name: "simpleTag",
    label: "Price label",
    icon: () => (
      <svg {...svgProps}>
        <path d="M3 12l5-5h13v10H8z" />
      </svg>
    ),
  },
  {
    name: "cgPriceRange",
    label: "Price range",
    icon: () => (
      <svg {...svgProps}>
        <rect x="5" y="4" width="14" height="16" fill="currentColor" fill-opacity="0.15" />
        <path d="M12 6v12M9.5 8.5L12 6l2.5 2.5M9.5 15.5L12 18l2.5-2.5" />
      </svg>
    ),
  },
  {
    name: "cgDateRange",
    label: "Date range",
    icon: () => (
      <svg {...svgProps}>
        <rect x="3" y="7" width="18" height="10" fill="currentColor" fill-opacity="0.15" />
        <path d="M6 12h12M8.5 9.5L6 12l2.5 2.5M15.5 9.5L18 12l-2.5 2.5" />
      </svg>
    ),
  },
  {
    name: "cgDatePriceRange",
    label: "Date and price range",
    icon: () => (
      <svg {...svgProps}>
        <rect x="4" y="5" width="16" height="14" fill="currentColor" fill-opacity="0.15" />
        <path d="M12 7v10M8 12h8" />
        <path d="M10 9l2-2 2 2M10 15l2 2 2-2M10 10l-2 2 2 2M14 10l2 2-2 2" />
      </svg>
    ),
  },
  {
    name: "cgPitchfork",
    label: "Pitchfork",
    icon: () => (
      <svg {...svgProps}>
        <path d="M4 19L21 6M9 8L22 9M9 8L21 6M12 21L22 9" />
        <path d="M9 8l3 13" stroke-dasharray="2 2" />
        <Dot x={4} y={19} />
        <Dot x={9} y={8} />
        <Dot x={12} y={21} />
      </svg>
    ),
  },
  {
    name: "cgLong",
    label: "Long position",
    icon: () => (
      <svg {...svgProps}>
        <rect x="4" y="4" width="16" height="8" fill="#089981" fill-opacity="0.35" stroke="#089981" />
        <rect x="4" y="12" width="16" height="8" fill="#f23645" fill-opacity="0.35" stroke="#f23645" />
      </svg>
    ),
  },
  {
    name: "cgShort",
    label: "Short position",
    icon: () => (
      <svg {...svgProps}>
        <rect x="4" y="4" width="16" height="8" fill="#f23645" fill-opacity="0.35" stroke="#f23645" />
        <rect x="4" y="12" width="16" height="8" fill="#089981" fill-opacity="0.35" stroke="#089981" />
      </svg>
    ),
  },
];

/** Display name of each drawing tool. */
export const DRAWING_LABELS = Object.fromEntries(
  TOOLS.map((t) => [t.name, t.label]),
) as Record<DrawingName, string>;

const SHORTCUTS: Record<string, DrawingName> = Object.fromEntries(
  TOOLS.filter((t) => t.code).map((t) => [t.code, t.name]),
);

/** Toolbar groups in display order; each shows one button with a flyout of its tools. */
const GROUPS: readonly { name: string; tools: readonly DrawingName[] }[] = [
  {
    name: "Lines",
    tools: [
      "segment",
      "rayLine",
      "straightLine",
      "horizontalStraightLine",
      "horizontalRayLine",
      "verticalStraightLine",
      "parallelStraightLine",
    ],
  },
  { name: "Fib", tools: ["fibonacciLine", "cgFibExt"] },
  { name: "Pitchfork", tools: ["cgPitchfork"] },
  { name: "Shapes and text", tools: ["cgRect", "cgText", "simpleTag"] },
  { name: "Measure", tools: ["cgPriceRange", "cgDateRange", "cgDatePriceRange"] },
  { name: "Forecasting", tools: ["cgLong", "cgShort"] },
];
const TOOL_BY_NAME = new Map(TOOLS.map((t) => [t.name, t]));
const GROUP_OF = new Map(GROUPS.flatMap((g) => g.tools.map((t) => [t, g.name] as const)));

const isMac = typeof navigator !== "undefined" && /mac/i.test(navigator.platform);

const LAST_KEY = "chartgen.drawingTools.last";
/** Last-used tool per group, from storage; unknown names are ignored. */
function loadLast(): Record<string, DrawingName> {
  const out = Object.fromEntries(GROUPS.map((g) => [g.name, g.tools[0]!])) as Record<string, DrawingName>;
  try {
    const raw = JSON.parse(localStorage.getItem(LAST_KEY) ?? "{}") as Record<string, unknown>;
    for (const g of GROUPS) {
      const v = raw[g.name];
      if (typeof v === "string" && (g.tools as readonly string[]).includes(v)) out[g.name] = v as DrawingName;
    }
  } catch {
    // Unreadable storage: the defaults stand.
  }
  return out;
}

const btnClass =
  "relative shrink-0 disabled:opacity-40 disabled:cursor-not-allowed disabled:hover:bg-transparent w-9 h-9 inline-flex items-center justify-center rounded-md cursor-pointer text-[color:var(--color-fg-muted)] hover:bg-[color:var(--color-border)] hover:text-[color:var(--color-fg)] aria-pressed:bg-[color:var(--color-border)] aria-pressed:text-[color:var(--color-accent)]";
const menuItemClass =
  "w-full text-left px-3 py-1.5 text-sm cursor-pointer hover:bg-[color:var(--color-border)] outline-none focus:bg-[color:var(--color-border)]";

/** The internals of klinecharts ~9.8 that hold the overlay under a press. */
interface PressStoreOwner {
  getChartStore(): {
    getOverlayStore(): {
      getPressedInstanceInfo(): { instance: unknown };
      setPressedInstanceInfo(info: {
        paneId: string;
        instance: null;
        figureType: 0;
        figureKey: string;
        figureIndex: number;
        attrsIndex: number;
      }): void;
    };
  };
}

const LIMIT_NOTICE = `Drawing limit reached (${MAX_DRAWINGS})`;
const SAVE_ERROR = "Drawings not saved: storage full";

/** A drawing of the current symbol, as the Object Tree lists it. */
export interface DrawingItem extends DrawingRecord {
  id: string;
  /** A point precedes the first loaded bar: the drawing is hidden and locked until bars cover it. */
  offRange: boolean;
  /** Both points on one bar of the chart: no value for an alert to read. */
  vertical: boolean;
}

/** What the Object Tree needs from the drawing controller. */
export interface DrawingsApi {
  list: Accessor<DrawingItem[]>;
  /** Select the drawing (highlighted, Delete removes it) and scroll to it if it is out of view. */
  select: (id: string) => void;
  setVisible: (id: string, visible: boolean) => void;
  setLocked: (id: string, lock: boolean) => void;
  remove: (id: string) => void;
  /** Add a persisted horizontal line at `price`, at the last bar. */
  addHorizontalLine: (price: number) => void;
  /** Remove every drawing after a confirm. */
  removeAll: () => void;
  undo: () => void;
  redo: () => void;
  /** True when the latest right-click landed on a drawing (its own menu wins). */
  rightClickedDrawing: () => boolean;
}

export interface DrawingToolbarProps {
  /** The klinecharts instance; null until the chart has mounted. */
  chart: KChart | null;
  /** Active symbol — selects which stored drawings are shown. */
  symbol: string;
  /** Receives the drawings API once, at mount. */
  onApi?: (api: DrawingsApi) => void;
  /** Bar replay (ADR-0023): told whether a tool is armed (Esc is then the tool's). */
  onArmedChange?: (armed: boolean) => void;
  /** Bar replay (ADR-0023): turning true cancels an armed tool (picking a start bar). */
  disarm?: boolean;
}

export function DrawingToolbar(props: DrawingToolbarProps): JSX.Element {
  const [armed, setArmed] = createSignal<DrawingName | null>(null);
  // --- Bar replay (ADR-0023) ------------------------------------------
  // Armed tool or a selected drawing: either way Esc is the drawing's, not replay's.
  createEffect(() => props.onArmedChange?.(armed() !== null || selectedId() !== null));
  createEffect(() => {
    if (props.disarm) untrack(cancelArmed);
  });
  // ---------------------------------------------------------------------
  const [lastTool, setLastTool] = createSignal(loadLast());
  const [magnet, setMagnet] = createSignal<Magnet>("normal");
  // Ctrl (Cmd on Mac) held inverts the magnet until release; `magnet` stays the user's choice.
  const [invertMagnet, setInvertMagnet] = createSignal(false);
  const magnetMode = () => effectiveMagnet(magnet(), invertMagnet());
  const [crosshair, setCrosshair] = createSignal(true);
  const [lockAll, setLockAll] = createSignal(false);
  const [hideAll, setHideAll] = createSignal(false);
  // Keyed by id so a row keeps its identity (and focus) across commits.
  const [items, setItems] = createStore<DrawingItem[]>([]);
  const [selectedId, setSelectedId] = createSignal<string | null>(null);
  const [saveFailed, setSaveFailed] = createSignal(false);
  const [limitHit, setLimitHit] = createSignal(false);
  const [menu, setMenu] = createSignal<{
    x: number;
    y: number;
    id: string;
  } | null>(null);
  // Annotation text being edited in the dialog.
  const [editing, setEditing] = createSignal<{
    id: string;
    text: string;
  } | null>(null);

  // Overlay ids that belong to the current symbol's drawings, in
  // creation order. `canon` holds each one's stored record. `current` is
  // the last committed state; each commit pushes the previous one onto
  // `history` for Ctrl+Z.
  let ids: string[] = [];
  const canon = new Map<string, DrawingRecord>();
  // Drawings with a point before the first loaded bar. They are hidden and
  // locked (klinecharts would clamp them onto the first bar) and never
  // re-serialized from the chart, so their stored points stay intact.
  const offRange = new Set<string>();
  // Restored drawings waiting for the first data load.
  let pending: DrawingRecord[] | null = null;
  let armedId: string | null = null;
  let current: DrawingRecord[] = [];
  const history = createHistory<DrawingRecord[]>();
  const [canUndo, setCanUndo] = createSignal(false);
  const [canRedo, setCanRedo] = createSignal(false);
  const syncHistory = () => {
    setCanUndo(history.canUndo());
    setCanRedo(history.canRedo());
  };
  // Storage writes of a slider drag are debounced; `symbol` is captured so a
  // symbol switch cannot send them to the wrong key.
  let saveTimer: number | undefined;
  let pendingSave: { symbol: string; records: DrawingRecord[] } | null = null;
  let limitTimer: number | undefined;
  // klinecharts fires an overlay's onRightClick on mousedown, before the
  // browser's contextmenu event; the flag lets the chart menu stand down.
  let rightClickHit = false;
  // klinecharts passes no modifier keys to overlay callbacks.
  let shiftHeld = false;
  // The drawing under a press: `refresh` leaves it alone until the release commits it.
  let pressedId: string | null = null;
  // The bars that press is on.
  let pressedAxis: TimeAxis | null = null;

  const axis = (): TimeAxis | null =>
    timeAxisOf(props.chart?.getDataList().map((d) => d.timestamp) ?? []);
  // The overlays ask for the bar timing on every frame; recompute it only
  // when the loaded bars change.
  let axisKey: { list: unknown; n: number; last: number } | null = null;
  let axisCache: TimeAxis | null = null;
  setOverlayAxis(() => {
    const list = props.chart?.getDataList();
    if (!list) return null;
    const last = list.at(-1)?.timestamp ?? 0;
    if (axisKey?.list !== list || axisKey.n !== list.length || axisKey.last !== last) {
      axisKey = { list, n: list.length, last };
      axisCache = axis();
    }
    return axisCache;
  });

  const syncFlags = () => {
    setLockAll(current.length > 0 && current.every((r) => r.lock));
    setHideAll(current.length > 0 && current.every((r) => !r.visible));
    const ax = axis();
    setItems(
      reconcile(
        ids.flatMap((id) =>
          // Cloned: reconcile edits the objects it is given in place, which
          // would rewrite the snapshots held by canon and the undo history.
          canon.has(id)
            ? [
                {
                  id,
                  ...structuredClone(canon.get(id)!),
                  offRange: offRange.has(id),
                  vertical: isVerticalLine(canon.get(id)!, ax),
                },
              ]
            : [],
        ),
        { key: "id" },
      ),
    );
  };

  const flushSave = () => {
    window.clearTimeout(saveTimer);
    const p = pendingSave;
    pendingSave = null;
    if (!p) return;
    setSaveFailed(!saveDrawings(p.symbol, p.records));
    // Alerts on a drawing that is gone or vertical are switched off (ADR-0034).
    syncDrawingAlerts(p.symbol, p.records);
  };
  const persist = () => {
    pendingSave = { symbol: props.symbol, records: current };
    flushSave();
  };
  const persistSoon = () => {
    pendingSave = { symbol: props.symbol, records: current };
    window.clearTimeout(saveTimer);
    saveTimer = window.setTimeout(flushSave, 300);
  };
  window.addEventListener("pagehide", flushSave);
  onCleanup(() => {
    window.removeEventListener("pagehide", flushSave);
    flushSave();
  });

  /**
   * Record the chart's present state: history for undo, then storage.
   * Commits with the same `key` in quick succession are one undo step.
   */
  const commit = (key?: string) => {
    const chart = props.chart;
    if (!chart) return;
    const ax = axis();
    const next = ids.flatMap((id) => {
      if (!offRange.has(id)) {
        const o = chart.getOverlayById(id);
        const [rec] = o ? serialize([o], ax) : [];
        if (rec) {
          // Live styles carry the selection highlight; the stored ones do not.
          const old = canon.get(id);
          canon.set(id, {
            ...rec,
            styles: old ? old.styles : rec.styles,
            style: old?.style,
          });
        }
      }
      const rec = canon.get(id);
      return rec ? [rec] : [];
    });
    if (JSON.stringify(next) === JSON.stringify(current)) return;
    history.record(current, key);
    syncHistory();
    current = next;
    if (key === undefined) persist();
    else persistSoon();
    syncFlags();
  };

  /**
   * Chart-ready points for a record. `off` means a point precedes the first
   * loaded bar; those points are parked on the first bar of a hidden copy.
   */
  const place = (
    rec: DrawingRecord,
    ax: TimeAxis,
  ): { points: ChartPoint[]; off: boolean } => {
    let off = false;
    const points = rec.points.map((p) => {
      const cp = toChartPoint(ax, p);
      if (cp !== null) return cp;
      off = true;
      return { timestamp: ax.firstTs, value: p.value };
    });
    return { points, off };
  };

  /** Shift on a line: its moving point snaps to horizontal, vertical or 45° from the other. */
  const snapMovingPoint = (overlay: Overlay, index: number, mouse: Px) => {
    const chart = props.chart;
    const moving = overlay.points[index];
    const anchor = overlay.points[1 - index];
    if (!chart || !moving || !anchor) return;
    const paneId = overlay.paneId;
    const toPx = (p: Partial<Point>) => chart.convertToPixel(p, { paneId }) as Px;
    Object.assign(
      moving,
      snapToAngle(
        anchor,
        moving,
        mouse,
        toPx,
        (y) => (chart.convertFromPixel([{ y }], { paneId }) as Partial<Point>[])[0]?.value,
        (value) => {
          // The anchor's bar; none on a future bar. As in klinecharts, the magnet only
          // acts on the price pane.
          const ax = axis();
          const i = ax ? pointBarIndex(ax, anchor) : undefined;
          const bar = paneId === "candle_pane" && i !== undefined ? chart.getDataList()[i] : undefined;
          return bar
            ? magnetValue(value, bar, overlay.mode as never, overlay.modeSensitivity, (v) => toPx({ value: v }).y)
            : value;
        },
      ),
    );
  };

  const handlers = (): Partial<OverlayCreate> => ({
    onDrawEnd: ({ overlay }: OverlayEvent) => {
      armedId = null;
      setArmed(null);
      ids.push(overlay.id);
      if (lockAll()) props.chart?.overrideOverlay({ id: overlay.id, lock: true });
      commit();
      // onSelected fires before the drawing is registered, so highlight now.
      if (selectedId() === overlay.id) highlight(overlay.id, true);
      return false;
    },
    onPressedMoveStart: ({ overlay }: OverlayEvent) => {
      pressedId = overlay.id;
      pressedAxis = axis();
      return false;
    },
    // Fires after klinecharts has set the moving point, also on the placing click.
    onDrawing: ({ overlay, x, y }: OverlayEvent) => {
      if (shiftHeld && SNAP_TOOLS.has(overlay.name) && overlay.points.length === 2 && x !== undefined && y !== undefined) {
        snapMovingPoint(overlay, 1, { x, y });
      }
      return false;
    },
    // Fires before klinecharts moves the point, and the only hook after the move
    // is the template's perform one, which these lines leave null: borrow it for
    // this move. Dragging the body stays untouched.
    onPressedMoving: ({ overlay, figureKey, figureIndex, x, y }: OverlayEvent) => {
      if (
        shiftHeld &&
        SNAP_TOOLS.has(overlay.name) &&
        figureIndex !== undefined &&
        figureKey === `overlay_figure_point_${figureIndex}` &&
        x !== undefined &&
        y !== undefined
      ) {
        overlay.performEventPressedMove = () => {
          overlay.performEventPressedMove = null;
          snapMovingPoint(overlay, figureIndex, { x, y });
        };
      }
      return false;
    },
    onPressedMoveEnd: () => {
      commit();
      return false;
    },
    onSelected: ({ overlay }: OverlayEvent) => {
      setSelectedId(overlay.id);
      return false;
    },
    onDeselected: ({ overlay }: OverlayEvent) => {
      if (selectedId() === overlay.id) setSelectedId(null);
      return false;
    },
    onRightClick: ({ overlay, pageX, pageY }: OverlayEvent) => {
      rightClickHit = true;
      if (overlay.id === armedId) {
        cancelArmed();
      } else if (pageX !== undefined && pageY !== undefined) {
        setMenu({ x: pageX, y: pageY, id: overlay.id });
      }
      return true;
    },
    onDoubleClick: ({ overlay }: OverlayEvent) => {
      if (overlay.name !== "cgText") return false;
      setEditing({
        id: overlay.id,
        text: typeof overlay.extendData === "string" ? overlay.extendData : "",
      });
      return true;
    },
    onRemoved: ({ overlay }: OverlayEvent) => {
      ids = ids.filter((id) => id !== overlay.id);
      canon.delete(overlay.id);
      offRange.delete(overlay.id);
      if (selectedId() === overlay.id) setSelectedId(null);
      return false;
    },
  });

  /** Stored `styles` overlaid with the user's style (plus the selection thickening). */
  const liveStyles = (rec: DrawingRecord, selected: boolean) => {
    const own = overlayStyles(rec.name, rec.style, selected);
    const base = (rec.styles ?? {}) as Record<string, Record<string, unknown>>;
    // Per-tool defaults first, so older records without stored styles
    // still get them on restore.
    const dflt = (rec.name === "cgRect" ? RECT_STYLES : {}) as Record<
      string,
      Record<string, unknown>
    >;
    return {
      ...own,
      polygon: { ...dflt.polygon, ...base.polygon, ...own.polygon },
    };
  };

  /** Add a stored drawing to the chart; needs loaded data. */
  const create = (rec: DrawingRecord, ax: TimeAxis) => {
    const { points, off } = place(rec, ax);
    // The stored id is kept, so alerts on the drawing survive reloads (ADR-0034).
    const keep = rec.id !== undefined && !props.chart?.getOverlayById(rec.id);
    const id = props.chart?.createOverlay({
      ...(keep ? { id: rec.id } : {}),
      name: rec.name,
      points,
      styles: liveStyles(rec, false) as never,
      extendData: rec.extendData,
      lock: off || rec.lock,
      visible: !off && rec.visible,
      mode: untrack(magnetMode) as never,
      ...handlers(),
    });
    if (typeof id !== "string") return;
    ids.push(id);
    canon.set(id, { ...rec, id });
    if (off) offRange.add(id);
  };

  /** Replace everything on the chart with `records` (restore / undo). */
  const rebuild = (records: DrawingRecord[]) => {
    pressedId = null;
    cancelArmed();
    for (const id of [...ids]) props.chart?.removeOverlay(id);
    ids = [];
    canon.clear();
    offRange.clear();
    setSelectedId(null);
    const ax = axis();
    if (ax === null) {
      // No bars yet; refresh() restores them once data is ready.
      pending = records;
      current = records;
    } else {
      pending = null;
      records.forEach((r) => create(r, ax));
      current = ids.flatMap((id) => canon.get(id) ?? []);
      // Records saved before drawings had ids get theirs stored now.
      if (records.some((r) => r.id === undefined)) persist();
    }
    syncFlags();
  };

  /**
   * Re-evaluate every drawing against the loaded bars: after a timeframe
   * change or when older bars are paged in, points move or come into
   * range. Hidden ones stay hidden until all their points are covered.
   */
  const refresh = () => {
    if (pending !== null) {
      const records = pending;
      rebuild(records);
      return;
    }
    const ax = axis();
    if (ax === null) return;
    // New bars under a press leave the bar indices of its future points meaning other
    // times, so the pressed drawing is placed too, once; the press goes on with these bars.
    const sameBars = pressedAxis?.firstTs === ax.firstTs && pressedAxis.intervalMs === ax.intervalMs;
    pressedAxis = ax;
    for (const id of ids) {
      const rec = canon.get(id);
      if (!rec || (id === pressedId && sameBars)) continue;
      const { points, off } = place(rec, ax);
      if (off) offRange.add(id);
      else offRange.delete(id);
      props.chart?.overrideOverlay({
        id,
        points,
        visible: !off && rec.visible,
        lock: off || rec.lock,
      });
    }
    syncFlags();
  };

  function cancelArmed() {
    if (armedId !== null) props.chart?.removeOverlay(armedId);
    armedId = null;
    setArmed(null);
  }

  const showLimitNotice = () => {
    setLimitHit(true);
    window.clearTimeout(limitTimer);
    limitTimer = window.setTimeout(() => setLimitHit(false), 4000);
  };

  /** What a fresh position is sized from: the visible price span and bar count, read via the axis. */
  const readPositionSize = (chart: KChart) => {
    const h = chart.getSize("candle_pane", "main" as never)?.height ?? 0;
    const [top, bottom] = chart.convertFromPixel(
      [{ y: 0 }, { y: h }],
      { paneId: "candle_pane" },
    ) as { value?: number }[];
    const span =
      top?.value !== undefined && bottom?.value !== undefined ? Math.abs(top.value - bottom.value) : null;
    const range = chart.getVisibleRange();
    return { reward: positionReward(span), bars: range.to - range.from };
  };

  const arm = (name: DrawingName) => {
    const chart = props.chart;
    if (!chart) return;
    const same = armed() === name;
    cancelArmed();
    if (same) return;
    if (ids.length >= MAX_DRAWINGS) {
      showLimitNotice();
      return;
    }
    // Drawing on a hidden chart would be invisible, so show everything.
    if (hideAll()) toggleHide();
    const group = GROUP_OF.get(name);
    if (group !== undefined && lastTool()[group] !== name) {
      setLastTool({ ...lastTool(), [group]: name });
      try {
        localStorage.setItem(LAST_KEY, JSON.stringify(lastTool()));
      } catch {
        // Not persisted; the choice still holds for this session.
      }
    }
    if (name === "cgLong" || name === "cgShort") setPositionSize(readPositionSize(chart));
    const id = chart.createOverlay({
      name,
      extendData: name === "cgText" ? DEFAULT_TEXT : undefined,
      styles: name === "cgRect" ? (RECT_STYLES as never) : undefined,
      mode: untrack(magnetMode) as never,
      ...handlers(),
    });
    if (typeof id === "string") {
      armedId = id;
      setArmed(name);
    }
  };

  const setAll = (patch: { lock: boolean } | { visible: boolean }) => {
    for (const id of ids) {
      if (!offRange.has(id)) props.chart?.overrideOverlay({ id, ...patch });
    }
    commit();
  };

  function toggleHide() {
    if (!hideAll()) setSelectedId(null);
    setAll({ visible: hideAll() });
  }

  const toggleLock = () => setAll({ lock: !lockAll() });

  const cycleMagnet = () => {
    const next: Magnet =
      magnet() === "normal"
        ? "weak_magnet"
        : magnet() === "weak_magnet"
          ? "strong_magnet"
          : "normal";
    setMagnet(next);
    applyMagnet();
  };

  /** Push the effective magnet to every drawing, the one being drawn included. */
  function applyMagnet() {
    const mode = magnetMode();
    for (const id of armedId === null ? ids : [...ids, armedId]) {
      props.chart?.overrideOverlay({ id, mode: mode as never });
    }
  }

  const setInverted = (on: boolean) => {
    if (on === invertMagnet()) return;
    setInvertMagnet(on);
    applyMagnet();
  };

  const toggleCrosshair = () => {
    cancelArmed();
    const next = !crosshair();
    setCrosshair(next);
    props.chart?.setStyles({ crosshair: { show: next } });
  };

  const removeIds = (targets: string[]) => {
    for (const id of targets) props.chart?.removeOverlay(id);
    commit();
  };

  const removeAll = () => {
    if (ids.length === 0) return;
    if (window.confirm("Remove all drawings?")) removeIds([...ids]);
  };

  const addHorizontalLine = (price: number) => {
    const last = props.chart?.getDataList().at(-1);
    const ax = axis();
    if (!last || ax === null) return;
    if (ids.length >= MAX_DRAWINGS) {
      showLimitNotice();
      return;
    }
    // A line on a hidden chart would be invisible, so show everything.
    if (hideAll()) toggleHide();
    create(
      {
        name: "horizontalStraightLine",
        points: [{ timestamp: last.timestamp, value: price }],
        lock: lockAll(),
        visible: true,
      },
      ax,
    );
    commit();
  };

  const travel = (to: DrawingRecord[] | undefined) => {
    if (to === undefined) return;
    // Keep the selection on the same drawing (by position) if it survives.
    const at = ids.indexOf(selectedId() ?? "");
    const name = at >= 0 ? canon.get(ids[at]!)?.name : undefined;
    rebuild(to);
    const again = at >= 0 ? ids[at] : undefined;
    if (again !== undefined && canon.get(again)?.name === name) {
      fromTree = again;
      setSelectedId(again);
    }
    syncHistory();
    persist();
  };
  const undo = () => travel(history.undo(current));
  const redo = () => travel(history.redo(current));

  /** Change a drawing's look; locked or off-range drawings keep theirs. */
  const setStyle = (id: string, patch: Partial<DrawingStyle>, key: string) => {
    const rec = canon.get(id);
    if (!rec || rec.lock || offRange.has(id)) return;
    const style = { ...rec.style, ...patch };
    canon.set(id, { ...rec, style });
    props.chart?.overrideOverlay({
      id,
      styles: liveStyles({ ...rec, style }, selectedId() === id) as never,
    });
    commit(`${id}:${key}`);
  };

  const patchOne = (id: string, patch: { lock: boolean } | { visible: boolean }) => {
    if (offRange.has(id)) return;
    props.chart?.overrideOverlay({ id, ...patch });
    commit();
  };

  // klinecharts has no select call, so the selection is ours: set by its
  // select events and by the tree, drawn as a thicker stroke.
  const highlight = (id: string, on: boolean) => {
    if (!props.chart?.getOverlayById(id)) return;
    const rec = canon.get(id);
    if (!rec) return;
    props.chart.overrideOverlay({ id, styles: liveStyles(rec, on) as never });
  };
  createEffect(
    on(selectedId, (id, prev) => {
      if (prev != null) highlight(prev, false);
      if (id !== null) highlight(id, true);
    }),
  );
  // A press anywhere on the chart ends a selection made from the tree (one
  // klinecharts does not know of); the capture phase runs before klinecharts
  // selects what the press landed on. Its own selections end with its events.
  let fromTree: string | null = null;
  createEffect(() => {
    const dom = props.chart?.getDom();
    if (!dom) return;
    const clear = () => {
      if (fromTree !== null && selectedId() === fromTree) setSelectedId(null);
      fromTree = null;
    };
    dom.addEventListener("mousedown", clear, true);
    onCleanup(() => dom.removeEventListener("mousedown", clear, true));
  });

  const select = (id: string) => {
    const chart = props.chart;
    // The stored point has a timestamp even where the live one (a point past
    // the last bar) only has a data index.
    const p = canon.get(id)?.points[0];
    if (!chart || !p || !chart.getOverlayById(id)?.visible) return;
    cancelArmed();
    fromTree = id;
    setSelectedId(id);
    const x = (chart.convertToPixel(
      { timestamp: p.timestamp, value: p.value },
      { paneId: "candle_pane" },
    ) as { x?: number }).x;
    const width = chart.getSize("candle_pane", "main" as never)?.width ?? 0;
    if (!(x !== undefined && x >= 0 && x <= width)) chart.scrollToTimestamp(p.timestamp);
  };

  const menuAction = (action: "remove" | "lock" | "alert") => {
    const m = menu();
    setMenu(null);
    if (!m) return;
    if (action === "alert") openAlertDialog({ symbol: props.symbol, drawingId: m.id });
    else if (action === "remove") removeIds([m.id]);
    else {
      const locked = props.chart?.getOverlayById(m.id)?.lock ?? false;
      props.chart?.overrideOverlay({ id: m.id, lock: !locked });
      commit();
    }
  };

  const saveText = (e: SubmitEvent) => {
    e.preventDefault();
    const ed = editing();
    setEditing(null);
    if (ed && ed.text.length > 0) {
      props.chart?.overrideOverlay({ id: ed.id, extendData: ed.text });
      commit();
    }
  };

  // (Re)load stored drawings whenever the chart appears or the symbol
  // changes. History is reset only after the rebuild.
  createEffect(
    on(
      () => [props.chart, props.symbol] as const,
      ([chart, symbol]) => {
        if (!chart) return;
        flushSave();
        rebuild(loadDrawings(symbol));
        history.clear();
        syncHistory();
        setSaveFailed(false);
      },
    ),
  );

  // Data was (re)loaded or older bars were paged in.
  createEffect(() => {
    const chart = props.chart;
    if (!chart) return;
    chart.subscribeAction(ActionType.OnDataReady, refresh);
    onCleanup(() => chart.unsubscribeAction(ActionType.OnDataReady, refresh));
  });

  // E2E hook: the overlays that actually exist on the chart right now.
  if (import.meta.env.DEV) {
    const w = window as unknown as Record<string, unknown>;
    w.__chartgenDrawings = () =>
      ids.flatMap((id) => {
        const o = props.chart?.getOverlayById(id);
        return o
          ? [
              {
                id,
                name: o.name,
                lock: o.lock,
                visible: o.visible,
                mode: o.mode,
                offRange: offRange.has(id),
                paneId: o.paneId,
                styles: o.styles,
                points: o.points,
                labels: overlayLabels.get(id),
                precision: props.chart?.getPriceVolumePrecision().price,
                pixels: o.points.map(
                  (pt) => props.chart?.convertToPixel(pt, { paneId: o.paneId }) as { x?: number; y?: number },
                ),
              },
            ]
          : [];
      });
    w.__chartgenPositionSize = () => (props.chart ? readPositionSize(props.chart) : null);
    w.__chartgenShiftHeld = () => shiftHeld;
    onCleanup(() => {
      delete w.__chartgenDrawings;
      delete w.__chartgenPositionSize;
      delete w.__chartgenShiftHeld;
    });
  }

  // Native context menu would sit on top of ours.
  createEffect(() => {
    const dom = props.chart?.getDom();
    if (!dom) return;
    const stop = (e: Event) => e.preventDefault();
    dom.addEventListener("contextmenu", stop);
    onCleanup(() => dom.removeEventListener("contextmenu", stop));
  });

  // klinecharts ignores the pointer in any pane but the one a drawing was
  // started in, so a second click over an indicator pane stalls the
  // drawing. Clamp the pointer into the start pane instead.
  let pinning = false;
  const pinToStartPane = (e: MouseEvent) => {
    const chart = props.chart;
    if (pinning || armedId === null || !chart) return;
    if (!(e.target instanceof Node) || !chart.getDom()?.contains(e.target)) return;
    const o = chart.getOverlayById(armedId) as unknown as {
      paneId: string;
      currentStep: number;
    } | null;
    if (!o || o.currentStep < 2) return;
    const r = chart.getDom(o.paneId, "main" as never)?.getBoundingClientRect();
    if (!r || (e.clientY >= r.top && e.clientY <= r.bottom)) return;
    e.stopImmediatePropagation();
    pinning = true;
    try {
      e.target.dispatchEvent(
        new MouseEvent(e.type, {
          bubbles: true,
          cancelable: true,
          view: window,
          detail: e.detail,
          screenX: e.screenX,
          screenY: e.screenY,
          shiftKey: e.shiftKey,
          ctrlKey: e.ctrlKey,
          altKey: e.altKey,
          metaKey: e.metaKey,
          button: e.button,
          buttons: e.buttons,
          clientX: e.clientX,
          clientY: Math.min(Math.max(e.clientY, r.top + 4), r.bottom - 4),
        }),
      );
    } finally {
      pinning = false;
    }
  };
  const pinEvents = ["mousemove", "mousedown", "mouseup", "click"] as const;
  for (const type of pinEvents) document.addEventListener(type, pinToStartPane, true);
  onCleanup(() => {
    for (const type of pinEvents) document.removeEventListener(type, pinToStartPane, true);
  });

  // klinecharts drops a second click within 500 ms of the first press unless it is within
  // 5 px of it, a double click (`_mouseUpHandler`), and the next point of a drawing with it.
  // That also ends its click timer, so the same click sent again counts as a first one.
  // A press that went 5 px or more is a cancelled click to klinecharts, wherever it ends.
  let armedPress: { step: number; x: number; y: number; target: EventTarget; moved: boolean } | null = null;
  let replaying = false;
  const armedStep = () =>
    armedId === null
      ? undefined
      : (props.chart?.getOverlayById(armedId) as unknown as { currentStep: number } | null)?.currentStep;
  // Trusted events only here: `pinToStartPane`'s copies are clamped into the pane, so against
  // the real press position a pinned click would look like it had moved.
  const noteArmedPress = (e: MouseEvent) => {
    if (!e.isTrusted) return;
    const step = armedStep();
    armedPress =
      step !== undefined && e.button === 0 && e.target instanceof Node && props.chart?.getDom()?.contains(e.target)
        ? { step, x: e.clientX, y: e.clientY, target: e.target, moved: false }
        : null;
  };
  const trackArmedPress = (e: MouseEvent) => {
    if (e.isTrusted && armedPress && Math.abs(e.clientX - armedPress.x) + Math.abs(e.clientY - armedPress.y) >= 5) {
      armedPress.moved = true;
    }
  };
  // Bubble, so after klinecharts has had the release. A pinned release only gets here as a copy.
  const replayDroppedClick = (e: MouseEvent) => {
    const press = armedPress;
    if (replaying || !press || e.button !== 0) return;
    armedPress = null;
    // The step moved on (the click counted), or the press moved (a pan).
    if (armedStep() !== press.step || press.moved) return;
    replaying = true;
    press.target.dispatchEvent(new MouseEvent("mousedown", e));
    (e.target ?? press.target).dispatchEvent(new MouseEvent("mouseup", e));
    replaying = false;
  };
  window.addEventListener("mousedown", noteArmedPress, true);
  window.addEventListener("mousemove", trackArmedPress, true);
  window.addEventListener("mouseup", trackArmedPress, true);
  window.addEventListener("mouseup", replayDroppedClick);
  onCleanup(() => {
    window.removeEventListener("mousedown", noteArmedPress, true);
    window.removeEventListener("mousemove", trackArmedPress, true);
    window.removeEventListener("mouseup", trackArmedPress, true);
    window.removeEventListener("mouseup", replayDroppedClick);
  });

  const onKey = (e: KeyboardEvent) => {
    if (e.defaultPrevented) return;
    // Our own menu has role=menu, so it is handled before the guard.
    if (e.key === "Escape" && menu()) {
      setMenu(null);
      return;
    }
    if (isTypingContext(e)) return;
    if (e.key === "Escape") {
      cancelArmed();
      setSelectedId(null);
      return;
    }
    const mod = e.ctrlKey || e.metaKey;
    const ur = undoRedoKey(e);
    if (ur !== null) {
      e.preventDefault();
      if (ur === "redo") redo();
      else undo();
    } else if (e.altKey && !mod) {
      const tool = SHORTCUTS[e.code];
      if (tool) {
        e.preventDefault();
        arm(tool);
      }
    } else if (
      !mod &&
      (e.key === "Delete" || e.key === "Backspace") &&
      selectedId() !== null
    ) {
      e.preventDefault();
      removeIds([selectedId()!]);
    }
  };
  window.addEventListener("keydown", onKey);
  onCleanup(() => window.removeEventListener("keydown", onKey));

  // Modifier state is read from every key and mouse event (capture, so before
  // klinecharts sees the move). That covers a key pressed before the chart had
  // focus and makes auto-repeat a no-op. Blur clears it: keyup never comes.
  const syncModifiers = (e: KeyboardEvent | MouseEvent) => {
    const ctrl = magnetKeyHeld(e, isMac);
    if ((ctrl || e.shiftKey) && e instanceof KeyboardEvent && isTypingContext(e)) return;
    setInverted(ctrl);
    shiftHeld = e.shiftKey;
  };
  const releaseModifiers = () => {
    setInverted(false);
    shiftHeld = false;
  };
  const modifierEvents = ["keydown", "keyup", "mousemove", "mousedown"] as const;
  for (const type of modifierEvents) window.addEventListener(type, syncModifiers, true);
  window.addEventListener("blur", releaseModifiers);
  document.addEventListener("visibilitychange", releaseModifiers);
  onCleanup(() => {
    for (const type of modifierEvents) window.removeEventListener(type, syncModifiers, true);
    window.removeEventListener("blur", releaseModifiers);
    document.removeEventListener("visibilitychange", releaseModifiers);
  });

  // Any left release ends a press. Not onPressedMoveEnd: klinecharts only fires it when
  // the release lands on a pane. Capture, so this runs before it, in the same event. The
  // window's blur is not: an element's blur passes through capture too, and pressing on
  // the chart takes the focus from whatever had it.
  const endPress = (e: Event) => {
    if (isRelease(e)) pressedId = null;
  };
  // A release that misses a pane (off the chart, on a separator) also leaves klinecharts'
  // own press in place, and its next pan would move that drawing. No public call ends a
  // press, so this does what its mouseUpEvent does, without onPressedMoveEnd: the drag is
  // discarded, and the drawing goes back at once. Bubble, so after klinecharts has had the event.
  // A new press also ends one, in capture, before klinecharts sees it: Firefox can take a
  // mouseleave for the release outside the window, and no mouseup follows (not run on Firefox).
  const pressStore = () =>
    (props.chart as unknown as PressStoreOwner | null)?.getChartStore().getOverlayStore();
  const dropStuckPress = (e: Event) => {
    const store = pressStore();
    if (!isRelease(e) || !store?.getPressedInstanceInfo().instance) return;
    pressedId = null;
    store.setPressedInstanceInfo({
      paneId: "",
      instance: null,
      figureType: 0,
      figureKey: "",
      figureIndex: -1,
      attrsIndex: -1,
    });
    refresh();
  };
  // A blur has no release, and klinecharts would treat later moves as still pressed: give
  // it one, off the panes.
  const releaseOnBlur = () => {
    armedPress = null;
    if (!pressStore()?.getPressedInstanceInfo().instance) return;
    document.documentElement.dispatchEvent(
      new MouseEvent("mouseup", { bubbles: true, button: 0, clientX: -1, clientY: -1 }),
    );
  };
  const endPressEvents = ["mouseup", "touchend", "touchcancel"] as const;
  for (const type of endPressEvents) {
    window.addEventListener(type, endPress, true);
    window.addEventListener(type, dropStuckPress);
  }
  window.addEventListener("mousedown", dropStuckPress, true);
  window.addEventListener("blur", releaseOnBlur);
  onCleanup(() => {
    for (const type of endPressEvents) {
      window.removeEventListener(type, endPress, true);
      window.removeEventListener(type, dropStuckPress);
    }
    window.removeEventListener("mousedown", dropStuckPress, true);
    window.removeEventListener("blur", releaseOnBlur);
  });

  const resetRightClick = () => {
    rightClickHit = false;
  };
  // A keyboard-opened menu or a macOS Ctrl+click has no right mousedown, so
  // any other input also clears a stale flag.
  const resetEvents = ["mousedown", "pointerdown", "keydown"] as const;
  for (const type of resetEvents) {
    window.addEventListener(type, resetRightClick, true);
  }
  onCleanup(() => {
    for (const type of resetEvents) {
      window.removeEventListener(type, resetRightClick, true);
    }
  });

  const closeMenu = (e: PointerEvent) => {
    if (!(e.target instanceof Element && e.target.closest("[data-drawing-menu]"))) {
      setMenu(null);
    }
  };
  window.addEventListener("pointerdown", closeMenu);
  onCleanup(() => window.removeEventListener("pointerdown", closeMenu));

  const Separator = () => (
    <hr class="w-6 border-0 border-t border-[color:var(--color-border)] my-1" />
  );

  const selectedItem = () => {
    const id = selectedId();
    return id === null ? undefined : items.find((i) => i.id === id);
  };

  const magnetLabel = () =>
    magnetMode() === "normal"
      ? "Magnet: off"
      : magnetMode() === "weak_magnet"
        ? "Magnet: weak"
        : "Magnet: strong";

  props.onApi?.({
    list: () => items,
    select,
    setVisible: (id, visible) => patchOne(id, { visible }),
    setLocked: (id, lock) => patchOne(id, { lock }),
    remove: (id) => removeIds([id]),
    addHorizontalLine,
    removeAll,
    undo,
    redo,
    rightClickedDrawing: () => rightClickHit,
  });

  return (
    <div
      role="toolbar"
      aria-label="Drawing tools"
      aria-orientation="vertical"
      class="w-12 shrink-0 flex flex-col items-center gap-0.5 py-2 border-r border-[color:var(--color-border)] bg-[color:var(--color-bg)] min-h-0"
    >
      <button
        type="button"
        class={btnClass}
        aria-label={crosshair() ? "Cursor: crosshair" : "Cursor: arrow"}
        title={crosshair() ? "Cursor: crosshair (click for arrow)" : "Cursor: arrow (click for crosshair)"}
        aria-pressed={crosshair()}
        onClick={toggleCrosshair}
      >
        <svg {...svgProps}>
          <Show
            when={crosshair()}
            fallback={<path d="M5 3l14 7-6 2-2 6z" />}
          >
            <path d="M12 3v7M12 14v7M3 12h7M14 12h7" />
          </Show>
        </svg>
      </button>
      <Separator />
      <div class="flex-1 min-h-0 w-full overflow-y-auto flex flex-col items-center gap-0.5">
        <For each={GROUPS}>
          {(g) => (
            <ToolGroup
              group={g.name}
              tools={g.tools.map((n) => TOOL_BY_NAME.get(n)!)}
              last={lastTool()[g.name]!}
              armed={armed()}
              onArm={(n) => arm(n as DrawingName)}
              btnClass={btnClass}
            />
          )}
        </For>
      </div>
      <Separator />
      <button
        type="button"
        class={btnClass}
        aria-label={magnetLabel()}
        title={`${magnetLabel()} (click to cycle)`}
        aria-pressed={magnetMode() !== "normal"}
        onClick={cycleMagnet}
      >
        <svg {...svgProps}>
          <path d="M6 3v8a6 6 0 0 0 12 0V3h-4v8a2 2 0 0 1-4 0V3z" />
        </svg>
        <Show when={magnetMode() !== "normal"}>
          <span class="absolute bottom-0.5 right-0.5 text-[9px] leading-none font-semibold">
            {magnetMode() === "weak_magnet" ? "W" : "S"}
          </span>
        </Show>
      </button>
      <button
        type="button"
        class={btnClass}
        aria-label="Lock all drawings"
        title="Lock all drawings"
        aria-pressed={lockAll()}
        onClick={toggleLock}
      >
        <svg {...svgProps}>
          <rect x="5" y="11" width="14" height="10" rx="1" />
          <path d="M8 11V7a4 4 0 0 1 8 0v4" />
        </svg>
      </button>
      <button
        type="button"
        class={btnClass}
        aria-label={hideAll() ? "Show all drawings" : "Hide all drawings"}
        title={hideAll() ? "Show all drawings" : "Hide all drawings"}
        aria-pressed={hideAll()}
        onClick={toggleHide}
      >
        <svg {...svgProps}>
          <path d="M2 12s4-7 10-7 10 7 10 7-4 7-10 7S2 12 2 12z" />
          <circle cx="12" cy="12" r="3" />
          <Show when={hideAll()}>
            <path d="M4 4l16 16" />
          </Show>
        </svg>
      </button>
      <button
        type="button"
        class={btnClass}
        aria-label="Undo"
        title="Undo (Ctrl+Z)"
        disabled={!canUndo()}
        onClick={undo}
      >
        <svg {...svgProps}>
          <path d="M9 14L4 9l5-5" />
          <path d="M4 9h10a6 6 0 0 1 0 12h-3" />
        </svg>
      </button>
      <button
        type="button"
        class={btnClass}
        aria-label="Redo"
        title="Redo (Ctrl+Shift+Z)"
        disabled={!canRedo()}
        onClick={redo}
      >
        <svg {...svgProps}>
          <path d="M15 14l5-5-5-5" />
          <path d="M20 9H10a6 6 0 0 0 0 12h3" />
        </svg>
      </button>
      <button
        type="button"
        class={btnClass}
        aria-label="Remove all drawings"
        title="Remove all drawings"
        onClick={removeAll}
      >
        <svg {...svgProps}>
          <path d="M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13" />
        </svg>
      </button>

      <Show when={menu()}>
        {(m) => (
          <div
            role="menu"
            data-drawing-menu
            aria-label="Drawing actions"
            class="fixed z-50 min-w-32 py-1 rounded-md border border-[color:var(--color-border)] bg-[color:var(--color-bg-elev)] shadow-lg text-sm"
            style={{ left: `${m().x}px`, top: `${m().y}px` }}
          >
            <button type="button" role="menuitem" class={menuItemClass} onClick={() => menuAction("remove")}>
              Remove
            </button>
            <button type="button" role="menuitem" class={menuItemClass} onClick={() => menuAction("lock")}>
              {props.chart?.getOverlayById(m().id)?.lock ? "Unlock" : "Lock"}
            </button>
            <Show when={canon.get(m().id)?.name}>
              {(name) => (
                <Show when={isDrawingAlertTool(name()) && !items.find((i) => i.id === m().id)?.vertical}>
                  <button type="button" role="menuitem" class={menuItemClass} onClick={() => menuAction("alert")}>
                    Add alert on {DRAWING_LABELS[name()]}…
                  </button>
                </Show>
              )}
            </Show>
          </div>
        )}
      </Show>

      <Show when={selectedItem()}>
        {(item) => (
          <DrawingStyleBar
            chart={props.chart}
            drawing={item()}
            onChange={(patch, key) => setStyle(item().id, patch, key)}
          />
        )}
      </Show>

      <Show when={limitHit() || saveFailed()}>
        <span
          role="status"
          class="mt-1 w-2.5 h-2.5 rounded-full bg-[color:var(--color-danger)]"
          title={limitHit() ? LIMIT_NOTICE : SAVE_ERROR}
          aria-label={limitHit() ? LIMIT_NOTICE : SAVE_ERROR}
        />
      </Show>

      <Dialog.Root
        open={editing() !== null}
        onOpenChange={(open) => {
          if (!open) setEditing(null);
        }}
      >
        <Dialog.Portal>
          <Dialog.Overlay class="fixed inset-0 z-40 bg-black/60" />
          <div class="fixed inset-0 z-50 flex items-start justify-center pt-[10vh] px-4">
            <Dialog.Content class="w-full max-w-sm bg-[color:var(--color-bg)] border border-[color:var(--color-border)] rounded-lg shadow-xl">
              <form class="flex flex-col gap-3 p-4" onSubmit={saveText}>
                <Dialog.Title class="text-sm font-semibold">
                  Annotation text
                </Dialog.Title>
                <input
                  type="text"
                  aria-label="Annotation text"
                  maxLength={200}
                  value={editing()?.text ?? ""}
                  onInput={(e) => {
                    const text = e.currentTarget.value;
                    setEditing((ed) => (ed ? { ...ed, text } : ed));
                  }}
                  class="bg-[color:var(--color-bg-elev)] border border-[color:var(--color-border)] rounded-md px-3 py-1.5 text-sm text-[color:var(--color-fg)] focus:outline-none focus:border-[color:var(--color-accent)]"
                />
                <div class="flex justify-end gap-2">
                  <Dialog.CloseButton class="border border-[color:var(--color-border)] rounded-md px-3 py-1.5 text-sm hover:border-[color:var(--color-accent)] cursor-pointer">
                    Cancel
                  </Dialog.CloseButton>
                  <button
                    type="submit"
                    class="bg-[color:var(--color-accent)] text-white rounded-md px-3 py-1.5 text-sm cursor-pointer"
                  >
                    Save
                  </button>
                </div>
              </form>
            </Dialog.Content>
          </div>
        </Dialog.Portal>
      </Dialog.Root>
    </div>
  );
}
