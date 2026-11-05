// OAuth 2.1 PKCE flow for chartgen. Hand-rolled (no SDK OAuth helper) so
// that the single-screen app can own the redirect lifecycle.
//
// Endpoints follow chartgen's own server (see
// website/src/content/docs/reference/oauth.md). The `access_token` is
// held in memory only; `client_id`/`client_secret` persist in
// localStorage because the authorization server treats dynamic
// registrations as stable identifiers. PKCE state survives the redirect
// via sessionStorage.

import { z } from "zod";

const LS_CLIENT_ID = "chartgen.oauth.client_id";
const LS_CLIENT_SECRET = "chartgen.oauth.client_secret";
const SS_PKCE = "chartgen.oauth.pkce";
/**
 * Set when a failed redirect consumption started one automatic retry of
 * the flow; cleared by the next successful exchange. A second failure in
 * the same tab stops there and asks the user instead of looping.
 */
const SS_RETRIED = "chartgen.oauth.retried";

const BASE_URL =
  (import.meta.env.VITE_CHARTGEN_BASE_URL as string | undefined) ?? "";

/** Absolute URL relative to the chartgen origin (or same-origin if unset). */
function url(path: string): string {
  if (BASE_URL) {
    return `${BASE_URL.replace(/\/$/, "")}${path}`;
  }
  return path;
}

// Zod schemas for OAuth HTTP responses (ADR-0004: validate at the boundary).
const DiscoveryDocumentSchema = z.object({
  authorization_endpoint: z.string().url(),
  token_endpoint: z.string().url(),
  registration_endpoint: z.string().url(),
});
type DiscoveryDocument = z.infer<typeof DiscoveryDocumentSchema>;

const RegistrationResponseSchema = z.object({
  client_id: z.string(),
  client_secret: z.string().optional(),
});

const TokenResponseSchema = z.object({
  access_token: z.string(),
  token_type: z.string().optional(),
  expires_in: z.number().optional(),
});

interface PkceState {
  code_verifier: string;
  state: string;
}

/** Cached in-memory access token. Cleared on 401 or full re-auth. */
let cachedAccessToken: string | null = null;

