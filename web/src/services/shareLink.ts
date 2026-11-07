// Shareable chart links (ADR-0029): apply the share params of the URL once
// at boot, and copy the current chart's link. Parsing lives in lib/shareLink.

import { createSignal } from "solid-js";

import {
  decodeShare,
  encodeShare,
  hasShareParams,
  reconcileIndicators,
  stripShareParams,
  type RegistryEntry,
} from "../lib/shareLink";
import {
  compareSymbols,
  addCompare,
  removeCompare,
} from "../stores/compare";
import { activePane } from "../stores/grid";
import {
  activeIndicators,
  indicatorStoreOf,
  instanceKey,
  normalizeInstances,
  setActiveIndicators,
  type ActiveIndicator,
} from "../stores/indicators";
import {
  chartParams,
  chartType,
  setChartParams,
  setChartType,
  setSymbol,
  setTimeframe,
  setTimeZone,
  symbol,
  timeframe,
  timeZone,
} from "../stores/layout";
import { paramDefaults } from "../stores/layouts";
import { showToast } from "./alertEngine";

// Raw indicator entries a link brought in, untyped and unchecked. Kept only in
// sessionStorage (survives the OAuth redirect of the tab) and written to the
// indicator store once `reconcileImported` has validated them against the
// registry, so another tab or a closed tab never loads unvalidated entries.
const PENDING_KEY = "chartgen.share.pending";

function loadPending(): ActiveIndicator[] {
  try {
    const raw: unknown = JSON.parse(sessionStorage.getItem(PENDING_KEY) ?? "[]");
    return Array.isArray(raw)
      ? normalizeInstances(
          raw.filter(
            (e): e is ActiveIndicator =>
              typeof e === "string" || (typeof e === "object" && e !== null),
          ),
        )
      : [];
  } catch {
    return [];
  }
}

function savePending(entries: ActiveIndicator[]): void {
  try {
    if (entries.length === 0) sessionStorage.removeItem(PENDING_KEY);
    else sessionStorage.setItem(PENDING_KEY, JSON.stringify(entries));
  } catch {
    // Storage unavailable: the entries are still checked in this page.
  }
}

const [pending, setPending] = createSignal<ActiveIndicator[]>(loadPending());
// The chart a link opens in, whichever is active once the registry has answered.
const importPane = activePane();

/** True while imported indicators wait for the registry; hold the chart query. */
export const importPending = (): boolean => pending().length > 0;

/**
 * Apply the share params of the current URL to the stores and remove them
 * from the address bar (other params, such as an OAuth `code`, stay). Call
 * once before the first render. Because the values go through the normal
 * setters they persist, so they also survive the OAuth redirect.
 */
export function applyShareLink(): void {
  if (typeof window === "undefined") return;
  const { search, pathname, hash } = window.location;
  if (!hasShareParams(search)) return;
  const patch = decodeShare(search);
  if (patch.symbol !== undefined) setSymbol(patch.symbol);
  if (patch.timeframe !== undefined) setTimeframe(patch.timeframe);
  if (patch.chartType !== undefined) setChartType(patch.chartType);
  if (patch.chartParams !== undefined) setChartParams(patch.chartParams);
  if (patch.timeZone !== undefined) setTimeZone(patch.timeZone);
  if (patch.indicators !== undefined) {
    savePending(patch.indicators);
    setPending(patch.indicators);
    // An explicit empty `ind=` clears; nothing to validate, so apply now.
    if (patch.indicators.length === 0) setActiveIndicators([]);
  }
  // Skip an unchanged list: replacing it would reset the scale mode.
  if (
    patch.compare !== undefined &&
    patch.compare.join(",") !== compareSymbols().join(",")
  ) {
    for (const s of compareSymbols()) removeCompare(s);
    for (const s of patch.compare) addCompare(s, symbol());
  }
  window.history.replaceState({}, "", pathname + stripShareParams(search) + hash);
}

/**
 * Check the indicators a link brought in against the registry (`undefined`
 * when it could not be loaded) and release the chart query. Only those
 * names are touched: unknown ones are removed, invalid params dropped, and
 * a toast says what was left out.
 */
export function reconcileImported(
  registry: readonly RegistryEntry[] | undefined,
): void {
  const raw = pending();
  if (raw.length === 0) return;
  const { entries, unknown, invalid } = reconcileIndicators(
    raw,
    raw.map(instanceKey),
    registry,
  );
  indicatorStoreOf(importPane).setActiveIndicators(entries);
  savePending([]);
  setPending([]);
  if (unknown.length > 0) {
    showToast(`Unknown indicator(s) left out: ${unknown.join(", ")}`);
  }
  if (invalid.length > 0) {
    showToast(`Invalid parameter(s) ignored: ${invalid.join(", ")}`);
  }
}

/** URL of the current chart. */
export function chartLink(): string {
  const query = encodeShare(
    {
      symbol: symbol(),
      timeframe: timeframe(),
      chartType: chartType(),
      chartParams: chartParams(),
      indicators: activeIndicators(),
      compare: compareSymbols(),
      timeZone: timeZone(),
    },
    paramDefaults(),
  );
  return `${window.location.origin}/?${query}`;
}

/** Copy through a readonly input when the async clipboard API is unavailable. */
function copyViaInput(text: string): boolean {
  const input = document.createElement("input");
  input.readOnly = true;
  input.value = text;
  input.setAttribute("aria-hidden", "true");
  input.style.cssText = "position:fixed;top:0;left:0;opacity:0";
  document.body.appendChild(input);
  try {
    input.select();
    return document.execCommand("copy");
  } catch {
    return false;
  } finally {
    input.remove();
  }
}

/** Put text on the clipboard; false when the browser refuses. */
export async function copyText(text: string): Promise<boolean> {
  try {
    // Undefined on non-secure origins; rejects without permission.
    if (!navigator.clipboard) throw new Error("clipboard unavailable");
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return copyViaInput(text);
  }
}

/** Put the chart's link on the clipboard and confirm with a toast. */
export async function copyChartLink(): Promise<void> {
  showToast((await copyText(chartLink())) ? "Link copied" : "Copy failed");
}
