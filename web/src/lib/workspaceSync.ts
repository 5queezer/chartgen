// A signed-in user's workspace in the browser (ADR-0050), with no I/O beyond the
// Storage it is given. localStorage stays what the stores read and write. This
// keeps each account's keys apart on a shared browser, and works out what to send
// and what to do with the answer, three-way against what the server last held.

export const MARKER = "chartgen.workspace.account";
const OFFER = "chartgen.workspace.legacyOffer";
/** The account that said yes to the offer, until a boot has done it with no store running. */
const IMPORT = "chartgen.workspace.import";
const LEGACY = "chartgen.legacy|";
const SYNC = "chartgen.sync.";
const CURSOR = SYNC + "cursor";
const INDEX = "chartgen.drawings.index";
/** Symbols the LRU dropped: no key of a symbol means this, or that someone deleted them all. */
export const EVICTED_KEY = "chartgen.drawingsEvicted";

/** Up to here a document goes to the server; past it the answer is known. */
const MAX_DOC_BYTES = 512 * 1024;
const MAX_WRITES = 50;
/** The server's limit (`MAX_BODY_BYTES` in `src/server/workspace.rs`). */
const MAX_BODY_BYTES = 1024 * 1024;
/** What a request holds besides its values: 50 keys, versions and the JSON around them. */
const BODY_OVERHEAD_BYTES = 16 * 1024;

const NOT_SCOPED = [
  "chartgen.oauth.",
  "chartgen.push.",
  "chartgen.u.",
  "chartgen.workspace.",
  LEGACY,
  "chartgen.share.",
];

/** Belongs to whoever is signed in. The browser's own keys stay where they are. */
export const isScoped = (key: string): boolean =>
  key.startsWith("chartgen.") && !NOT_SCOPED.some((p) => key.startsWith(p));

/** Follows the account to other devices. The rest of the scoped keys stay on this one. */
export function isSynced(key: string): boolean {
  if (key === INDEX) return false;
  return (
    key === "chartgen.layouts" ||
    key === "chartgen.indicators.active" ||
    key === "chartgen.alerts.list" ||
    key === "chartgen.chartSettings" ||
    key === "chartgen.orderflow" ||
    ["chartgen.layout.", "chartgen.pane.", "chartgen.watchlist.", "chartgen.drawings."].some((p) =>
      key.startsWith(p),
    )
  );
}

const keysOf = (s: Storage): string[] =>
  Array.from({ length: s.length }, (_, i) => s.key(i)).filter((k): k is string => k !== null);

const scopeOf = (account: string): string => `chartgen.u.${encodeURIComponent(account)}|`;

/**
 * Written before the live key goes: a running tab that sees a key gone also sees
 * its account's scope in use, and stops (`isHidden`) instead of taking it for a
 * deletion. A copy already there wins over a stray write since.
 */
function stow(s: Storage, from: string, to: string): void {
  const value = s.getItem(from);
  if (value === null) return;
  if (s.getItem(to) === null) {
    try {
      s.setItem(to, value);
    } catch {
      // Full: its own room is enough, unless the longer name is what does not fit.
      s.removeItem(from);
      try {
        s.setItem(to, value);
      } catch (err) {
        s.setItem(from, value);
        throw err;
      }
      return;
    }
  }
  s.removeItem(from);
}

function reveal(s: Storage, prefix: string): void {
  for (const scoped of keysOf(s)) {
    if (!scoped.startsWith(prefix)) continue;
    const value = s.getItem(scoped);
    if (value === null) continue;
    const live = scoped.slice(prefix.length);
    s.removeItem(live);
    try {
      s.setItem(live, value);
    } catch {
      s.removeItem(scoped);
      s.setItem(live, value);
      continue;
    }
    s.removeItem(scoped);
  }
}

