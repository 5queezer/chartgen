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

The `~/.chartgen/` volume mount persists `alerts.json`, `trades.log`, and
`subscriptions.json` across restarts. See the [persistence reference](/chartgen/reference/persistence/).

### Environment variables

| Name | Read by | Purpose |
|------|---------|---------|
| `CHARTGEN_BASE_URL` | `src/server.rs` | Public HTTPS URL that chartgen embeds in OAuth metadata, redirect URLs, and `logo_uri`. Falls back to `http://localhost:<port>`, which Claude.ai rejects — always set this in production. |
| `CHARTGEN_WEB_DIR` | `src/server.rs` | Directory of a built web app (`web/dist`). When set, `GET /` with `Accept: text/html` returns its `index.html`, other paths fall back to its static files (unknown browser paths get `index.html`), and `/assets/*` is cached as immutable. Non-browser `GET /` still returns the health JSON. Unset means no static files. The Docker image sets it to `/usr/share/chartgen/web`. |
| `CHARTGEN_DATA_SOURCE` | `src/fetch.rs` | Set to `sample` to make `generate_chart`, `get_indicators` and `get_quotes` use a deterministic offline source and never call Binance or Yahoo. Bars follow the interval grid (weekly bars open on Monday), each bar's prices depend only on the symbol and its open time (so overlapping pages match exactly), and the forming candle moves every second. Intended for E2E tests and demos. Unset means live data. |
| `CHARTGEN_SCRIPTING` | `src/scripting/mod.rs` | `1` enables [scripted indicators](/chartgen/guides/scripted-indicators/): the four scripting MCP tools and user scripts in `generate_chart` / `list_indicators`. Off when unset. |
| `CHARTGEN_DB` | `src/scripting/persistence.rs` | SQLite file for scripted indicators and registered OAuth clients (only used with `CHARTGEN_SCRIPTING=1`). Default `~/.chartgen/chartgen.db`, inside the persisted volume. |
| `HOME` / `USERPROFILE` | `src/main.rs` | Picks the data directory (`~/.chartgen/`). Override to relocate `alerts.json` / `trades.log` / `subscriptions.json`. |
| `_CHARTGEN_PORT` | `src/server.rs` | Internal — set by the process to its own bind port so `base_url()` can compose the fallback URL. Do not set by hand. |

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

`.github/workflows/deploy.yml` fires when `CI` finishes successfully on
`master` and calls Coolify's deploy endpoint:

```
GET ${COOLIFY_BASE_URL}/api/v1/deploy?uuid=${COOLIFY_APP_UUID}&force=false
Authorization: Bearer ${COOLIFY_API_TOKEN}
```

Required repository secrets:

| Secret | What it is |
|--------|------------|
| `COOLIFY_BASE_URL` | Coolify instance root, e.g. `https://coolify.example.com`. |
| `COOLIFY_APP_UUID` | UUID of the chartgen application in Coolify. |
| `COOLIFY_API_TOKEN` | Coolify API token with deploy permission. |

## ARM cross-compilation

`Cross.toml` configures `cross` to apt-install the ARM64 variants of `libssl`,
`libfontconfig`, and `libfreetype` before building for `aarch64-unknown-linux-gnu`:

```bash
cargo install cross
cross build --release --target aarch64-unknown-linux-gnu
```

This is the same recipe the release workflow uses for the
`chartgen-linux-aarch64.tar.gz` artifact.