/** Base64url-encode a Uint8Array without padding. */
function base64urlEncode(bytes: Uint8Array): string {
  let s = "";
  for (const b of bytes) s += String.fromCharCode(b);
  return btoa(s).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

function randomBytes(n: number): Uint8Array {
  const out = new Uint8Array(n);
  crypto.getRandomValues(out);
  return out;
}

/** 43-char base64url-unpadded cryptographic random. */
function generateCodeVerifier(): string {
  // 32 bytes → 43 base64url chars without padding.
  return base64urlEncode(randomBytes(32));
}

async function deriveCodeChallenge(verifier: string): Promise<string> {
  const enc = new TextEncoder().encode(verifier);
  const digest = await crypto.subtle.digest("SHA-256", enc);
  return base64urlEncode(new Uint8Array(digest));
}

function generateState(): string {
  return base64urlEncode(randomBytes(16));
}

async function discover(): Promise<DiscoveryDocument> {
  const res = await fetch(url("/.well-known/oauth-authorization-server"));
  if (!res.ok) {
    throw new Error(`discovery failed: ${res.status}`);
  }
  return DiscoveryDocumentSchema.parse(await res.json());
}

const MAX_REGISTRATION_RETRIES = 4;

/** Wait before retry `attempt` (0-based): Retry-After seconds (max 60), else 2s, 4s, 8s... */
export function retryDelayMs(attempt: number, retryAfter: string | null): number {
  // Retry-After is delta-seconds or an HTTP-date.
  let secs = retryAfter === null || retryAfter.trim() === "" ? NaN : Number(retryAfter);
  if (retryAfter !== null && !Number.isFinite(secs)) {
    secs = (Date.parse(retryAfter) - Date.now()) / 1000;
  }
  if (Number.isFinite(secs)) return Math.min(Math.max(secs, 0), 60) * 1000;
  return Math.min(2 ** (attempt + 1), 60) * 1000;
}

// A registration the server rate-limits is retried; the countdown is
// published so the UI can say why nothing happens.
let notice: string | null = null;
const noticeListeners = new Set<(n: string | null) => void>();

function setNotice(n: string | null): void {
  notice = n;
  for (const fn of noticeListeners) fn(n);
}

/** Subscribe to the sign-in wait notice (null when none); returns unsubscribe. */
export function subscribeSignInNotice(
  fn: (n: string | null) => void,
): () => void {
  noticeListeners.add(fn);
  fn(notice);
  return () => noticeListeners.delete(fn);
}

async function waitWithNotice(ms: number): Promise<void> {
  const end = Date.now() + ms;
  for (let left = ms; left > 0; left = end - Date.now()) {
    setNotice(
      `Too many sign-ins from this network \u2014 retrying in ${Math.ceil(left / 1000)} s`,
    );
    await new Promise((r) => setTimeout(r, Math.min(left, 1000)));
  }
}

async function registerClient(registration_endpoint: string): Promise<{
  client_id: string;
  client_secret: string;
}> {
  const redirectUri = window.location.origin + "/";
  let res: Response;
  try {
    for (let attempt = 0; ; attempt++) {
      res = await fetch(registration_endpoint, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          client_name: "chartgen-web",
          redirect_uris: [redirectUri],
          grant_types: ["authorization_code"],
          response_types: ["code"],
          token_endpoint_auth_method: "none",
        }),
      });
      if (res.status !== 429) break;
      if (attempt >= MAX_REGISTRATION_RETRIES) {
        setNotice(null);
        throw new Error(
          "Too many sign-ins from this network. Try again in a minute.",
        );
      }
      await waitWithNotice(retryDelayMs(attempt, res.headers.get("Retry-After")));
    }
  } finally {
    setNotice(null);
  }
  if (!res.ok) {
    throw new Error(`registration failed: ${res.status}`);
  }
  const body = RegistrationResponseSchema.parse(await res.json());
  const client_secret = body.client_secret ?? "";
  return { client_id: body.client_id, client_secret };
}

async function ensureClient(
  discovery: DiscoveryDocument,
): Promise<{ client_id: string; client_secret: string }> {
  const existingId = localStorage.getItem(LS_CLIENT_ID);
  const existingSecret = localStorage.getItem(LS_CLIENT_SECRET);
  if (existingId !== null && existingSecret !== null) {
    return { client_id: existingId, client_secret: existingSecret };
  }
  const registered = await registerClient(discovery.registration_endpoint);
  localStorage.setItem(LS_CLIENT_ID, registered.client_id);
  localStorage.setItem(LS_CLIENT_SECRET, registered.client_secret);
  return registered;
}

function savePkce(pkce: PkceState): void {
  sessionStorage.setItem(SS_PKCE, JSON.stringify(pkce));
}

function consumePkce(): PkceState | null {
  const raw = sessionStorage.getItem(SS_PKCE);
  if (raw === null) return null;
  sessionStorage.removeItem(SS_PKCE);
  try {
    return JSON.parse(raw) as PkceState;
  } catch {
    return null;
  }
}

async function redirectToAuthorize(
  discovery: DiscoveryDocument,
  clientId: string,
): Promise<never> {
  const codeVerifier = generateCodeVerifier();
  const codeChallenge = await deriveCodeChallenge(codeVerifier);
  const state = generateState();
  savePkce({ code_verifier: codeVerifier, state });

  const params = new URLSearchParams({
    response_type: "code",
    client_id: clientId,
    redirect_uri: window.location.origin + "/",
    code_challenge: codeChallenge,
    code_challenge_method: "S256",
    state,
  });
  window.location.assign(`${discovery.authorization_endpoint}?${params}`);
  // `assign` navigates away; resolve with a never-settling promise to
  // keep the type system honest.
  return new Promise<never>(() => {});
}

