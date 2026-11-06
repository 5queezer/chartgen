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
  chartgen --serve
```

Production must mount this volume. The `~/.chartgen/` mount persists
`alerts.json`, `trades.log`, `subscriptions.json`, and in `chartgen.db` the
registered OAuth clients, across restarts. Without it, every deploy forgets the
registrations of external MCP clients. See the [persistence reference](/chartgen/reference/persistence/).

### Environment variables

| Name | Read by | Purpose |
|------|---------|---------|
| `CHARTGEN_BASE_URL` | `src/server.rs` | Public HTTPS URL that chartgen embeds in OAuth metadata, redirect URLs, and `logo_uri`. Falls back to `http://localhost:<port>`, which Claude.ai rejects — always set this in production. |
| `CHARTGEN_WEB_DIR` | `src/server.rs` | Directory of a built web app (`web/dist`). When set, `GET /` with `Accept: text/html` returns its `index.html`, other paths fall back to its static files (unknown browser paths get `index.html`), and `/assets/*` is cached as immutable. Non-browser `GET /` still returns the health JSON. Unset means no static files. The Docker image sets it to `/usr/share/chartgen/web`. |
| `CHARTGEN_DATA_SOURCE` | `src/fetch.rs` | Set to `sample` to make `generate_chart`, `get_indicators` and `get_quotes` use a deterministic offline source and never call Binance or Yahoo. Bars follow the interval grid (weekly bars open on Monday), each bar's prices depend only on the symbol and its open time (so overlapping pages match exactly), and the forming candle moves every second. Intended for E2E tests and demos. Unset means live data. |
| `CHARTGEN_SCRIPTING` | `src/scripting/mod.rs` | `1` enables [scripted indicators](/chartgen/guides/scripted-indicators/): the four scripting MCP tools and user scripts in `generate_chart` / `list_indicators`. Off when unset. |
| `CHARTGEN_DB` | `src/scripting/persistence.rs` | SQLite file for registered OAuth clients, with or without `CHARTGEN_SCRIPTING=1`, and for scripted indicators. Opened at server startup; a failure is logged and leaves clients in memory only. Default `~/.chartgen/chartgen.db`, inside the persisted volume. |
| `CHARTGEN_SERVER_ALERTS` | `src/server_alerts.rs` | `1` enables [alerts on the server](/chartgen/guides/web/#alerts-on-the-server): four MCP tools, and **Run on server** in the web app. Off when unset; alerts already stored wait in `chartgen.db`, and pin nobody. Needs a build with the `scripting` feature, which the Docker image has, and the store. Read [before turning it on](#server-alerts). |
| `CHARTGEN_SERVER_ALERT_CLIENTS` | `src/server_alerts.rs` | Comma-separated OAuth client ids that may create server alerts and use webhooks. Unset: anyone may create alerts, nobody gets webhooks. `*`: anyone, with webhooks. |
| `CHARTGEN_SERVER_ALERT_PAIRS_PER_CLIENT` | `src/server_alerts.rs` | Symbol and interval pairs one client may watch. Default 2. |
| `CHARTGEN_WEBHOOK_PROXY` | `src/webhook.rs` | `http` or `https` URL of a proxy that sends the webhooks, so a receiver does not see this server's address. Unset means direct, ignoring `HTTPS_PROXY` and the like. |
| `CHARTGEN_VAPID_SUBJECT` | `src/server_alerts.rs` | `mailto:` or `https:` URL where push services can reach you. Falls back to an https `CHARTGEN_BASE_URL`; with neither, [browser push](#browser-push) stays off. |
| `CHARTGEN_VAPID_PRIVATE_KEY` | `src/server_alerts.rs` | Application server key for push, 32 bytes of base64url. Unset, one is made once and kept in `chartgen.db`. Changing it stops every device until its browser next opens chartgen. |
| `CHARTGEN_WEBHOOK_BLOCKED_PREFIXES` | `src/webhook.rs` | Comma-separated addresses or prefixes a webhook may not reach besides the private ranges (`203.0.113.7,2001:db8:1::/48`). For this network's own public addresses. |
| `CHARTGEN_WEBHOOK_TEST_ORIGIN` | `src/webhook.rs` | Tests only. One loopback origin (`http://127.0.0.1:9000`) a webhook may reach. Ignored unless `CHARTGEN_DATA_SOURCE=sample`, so it does nothing on a live server. |
| `HOME` / `USERPROFILE` | `src/main.rs` | Picks the data directory (`~/.chartgen/`). Override to relocate `alerts.json` / `trades.log` / `subscriptions.json` / `chartgen.db`. |
| `_CHARTGEN_PORT` | `src/server.rs` | Internal — set by the process to its own bind port so `base_url()` can compose the fallback URL. Do not set by hand. |

### Server alerts

`CHARTGEN_SERVER_ALERTS` is off by default. With open registration, leave it off, or turn it on
with `CHARTGEN_SERVER_ALERT_CLIENTS` listing the clients you trust. **That allowlist is the
recommended setup**: client ids are free, so no limit per client can stand in for it. A web user's
client id is `chartgen.oauth.client_id` in that browser's local storage, and an MCP client's is
what `/register` gave it. The list is read at startup, so a change takes a restart. From then on a
client taken off it runs no alerts and sends no webhooks, and what it had queued is dropped.

Before turning it on:

- Treat each listed id as a credential ([below](#a-listed-client-id-is-a-credential)).
- Log the live event order for a few minutes. That the closing kline follows the next candle's first
  trade is inferred from the parser and from test sources, and has not been observed on a live Binance
  stream. Watch `wss://stream.binance.com:9443/stream?streams=btcusdt@kline_1m/btcusdt@aggTrade` across
  a few minute boundaries and note when the kline with `"x":true` arrives against the first trade past
  the boundary. Both orders are handled and tested; anything else, such as closes that never arrive,
  means **Bar close** alerts can't be relied on. Then check that one fires.

| `CHARTGEN_SERVER_ALERT_CLIENTS` | Who can create alerts | Who gets webhooks |
|---|---|---|
| A list of ids (recommended) | Those clients | Those clients |
| Unset | Anyone | Nobody |
| `*` | Anyone | Anyone |

Unset, a stranger can hold alerts but cannot make the server send anything. Even then they can use
up the 200 alerts or the 16 watched symbol and interval pairs until those expire, after which other
people's new alerts are refused. Spent alerts are deleted after 7 days, and alerts whose owner had
no token for 30 days are too, unless you listed that client. There is no command to list or delete
someone's alerts yet: it takes SQL on `chartgen.db` and a restart.

**One webhook reveals your IP.** A receiver sees where the request came from. A tunnel hides your
home address from visitors, and a webhook gives it to anyone it is sent for, because the server
connects directly. List only clients you trust with your address, or set `CHARTGEN_WEBHOOK_PROXY`
to a proxy elsewhere, which hides it from receivers. The proxy resolves and connects to the receiver
itself, so run it somewhere that cannot reach this network. It does not hide everything: the name is
looked up from here first, so a receiver's DNS server sees whoever resolves for this network. That is
your ISP or a public resolver, or your home IP if you run your own recursive resolver.

Webhooks cannot reach private, loopback, link-local, CGNAT, multicast or metadata addresses, on IPv4
or IPv6, names with no dot, the hosts this server fetches data from, or its own host, and follow no
redirects ([ADR-0042](/chartgen/decisions/0042-server-side-alerts/)). What the checks cannot know is
which public addresses are yours:

- Your WAN address: a webhook at it reaches whatever your router forwards on port 443. A server
  behind a tunnel needs no forwarded port, so forward nothing there.
- A LAN device with a global IPv6 address, if the container has IPv6 egress: the router's firewall
  does not apply on-link.
- Anything that trusts your IP. Webhooks come from it, so a Cloudflare Access bypass or service rule
  that trusts your home address also trusts them. Check those before turning this on.

List such addresses in `CHARTGEN_WEBHOOK_BLOCKED_PREFIXES`. TLS limits the rest: a webhook only
delivers HTTP where a publicly trusted certificate matches the name.

A value in `CHARTGEN_WEBHOOK_BLOCKED_PREFIXES` or `CHARTGEN_WEBHOOK_PROXY` that does not parse keeps
server alerts off, with a line in the log, rather than running without it.

#### A listed client id is a credential

This server auto-approves sign-in. Anyone holding a client id and one of its redirect URIs gets a token
for that client, with no login and no consent, and a web client's redirect URI is simply this server's
own address. So whoever holds a listed id can do all that client can: create alerts, send webhooks, which
reveals your IP, and read, change or delete its alerts and notes. They cannot read its existing webhook
URLs or signing secrets, which are never returned. No other secret backs the list.

An id cannot be guessed, and the server does not log it, but nothing else treats it as a secret. It is in
`/authorize` URLs, so in browser history and proxy logs, in local storage, and in your deployment's
environment. Keep it as you would a password, and do not paste it into tickets or chats.

To replace one that leaked:

1. Get a new id. In the browser, clear this site's data, or remove `chartgen.oauth.client_id` and
   `chartgen.oauth.client_secret` from local storage, and reload. An MCP client registers again.
2. Stop the server. A restart also ends every token, which live in memory only, including any the old
   id was given.
3. Put the new id in `CHARTGEN_SERVER_ALERT_CLIENTS` in place of the old one.
4. Delete the old client and its alerts, or it can still sign in, as any stranger can, and see them:

   ```sql
   DELETE FROM server_alert_events WHERE alert_id IN (SELECT id FROM server_alerts WHERE owner = 'OLD_ID');
   DELETE FROM server_alerts WHERE owner = 'OLD_ID';
   DELETE FROM oauth_clients WHERE client_id = 'OLD_ID';
   ```

5. Start the server and create the alerts again under the new sign-in. They do not move across, and
   new signing secrets come with them.

Whoever held the id may already have sent a webhook, so treat your IP as known to them.

### Browser push

An alert can notify a browser or phone with chartgen closed
([ADR-0045](/chartgen/decisions/0045-web-push-for-server-alerts/)). It runs where server alerts do,
needs no setting of its own beyond `CHARTGEN_VAPID_SUBJECT` or an https `CHARTGEN_BASE_URL`, and needs
the site on https, because browsers only allow push there and on localhost.

| `CHARTGEN_SERVER_ALERT_CLIENTS` | Who gets push |
|---|---|
| A list of ids (recommended) | Those clients |
| Unset | Anyone, though nobody gets webhooks |
| `*` | Anyone |

**Requests go only to** `fcm.googleapis.com`, `updates.push.services.mozilla.com`, `*.push.apple.com` and
`*.notify.windows.com`, after the same address checks as a webhook, so a stranger cannot aim one at a
host of their choice. Those services do see this server's IP, unless `CHARTGEN_WEBHOOK_PROXY` is set,
which push also uses. If you filter outbound traffic, allow those hosts.

With the list unset, a stranger can register up to 5 devices per client id, and client ids are free, so
**the 2000 devices can be filled quickly**: 400 clients with made-up endpoints took 2.1 s in a review, from
loopback, which the limit on registrations exempts. From any other address that limit (20 per 10 minutes
per TCP peer) makes it about 3 h 20 min, and behind a tunnel all clients may share one peer, so real users
then wait with the attacker. Check which peer yours presents. When full, a new device replaces someone
else's: never a listed client's, and first those with no active alert that notifies, idle longest first. So a flood
mostly replaces itself, but it can push out a real user who has push on and no such alert, until their
browser next opens chartgen. The 120 deliveries a minute are shared in the same way, per client and not per
person, and the real ceiling is about 360 requests, because retries do not count.

**Recommended for this server until sign-in lands:** `CHARTGEN_SERVER_ALERT_CLIENTS`, and
`CHARTGEN_VAPID_SUBJECT` with your address. **Each browser is its own client**, so list one id for each browser
you use, and expect an alert to notify only the devices of the browser that made it: to get an alert on your
phone, make it on your phone.

**A listed client id is a password, for push as well.** Whoever has it can get a token, replace that
client's devices with their own, and receive its fires and notes. It appears in `/authorize` URLs, so in browser
history and in any proxy that logs query strings. Sign-in fixes this; until then, replace a leaked id as
[above](#server-alerts).

Tests only use a fake push service. Before relying on it, check real delivery by hand:

1. On the server, set `CHARTGEN_SERVER_ALERTS=1`, `CHARTGEN_SERVER_ALERT_CLIENTS` and
   `CHARTGEN_VAPID_SUBJECT`, and open the site over https.
2. **Desktop Chrome (FCM):** in the Alerts tab press **Enable notifications** and allow. Create a
   **Run on server** alert with **Send a notification** ticked, on a level the price will soon cross.
   Close every chartgen tab, and wait. A notification should appear; clicking it should open the chart
   on that symbol. The alert's row should then show no error.
3. **Firefox (Mozilla):** the same.
4. **Edge on Windows (Windows push):** the same.
5. **Android Chrome:** the same, with the app closed and the screen off.
6. **iPhone or iPad, iOS 16.4 or later (Apple):** in Safari the control should say to add it to the Home
   Screen. Do that, open it from there, sign in, then as above. Lock the phone and wait.
7. With a tab open and push on, check that one fire gives one notification and no toast.
8. **Turn off**: the next fire should not notify this device, and its row should say that no device has
   notifications on. Each browser is its own client, so two browsers cannot share an alert yet.

If a row shows `HTTP 401` or `HTTP 403`, the push service refused the VAPID token: check the subject.

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
