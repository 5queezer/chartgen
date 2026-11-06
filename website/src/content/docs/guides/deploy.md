---
title: Deployment
description: Docker, prebuilt release binaries, reverse proxy, Coolify auto-deploy, ARM cross-compilation.
---

chartgen ships as a single binary. Below are the supported paths to get it running remotely.

## Prebuilt release binaries

Tagging a release (`v*`) kicks off `.github/workflows/release.yml`, which
uploads four artifacts to the GitHub release:

| Artifact | Platform |
|----------|----------|
| `chartgen-linux-x86_64.tar.gz` | Linux x86_64 |
| `chartgen-linux-aarch64.tar.gz` | Linux ARM64 |
| `chartgen-macos-x86_64.tar.gz` | macOS Intel |
| `chartgen-macos-aarch64.tar.gz` | macOS Apple Silicon |

Grab the one you want from the [Releases page](https://github.com/5queezer/chartgen/releases),
extract, and run `./chartgen --serve`.

## Docker

The repo's `Dockerfile` is a 3-stage build: `rust:1.95-slim-bookworm`
builds the binary, `node:22-bookworm-slim` builds the [web app](/chartgen/guides/web/),
and `debian:bookworm-slim` runs both. The image serves the app at `/` for
browsers and MCP at `/mcp` on the same origin
([ADR-0027](/chartgen/decisions/0027-serve-web-app-same-origin/)):

```bash
docker build -t chartgen .
docker run -p 9315:9315 \
  -e CHARTGEN_BASE_URL=https://chartgen.example.com \
  -v chartgen-data:/root/.chartgen \
  chartgen --serve --trade --testnet
```

Production must mount this volume. The `~/.chartgen/` mount persists
`alerts.json`, `trades.log`, `subscriptions.json`, and in `chartgen.db` the
registered OAuth clients, across restarts. Without it, every deploy forgets the
registrations of external MCP clients. See the [persistence reference](/chartgen/reference/persistence/).

The image's default command is `--serve`. Add `--trade` only on a server nobody else can
reach: its order tools act on the one configured exchange account, for anyone with a
token. See [what a signed-in stranger can still do](/chartgen/decisions/0044-sign-in-through-github-and-google/#what-a-signed-in-stranger-can-still-do).

### Environment variables

| Name | Read by | Purpose |
|------|---------|---------|
| `CHARTGEN_BASE_URL` | `src/server.rs` | Public HTTPS URL that chartgen embeds in OAuth metadata, redirect URLs, and `logo_uri`. Falls back to `http://localhost:<port>`, which Claude.ai rejects — always set this in production. |
| `CHARTGEN_WEB_DIR` | `src/server.rs` | Directory of a built web app (`web/dist`). When set, `GET /` with `Accept: text/html` returns its `index.html`, other paths fall back to its static files (unknown browser paths get `index.html`), and `/assets/*` is cached as immutable. Non-browser `GET /` still returns the health JSON. Unset means no static files. The Docker image sets it to `/usr/share/chartgen/web`. |
| `CHARTGEN_DATA_SOURCE` | `src/fetch.rs` | Set to `sample` to make `generate_chart`, `get_indicators` and `get_quotes` use a deterministic offline source and never call Binance or Yahoo. Bars follow the interval grid (weekly bars open on Monday), each bar's prices depend only on the symbol and its open time (so overlapping pages match exactly), and the forming candle moves every second. Intended for E2E tests and demos. Unset means live data. |
| `CHARTGEN_SCRIPTING` | `src/scripting/mod.rs` | `1` enables [scripted indicators](/chartgen/guides/scripted-indicators/): the four scripting MCP tools and user scripts in `generate_chart` / `list_indicators`. Off when unset. |
| `CHARTGEN_DB` | `src/scripting/persistence.rs` | SQLite file for registered OAuth clients, with or without `CHARTGEN_SCRIPTING=1`, for scripted indicators, and with sign-in on for users and sessions. Opened at server startup; a failure is logged and leaves clients in memory only. Default `~/.chartgen/chartgen.db`, inside the persisted volume. |
| `CHARTGEN_GITHUB_CLIENT_ID`, `CHARTGEN_GITHUB_CLIENT_SECRET` | `src/server/signin.rs` | [Sign-in through GitHub](#sign-in-through-github-and-google). Unset: not offered. One without the other stops the server at startup. |
| `CHARTGEN_GOOGLE_CLIENT_ID`, `CHARTGEN_GOOGLE_CLIENT_SECRET` | `src/server/signin.rs` | The same for Google. |
| `CHARTGEN_SIGNIN_TEST_PROVIDER_ORIGIN` | `src/server/signin.rs` | Tests only: a loopback `http` origin that replaces both providers. Honored only with `CHARTGEN_DATA_SOURCE=sample`, ignored on live data. Never set it in production. |
| `HOME` / `USERPROFILE` | `src/main.rs` | Picks the data directory (`~/.chartgen/`). Override to relocate `alerts.json` / `trades.log` / `subscriptions.json` / `chartgen.db`. |
| `_CHARTGEN_PORT` | `src/server.rs` | Internal — set by the process to its own bind port so `base_url()` can compose the fallback URL. Do not set by hand. |

## Sign-in through GitHub and Google

Without sign-in, anyone who knows a `client_id` gets a token, and a public server logs a
warning about that at startup. Configure one or both providers to require a login.
**Anyone with an account can then sign in**; there is no allowlist. Read
[what they can still do](/chartgen/decisions/0044-sign-in-through-github-and-google/#what-a-signed-in-stranger-can-still-do) first. In short, keep
`CHARTGEN_SCRIPTING` off and do not use `--trade` on a public server.

Both callbacks are `<CHARTGEN_BASE_URL>/oauth/callback/<provider>`, and they must match
exactly. `CHARTGEN_BASE_URL` must be the public https origin: session cookies are only
marked Secure with https, and a mismatch makes the provider refuse the sign-in.

**GitHub**

1. Open **Settings → Developer settings → OAuth Apps → New OAuth App**.
2. Set **Homepage URL** to `https://chartgen.example.com`.
3. Set **Authorization callback URL** to `https://chartgen.example.com/oauth/callback/github`.
4. Register it, then **Generate a new client secret**.
5. Set `CHARTGEN_GITHUB_CLIENT_ID` and `CHARTGEN_GITHUB_CLIENT_SECRET`.

chartgen asks for no scope: it reads the public profile's id and login.

**Google**

1. In the [Google Cloud console](https://console.cloud.google.com/), create or pick a
   project and configure the **OAuth consent screen** with the scopes `openid`, `profile`
   and `email`. Publish it, or only listed test users can sign in.
2. Open **APIs & Services → Credentials → Create credentials → OAuth client ID**.
3. Choose **Web application**, and add `https://chartgen.example.com/oauth/callback/google`
   under **Authorized redirect URIs**.
4. Set `CHARTGEN_GOOGLE_CLIENT_ID` and `CHARTGEN_GOOGLE_CLIENT_SECRET`.

Restart, and the log says `Sign-in required, through: GitHub, Google`. With one provider
configured the choice of provider is skipped. Tokens are in memory, so clients sign in
once after that restart; existing clients keep their `client_id`.

Keep the volume at `~/.chartgen` mounted: sessions and users are stored there. Existing
scripts move to the first user who signs in through their client, and nothing is deleted
([ADR-0044](/chartgen/decisions/0044-sign-in-through-github-and-google/#existing-rows)). Check that the first person through a shared browser is
the right one.

Turning sign-in off again means removing the variables. Users' scripts stay in the file,
out of reach until it is turned back on.

## Reverse proxy

Claude.ai requires HTTPS. Terminate TLS at your reverse proxy and forward to
chartgen's HTTP port (default `9315`). The SSE stream on `/mcp` and `/sse`
must not be buffered — for nginx:

```nginx
location / {
    proxy_pass http://127.0.0.1:9315;
    proxy_http_version 1.1;
    proxy_buffering off;
    proxy_read_timeout 24h;
    proxy_set_header Host $host;
}
```

## Coolify auto-deploy

A systemd timer on the Coolify host (`scripts/autodeploy/`) polls the GitHub
API every 5 minutes. When `master` has a new head SHA and the `CI` workflow
succeeded for it, the script calls Coolify's deploy endpoint locally:

```
POST ${COOLIFY_URL}/api/v1/deploy?uuid=${COOLIFY_APP_UUID}&force=false
Authorization: Bearer ${COOLIFY_API_TOKEN}
```

A pending CI run is retried on the next tick; a failed one is not deployed.
A failed Coolify call keeps the old state, so the next tick retries. The
first run only records the current SHA. A deploy takes 20–25 minutes.
Coolify's endpoint takes no SHA and builds the branch head, so a commit that
lands between the CI check and the build can ship before its own CI on
`master` finishes. This is accepted: every merge passes pull-request CI
first and the window is small. While a new head is undeployed each tick
makes 2 GitHub calls (about 24 per hour, against the unauthenticated limit
of 60); an idle tick makes 1.
Nothing is exposed inbound and the repository holds no Coolify secrets; see
[ADR-0037](/chartgen/decisions/0037-pull-based-deploy/).

Install, pause and uninstall steps are in `scripts/autodeploy/README.md`.
The token lives in `/etc/chartgen-autodeploy.env` on the host:

| Variable | What it is |
|----------|------------|
| `COOLIFY_URL` | Coolify instance root, e.g. `http://coolify.lan:8000`. |
| `COOLIFY_APP_UUID` | UUID of the chartgen application in Coolify. |
| `COOLIFY_API_TOKEN` | Coolify API token with deploy permission. |
| `GITHUB_REPO` | `owner/name` to poll, default `5queezer/chartgen`. |

### The old GitHub workflow

`.github/workflows/deploy.yml` used to call a previous Coolify host with
`GET` from a GitHub runner. That host returned 403 and the current Coolify
is LAN-only, so GitHub-hosted runners cannot reach it. The workflow is
removed; the `COOLIFY_BASE_URL`, `COOLIFY_APP_UUID` and `COOLIFY_API_TOKEN`
repository secrets are unused and can be deleted.

## ARM cross-compilation

`Cross.toml` configures `cross` to apt-install the ARM64 variants of `libssl`,
`libfontconfig`, and `libfreetype` before building for `aarch64-unknown-linux-gnu`:

```bash
cargo install cross
cross build --release --target aarch64-unknown-linux-gnu
```

This is the same recipe the release workflow uses for the
`chartgen-linux-aarch64.tar.gz` artifact.