async function exchangeCode(
  discovery: DiscoveryDocument,
  clientId: string,
  code: string,
  codeVerifier: string,
): Promise<string> {
  const body = new URLSearchParams({
    grant_type: "authorization_code",
    code,
    redirect_uri: window.location.origin + "/",
    client_id: clientId,
    code_verifier: codeVerifier,
  });
  const res = await fetch(discovery.token_endpoint, {
    method: "POST",
    headers: { "Content-Type": "application/x-www-form-urlencoded" },
    body,
  });
  if (!res.ok) {
    throw new Error(`token exchange failed: ${res.status}`);
  }
  const json = TokenResponseSchema.parse(await res.json());
  return json.access_token;
}

/**
 * Consume a pending redirect result from the URL, if present.
 * Returns `true` if this call handled a redirect (access token set),
 * `false` if no redirect was in progress. Throws if the state check
 * or the exchange fails — the caller should surface the error.
 */
async function tryConsumeRedirect(): Promise<boolean> {
  const params = new URLSearchParams(window.location.search);
  const code = params.get("code");
  const returnedState = params.get("state");
  if (code === null) return false;

  // Scrub `code`/`state` from the URL before anything can fail: a code is
  // single-use, so a refresh must start a new flow instead of retrying
  // this one. Preserve the hash so any in-page anchor/route survives.
  window.history.replaceState(
    {},
    "",
    window.location.origin + window.location.pathname + window.location.hash,
  );

  const pending = consumePkce();
  if (pending === null) {
    // No stored PKCE — can't validate. Fail closed.
    throw new Error("oauth: no pending PKCE state");
  }
  // CSRF check: treat missing state as failure (fix from PR #66 review).
  if (!returnedState || pending.state !== returnedState) {
    throw new Error("oauth: state mismatch");
  }

  const discovery = await discover();
  const { client_id } = await ensureClient(discovery);
  const token = await exchangeCode(
    discovery,
    client_id,
    code,
    pending.code_verifier,
  );
  cachedAccessToken = token;
  sessionStorage.removeItem(SS_RETRIED);
  return true;
}

/**
 * Start a new PKCE flow: navigates to the authorization endpoint and
 * never resolves. Also the "Sign in again" action after a failed sign-in.
 */
export async function startSignIn(): Promise<never> {
  const discovery = await discover();
  const { client_id } = await ensureClient(discovery);
  return redirectToAuthorize(discovery, client_id);
}

/**
 * Return an access token, running the full PKCE flow if there isn't one
 * cached. If a redirect is required this function navigates the page and
 * never resolves.
 */
export async function getAccessToken(): Promise<string> {
  if (cachedAccessToken !== null) return cachedAccessToken;

  // Step 1: handle redirect response if present.
  if (await tryConsumeRedirect()) {
    // cachedAccessToken now set by tryConsumeRedirect.
    return cachedAccessToken as unknown as string;
  }

  // Step 2: no token and no in-flight redirect — start a new flow.
  return startSignIn();
}

/** Drop the cached token, e.g. on a 401 from the MCP call. */
export function clearAccessToken(): void {
  cachedAccessToken = null;
}

/**
 * If the URL on load already carries `?code=...`, run the exchange
 * eagerly so the rest of the app sees a token immediately.
 *
 * Resolves `null` when there was nothing to consume or the exchange
 * worked. When it fails, the first time in this tab it restarts the flow
 * once by itself (navigates away, never resolves); after that it resolves
 * with the error, and the caller must not request a token again on its
 * own — that would redirect in a loop — but offer `startSignIn()`.
 */
export async function tryHandlePendingRedirect(): Promise<Error | null> {
  try {
    await tryConsumeRedirect();
    return null;
  } catch (err) {
    console.error("[oauth] redirect handler failed:", err);
    if (sessionStorage.getItem(SS_RETRIED) === null) {
      sessionStorage.setItem(SS_RETRIED, "1");
      try {
        return await startSignIn();
      } catch (retryErr) {
        return toError(retryErr);
      }
    }
    return toError(err);
  }
}

const toError = (err: unknown): Error =>
  err instanceof Error ? err : new Error(String(err));
