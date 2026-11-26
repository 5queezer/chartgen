// The Setups page (ADR-0091): its filters, the columns its tables sort by, what its
// chart plots and which groupings it offers. Counts and sums of the record are the
// server's (`get_track_record`); the chart only plots each setup's own result.

import type { GetTrackRecordInput } from "../types/generated-input";
import type { SetupRow, SetupStatus } from "../types/responses";
import { endedAt, isOpen } from "./setups";

export interface Filters {
  status: "all" | SetupStatus;
  decision: "all" | "take" | "skip";
  direction: "all" | "long" | "short";
  /** "" for every symbol, tag. */
  symbol: string;
  tag: string;
  /** Days as `YYYY-MM-DD` in the browser's time zone, both included; "" for open-ended. */
  from: string;
  to: string;
}

export const NO_FILTERS: Filters = {
  status: "all",
  decision: "all",
  direction: "all",
  symbol: "",
  tag: "",
  from: "",
  to: "",
};

/** The start of a `YYYY-MM-DD` day in the browser's time zone, `days` later; null for "". */
const dayStart = (day: string, days = 0): number | null => {
  if (day === "") return null;
  const d = new Date(`${day}T00:00`);
  d.setDate(d.getDate() + days);
  return Number.isNaN(d.getTime()) ? null : d.getTime();
};
/** `from` and `to` as the record takes them: made at or after, and before the day after `to`. */
const range = (f: Filters) => ({ from: dayStart(f.from), to: dayStart(f.to, 1) });

/** The newest setups, and the open ones that are older than those: each once. */
export function withOpen<T extends { id: string }>(newest: readonly T[], open: readonly T[]): T[] {
  const listed = new Set(newest.map((s) => s.id));
  return [...newest, ...open.filter((s) => !listed.has(s.id))];
}

/** The setups the filters let through. A day range is about when a setup was made. */
export function filterRows<T extends SetupRow>(rows: readonly T[], f: Filters): T[] {
  const { from, to } = range(f);
  return rows.filter(
    (s) =>
      (f.status === "all" || s.status === f.status) &&
      (f.decision === "all" || s.decision === f.decision) &&
      (f.direction === "all" || s.direction === f.direction) &&
      (f.symbol === "" || s.symbol === f.symbol) &&
      (f.tag === "" || s.tags.includes(f.tag)) &&
      (from === null || s.created_at >= from) &&
      (to === null || s.created_at < to),
  );
}

/**
 * The same filters as `get_track_record` takes them. Status and tag are not among its
 * filters: the record counts closed setups, and groups by tag instead.
 */
export function recordArgs(f: Filters): GetTrackRecordInput {
  const { from, to } = range(f);
  return {
    ...(f.symbol !== "" && { symbol: f.symbol }),
    ...(f.direction !== "all" && { direction: f.direction }),
    ...(f.decision !== "all" && { decision: f.decision }),
    ...(from !== null && { from }),
    ...(to !== null && { to }),
  };
}

export const COLUMNS = [
  "symbol",
  "side",
  "status",
  "entry",
  "stop",
  "target",
  "mfe",
  "mae",
  "result",
  "made",
  "ended",
  "expires",
] as const;
export type Column = (typeof COLUMNS)[number];

const SORT_KEY: Record<Column, (s: SetupRow) => number | string | null> = {
  symbol: (s) => s.symbol,
  side: (s) => `${s.direction} ${s.decision}`,
  status: (s) => s.status,
  entry: (s) => s.fill?.price ?? s.entry.low,
  stop: (s) => s.invalidation,
  target: (s) => s.targets[0] ?? null,
  mfe: (s) => s.mfe_r ?? null,
  mae: (s) => s.mae_r ?? null,
  result: (s) => s.result_r,
  made: (s) => s.created_at,
  ended: (s) => (isOpen(s) ? null : endedAt(s)),
  expires: (s) => s.expires_at,
};

export interface Sort {
  column: Column;
  descending: boolean;
}

