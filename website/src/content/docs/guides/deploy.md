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

The repo's `Dockerfile` is a 2-stage build (`rust:1.86-slim` →
`debian:bookworm-slim`, final image ~93 MB):

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
