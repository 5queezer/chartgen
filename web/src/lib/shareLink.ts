// Shareable chart links (ADR-0029): encode the chart into query params and
// decode them again. Pure functions; the stores are touched by
// services/shareLink.ts. Decoding never throws — every field that is
// missing or invalid is left out so the stored state stays.
//
//   ?symbol=ETHUSDT&interval=4h&type=candles&ind=rsi(length:21),ema_stack
//    &compare=BTCUSDT,SOLUSDT&tz=Europe/Berlin
//
// `ind` is a comma list of names with optional `(key:value;key:value)`
// params (only non-default ones are written). A param value is a number,
// `true`/`false`, or a word of [A-Za-z0-9._-]; anything else is dropped.

import { parseCompare } from "../stores/compare";
import {
  CHART_TYPES,
  normalizeSymbol,
  type ChartType,
} from "../stores/layout";
import {
  indicatorName,
  indicatorParams,
  isHidden,
  type ActiveIndicator,
} from "../stores/indicators";
import { parseTimeframe, type Timeframe } from "./timeframe";
import { isTimeZoneChoice, type TimeZoneChoice } from "./timezone";

/** Query params owned by share links; everything else (OAuth `code`) is left alone. */
export const SHARE_PARAMS = [
  "symbol",
  "interval",
  "type",
  "ind",
  "compare",
  "tz",
] as const;

export interface ShareState {
  symbol: string;
  timeframe: Timeframe;
  chartType: ChartType;
  indicators: ActiveIndicator[];
  compare: string[];
  timeZone: TimeZoneChoice;
}

/** What a link carried; absent fields keep the stored state. */
export type SharePatch = Partial<ShareState>;

/** Default params per indicator name, from the registry. */
export type ShareDefaults = Record<string, Record<string, unknown>>;

const NAME = /^[A-Za-z0-9_]{1,40}$/;
const KEY = /^[A-Za-z_][A-Za-z0-9_]{0,40}$/;
const WORD = /^[A-Za-z0-9._-]{1,40}$/;

/** Split on commas outside parentheses. */
function splitTop(s: string): string[] {
  const out: string[] = [];
  let depth = 0;
  let cur = "";
  for (const ch of s) {
    if (ch === "(") depth++;
    else if (ch === ")") depth = Math.max(0, depth - 1);
    if (ch === "," && depth === 0) {
      out.push(cur);
      cur = "";
    } else cur += ch;
  }
  out.push(cur);
  return out.map((p) => p.trim()).filter((p) => p !== "");
}

function decodeValue(v: string): unknown {
  if (v === "true") return true;
  if (v === "false") return false;
  if (/^-?\d+(\.\d+)?$/.test(v)) return Number(v);
  return WORD.test(v) ? v : undefined;
}

function decodeIndicator(part: string): ActiveIndicator | null {
  const m = /^([^()]+?)\s*(?:\((.*)\))?$/.exec(part);
  const name = m?.[1]?.trim() ?? "";
  if (!NAME.test(name)) return null;
  const params: Record<string, unknown> = {};
  for (const kv of (m?.[2] ?? "").split(";")) {
    const i = kv.indexOf(":");
    if (i < 0) continue;
    const key = kv.slice(0, i).trim();
    const value = decodeValue(kv.slice(i + 1).trim());
    if (KEY.test(key) && value !== undefined && key !== "name") {
      // defineProperty-safe: KEY allows `__proto__`, which must stay data.
      Object.defineProperty(params, key, {
        value,
        enumerable: true,
        writable: true,
        configurable: true,
      });
    }
  }
  return Object.keys(params).length === 0 ? name : { name, ...params };
}

/** Indicators of an `ind` value; malformed items and duplicate names are skipped. */
export function decodeIndicators(raw: string): ActiveIndicator[] {
  const seen = new Set<string>();
  const out: ActiveIndicator[] = [];
  for (const part of splitTop(raw)) {
    const entry = decodeIndicator(part);
    if (entry === null) continue;
    const name = indicatorName(entry);
    if (seen.has(name)) continue;
    seen.add(name);
    out.push(entry);
  }
  return out;
}

function encodeValue(v: unknown): string | null {
  if (typeof v === "number") return Number.isFinite(v) ? String(v) : null;
  if (typeof v === "boolean") return String(v);
  return typeof v === "string" && WORD.test(v) ? v : null;
}

/** `ind` value for the visible entries; params equal to `defaults` are left out. */
export function encodeIndicators(
  entries: readonly ActiveIndicator[],
  defaults: ShareDefaults = {},
): string {
  return entries
    .filter((e) => !isHidden(e) && NAME.test(indicatorName(e)))
    .map((e) => {
      const name = indicatorName(e);
      const pairs = Object.entries(indicatorParams(e))
        .filter(([k, v]) => KEY.test(k) && v !== defaults[name]?.[k])
        .flatMap(([k, v]) => {
          const s = encodeValue(v);
          return s === null ? [] : [`${k}:${s}`];
        });
      return pairs.length === 0 ? name : `${name}(${pairs.join(";")})`;
    })
    .join(",");
}

/** Query string (no leading `?`) for a chart; readable characters stay unescaped. */
export function encodeShare(
  s: ShareState,
  defaults: ShareDefaults = {},
): string {
  const p = new URLSearchParams();
  p.set("symbol", s.symbol);
  p.set("interval", s.timeframe);
  p.set("type", s.chartType);
  // `ind=` (empty) means "no indicators", which differs from absent.
  p.set("ind", encodeIndicators(s.indicators, defaults));
  if (s.compare.length > 0) p.set("compare", s.compare.join(","));
  p.set("tz", s.timeZone);
  return p
    .toString()
    .replace(/%(2C|3A|28|29|3B|2F)/g, (_, h: string) =>
      String.fromCharCode(parseInt(h, 16)),
    );
}

/** True when the query carries at least one share param. */
export function hasShareParams(search: string): boolean {
  const p = new URLSearchParams(search);
  return SHARE_PARAMS.some((k) => p.has(k));
}

/** Decode a query string; invalid fields are omitted, never thrown on. */
export function decodeShare(search: string): SharePatch {
  const p = new URLSearchParams(search);
  const patch: SharePatch = {};

  const symbol = normalizeSymbol(p.get("symbol"));
  if (symbol !== null) patch.symbol = symbol;

  const interval = p.get("interval");
  const timeframe = interval === null ? null : parseTimeframe(interval);
  if (timeframe !== null) patch.timeframe = timeframe;

  const type = p.get("type")?.trim().toLowerCase();
  const chartType = CHART_TYPES.find((t) => t === type);
  if (chartType !== undefined) patch.chartType = chartType;

  const ind = p.get("ind");
  if (ind !== null) {
    const indicators = decodeIndicators(ind);
    // A non-empty value that yields nothing is garbage, not "no indicators".
    if (indicators.length > 0 || ind.trim() === "") patch.indicators = indicators;
  }

  const compare = p.get("compare");
  if (compare !== null) patch.compare = parseCompare(compare.split(","));

  const tz = p.get("tz");
  if (isTimeZoneChoice(tz)) patch.timeZone = tz;

  return patch;
}

/** `search` without the share params (OAuth params and others kept); "" or "?…". */
export function stripShareParams(search: string): string {
  const p = new URLSearchParams(search);
  for (const k of SHARE_PARAMS) p.delete(k);
  const rest = p.toString();
  return rest === "" ? "" : `?${rest}`;
}
