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
  type OverlayEvent,
} from "klinecharts";

import { isTypingContext, undoRedoKey } from "../lib/keys";
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
import { DrawingStyleBar } from "./DrawingStyleBar";
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

type Magnet ="normal" | "weak_magnet" | "strong_magnet";

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
];

/** Display name of each drawing tool. */
export const DRAWING_LABELS = Object.fromEntries(
  TOOLS.map((t) => [t.name, t.label]),
) as Record<DrawingName, string>;

const SHORTCUTS: Record<string, DrawingName> = Object.fromEntries(
  TOOLS.filter((t) => t.code).map((t) => [t.code, t.name]),
);

const btnClass =
  "relative disabled:opacity-40 disabled:cursor-not-allowed disabled:hover:bg-transparent w-9 h-9 inline-flex items-center justify-center rounded-md cursor-pointer text-[color:var(--color-fg-muted)] hover:bg-[color:var(--color-border)] hover:text-[color:var(--color-fg)] aria-pressed:bg-[color:var(--color-border)] aria-pressed:text-[color:var(--color-accent)]";
const menuItemClass =
  "w-full text-left px-3 py-1.5 text-sm cursor-pointer hover:bg-[color:var(--color-border)] outline-none focus:bg-[color:var(--color-border)]";

const LIMIT_NOTICE = `Drawing limit reached (${MAX_DRAWINGS})`;
const SAVE_ERROR = "Drawings not saved: storage full";

/** A drawing of the current symbol, as the Object Tree lists it. */
export interface DrawingItem extends DrawingRecord {
  id: string;
  /** A point precedes the first loaded bar: the drawing is hidden and locked until bars cover it. */
  offRange: boolean;
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
  const [magnet, setMagnet] = createSignal<Magnet>("normal");
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

  const axis = (): TimeAxis | null =>
    timeAxisOf(props.chart?.getDataList().map((d) => d.timestamp) ?? []);

  const syncFlags = () => {
    setLockAll(current.length > 0 && current.every((r) => r.lock));
    setHideAll(current.length > 0 && current.every((r) => !r.visible));
    setItems(
      reconcile(
        ids.flatMap((id) =>
          // Cloned: reconcile edits the objects it is given in place, which
          // would rewrite the snapshots held by canon and the undo history.
          canon.has(id)
            ? [{ id, ...structuredClone(canon.get(id)!), offRange: offRange.has(id) }]
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
    if (p) setSaveFailed(!saveDrawings(p.symbol, p.records));
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
    const id = props.chart?.createOverlay({
      name: rec.name,
      points,
      styles: liveStyles(rec, false) as never,
      extendData: rec.extendData,
      lock: off || rec.lock,
      visible: !off && rec.visible,
      mode: magnet() as never,
      ...handlers(),
    });
    if (typeof id !== "string") return;
    ids.push(id);
    canon.set(id, rec);
    if (off) offRange.add(id);
  };

  /** Replace everything on the chart with `records` (restore / undo). */
  const rebuild = (records: DrawingRecord[]) => {
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
    for (const id of ids) {
      const rec = canon.get(id);
      if (!rec) continue;
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
    const id = chart.createOverlay({
      name,
      extendData: name === "cgText" ? DEFAULT_TEXT : undefined,
      styles: name === "cgRect" ? (RECT_STYLES as never) : undefined,
      mode: magnet() as never,
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
    for (const id of ids) {
      props.chart?.overrideOverlay({ id, mode: next as never });
    }
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

  const menuAction = (action: "remove" | "lock") => {
    const m = menu();
    setMenu(null);
    if (!m) return;
    if (action === "remove") removeIds([m.id]);
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
                offRange: offRange.has(id),
                paneId: o.paneId,
                styles: o.styles,
                points: o.points,
              },
            ]
          : [];
      });
    onCleanup(() => delete w.__chartgenDrawings);
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
    magnet() === "normal"
      ? "Magnet: off"
      : magnet() === "weak_magnet"
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
      class="w-12 shrink-0 flex flex-col items-center gap-0.5 py-2 border-r border-[color:var(--color-border)] bg-[color:var(--color-bg)] overflow-y-auto"
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
      <For each={TOOLS}>
        {(tool) => (
          <button
            type="button"
            class={btnClass}
            aria-label={tool.label}
            title={tool.code ? `${tool.label} (Alt+${tool.code.slice(3)})` : tool.label}
            aria-pressed={armed() === tool.name}
            onClick={() => arm(tool.name)}
          >
            {tool.icon()}
          </button>
        )}
      </For>
      <Separator />
      <button
        type="button"
        class={btnClass}
        aria-label={magnetLabel()}
        title={`${magnetLabel()} (click to cycle)`}
        aria-pressed={magnet() !== "normal"}
        onClick={cycleMagnet}
      >
        <svg {...svgProps}>
          <path d="M6 3v8a6 6 0 0 0 12 0V3h-4v8a2 2 0 0 1-4 0V3z" />
        </svg>
        <Show when={magnet() !== "normal"}>
          <span class="absolute bottom-0.5 right-0.5 text-[9px] leading-none font-semibold">
            {magnet() === "weak_magnet" ? "W" : "S"}
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