/** Take the live keys out of sight, for whoever the marker says they belong to. */
export function hideLive(s: Storage): void {
  const marker = s.getItem(MARKER);
  const prefix = marker === null ? LEGACY : scopeOf(marker);
  for (const key of keysOf(s)) {
    if (isScoped(key)) stow(s, key, prefix + key);
  }
}

/**
 * Make the live keys this account's. What was there goes to whoever it belonged
 * to, and data from before any sign-in goes aside: only the first account to sign
 * in here is offered it, and nobody gets it without asking.
 */
export function enterAccount(s: Storage, account: string): void {
  const marker = s.getItem(MARKER);
  if (marker !== account) {
    hideLive(s);
    if (marker === null && keysOf(s).some((k) => k.startsWith(LEGACY))) {
      s.setItem(OFFER, account);
    }
    s.setItem(MARKER, account);
  }
  reveal(s, scopeOf(account));
}

/** No sign-in on this server. With nothing from one here, writes nothing at all. */
export function enterAnonymous(s: Storage): void {
  if (s.getItem(MARKER) !== null) hideLive(s);
  reveal(s, LEGACY);
  s.removeItem(MARKER);
  s.removeItem(OFFER);
  s.removeItem(IMPORT);
}

/** Whether these keys are still the account's they were when this was called. */
export function accountGuard(s: Storage): () => boolean {
  const account = s.getItem(MARKER);
  return () => s.getItem(MARKER) === account;
}

/** A round must not run: these keys are out of sight or another account's. */
export function isHidden(s: Storage, account: string): boolean {
  if (s.getItem(MARKER) !== account) return true;
  const prefix = scopeOf(account);
  return keysOf(s).some((k) => k.startsWith(prefix));
}

export const hasLegacyOffer = (s: Storage, account: string): boolean =>
  s.getItem(OFFER) === account &&
  s.getItem(IMPORT) === null &&
  keysOf(s).some((k) => k.startsWith(LEGACY));

/** The yes. Only a boot acts on it, as running stores could save over the result. */
export const requestImport = (s: Storage, account: string): void => {
  s.setItem(IMPORT, account);
};

/**
 * Bring the data from before sign-in into this account, if it asked. It only adds:
 * a value the account already has stays, and collections are united item by item.
 */
export function runRequestedImport(s: Storage, account: string): void {
  if (s.getItem(IMPORT) !== account) return;
  for (const stashed of keysOf(s)) {
    if (!stashed.startsWith(LEGACY)) continue;
    const key = stashed.slice(LEGACY.length);
    const value = s.getItem(stashed);
    const live = s.getItem(key);
    if (live === null && value !== null) {
      s.setItem(key, value);
    } else if (value !== null && collectionField(key) !== undefined) {
      const union = mergeCollection(key, null, live, value);
      if (union?.value != null) s.setItem(key, union.value);
    }
    s.removeItem(stashed);
  }
  s.removeItem(OFFER);
  s.removeItem(IMPORT);
}

export function discardLegacy(s: Storage): void {
  for (const key of keysOf(s)) {
    if (key.startsWith(LEGACY)) s.removeItem(key);
  }
  s.removeItem(OFFER);
}

// ---------------------------------------------------------------------------
// Merge
// ---------------------------------------------------------------------------

export interface Merged {
  value: string | null;
  /** Someone's change gave way, which must not go unsaid. */
  conflict: boolean;
}

/** Where a document keeps its items by id: `""` for a list at the root. */
function collectionField(key: string): string | undefined {
  if (key === "chartgen.alerts.list" || (key.startsWith("chartgen.drawings.") && key !== INDEX)) {
    return "";
  }
  if (key === "chartgen.layouts") return "items";
  if (key === "chartgen.watchlist.v2") return "lists";
  return undefined;
}

interface Parsed {
  root: Record<string, unknown>;
  items: Map<string, string>;
}

