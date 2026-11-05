// Compare symbols of the chart (ADR-0022): other symbols drawn as lines over
// the price series. Persisted under `chartgen.layout.compare`, validated on
// load, capped at MAX_COMPARE. The first compare symbol switches the price
// axis to percent, as TradingView does; the scale mode it replaced is kept
// (`compareRestore`, persisted so it survives a reload) and comes back when
// the last one is removed.

import { createSignal } from "solid-js";

import { MAX_COMPARE } from "../lib/compare";
import {
  DEFAULT_SCALE_MODE,
  isScaleMode,
  normalizeSymbol,
  persist,
  read,
  scaleMode,
  setScaleMode,
  type ScaleMode,
} from "./layout";

/** Valid, unique, normalised symbols of a stored or imported value, capped. */
export function parseCompare(x: unknown): string[] {
  if (!Array.isArray(x)) return [];
  const symbols = x
    .map(normalizeSymbol)
    .filter((s): s is string => s !== null);
  return [...new Set(symbols)].slice(0, MAX_COMPARE);
}

const initial = parseCompare(read("compare"));
const storedRestore = read("compareRestore");

const [compareSymbols, setCompareSignal] = createSignal<string[]>(initial);
// Scale mode to return to when the last compare symbol goes; null while none.
let restore: ScaleMode | null =
  initial.length === 0
    ? null
    : isScaleMode(storedRestore)
      ? storedRestore
      : DEFAULT_SCALE_MODE;

export { compareSymbols };

function commit(list: string[]): void {
  setCompareSignal(list);
  persist("compare", list);
  persist("compareRestore", restore);
}

/**
 * Add a compare symbol; false if it is invalid, already there, the same as
 * `main` or the cap is reached. The first one switches the axis to percent.
 */
export function addCompare(next: string, main: string): boolean {
  const value = normalizeSymbol(next);
  const list = compareSymbols();
  if (
    value === null ||
    value === main ||
    list.includes(value) ||
    list.length >= MAX_COMPARE
  ) {
    return false;
  }
  if (list.length === 0) {
    restore = scaleMode();
    setScaleMode("percentage");
  }
  commit([...list, value]);
  return true;
}

/** Remove a compare symbol; the last one restores the previous scale mode. */
export function removeCompare(symbol: string): void {
  const list = compareSymbols();
  if (!list.includes(symbol)) return;
  const rest = list.filter((s) => s !== symbol);
  if (rest.length === 0 && restore !== null) {
    setScaleMode(restore);
    restore = null;
  }
  commit(rest);
}

/**
 * Replace the list without touching the scale mode: a layout switch sets the
 * mode itself (a layout stores it with its compare symbols).
 */
export function setCompareSymbols(next: unknown): void {
  const list = parseCompare(next);
  restore = list.length === 0 ? null : (restore ?? DEFAULT_SCALE_MODE);
  commit(list);
}
