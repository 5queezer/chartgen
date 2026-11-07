// Compare symbols of the chart (ADR-0022): other symbols drawn as lines over
// the price series. Persisted under `chartgen.layout.compare`, validated on
// load, capped at MAX_COMPARE. Each symbol keeps a color slot
// (`chartgen.layout.compareSlots`) so removing one does not recolor the
// others. The first compare symbol switches the price axis to percent, as
// TradingView does; the scale mode it replaced is kept (`compareRestore`,
// persisted so it survives a reload) and comes back when the last one is
// removed, unless the user has picked another mode since.

import { createSignal } from "solid-js";

import { MAX_COMPARE } from "../lib/compare";
import { isPriceBarType } from "../lib/priceBars";
import { activePane } from "./grid";
import {
  chartStoreOf,
  DEFAULT_SCALE_MODE,
  isScaleMode,
  normalizeSymbol,
  type ScaleMode,
} from "./layout";

export interface CompareEntry {
  symbol: string;
  /** Index into the compare palette. */
  slot: number;
}

/** Valid, unique, normalised symbols of a stored or imported value, capped. */
export function parseCompare(x: unknown): string[] {
  if (!Array.isArray(x)) return [];
  const symbols = x
    .map(normalizeSymbol)
    .filter((s): s is string => s !== null);
  return [...new Set(symbols)].slice(0, MAX_COMPARE);
}

/** Stored slots when they fit `count` symbols (unique, in range), else 0..count-1. */
function parseSlots(x: unknown, count: number): number[] {
  const ok =
    Array.isArray(x) &&
    x.length === count &&
    x.every((s) => Number.isInteger(s) && s >= 0 && s < MAX_COMPARE) &&
    new Set(x).size === count;
  return ok ? (x as number[]) : Array.from({ length: count }, (_, i) => i);
}

const firstFree = (used: readonly number[]): number => {
  let slot = 0;
  while (used.includes(slot)) slot++;
  return slot;
};

/** Compare symbols of one chart of the grid (ADR-0043). */
function createCompareStore(pane: number) {
  const { read, persist, scaleMode, setScaleMode, chartType } = chartStoreOf(pane);
  // Price-based bars have no time grid to join on (ADR-0047): the list stays for
  // when the chart type changes back, and acts as if empty until then.
  const drawn = () => !isPriceBarType(chartType());

  const initialSymbols = parseCompare(read("compare"));
  const initialSlots = parseSlots(read("compareSlots"), initialSymbols.length);
  const storedRestore = read("compareRestore");

  const [compareEntries, setEntries] = createSignal<CompareEntry[]>(
    initialSymbols.map((symbol, i) => ({ symbol, slot: initialSlots[i]! })),
  );
  // Scale mode to return to when the last compare symbol goes; null while none.
  let restore: ScaleMode | null =
    initialSymbols.length === 0
      ? null
      : isScaleMode(storedRestore)
        ? storedRestore
        : DEFAULT_SCALE_MODE;

  function commit(list: CompareEntry[]): void {
    setEntries(list);
    persist("compare", list.map((e) => e.symbol));
    persist("compareSlots", list.map((e) => e.slot));
    persist("compareRestore", restore);
  }

  return {
    compareEntries,

    compareSymbols: (): string[] => compareEntries().map((e) => e.symbol),

    /** The entries this chart draws. */
    drawnEntries: (): CompareEntry[] => (drawn() ? compareEntries() : []),

    /**
     * The chart type changed whether the list is drawn. Only a drawn compare holds
     * the axis in percent. A mode already there on the way back is the layout's or
     * the user's, and `restore` stays as it was set.
     */
    syncDrawn(): void {
      if (compareEntries().length === 0) return;
      if (!drawn()) {
        if (scaleMode() === "percentage") setScaleMode(restore ?? DEFAULT_SCALE_MODE);
      } else if (scaleMode() !== "percentage") {
        restore = scaleMode();
        setScaleMode("percentage");
        persist("compareRestore", restore);
      }
    },

    /** Scale mode the last compare symbol will return to; null while there are none. */
    compareRestore: (): ScaleMode | null => restore,

    /**
     * Add a compare symbol; false if it is invalid, already there, the same as
     * `main` or the cap is reached. The first one switches the axis to percent.
     */
    addCompare(next: string, main: string): boolean {
      const symbol = normalizeSymbol(next);
      const list = compareEntries();
      if (
        symbol === null ||
        symbol === main ||
        list.some((e) => e.symbol === symbol) ||
        list.length >= MAX_COMPARE
      ) {
        return false;
      }
      if (list.length === 0) {
        restore = scaleMode();
        if (drawn()) setScaleMode("percentage");
      }
      commit([...list, { symbol, slot: firstFree(list.map((e) => e.slot)) }]);
      return true;
    },

    /**
     * Remove a compare symbol. The last one restores the previous scale mode,
     * but only while the axis is still in percent: a mode the user picked since
     * stays.
     */
    removeCompare(symbol: string): void {
      const list = compareEntries();
      if (!list.some((e) => e.symbol === symbol)) return;
      const rest = list.filter((e) => e.symbol !== symbol);
      if (rest.length === 0) {
        if (restore !== null && scaleMode() === "percentage") setScaleMode(restore);
        restore = null;
      }
      commit(rest);
    },

    /**
     * Replace the list without touching the scale mode: a layout switch sets the
     * mode itself (a layout stores it with its compare symbols). Symbols already
     * shown keep their color slot. `restoreTo` is the layout's scale mode to return
     * to when the last compare symbol goes (ADR-0040); layouts saved without one
     * return to the default.
     */
    setCompareSymbols(next: unknown, restoreTo?: ScaleMode): void {
      const symbols = parseCompare(next);
      const kept = compareEntries().filter((e) => symbols.includes(e.symbol));
      const used = kept.map((e) => e.slot);
      const list = symbols.map((symbol) => {
        const had = kept.find((e) => e.symbol === symbol);
        if (had) return had;
        const slot = firstFree(used);
        used.push(slot);
        return { symbol, slot };
      });
      restore = list.length === 0 ? null : (restoreTo ?? DEFAULT_SCALE_MODE);
      commit(list);
    },
  };
}

export type CompareStore = ReturnType<typeof createCompareStore>;

const stores = new Map<number, CompareStore>();

export function compareStoreOf(pane: number): CompareStore {
  let store = stores.get(pane);
  if (!store) {
    store = createCompareStore(pane);
    stores.set(pane, store);
  }
  return store;
}

compareStoreOf(0);
const active = (): CompareStore => compareStoreOf(activePane());

// The active chart's compare symbols.
export const compareEntries = (): CompareEntry[] => active().compareEntries();
export const compareSymbols = (): string[] => active().compareSymbols();
export const drawnCompareEntries = (): CompareEntry[] => active().drawnEntries();
export const compareRestore = (): ScaleMode | null => active().compareRestore();
export const addCompare = (next: string, main: string): boolean =>
  active().addCompare(next, main);
export const removeCompare = (symbol: string): void => active().removeCompare(symbol);
export const setCompareSymbols = (next: unknown, restoreTo?: ScaleMode): void =>
  active().setCompareSymbols(next, restoreTo);