/** Items stay as their text, so fields only a newer build knows come through whole. */
function parseCollection(text: string | null, field: string): Parsed | null {
  const parsed: Parsed = { root: {}, items: new Map() };
  if (text === null) return parsed;
  let raw: unknown;
  try {
    raw = JSON.parse(text);
  } catch {
    return null;
  }
  let list = raw;
  if (field !== "") {
    if (typeof raw !== "object" || raw === null || Array.isArray(raw)) return null;
    parsed.root = raw as Record<string, unknown>;
    list = parsed.root[field];
  }
  if (!Array.isArray(list)) return null;
  for (const item of list as unknown[]) {
    const id = (item as { id?: unknown } | null)?.id;
    const text = JSON.stringify(item);
    // Drawings saved before they had ids (ADR-0034) are told apart by what they are.
    const name = typeof id === "string" || typeof id === "number" ? `#${id}` : text;
    if (parsed.items.has(name)) return null;
    parsed.items.set(name, text);
  }
  return parsed;
}

function pick(
  b: string | undefined,
  l: string | undefined,
  r: string | undefined,
): { value: string | undefined; conflict: boolean } {
  if (l === r) return { value: l, conflict: false };
  if (l === b) return { value: r, conflict: false };
  if (r === b) return { value: l, conflict: false };
  // Both changed. An edit beats a deletion; otherwise the later writer, this one.
  return { value: l ?? r, conflict: true };
}

function mergeCollection(
  key: string,
  base: string | null,
  local: string | null,
  remote: string | null,
): Merged | null {
  const field = collectionField(key);
  if (field === undefined) return null;
  // A whole file gone is no list of items gone, so it is not merged as one.
  if (field !== "" && (local === null || remote === null)) return null;
  const l = parseCollection(local, field);
  const r = parseCollection(remote, field);
  if (l === null || r === null) return null;
  const b = parseCollection(base, field) ?? { root: {}, items: new Map<string, string>() };
  let conflict = false;

  const items: unknown[] = [];
  for (const id of new Set([...l.items.keys(), ...r.items.keys()])) {
    const one = pick(b.items.get(id), l.items.get(id), r.items.get(id));
    conflict ||= one.conflict;
    if (one.value !== undefined) items.push(JSON.parse(one.value));
  }
  if (field === "") {
    return { value: items.length === 0 && local === null ? null : JSON.stringify(items), conflict };
  }

  const root: Record<string, unknown> = {};
  for (const name of new Set([...Object.keys(l.root), ...Object.keys(r.root)])) {
    if (name === field) {
      root[field] = items;
      continue;
    }
    const text = (o: Record<string, unknown>) => (name in o ? JSON.stringify(o[name]) : undefined);
    const one = pick(text(b.root), text(l.root), text(r.root));
    if (one.value === undefined) continue;
    const value: unknown = JSON.parse(one.value);
    // Which entry is active is no data to lose; a map of flags is.
    conflict ||= one.conflict && typeof value === "object";
    root[name] = value;
  }
  return { value: JSON.stringify(root), conflict };
}

/** What a document should become, from what it was, what is here and what the server holds. */
export function merge(
  key: string,
  base: string | null,
  local: string | null,
  remote: string | null,
): Merged {
  if (local === remote || remote === base) return { value: local, conflict: false };
  if (local === base) return { value: remote, conflict: false };
  const collection = mergeCollection(key, base, local, remote);
  if (collection !== null) return collection;
  // A new device's defaults must not replace what the account already has, and an
  // edit beats a deletion. Between two edits the later writer wins, this one.
  if (base === null || local === null) return { value: remote, conflict: true };
  return { value: local, conflict: true };
}

// ---------------------------------------------------------------------------
// Sync state
// ---------------------------------------------------------------------------

/**
 * `b`: what the stores last saw, the base of a merge. `s`: what the server holds,
 * left out when the same. `e`: dropped here by the LRU, not by the user.
 */
export interface DocState {
  v: number;
  b: string | null;
  s?: string | null;
  e?: 1;
}

