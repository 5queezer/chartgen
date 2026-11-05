// Shareable chart links (ADR-0029): encode the chart into query params and
// decode them again. Pure functions; the stores are touched by
// services/shareLink.ts. Decoding never throws — every field that is
// missing or invalid is left out so the stored state stays.
//
//   ?symbol=ETHUSDT&interval=4h&type=candles&ind=rsi(length:21),ema_stack
//    &compare=BTCUSDT,SOLUSDT&tz=Europe/Berlin
//
// `ind` is a comma list of names with optional `(key:value;key:value)`
// params (only non-default ones are written). Values are written as plain
// numbers (no exponent), `true`/`false`, or text with everything outside
// [A-Za-z0-9._-] percent-encoded. Decoding keeps every value as the raw
// string: the type comes from the indicator registry (`reconcileIndicators`),
// which also drops params the registry rejects.

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
import type { ParamDefaults } from "../stores/layouts";
import type { ParamSpec } from "../types/responses";
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

/** Longest `ind` value and most indicators a link may carry. */
export const MAX_IND_LENGTH = 2048;
export const MAX_INDICATORS = 50;
const MAX_VALUE_LENGTH = 200;

const NAME = /^[A-Za-z0-9_]{1,40}$/;
const KEY = /^[A-Za-z_][A-Za-z0-9_]{0,40}$/;
/** Client-only fields and the name never come from a link. */
const RESERVED_KEYS = new Set(["name", "hidden", "style"]);

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

/** Raw text of a value: percent-escapes undone; undefined when malformed or too long. */
function decodeValue(v: string): string | undefined {
  if (v === "" || v.length > MAX_VALUE_LENGTH) return undefined;
  try {
    const text = decodeURIComponent(v);
    return /[\u0000-\u001f]/.test(text) ? undefined : text;
  } catch {
    return undefined;
  }
}

