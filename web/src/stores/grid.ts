// Multi-chart grid (ADR-0043): how many charts show, and which one is active.
// A pane is a number; pane 0 owns the storage keys the single-chart app has
// always used, so nothing here is written until the user changes the grid or
// the active chart, or a layout is created or switched to.
// The chart stores (layout, indicators, compare) keep one instance per pane;
// their module-level exports read the active pane.

import { createContext, createSignal, useContext } from "solid-js";

export const GRID_IDS = ["1", "2h", "2v", "4"] as const;
export type GridId = (typeof GRID_IDS)[number];

/** Columns and rows of each grid. `2h` is side by side, `2v` one above the other. */
export const GRID_SHAPES: Record<GridId, { columns: number; rows: number }> = {
  "1": { columns: 1, rows: 1 },
  "2h": { columns: 2, rows: 1 },
  "2v": { columns: 1, rows: 2 },
  "4": { columns: 2, rows: 2 },
};

export const GRID_LABELS: Record<GridId, string> = {
  "1": "1 chart",
  "2h": "2 charts, side by side",
  "2v": "2 charts, stacked",
  "4": "4 charts",
};

const GRID_KEY = "chartgen.layout.grid";
const ACTIVE_KEY = "chartgen.layout.activePane";

/** A valid grid id, else the single chart. */
export function parseGrid(x: unknown): GridId {
  return (GRID_IDS as readonly unknown[]).includes(x) ? (x as GridId) : "1";
}

export const paneCount = (g: GridId): number =>
  GRID_SHAPES[g].columns * GRID_SHAPES[g].rows;

/** Stored active pane when the grid has it, else the first. */
export function parseActivePane(x: unknown, g: GridId): number {
  return Number.isInteger(x) && (x as number) >= 0 && (x as number) < paneCount(g)
    ? (x as number)
    : 0;
}

function read(key: string): unknown {
  if (typeof localStorage === "undefined") return undefined;
  try {
    const raw = localStorage.getItem(key);
    return raw === null ? undefined : JSON.parse(raw);
  } catch {
    return undefined;
  }
}

function write(key: string, value: unknown): void {
  if (typeof localStorage === "undefined") return;
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // Quota exceeded or private-mode storage: best-effort only.
  }
}

const initialGrid = parseGrid(read(GRID_KEY));
const [grid, setGridSignal] = createSignal<GridId>(initialGrid);
const [activePane, setActivePaneSignal] = createSignal(
  parseActivePane(read(ACTIVE_KEY), initialGrid),
);

export { activePane, grid };

export function setActivePane(pane: number): void {
  if (pane === activePane()) return;
  setActivePaneSignal(pane);
  write(ACTIVE_KEY, pane);
}

/** Set the grid. An active pane the new grid does not have hands over to the first. */
export function setGrid(next: GridId): void {
  setGridSignal(next);
  write(GRID_KEY, next);
  if (activePane() >= paneCount(next)) setActivePane(0);
}

const PaneContext = createContext(0);

/** Provides the pane to what renders inside a chart cell. */
export const PaneProvider = PaneContext.Provider;

/** The pane this component renders in; 0 outside a grid. */
export const usePane = (): number => useContext(PaneContext);
