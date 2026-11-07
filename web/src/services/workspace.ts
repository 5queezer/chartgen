// A signed-in user's workspace (ADR-0050). Runs before the stores are imported, so
// they read the right account's data and no other account's ever shows; with no
// sign-in on the server it writes nothing. While the app runs, the stores keep
// using localStorage and this sends what changed and collects what arrives.

import { createSignal } from "solid-js";
import { z } from "zod";

import {
  applyAnswer,
  collectWrites,
  discardLegacy,
  docLabel,
  enterAccount,
  enterAnonymous,
  hasLegacyOffer,
  hasRemoteChanges,
  hideLive,
  importLegacy,
  isHidden,
  MARKER,
  readCursor,
  settle,
  type SentWrite,
} from "../lib/workspaceSync";
import {
  currentAccessToken,
  fetchSignIn,
  signOut,
  startSignIn,
  tryHandlePendingRedirect,
  url,
  type SignedInUser,
} from "../oauth";

/** The address a sign-in interrupted, share params and all (sessionStorage). */
const RETURN_URL = "chartgen.share.return";
/** Written to reload the other tabs, which hold the same data in memory. */
const RELOAD = "chartgen.workspace.reload";
/** Held by every running app, in shared mode: while another tab's stores are up, boot applies nothing. */
const TAB_LOCK = "chartgen.workspace.tab";

const ME_TIMEOUT_MS = 5_000;
const FIRST_SYNC_TIMEOUT_MS = 2_500;
/** A browser new to the account has nothing to show meanwhile, and stores that start empty write defaults. */
const FIRST_EVER_SYNC_TIMEOUT_MS = 10_000;
const SCAN_MS = 5_000;
const REFRESH_MS = 30_000;
const POST_TIMEOUT_MS = 15_000;
const SIGN_OUT_FLUSH_MS = 2_000;
const BACKOFF_FIRST_MS = 30_000;
const BACKOFF_MAX_MS = 5 * 60_000;
const KEEPALIVE_MAX_BYTES = 60_000;

const SyncAnswerSchema = z.object({
  cursor: z.number(),
  results: z.array(
    z.discriminatedUnion("status", [
      z.object({ key: z.string(), status: z.literal("ok"), version: z.number() }),
      z.object({
        key: z.string(),
        status: z.literal("conflict"),
        version: z.number(),
        value: z.string().nullable(),
      }),
      z.object({
        key: z.string(),
        status: z.enum(["too_large", "quota", "bad_key"]),
      }),
    ]),
  ),
  changes: z.array(
    z.object({ key: z.string(), value: z.string().nullable(), version: z.number() }),
  ),
});

const [notices, setNotices] = createSignal<readonly string[]>([]);
const [remoteChanged, setRemoteChanged] = createSignal(false);
const [legacyOffer, setLegacyOffer] = createSignal(false);
export { legacyOffer, notices, remoteChanged };

const addNotice = (text: string): void => {
  setNotices((all) => (all.includes(text) ? all : [...all, text]));
};
export const dismissNotice = (text: string): void => {
  setNotices((all) => all.filter((n) => n !== text));
};

let account: string | null = null;
let timer: number | undefined;
let inFlight: Promise<void> | null = null;
let lastPost = 0;
let notBefore = 0;
let backoff = 0;
let signingOut = false;
/** What the server turned down, so it is not sent again until it changes. */
const refused = new Map<string, string | null>();
const isRefused = (key: string, live: string | null): boolean =>
  refused.has(key) && refused.get(key) === live;

function stop(): void {
  account = null;
  window.clearInterval(timer);
}

function bodyOf(writes: readonly SentWrite[]): string {
  return JSON.stringify({
    since: readCursor(localStorage),
    writes: writes.map(({ key, value, base_version }) => ({ key, value, base_version })),
  });
}

function report(gaveWay: readonly string[], replaced: readonly string[]): void {
  for (const key of gaveWay) {
    addNotice(
      `Your ${docLabel(key)} also changed on another device. That version stays, and this device's change gave way.`,
    );
  }
  for (const key of replaced) {
    addNotice(
      `Your ${docLabel(key)} also changed on another device. This device's version stays, and the other device's change to the same thing was replaced.`,
    );
  }
}