function decodeIndicator(part: string): ActiveIndicator | null {
  const open = part.indexOf("(");
  if (open >= 0 && !part.endsWith(")")) return null;
  const name = (open < 0 ? part : part.slice(0, open)).trim();
  if (!NAME.test(name)) return null;
  const params: Record<string, unknown> = {};
  const inner = open < 0 ? "" : part.slice(open + 1, -1);
  for (const kv of inner.split(";")) {
    const i = kv.indexOf(":");
    if (i < 0) continue;
    const key = kv.slice(0, i).trim();
    const value = decodeValue(kv.slice(i + 1).trim());
    if (KEY.test(key) && !RESERVED_KEYS.has(key) && value !== undefined) {
      // defineProperty keeps a `__proto__` key as plain data.
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

/**
 * Indicators of an `ind` value, params as raw strings. Malformed items and
 * duplicate names are skipped; a value over MAX_IND_LENGTH is ignored whole
 * and at most MAX_INDICATORS entries are kept.
 */
export function decodeIndicators(raw: string): ActiveIndicator[] {
  if (raw.length > MAX_IND_LENGTH) return [];
  const seen = new Set<string>();
  const out: ActiveIndicator[] = [];
  for (const part of splitTop(raw)) {
    if (out.length >= MAX_INDICATORS) break;
    const entry = decodeIndicator(part);
    if (entry === null) continue;
    const name = indicatorName(entry);
    if (seen.has(name)) continue;
    seen.add(name);
    out.push(entry);
  }
  return out;
}

/** Decimal text of a finite number, never in exponent notation. */
function numberText(n: number): string {
  const t = String(n);
  if (!/e/i.test(t)) return t;
  if (Math.abs(n) >= 1e21) return BigInt(n).toString();
  return n.toFixed(20).replace(/(\.\d*?)0+$/, "$1").replace(/\.$/, "");
}

/** Percent-encode every UTF-8 byte outside [A-Za-z0-9._-]. */
function textValue(v: string): string {
  return Array.from(new TextEncoder().encode(v), (b) =>
    /[A-Za-z0-9._-]/.test(String.fromCharCode(b))
      ? String.fromCharCode(b)
      : `%${b.toString(16).toUpperCase().padStart(2, "0")}`,
  ).join("");
}

function encodeValue(v: unknown): string | null {
  if (typeof v === "number") return Number.isFinite(v) ? numberText(v) : null;
  if (typeof v === "boolean") return String(v);
  return typeof v === "string" && v !== "" && v.length <= MAX_VALUE_LENGTH
    ? textValue(v)
    : null;
}

/** `ind` value for the visible entries; params equal to `defaults` are left out. */
export function encodeIndicators(
  entries: readonly ActiveIndicator[],
  defaults: ParamDefaults = {},
): string {
  return entries
    .filter((e) => !isHidden(e) && NAME.test(indicatorName(e)))
    .map((e) => {
      const name = indicatorName(e);
      const pairs = Object.entries(indicatorParams(e))
        .filter(
          ([k, v]) =>
            KEY.test(k) && !RESERVED_KEYS.has(k) && v !== defaults[name]?.[k],
        )
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
  defaults: ParamDefaults = {},
): string {
  const p = new URLSearchParams();
  p.set("symbol", s.symbol);
  p.set("interval", s.timeframe);
  p.set("type", s.chartType);
  // `ind=` (empty) means "no indicators", which differs from absent.
  p.set("ind", encodeIndicators(s.indicators, defaults));
  // `compare=` (empty) clears the receiver's compare symbols.
  p.set("compare", s.compare.join(","));
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
  const keyword = tz?.toLowerCase();
  const tzChoice =
    keyword === "utc" || keyword === "exchange" || keyword === "local"
      ? keyword
      : tz;
  if (isTimeZoneChoice(tzChoice)) patch.timeZone = tzChoice;

  return patch;
}

/** `search` without the share params (OAuth params and others kept); "" or "?…". */
export function stripShareParams(search: string): string {
  const p = new URLSearchParams(search);
  for (const k of SHARE_PARAMS) p.delete(k);
  const rest = p.toString();
  return rest === "" ? "" : `?${rest}`;
}

/** Registry facts `reconcileIndicators` needs: name, aliases and param specs. */
export interface RegistryEntry {
  name: string;
  aliases?: string[];
  configurable_params?: ParamSpec[];
}

/** A param value typed by its registry spec; undefined when the spec rejects it. */
export function coerceParam(spec: ParamSpec, raw: unknown): unknown {
  const text = typeof raw === "string" ? raw.trim() : String(raw);
  switch (spec.type) {
    case "boolean":
      return text === "true" ? true : text === "false" ? false : undefined;
    case "string":
      if (spec.enum) return spec.enum.includes(text) ? text : undefined;
      return text === "" ? undefined : text;
    default: {
      const integer = spec.type === "integer";
      if (!/^-?\d+(\.\d+)?$/.test(text)) return undefined;
      const n = Number(text);
      if (integer ? !Number.isSafeInteger(n) : !Number.isFinite(n)) {
        return undefined;
      }
      // Integer params are counts/lengths: 0 can panic the server (paramForm).
      const min = spec.min ?? (integer ? 1 : undefined);
      if (min !== undefined && n < min) return undefined;
      if (spec.max !== undefined && n > spec.max) return undefined;
      return n;
    }
  }
}

/** Type a raw value by its look, for when no registry is available. */
function guess(raw: unknown): unknown {
  if (typeof raw !== "string") return raw;
  if (raw === "true") return true;
  if (raw === "false") return false;
  return /^-?\d+(\.\d+)?$/.test(raw) ? Number(raw) : raw;
}

export interface Reconciled {
  entries: ActiveIndicator[];
  /** Imported names the registry does not know; removed from `entries`. */
  unknown: string[];
  /** `name.param` of imported params the registry rejected; defaults apply. */
  invalid: string[];
}

/**
 * Check the entries named in `pending` (what a link brought in) against the
 * registry: unknown names are removed, params are typed by their spec and
 * the ones that fail (or are not params of the indicator) are dropped. Other
 * entries are untouched. Without a registry (`undefined`, the list could not
 * be loaded) every name is kept and params are typed by their look.
 */
export function reconcileIndicators(
  entries: readonly ActiveIndicator[],
  pending: readonly string[],
  registry: readonly RegistryEntry[] | undefined,
): Reconciled {
  const out: ActiveIndicator[] = [];
  const unknown: string[] = [];
  const invalid: string[] = [];
  for (const entry of entries) {
    const name = indicatorName(entry);
    if (!pending.includes(name)) {
      out.push(entry);
      continue;
    }
    const info = registry?.find(
      (i) => i.name === name || i.aliases?.includes(name),
    );
    if (registry !== undefined && info === undefined) {
      unknown.push(name);
      continue;
    }
    const params: Record<string, unknown> = {};
    for (const [key, raw] of Object.entries(indicatorParams(entry))) {
      const spec = info?.configurable_params?.find((p) => p.name === key);
      const value = spec
        ? coerceParam(spec, raw)
        : info === undefined
          ? guess(raw)
          : undefined;
      if (value === undefined) invalid.push(`${name}.${key}`);
      else {
        Object.defineProperty(params, key, {
          value,
          enumerable: true,
          writable: true,
          configurable: true,
        });
      }
    }
    out.push(Object.keys(params).length === 0 ? name : { name, ...params });
  }
  return { entries: out, unknown, invalid };
}