export function readState(s: Storage, key: string): DocState {
  try {
    const raw = JSON.parse(s.getItem(SYNC + key) ?? "null") as DocState | null;
    if (raw !== null && typeof raw.v === "number") return raw;
  } catch {
    // Unreadable: the server's answer to a write from version 0 puts it right.
  }
  return { v: 0, b: null };
}

const heldOf = (st: DocState): string | null => (st.s !== undefined ? st.s : st.b);

function writeState(s: Storage, key: string, v: number, b: string | null, held: string | null): void {
  const state: DocState = held === b ? { v, b } : { v, b, s: held };
  s.setItem(SYNC + key, JSON.stringify(state));
}

function docKeys(s: Storage): string[] {
  const keys = new Set<string>();
  for (const key of keysOf(s)) {
    if (key === CURSOR) continue;
    if (key.startsWith(SYNC)) keys.add(key.slice(SYNC.length));
    else if (isSynced(key)) keys.add(key);
  }
  return [...keys];
}

export const readCursor = (s: Storage): number => Number(s.getItem(CURSOR) ?? 0) || 0;

export interface SentWrite {
  key: string;
  value: string | null;
  base_version: number;
  /** What the stores had when this was sent, the next base. */
  live: string | null;
  conflict: boolean;
}

export interface Round {
  writes: SentWrite[];
  /** Local changes that gave way to what the server has, with nothing to send. */
  gaveWay: string[];
  tooLarge: string[];
}

const sizeOf = (text: string): number => new TextEncoder().encode(text).length;

/**
 * What this round sends. `refused` says a value was already turned down, so it is
 * not sent again until it changes. Always merges against the base, so a store that
 * saves from stale memory cannot take away what it never saw.
 */
export function collectWrites(
  s: Storage,
  refused: (key: string, live: string | null) => boolean,
  maxBody = MAX_BODY_BYTES - BODY_OVERHEAD_BYTES,
): Round {
  const round: Round = { writes: [], gaveWay: [], tooLarge: [] };
  let body = 0;
  for (const key of docKeys(s)) {
    if (!isSynced(key)) continue;
    const live = s.getItem(key);
    const st = readState(s, key);
    if (live === st.b || refused(key, live)) continue;
    const held = heldOf(st);
    const merged = merge(key, st.b, live, held);
    if (merged.value === held) {
      writeState(s, key, st.v, live, held);
      if (merged.conflict) round.gaveWay.push(key);
      continue;
    }
    // A document is mostly quotes, which take twice the room inside a request's JSON.
    const size = merged.value === null ? 0 : sizeOf(JSON.stringify(merged.value));
    if (size > maxBody || (merged.value !== null && sizeOf(merged.value) > MAX_DOC_BYTES)) {
      round.tooLarge.push(key);
      continue;
    }
    if (round.writes.length >= MAX_WRITES || body + size > maxBody) continue;
    body += size;
    round.writes.push({
      key,
      value: merged.value,
      base_version: st.v,
      live,
      conflict: merged.conflict,
    });
  }
  return round;
}

export type WriteResult =
  | { key: string; status: "ok"; version: number }
  | { key: string; status: "conflict"; version: number; value: string | null }
  | { key: string; status: "too_large" | "quota" | "bad_key" };

export interface SyncAnswer {
  cursor: number;
  /** This device's cursor was past the server's versions: a restored file, or removed rows. */
  restored: boolean;
  results: WriteResult[];
  changes: { key: string; value: string | null; version: number }[];
}

/** A deletion arrived for this key, so its being gone here is no more the LRU's doing. */
function forgetEvictionOf(s: Storage, key: string): void {
  if (!key.startsWith("chartgen.drawings.")) return;
  const symbol = key.slice("chartgen.drawings.".length);
  const evicted = JSON.parse(s.getItem(EVICTED_KEY) ?? "[]") as unknown;
  if (!Array.isArray(evicted) || !evicted.includes(symbol)) return;
  s.setItem(EVICTED_KEY, JSON.stringify(evicted.filter((e) => e !== symbol)));
}