const REFUSED_BECAUSE = {
  too_large: "it is larger than one document may be",
  quota: "your account's storage limit is reached",
  bad_key: "the server does not accept its name",
};

async function runRound(force: boolean): Promise<void> {
  const s = localStorage;
  const me = account;
  const token = currentAccessToken();
  if (me === null || token === null || isHidden(s, me)) return;
  const round = collectWrites(s, isRefused);
  // Seen again after reading, as a key that went missing may be on its way to a scope.
  if (isHidden(s, me)) return;
  report(round.gaveWay, []);
  for (const key of round.tooLarge) {
    addNotice(`Your ${docLabel(key)} did not sync: ${REFUSED_BECAUSE.too_large}. It stays in this browser.`);
  }
  if (round.writes.length > 0 || force) {
    const since = readCursor(s);
    lastPost = Date.now();
    let res: Response;
    try {
      res = await fetch(url("/workspace/sync"), {
        method: "POST",
        signal: AbortSignal.timeout(POST_TIMEOUT_MS),
        headers: { "Content-Type": "application/json", Authorization: `Bearer ${token}` },
        body: bodyOf(round.writes),
      });
    } catch {
      backOff();
      return;
    }
    // No server for this, or no session any more: the next MCP call deals with that.
    if (res.status === 404 || res.status === 401) {
      stop();
      return;
    }
    if (!res.ok) {
      backOff();
      return;
    }
    const parsed = SyncAnswerSchema.safeParse(await res.json().catch(() => null));
    if (!parsed.success) {
      backOff();
      return;
    }
    const answer = parsed.data;
    backoff = 0;
    // The account may have changed under a request; its answer is no one else's.
    if (account !== me || isHidden(s, me)) return;
    const applied = applyAnswer(s, since, round.writes, answer);
    report([], applied.replaced);
    for (const { key, live, status } of applied.refused) {
      refused.set(key, live);
      addNotice(`Your ${docLabel(key)} did not sync: ${REFUSED_BECAUSE[status]}. It stays in this browser.`);
    }
  }
  setRemoteChanged(hasRemoteChanges(s));
}

function backOff(): void {
  backoff = backoff === 0 ? BACKOFF_FIRST_MS : Math.min(backoff * 2, BACKOFF_MAX_MS);
  notBefore = Date.now() + backoff;
}

function syncRound(force = false): Promise<void> {
  inFlight ??= runRound(force)
    .catch((err: unknown) => {
      // localStorage full, most likely. Stopping keeps a half-written state from going on.
      console.warn("[workspace] sync stopped:", err);
      addNotice("Sync stopped: this browser could not save its state. Your changes stay in this browser.");
      stop();
    })
    .finally(() => {
      inFlight = null;
    });
  return inFlight;
}

const sleep = (ms: number): Promise<void> => new Promise((done) => window.setTimeout(done, ms));

function tick(): void {
  if (Date.now() < notBefore) return;
  const due = document.visibilityState === "visible" && Date.now() - lastPost >= REFRESH_MS;
  void syncRound(due);
}

/** Best effort as the page goes: a later load sends again whatever this does not get through. */
function flushOnLeaving(): void {
  const token = currentAccessToken();
  if (signingOut || account === null || token === null || isHidden(localStorage, account)) return;
  const { writes } = collectWrites(localStorage, isRefused);
  const body = bodyOf(writes);
  if (writes.length === 0 || body.length > KEEPALIVE_MAX_BYTES) return;
  if (isHidden(localStorage, account)) return;
  void fetch(url("/workspace/sync"), {
    method: "POST",
    keepalive: true,
    headers: { "Content-Type": "application/json", Authorization: `Bearer ${token}` },
    body,
  }).catch(() => {});
}

function startEngine(): void {
  timer = window.setInterval(tick, SCAN_MS);
  window.addEventListener("pagehide", flushOnLeaving);
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "hidden") void syncRound();
  });
  window.addEventListener("storage", (e) => {
    // Someone else signed in on this browser, so these stores hold the wrong data.
    if (e.key === MARKER && e.newValue !== account && !signingOut) location.reload();
    else if (e.key === RELOAD && e.newValue !== null) location.reload();
  });
  const locks = navigator.locks as LockManager | undefined;
  void locks?.request(TAB_LOCK, { mode: "shared" }, () => new Promise<void>(() => {}));
}

