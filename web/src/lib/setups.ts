// Setups (ADR-0059) as the Setups panel shows them (ADR-0071): their order, the
// open/closed filter, what a row says, and the drawings a click puts on the chart.
// Everything the server scores is taken as it answers; nothing is computed again here.

import { parseRecord, type DrawingRecord } from "../stores/drawings";
import type { Setup } from "../types/responses";

export const SETUP_FILTERS = ["open", "closed", "all"] as const;
export type SetupFilter = (typeof SETUP_FILTERS)[number];

/** Watching or triggered: the server still moves it. */
export const isOpen = (s: Setup): boolean => s.status === "watching" || s.status === "triggered";

/** When a closed setup ended, as far as the record says: its exit, or the last bar scored. */
const endedAt = (s: Setup): number => s.exit?.bar_time ?? s.scored_through ?? s.created_at;

/**
 * Open setups first, newest first; then closed ones, the most recently ended first. Ties
 * go by id, so the order is the same on every refresh.
 */
export function sortSetups(list: readonly Setup[]): Setup[] {
  const key = (s: Setup): [number, number] => (isOpen(s) ? [0, -s.created_at] : [1, -endedAt(s)]);
  return [...list].sort((a, b) => {
    const [ka, kb] = [key(a), key(b)];
    return ka[0] - kb[0] || ka[1] - kb[1] || a.id.localeCompare(b.id);
  });
}

export function filterSetups(list: readonly Setup[], filter: SetupFilter): Setup[] {
  if (filter === "all") return [...list];
  return list.filter((s) => isOpen(s) === (filter === "open"));
}

/**
 * "won" below 0 R: price reached the first target, but the fill was at or beyond it, or so
 * near it that the costs of getting in and out were more than the move. The status keeps
 * its meaning for the agent (ADR-0071); the row says what happened.
 */
export function wonAtALoss(s: Setup): boolean {
  return s.status === "won" && s.result_r !== null && s.result_r < 0;
}

export const WON_AT_A_LOSS =
  "Reached the first target, but below 0 R after costs: the fill was at or past the target, or too near it to cover the costs of getting in and out.";

/** The fill once there is one, else the entry zone, one price when it is a level. */
export function entryText(s: Setup, price: (p: number) => string): string {
  if (s.fill) return `fill ${price(s.fill.price)}`;
  const { low, high } = s.entry;
  return low === high ? price(low) : `${price(low)}–${price(high)}`;
}

/** A result in R with its sign, two places; "–" while there is none. */
export function formatR(r: number | null | undefined): string {
  if (r === null || r === undefined) return "–";
  const text = Math.abs(r).toFixed(2);
  return `${r < 0 ? "−" : r > 0 ? "+" : ""}${text}R`;
}

/** A stored drawing (ADR-0054, as `generate_chart` resolved it) as one of the user's own. */
function asRecord(d: Setup["drawings"][number]): DrawingRecord | null {
  return parseRecord({
    name: d.tool,
    points: d.points.map((p) => ({ timestamp: p.timestamp, value: p.price })),
    style: d.style,
    extendData: d.text,
    lock: false,
    visible: true,
  });
}

/**
 * The server's bracket of a skip as a long or short position: from its fill (or, unfilled,
 * from when it was made, at its entry level) to its exit (or its expiry), with the target
 * and the invalidation the server scores it with.
 */
export function bracketDrawing(s: Setup): DrawingRecord | null {
  const target = s.targets[0];
  if (target === undefined) return null;
  const left = s.fill?.bar_time ?? s.created_at;
  const right = s.exit && s.exit.bar_time > left ? s.exit.bar_time : s.expires_at;
  const entry = s.fill?.price ?? s.entry.low;
  return parseRecord({
    name: s.direction === "long" ? "cgLong" : "cgShort",
    points: [
      { timestamp: left, value: entry },
      { timestamp: right, value: target },
      { timestamp: right, value: s.invalidation },
    ],
    lock: false,
    visible: true,
  });
}

/** What a click puts on the chart: the setup's stored drawings, and a skip's bracket. */
export function setupDrawings(s: Setup): DrawingRecord[] {
  const stored = s.drawings.map(asRecord);
  const bracket = s.decision === "skip" ? [bracketDrawing(s)] : [];
  return [...stored, ...bracket].filter((r): r is DrawingRecord => r !== null);
}