export interface Applied {
  /** A local change replaced someone else's, now it is stored. */
  replaced: string[];
  refused: { key: string; live: string | null; status: "too_large" | "quota" | "bad_key" }[];
}

/**
 * Take the server's answer into the sync state only. Live keys stay as they are
 * under the running stores, which would save over them from memory: `settle` applies
 * what arrived at the next boot, and until then `hasRemoteChanges` says so.
 */
export function applyAnswer(
  s: Storage,
  sent: readonly SentWrite[],
  answer: SyncAnswer,
): Applied {
  const applied: Applied = { replaced: [], refused: [] };
  for (const result of answer.results) {
    const write = sent.find((w) => w.key === result.key);
    if (write === undefined) continue;
    if (result.status === "ok") {
      writeState(s, result.key, result.version, write.live, write.value);
      if (write.conflict) applied.replaced.push(result.key);
    } else if (result.status === "conflict") {
      if (result.version === 0 && result.value === null) {
        // Never there as far as the server knows, so nobody deleted it: this copy is new to it.
        s.removeItem(SYNC + result.key);
      } else {
        // Next round merges with this. Base stays: this device has still not seen it.
        writeState(s, result.key, result.version, readState(s, result.key).b, result.value);
      }
    } else {
      applied.refused.push({ key: result.key, live: write.live, status: result.status });
    }
  }
  // In a restored file its versions are the truth.
  const { restored } = answer;
  if (restored) {
    // Then this answer holds all it has. What it lacks goes to it as new at the next round.
    const known = new Set([...answer.changes, ...answer.results].map((d) => d.key));
    for (const key of docKeys(s)) {
      if (isSynced(key) && !known.has(key) && readState(s, key).e === undefined) {
        s.removeItem(SYNC + key);
      }
    }
  }
  for (const change of answer.changes) {
    if (!isSynced(change.key)) continue;
    const st = readState(s, change.key);
    if (!restored && change.version <= st.v) continue;
    writeState(s, change.key, change.version, st.b, change.value);
    if (change.value === null) forgetEvictionOf(s, change.key);
  }
  s.setItem(CURSOR, String(answer.cursor));
  return applied;
}

/** Remote changes wait for a reload. A doc the LRU dropped comes back then too, unannounced. */
export function hasRemoteChanges(s: Storage): boolean {
  return docKeys(s).some((key) => {
    if (!isSynced(key)) return false;
    const st = readState(s, key);
    return st.e === undefined && st.s !== undefined && st.s !== st.b;
  });
}

/** Only with no store running: live keys become what the server holds, unless changed since. */
export function settle(s: Storage): void {
  for (const key of docKeys(s)) {
    if (!isSynced(key)) continue;
    const st = readState(s, key);
    const held = heldOf(st);
    if (held === st.b || s.getItem(key) !== st.b) continue;
    if (held === null) s.removeItem(key);
    else s.setItem(key, held);
    writeState(s, key, st.v, held, held);
  }
}

/** The LRU dropped this copy, which says nothing about the document: no deletion to send. */
export function forgetEvicted(s: Storage, key: string): void {
  if (s.getItem(SYNC + key) === null) return;
  const st = readState(s, key);
  const state: DocState = { v: st.v, b: null, s: heldOf(st), e: 1 };
  s.setItem(SYNC + key, JSON.stringify(state));
}

/** In words, for a notice. */
export function docLabel(key: string): string {
  if (key.startsWith("chartgen.drawings.")) {
    return `drawings on ${key.slice("chartgen.drawings.".length)}`;
  }
  if (key === "chartgen.layouts") return "layouts";
  if (key === "chartgen.alerts.list") return "browser alerts";
  if (key.startsWith("chartgen.watchlist.")) return "watchlists";
  if (key === "chartgen.chartSettings") return "chart settings";
  return "chart setup";
}
