/**
 * OAuth 2.1 PKCE client for chartgen's built-in authorization server.
 *
 * Authoritative spec: `website/src/content/docs/reference/oauth.md`.
 *
 * - Dynamic client registration (RFC 7591): `client_id` + `client_secret` live
 *   in localStorage. Per RFC 7591 §5 these are not sensitive credentials —
 *   they identify this browser install. Re-registration is transparent if
 *   the server restarts (its store is in-memory).
 * - PKCE S256 only. Spec: no refresh-token flow; on 401 we simply re-run
 *   the full flow.
 * - Access token is kept in memory only (no localStorage/sessionStorage).
 * - Auth code is delivered by `/authorize` via a 307 redirect back to
 *   `redirect_uri`. We survive the round-trip via `window.location` and
 *   `sessionStorage` for the code_verifier + state only (wiped on success).
 */

export interface OAuthConfig {
  baseUrl: string;
}

interface ServerMetadata {
  issuer: string;
  authorization_endpoint: string;
  token_endpoint: string;
  registration_endpoint: string;
}

interface ClientRegistration {
  client_id: string;
  client_secret?: string;
}

const LS_CLIENT_KEY = 'chartgen.oauth.client';
const SS_PKCE_KEY = 'chartgen.oauth.pkce';
const SS_RETURN_URL_KEY = 'chartgen.oauth.return_url';

// --- PKCE primitives ---------------------------------------------------------

function randomUrlSafe(byteLen = 32): string {
  const bytes = new Uint8Array(byteLen);
  crypto.getRandomValues(bytes);
  return base64UrlEncode(bytes);
}

function base64UrlEncode(bytes: Uint8Array): string {
  let bin = '';
  for (const b of bytes) bin += String.fromCharCode(b);
  return btoa(bin).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

async function sha256(input: string): Promise<Uint8Array> {
  const data = new TextEncoder().encode(input);
  const digest = await crypto.subtle.digest('SHA-256', data);
  return new Uint8Array(digest);
}

async function codeChallengeS256(verifier: string): Promise<string> {
  return base64UrlEncode(await sha256(verifier));
}

// --- Metadata + registration -------------------------------------------------

async function fetchMetadata(baseUrl: string): Promise<ServerMetadata> {
  const res = await fetch(
    `${baseUrl.replace(/\/$/, '')}/.well-known/oauth-authorization-server`,
    { credentials: 'omit' },
  );
  if (!res.ok) {
    throw new Error(`OAuth metadata discovery failed: ${res.status}`);
  }
  return (await res.json()) as ServerMetadata;
}

function redirectUri(): string {
  // Strip any query/hash so it matches a stable, registered URI.
  const { origin, pathname } = window.location;
  return `${origin}${pathname}`;
}

async function registerClient(
  meta: ServerMetadata,
): Promise<ClientRegistration> {
  const cached = localStorage.getItem(LS_CLIENT_KEY);
  if (cached) {
    try {
      return JSON.parse(cached) as ClientRegistration;
    } catch {
      localStorage.removeItem(LS_CLIENT_KEY);
    }
  }
  const res = await fetch(meta.registration_endpoint, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    credentials: 'omit',
    body: JSON.stringify({
      client_name: 'chartgen web',
      redirect_uris: [redirectUri()],
      grant_types: ['authorization_code'],
      response_types: ['code'],
      token_endpoint_auth_method: 'none',
    }),
  });
  if (!res.ok) {
    throw new Error(`Client registration failed: ${res.status}`);
  }
  const body = (await res.json()) as ClientRegistration;
  const compact: ClientRegistration = {
    client_id: body.client_id,
    client_secret: body.client_secret,
  };
  localStorage.setItem(LS_CLIENT_KEY, JSON.stringify(compact));
  return compact;
}

// --- Flow --------------------------------------------------------------------

interface PendingPkce {
  code_verifier: string;
  state: string;
  client_id: string;
  token_endpoint: string;
  redirect_uri: string;
}

