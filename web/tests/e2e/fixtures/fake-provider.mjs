// GitHub and Google on loopback for signin.spec.ts (ADR-0044): a page to approve
// or deny on, and the token and user endpoints as each answers them. It checks
// the secret, the callback and the PKCE verifier, so a wrong exchange fails here
// as it would for real. chartgen only talks to it through
// CHARTGEN_SIGNIN_TEST_PROVIDER_ORIGIN, honored on the sample source alone.

import { createHash, randomUUID } from "node:crypto";
import { createServer } from "node:http";

const PORT = Number(process.env.FAKE_PROVIDER_PORT);
const SECRET = process.env.FAKE_PROVIDER_SECRET;

const PROFILES = {
  github: { id: 1001, login: "octo", email: "not-used@example.com" },
  google: {
    sub: "google-sub-1",
    name: "Ann Example",
    email: "ann@example.com",
    email_verified: true,
  },
};
const NAMES = { github: "octo", google: "Ann Example" };

const codes = new Map();
const tokens = new Map();

const send = (res, status, type, body) => {
  res.writeHead(status, { "Content-Type": type });
  res.end(body);
};
const json = (res, status, value) =>
  send(res, status, "application/json", JSON.stringify(value));

function approvalPage(res, provider, query) {
  if (query.get("client_id") !== `${provider}-id`) {
    return send(res, 400, "text/plain", "wrong client_id");
  }
  const back = (params) => {
    const url = new URL(query.get("redirect_uri"));
    for (const [k, v] of Object.entries(params)) url.searchParams.set(k, v);
    url.searchParams.set("state", query.get("state"));
    return url.href.replace(/&/g, "&amp;");
  };
  const code = randomUUID();
  codes.set(code, {
    provider,
    challenge: query.get("code_challenge"),
    redirectUri: query.get("redirect_uri"),
  });
  send(
    res,
    200,
    "text/html",
    `<!doctype html><title>fake ${provider}</title><h1>Fake ${provider}</h1>
     <a href="${back({ code })}">Sign in as ${NAMES[provider]}</a>
     <a href="${back({ error: "access_denied" })}">Deny</a>
     <a href="${back({ code: "refused" })}">Answer with a bad code</a>`,
  );
}

function tokenEndpoint(res, provider, form) {
  const code = codes.get(form.get("code"));
  codes.delete(form.get("code"));
  const verifier = form.get("code_verifier") ?? "";
  const computed = createHash("sha256").update(verifier).digest("base64url");
  if (
    !code ||
    code.provider !== provider ||
    code.challenge !== computed ||
    code.redirectUri !== form.get("redirect_uri") ||
    form.get("client_id") !== `${provider}-id` ||
    form.get("client_secret") !== SECRET
  ) {
    // GitHub reports a bad code with a 200 and an `error` field.
    return json(res, 200, { error: "bad_verification_code" });
  }
  const token = randomUUID();
  tokens.set(token, provider);
  json(res, 200, { access_token: token, token_type: "bearer" });
}

createServer((req, res) => {
  const url = new URL(req.url, `http://127.0.0.1:${PORT}`);
  const [, provider, step] = url.pathname.split("/");
  let body = "";
  req.on("data", (chunk) => (body += chunk));
  req.on("end", () => {
    if (provider === "client") {
      // Where the third-party client of the spec lands.
      return send(res, 200, "text/html", "<!doctype html><title>client</title>back at the client");
    }
    if (!(provider in PROFILES)) return send(res, 404, "text/plain", "not found");
    if (req.method === "GET" && step === "authorize") {
      return approvalPage(res, provider, url.searchParams);
    }
    if (req.method === "POST" && step === "token") {
      return tokenEndpoint(res, provider, new URLSearchParams(body));
    }
    if (req.method === "GET" && step === "userinfo") {
      const token = (req.headers.authorization ?? "").replace(/^Bearer /, "");
      return tokens.get(token) === provider
        ? json(res, 200, PROFILES[provider])
        : json(res, 401, {});
    }
    send(res, 404, "text/plain", "not found");
  });
}).listen(PORT, "127.0.0.1");
