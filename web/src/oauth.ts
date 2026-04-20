// OAuth 2.1 PKCE flow for chartgen. Hand-rolled (no SDK OAuth helper) so
// that the single-screen app can own the redirect lifecycle.
//
// Endpoints follow chartgen's own server (see
// website/src/content/docs/reference/oauth.md). The `access_token` is
// held in memory only; `client_id`/`client_secret` persist in
// localStorage because the authorization server treats dynamic
// registrations as stable identifiers. PKCE state survives the redirect
// via sessionStorage.

const LS_CLIENT_ID = "chartgen.oauth.client_id";
const LS_CLIENT_SECRET = "chartgen.oauth.client_secret";
const SS_PKCE = "chartgen.oauth.pkce";

const BASE_URL =
  (import.meta.env.VITE_CHARTGEN_BASE_URL as string | undefined) ?? "";

/** Absolute URL relative to the chartgen origin (or same-origin if unset). */
function url(path: string): string {
  if (BASE_URL) {
    return `${BASE_URL.replace(/\/$/, "")}${path}`;
  }
  return path;
}

interface DiscoveryDocument {
  authorization_endpoint: string;
  token_endpoint: string;
  registration_endpoint: string;
}

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
  const doc = (await res.json()) as DiscoveryDocument;
  return doc;
}

async function registerClient(registration_endpoint: string): Promise<{
  client_id: string;
  client_secret: string;
}> {
  const redirectUri = window.location.origin + "/";
  const res = await fetch(registration_endpoint, {
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
  if (!res.ok) {
    throw new Error(`registration failed: ${res.status}`);
  }
  const body = (await res.json()) as {
    client_id: string;
    client_secret?: string;
  };
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
  const json = (await res.json()) as { access_token: string };
  return json.access_token;
}

/**
 * Consume a pending redirect result from the URL, if present.
 * Returns `true` if this call handled a redirect (access token set),
 * `false` if no redirect was in progress. Throws if the state check
 * fails — the caller should surface the error.
 */
async function tryConsumeRedirect(): Promise<boolean> {
  const params = new URLSearchParams(window.location.search);
  const code = params.get("code");
  const returnedState = params.get("state");
  if (code === null) return false;

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

  // Scrub `code`/`state` from the URL so a refresh doesn't retrigger.
  const cleanUrl = window.location.origin + window.location.pathname;
  window.history.replaceState({}, "", cleanUrl);
  return true;
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
  const discovery = await discover();
  const { client_id } = await ensureClient(discovery);
  await redirectToAuthorize(discovery, client_id);
  // unreachable
  throw new Error("oauth: redirect did not happen");
}

/** Drop the cached token, e.g. on a 401 from the MCP call. */
export function clearAccessToken(): void {
  cachedAccessToken = null;
}

/**
 * If the URL on load already carries `?code=...`, run the exchange
 * eagerly so the rest of the app sees a token immediately.
 * Swallows errors and leaves `cachedAccessToken` null — the first
 * `getAccessToken()` will surface the failure.
 */
export async function tryHandlePendingRedirect(): Promise<void> {
  try {
    await tryConsumeRedirect();
  } catch (err) {
    // Surface via console but do not throw during boot.
    console.error("[oauth] redirect handler failed:", err);
  }
}