async function otherTabsRunning(): Promise<boolean> {
  const locks = navigator.locks as LockManager | undefined;
  if (locks === undefined) return false;
  const { held } = await locks.query();
  return held?.some((lock) => lock.name === TAB_LOCK) ?? false;
}

function restoreReturnUrl(): void {
  const search = sessionStorage.getItem(RETURN_URL);
  if (search === null) return;
  sessionStorage.removeItem(RETURN_URL);
  history.replaceState({}, "", location.pathname + search + location.hash);
}

export interface Boot {
  authError: string | null;
  user: SignedInUser | null;
}

/**
 * Sort out who this is and put that account's data in place. Never resolves when
 * it leaves for the sign-in. Rejects if the accounts could not be kept apart, and
 * then the app must not start.
 */
export async function bootWorkspace(): Promise<Boot> {
  const failed = await tryHandlePendingRedirect();
  const authError = failed?.message ?? null;
  const s = localStorage;

  const who = await fetchSignIn(AbortSignal.timeout(ME_TIMEOUT_MS)).catch(() => null);
  if (who === null) {
    // A new sign-in we cannot put a name to must not see the last account's data.
    if (authError === null && currentAccessToken() !== null && s.getItem(MARKER) !== null) {
      hideLive(s);
      return { authError: "could not tell which account signed in", user: null };
    }
    // Offline, or the server is away: this browser's data, as it was left.
    return { authError, user: null };
  }

  if (!who.sign_in) {
    try {
      enterAnonymous(s);
    } catch (err) {
      console.warn("[workspace] cannot put back this browser's data:", err);
    }
    return { authError, user: null };
  }

  if (who.user !== null) {
    enterAccount(s, who.user.id);
    restoreReturnUrl();
    account = who.user.id;
    const waiting = readCursor(s) === 0 ? FIRST_EVER_SYNC_TIMEOUT_MS : FIRST_SYNC_TIMEOUT_MS;
    await Promise.race([syncRound(true), sleep(waiting)]);
    if (!(await otherTabsRunning())) {
      try {
        settle(s);
      } catch (err) {
        // Full: this browser stays as it was, and the banner keeps saying there is more.
        console.warn("[workspace] cannot apply what arrived:", err);
      }
    }
    setRemoteChanged(hasRemoteChanges(s));
    setLegacyOffer(hasLegacyOffer(s, who.user.id));
    if (account !== null) startEngine();
    return { authError: null, user: who.user };
  }

  // Signed in nowhere yet. Going to the provider shows nothing, so another tab
  // reloading leaves this data where the running ones need it.
  if (authError === null) {
    try {
      if (location.search !== "") sessionStorage.setItem(RETURN_URL, location.search);
      return await startSignIn();
    } catch (err) {
      hideLive(s);
      return { authError: err instanceof Error ? err.message : String(err), user: null };
    }
  }
  hideLive(s);
  return { authError, user: null };
}

/** Reload every tab of this browser, each having other data in memory. */
export function reloadEverywhere(): void {
  localStorage.setItem(RELOAD, String(Date.now()));
  location.reload();
}

export function acceptLegacy(): void {
  importLegacy(localStorage);
  setLegacyOffer(false);
  reloadEverywhere();
}

export function rejectLegacy(): void {
  discardLegacy(localStorage);
  setLegacyOffer(false);
}

/** Send what is pending, end the session, and leave no data live for the next user. */
export async function signOutOfWorkspace(): Promise<void> {
  signingOut = true;
  try {
    // What the stores hold back until the page goes (drawings, script drafts) goes now.
    window.dispatchEvent(new Event("pagehide"));
    await Promise.race([
      (inFlight ?? Promise.resolve()).then(() => syncRound(true)),
      sleep(SIGN_OUT_FLUSH_MS),
    ]);
    await signOut(() => {
      stop();
      try {
        hideLive(localStorage);
      } catch (err) {
        // The next boot tries again and refuses to start the app if it still cannot.
        console.error("[workspace] cannot put this account's data away:", err);
      }
    });
  } catch (err) {
    signingOut = false;
    throw err;
  }
}