/** Sorted by one column; setups without a value last either way, ties by id. */
export function sortRows<T extends SetupRow>(rows: readonly T[], sort: Sort): T[] {
  const key = SORT_KEY[sort.column];
  const sign = sort.descending ? -1 : 1;
  return [...rows].sort((a, b) => {
    const [ka, kb] = [key(a), key(b)];
    if (ka === null || kb === null) return ka === kb ? a.id.localeCompare(b.id) : ka === null ? 1 : -1;
    const c = typeof ka === "number" && typeof kb === "number" ? ka - kb : String(ka).localeCompare(String(kb));
    return c * sign || a.id.localeCompare(b.id);
  });
}

/** A click on a column's header: the same column flips, another starts with its natural order. */
export function nextSort(sort: Sort, column: Column): Sort {
  if (sort.column === column) return { column, descending: !sort.descending };
  return { column, descending: ["made", "ended", "result", "mfe"].includes(column) };
}

/** `YYYY-MM-DD` in the browser's time zone. */
export const localDay = (ms: number): string => new Date(ms).toLocaleDateString("en-CA");

export interface DayResult {
  day: string;
  /** The day's results, and the running sum through it. */
  r: number;
  cumulative: number;
  count: number;
}

/**
 * Each closed setup's own result (as the server scored it) on the day it ended, and the
 * running sum. A plot of the listed setups, not the record's totals: those are
 * `get_track_record`'s, which reads more setups than one list holds.
 */
export function resultsByDay(rows: readonly SetupRow[]): DayResult[] {
  const days = new Map<string, { r: number; count: number }>();
  for (const s of rows) {
    if (isOpen(s) || s.result_r === null) continue;
    const day = localDay(endedAt(s));
    const d = days.get(day) ?? { r: 0, count: 0 };
    days.set(day, { r: d.r + s.result_r, count: d.count + 1 });
  }
  let cumulative = 0;
  return [...days.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([day, d]) => ({ day, r: d.r, count: d.count, cumulative: (cumulative += d.r) }));
}

/** The most prefixes `get_track_record` takes at once (MAX_TAGS on the server). */
export const MAX_PREFIXES = 10;

export interface TagGrouping {
  label: string;
  prefixes: string[];
}

/**
 * The tag groupings worth offering for these setups: by family (the part of a tag up to
 * its first `:`), and within each family by its tags, the most used first.
 */
export function tagGroupings(rows: readonly SetupRow[]): TagGrouping[] {
  const uses = new Map<string, number>();
  for (const s of rows) for (const t of s.tags) uses.set(t, (uses.get(t) ?? 0) + 1);
  const byUse = (counts: Map<string, number>) =>
    [...counts.entries()]
      .sort(([a, x], [b, y]) => y - x || a.localeCompare(b))
      .map(([k]) => k)
      .slice(0, MAX_PREFIXES);
  const families = new Map<string, Map<string, number>>();
  for (const [tag, n] of uses) {
    const colon = tag.indexOf(":");
    if (colon <= 0) continue;
    const family = tag.slice(0, colon + 1);
    const of = families.get(family) ?? new Map<string, number>();
    families.set(family, of.set(tag, n));
  }
  const familyUses = new Map([...families].map(([f, of]) => [f, [...of.values()].reduce((a, b) => a + b, 0)]));
  const out: TagGrouping[] = [];
  if (families.size > 1) out.push({ label: "tag families", prefixes: byUse(familyUses) });
  for (const family of byUse(familyUses)) {
    out.push({ label: `${family}…`, prefixes: byUse(families.get(family)!) });
  }
  if (uses.size > 0) out.push({ label: "tags", prefixes: byUse(uses) });
  return out;
}

/** The names of the features the setups were made with, for a grouping by one. */
export function featureNames(rows: readonly SetupRow[]): string[] {
  return [...new Set(rows.flatMap((s) => Object.keys(s.features ?? {})))].sort();
}

/** How long ago, coarse: "45m", "5h", "3d 4h". */
export function age(ms: number, now: number): string {
  const minutes = Math.max(0, Math.floor((now - ms) / 60_000));
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h`;
  return `${Math.floor(hours / 24)}d ${hours % 24}h`;
}