async function startAuthorization(cfg: OAuthConfig): Promise<never> {
  const meta = await fetchMetadata(cfg.baseUrl);
  const client = await registerClient(meta);

  const verifier = randomUrlSafe(32);
  const challenge = await codeChallengeS256(verifier);
  const state = randomUrlSafe(16);
  const redirect_uri = redirectUri();

  const pending: PendingPkce = {
    code_verifier: verifier,
    state,
    client_id: client.client_id,
    token_endpoint: meta.token_endpoint,
    redirect_uri,
  };
  sessionStorage.setItem(SS_PKCE_KEY, JSON.stringify(pending));
  sessionStorage.setItem(SS_RETURN_URL_KEY, window.location.href);

  const url = new URL(meta.authorization_endpoint);
  url.searchParams.set('response_type', 'code');
  url.searchParams.set('client_id', client.client_id);
  url.searchParams.set('redirect_uri', redirect_uri);
  url.searchParams.set('code_challenge', challenge);
  url.searchParams.set('code_challenge_method', 'S256');
  url.searchParams.set('state', state);

  window.location.assign(url.toString());
  // window.location.assign never returns synchronously; satisfy TS.
  return new Promise<never>(() => {});
}

async function exchangeCode(
  code: string,
  pending: PendingPkce,
): Promise<string> {
  const form = new URLSearchParams();
  form.set('grant_type', 'authorization_code');
  form.set('code', code);
  form.set('redirect_uri', pending.redirect_uri);
  form.set('client_id', pending.client_id);
  form.set('code_verifier', pending.code_verifier);

  const res = await fetch(pending.token_endpoint, {
    method: 'POST',
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    credentials: 'omit',
    body: form.toString(),
  });
  if (!res.ok) {
    const text = await res.text().catch(() => '');
    throw new Error(`Token exchange failed: ${res.status} ${text}`);
  }
  const body = (await res.json()) as { access_token?: string };
  if (!body.access_token) {
    throw new Error('Token endpoint returned no access_token');
  }
  return body.access_token;
}

function consumePendingPkce(): PendingPkce | null {
  const raw = sessionStorage.getItem(SS_PKCE_KEY);
  if (!raw) return null;
  try {
    return JSON.parse(raw) as PendingPkce;
  } catch {
    return null;
  } finally {
    sessionStorage.removeItem(SS_PKCE_KEY);
  }
}

function stripOAuthQuery(): void {
  const url = new URL(window.location.href);
  url.searchParams.delete('code');
  url.searchParams.delete('state');
  window.history.replaceState({}, '', url.toString());
}

/**
 * Entry point. If we're mid-flow (URL has ?code=), finish the exchange and
 * return the access token. Otherwise kick off the authorize redirect.
 *
 * `forceReauth=true` wipes any client-side state before starting — use on 401.
 */
export async function getAccessToken(
  cfg: OAuthConfig,
  opts: { forceReauth?: boolean } = {},
): Promise<string> {
  if (opts.forceReauth) {
    localStorage.removeItem(LS_CLIENT_KEY);
    sessionStorage.removeItem(SS_PKCE_KEY);
    sessionStorage.removeItem(SS_RETURN_URL_KEY);
  }

  const params = new URLSearchParams(window.location.search);
  const code = params.get('code');
  const returnedState = params.get('state');

  if (code) {
    const pending = consumePendingPkce();
    if (!pending) {
      throw new Error('Received OAuth code but no PKCE state is pending');
    }
    if (returnedState && pending.state !== returnedState) {
      throw new Error('OAuth state mismatch — possible CSRF');
    }
    const token = await exchangeCode(code, pending);
    stripOAuthQuery();
    // Return to where the user originally was (same pathname in v0).
    sessionStorage.removeItem(SS_RETURN_URL_KEY);
    return token;
  }

  return startAuthorization(cfg);
}
