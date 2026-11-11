---
title: Deployment
description: Docker, the prebuilt arm64 image, release binaries, reverse proxy, Coolify auto-deploy, ARM cross-compilation.
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

On an arm64 host there is nothing to build. CI pushes the image of every commit of
`master` that passed it ([ADR-0060](/chartgen/decisions/0060-image-built-in-ci/)):

```bash
docker pull ghcr.io/5queezer/chartgen:master          # master's head
docker pull ghcr.io/5queezer/chartgen:<full commit sha>
```

The image is `linux/arm64` only; on x86 build it as above or use a release binary.
The release build is one large compile: on a machine with 4 GB of memory it can
exhaust the machine, which is why production pulls.

Production must mount this volume. The `~/.chartgen/` mount persists
`alerts.json`, `trades.log`, `subscriptions.json`, and in `chartgen.db` the
registered OAuth clients and their refresh tokens, across restarts. Without it, every deploy forgets the
registrations of external MCP clients, and a
[headless agent](/chartgen/reference/oauth/#connecting-a-headless-agent) has to be authorized again by a person. See the [persistence reference](/chartgen/reference/persistence/).

The image's default command is `--serve`. Add `--trade` only on a server nobody else can
reach: its order tools and engine alerts are one shared set, not per user, for anyone with
a token. See [what a signed-in stranger can still do](/chartgen/decisions/0044-sign-in-through-github-and-google/#what-a-signed-in-stranger-can-still-do).

### Environment variables

| Name | Read by | Purpose |
|------|---------|---------|
| `CHARTGEN_BASE_URL` | `src/server.rs` | Public HTTPS URL that chartgen embeds in OAuth metadata, redirect URLs, and `logo_uri`. Falls back to `http://localhost:<port>`, which Claude.ai rejects — always set this in production. |
| `CHARTGEN_WEB_DIR` | `src/server.rs` | Directory of a built web app (`web/dist`). When set, `GET /` with `Accept: text/html` returns its `index.html`, other paths fall back to its static files (unknown browser paths get `index.html`), and `/assets/*` is cached as immutable. Non-browser `GET /` still returns the health JSON. Unset means no static files. The Docker image sets it to `/usr/share/chartgen/web`. |
| `CHARTGEN_DATA_SOURCE` | `src/fetch.rs` | Set to `sample` to make `generate_chart`, `get_indicators` and `get_quotes` use a deterministic offline source and never call Binance or Yahoo. Bars follow the interval grid (weekly bars open on Monday), each bar's prices depend only on the symbol and its open time (so overlapping pages match exactly), and the forming candle moves every second. Intended for E2E tests and demos. Unset means live data. |
| `CHARTGEN_SCRIPTING` | `src/scripting/mod.rs` | `1` enables [scripted indicators](/chartgen/guides/scripted-indicators/): the four scripting MCP tools and user scripts in `generate_chart` / `list_indicators`. Off when unset. |
| `CHARTGEN_DB` | `src/scripting/persistence.rs` | SQLite file for registered OAuth clients, with or without `CHARTGEN_SCRIPTING=1`, for their refresh tokens, for scripted indicators, and with sign-in on for users and sessions. Opened at server startup; a failure is logged and leaves clients in memory only. Default `~/.chartgen/chartgen.db`, inside the persisted volume. |
| `CHARTGEN_SERVER_ALERTS` | `src/server_alerts.rs` | `1` enables [alerts on the server](/chartgen/guides/web/#alerts-on-the-server): four MCP tools, and **Run on server** in the web app. Off when unset; alerts already stored wait in `chartgen.db`, and pin nobody. Needs a build with the `scripting` feature, which the Docker image has, and the store. Read [before turning it on](#server-alerts). |
| `CHARTGEN_SERVER_ALERT_CLIENTS` | `src/server_alerts.rs` | Comma-separated OAuth client ids, or with [sign-in](#sign-in-through-github-and-google) on, accounts (`github:<id>`, `google:<sub>`), that may create server alerts and use webhooks. Unset: anyone may create alerts, nobody gets webhooks. `*`: anyone, with webhooks. |
| `CHARTGEN_SERVER_ALERT_PAIRS_PER_CLIENT` | `src/server_alerts.rs` | Symbol and interval pairs one client may watch. Default 2. |
| `CHARTGEN_SETUPS` | `src/setup_service.rs` | `1` enables [setups](/chartgen/guides/setups/): five MCP tools for trading setups the server scores on paper. Off when unset; setups already stored wait in `chartgen.db`. Needs a build with the `scripting` feature and the store. It does not need `CHARTGEN_SERVER_ALERTS`. Read [before turning it on](#setups). |
| `CHARTGEN_SETUP_SYMBOLS_PER_OWNER` | `src/setup_service.rs` | Symbols one owner may have open setups on. Default 5. |
| `CHARTGEN_REPLAY_FIXTURES` | `src/replay.rs` | A directory of 1m fixture bars. Set, the server is a [replay server](/chartgen/guides/setups/#replay): it answers only from those bars, as of the time each call names, and refuses to start with `--trade`, `--mcp`, `CHARTGEN_SERVER_ALERTS=1`, `CHARTGEN_SETUPS=1` or a sign-in provider. For evaluation, not for a public server. |
| `CHARTGEN_WEBHOOK_PROXY` | `src/webhook.rs` | `http` or `https` URL of a proxy that sends the webhooks, so a receiver does not see this server's address. Unset means direct, ignoring `HTTPS_PROXY` and the like. |
| `CHARTGEN_VAPID_SUBJECT` | `src/server_alerts.rs` | `mailto:` or `https:` URL where push services can reach you. Falls back to an https `CHARTGEN_BASE_URL`; with neither, [browser push](#browser-push) stays off. |
| `CHARTGEN_VAPID_PRIVATE_KEY` | `src/server_alerts.rs` | Application server key for push, 32 bytes of base64url. Unset, one is made once and kept in `chartgen.db`. Changing it stops every device until its browser next opens chartgen. |
| `CHARTGEN_WEBHOOK_BLOCKED_PREFIXES` | `src/webhook.rs` | Comma-separated addresses or prefixes a webhook may not reach besides the private ranges (`203.0.113.7,2001:db8:1::/48`). For this network's own public addresses. |
| `CHARTGEN_WEBHOOK_TEST_ORIGIN` | `src/webhook.rs` | Tests only. One loopback origin (`http://127.0.0.1:9000`) a webhook may reach. Ignored unless `CHARTGEN_DATA_SOURCE=sample`, so it does nothing on a live server. |
| `CHARTGEN_GITHUB_CLIENT_ID`, `CHARTGEN_GITHUB_CLIENT_SECRET` | `src/server/signin.rs` | [Sign-in through GitHub](#sign-in-through-github-and-google). Unset: not offered. One without the other stops the server at startup. |
| `CHARTGEN_GOOGLE_CLIENT_ID`, `CHARTGEN_GOOGLE_CLIENT_SECRET` | `src/server/signin.rs` | The same for Google. |
| `CHARTGEN_CLAIM_OWNER` | `src/server/signin.rs` | The one account (`github:<id>`, `google:<sub>`) that [inherits](#sign-in-through-github-and-google) scripts, server alerts and push devices from before sign-in, with webhook secrets. Unset: nothing moves. |
| `CHARTGEN_TRUSTED_PROXY_HEADER` | `src/server.rs` | Header holding the client's address, such as `CF-Connecting-IP`, for the limits per address. Only if the server cannot be reached except through that proxy. Unset: no header is trusted, and the address is the TCP peer. |
| `CHARTGEN_SIGNIN_TEST_PROVIDER_ORIGIN` | `src/server/signin.rs` | Tests only: a loopback `http` origin that replaces both providers. Honored only with `CHARTGEN_DATA_SOURCE=sample`, ignored on live data. Never set it in production. |
| `HOME` / `USERPROFILE` | `src/main.rs` | Picks the data directory (`~/.chartgen/`). Override to relocate `alerts.json` / `trades.log` / `subscriptions.json` / `chartgen.db`. |
| `_CHARTGEN_PORT` | `src/server.rs` | Internal — set by the process to its own bind port so `base_url()` can compose the fallback URL. Do not set by hand. |

### Upstream load and memory

Binance is loaded per symbol and interval, not per user
([ADR-0049](/chartgen/decisions/0049-one-kline-read-per-symbol/)). All readers of a chart share
one window of klines: its first read costs one request per 1000 klines, and after that at most
about one request a second, whatever the number of users. Identical history pages share a read
as well. So a polled forming candle can be up to 3 s old. A push seed always reads for itself,
about one request per subscribe.

The cache holds at most about 33 MB of live klines. Expect more in resident memory: a review
probe on a debug build measured about 2.3 times that, from transient JSON and malloc arenas. So
watch RSS under load, and if it climbs, try `MALLOC_ARENA_MAX=2`. A window unused for 2 minutes
goes at the next read of any Binance chart; with no traffic it stays, within the bound. Each
request also holds its own copy of its bars while it computes, which the bound does not cover:
a burst of large custom-interval charts costs about 2 MB each.

More active windows than fit evict each other, and each poll then walks again: more than 12
windows at 20,000 klines, or about 42 at 6000. While Binance answers 5xx, a failed read stores
nothing and the next caller tries again, up to one request per round trip per chart.

All of it answers to the rate gate of [ADR-0038](/chartgen/decisions/0038-custom-intervals/). When
the gate refuses, clients see an error starting `Binance rate limit:`. Yahoo is not shared this
way: every chart load and poll of a Yahoo symbol is its own request, with no gate in front.

## Sign-in through GitHub and Google

Without sign-in, anyone who knows a `client_id` gets a token, and a public server logs a
warning about that at startup. Configure one or both providers to require a login.
**Anyone with an account can then sign in**; there is no allowlist. Read
[what they can still do](/chartgen/decisions/0044-sign-in-through-github-and-google/#what-a-signed-in-stranger-can-still-do) first. In short, keep
`CHARTGEN_SCRIPTING` off, do not use `--trade`, and keep `CHARTGEN_SERVER_ALERTS` off or list
trusted accounts on a public server.

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
configured the choice of provider is skipped. A token from before sign-in, access or refresh,
is not valid with it on, so every client signs in
once after that restart; existing clients keep their `client_id`.

Keep the volume at `~/.chartgen` mounted: sessions and users are stored there.

**Existing scripts, server alerts and push devices stay where they are until you name who inherits them.** With
sign-in on nobody can reach them, and startup says how many wait. To collect them, set
`CHARTGEN_CLAIM_OWNER` to your own account, `github:<numeric id>` (the `id` in
`https://api.github.com/users/<login>`) or `google:<sub>`, and sign in, through any client. You
then own every client's rows, whichever client made them. **An alert's webhook and signing secret
move with it, and that account's alerts then notify those devices**, so name only yourself. A script name you already have stays with its client, and
so does the second of two clients' scripts with one name: nothing is overwritten or deleted, and
startup counts these name collisions. To move one, rename or delete your own and sign in again
([ADR-0044](/chartgen/decisions/0044-sign-in-through-github-and-google/#existing-rows)). Scripts wait for as long as it takes. Alerts keep
running meanwhile, and expire 30 days after their client last got a token, so do not leave it long.
You can unset the variable afterwards.

**On a public server, also name your proxy's address header.** With
`CHARTGEN_TRUSTED_PROXY_HEADER=CF-Connecting-IP`, or whatever single-address header your proxy
sets, each client gets its own limits on sign-in, registration and chart push, with an IPv6 /64
counted as one address. Without it every
visitor is the proxy's address, and one person can use up everyone's: 120 sign-in steps a minute
and 20 registrations per 10 minutes, shared by all, and 50 open sign-ins, while a same-host proxy
has no budget at all and a flood through it pushes out everyone's open sign-ins. **Only set it if
the server cannot be reached except through that proxy**, because anyone reaching it directly can
write that header and claim any address. **The proxy must replace the header, never append to
it**: with an appending proxy the client's own value comes first and wins, so `X-Forwarded-For`
will not do. A value that is not exactly one address shares a single budget with every other
such value.

Turning sign-in off again means removing the provider variables. Users' scripts and alerts stay in
the file, out of reach until it is turned back on, and those alerts expire after 30 days.

### Each user's workspace

With sign-in on, a user's settings, layouts, watchlists, drawings and browser alerts are stored in
`chartgen.db`, so keep the `~/.chartgen` volume. There is nothing to set. Each document may be
512 KiB, each user 2 MiB and 1000 documents, and all users together 1 GiB, past which nobody's
documents grow and existing ones stay; new accounts stop at nine tenths of it. Each user may send
8 MiB and 120 requests a minute, one at a time, with 8 running at once in all. Under that load from
ten accounts, a debug build stayed under 97 MB in a 256 MiB container. A build without the `scripting` feature has no store: the app
then keeps each account's data apart in the browser and syncs nothing
([ADR-0050](/chartgen/decisions/0050-per-user-workspace-sync/)). Back up the file as you would any
user data.

### Setups

`CHARTGEN_SETUPS` is off by default. A setup sends nothing anywhere, but each symbol with an
open setup holds one of the server's **16 watched pairs**, which setups share with
[server alerts](#server-alerts). An alert and a setup on the same symbol are one pair.

- **Who may create setups** follows `CHARTGEN_SERVER_ALERT_CLIENTS`, whether or not alerts are
  on. Where it lists accounts, only those may. Unset or `*`, anyone who can get a token may.
  The list is read at startup. Take an account off it and restart: its setups hold no feed and
  are not scored from then on, and it can still read them. Past their expiry they are marked
  expired, so they stop counting as open.
- **On a public server, list the accounts you trust.** Otherwise strangers can hold all 16
  pairs: four accounts with open setups on 5 symbols each are enough, and with sign-in off
  client ids are free. Your own agent is then refused a new symbol, and new alerts on new
  symbols are refused too.
- `CHARTGEN_SETUP_SYMBOLS_PER_OWNER` raises or lowers the 5 symbols per owner. The 16 is a
  constant in the server; it cannot be raised without building it.
- Closed setups are kept as the track record and are not deleted for inactivity. An owner
  holds at most 500 setups and 4 MiB, the server 20,000 and 256 MiB.
- After a restart or a deploy, the server reads the 1m bars each open setup missed, once per
  symbol and at most 1000 bars, through the same rate limits as a chart.

### Server alerts

`CHARTGEN_SERVER_ALERTS` is off by default. With open registration, leave it off, or turn it on
with `CHARTGEN_SERVER_ALERT_CLIENTS` listing the clients you trust. **That allowlist is the
recommended setup**: client ids are free, so no limit per client can stand in for it. A web user's
client id is `chartgen.oauth.client_id` in that browser's local storage, and an MCP client's is
what `/register` gave it. The list is read at startup, so a change takes a restart. From then on a
client taken off it runs no alerts and sends no webhooks, and what it had queued is dropped.

**With [sign-in](#sign-in-through-github-and-google) on, the list names accounts, not clients.**
Replace each client id with `github:<numeric id>` or `google:<sub>`. A client id then matches
nobody, and startup logs how many entries look like one. For GitHub the id is `id` in
`https://api.github.com/users/<login>`; for either provider, once the person has signed in, it
is in the file: `sqlite3 chartgen.db "SELECT id, name FROM users"`. An account is not a credential
like a client id is, so the section on those below does not apply.

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
no token for 30 days are too, unless you listed that client. For a signed-in owner a sign-in counts
as well, and signing out does not stop their alerts. There is no command to list or delete
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
2. Stop the server. A restart ends the web app's access tokens. What the old id was given beyond
   that, refresh tokens and the access tokens that come back with them, goes in step 4.
3. Put the new id in `CHARTGEN_SERVER_ALERT_CLIENTS` in place of the old one.
4. Delete the old client and its alerts, or it can still sign in, as any stranger can, and see them:

   ```sql
   DELETE FROM server_alert_events WHERE alert_id IN (SELECT id FROM server_alerts WHERE owner = 'OLD_ID');
   DELETE FROM server_alerts WHERE owner = 'OLD_ID';
   DELETE FROM refresh_tokens WHERE client_id = 'OLD_ID';
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

**Recommended for this server:** [sign-in](#sign-in-through-github-and-google),
`CHARTGEN_SERVER_ALERT_CLIENTS` listing the accounts you trust, and `CHARTGEN_VAPID_SUBJECT` with your address.
With sign-in, devices belong to the user, so an alert made on the desktop also reaches the phone, and the 5 devices
and 30 pushes a minute are per user. **Without sign-in each browser is its own client**, so list one id for each
browser you use, and expect an alert to notify only the devices of the browser that made it.

**Without sign-in, a listed client id is a password, for push as well.** Whoever has it can get a token, replace that
client's devices with their own, and receive its fires and notes. It appears in `/authorize` URLs, so in browser
history and in any proxy that logs query strings. Replace a leaked id as [above](#server-alerts).

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

The server limits what any client may cost it, whether or not there is a proxy
([ADR-0051](/chartgen/decisions/0051-bounded-connections/)): 128 connections at once, with further ones
waiting, a small socket buffer each, and an end to a connection that has stopped moving for 30 s or gone
quiet for 120 s. Long-lived streams stop at 64. Tool calls share a budget, with images counted by their pixels,
and past it a call gets `503` with `Retry-After` after 20 s, from which the web app recovers by itself. A request body may be 256 KiB. There is
nothing to set. Keep your proxy's own idle timeout below 120 s, or it may reuse a connection just as the server
closes it. Traefik's default of 90 s is fine. The numbers assume Linux with glibc, as in the Docker image.

What a proxy buffers towards a client that does not read is the proxy's memory, not the server's. A local
Caddy took 1.5 GB for 1500 such requests. If the proxy can be reached directly, limit it as well.

The server forbids framing of its pages and content sniffing by itself, and answers other
origins on the MCP and OAuth client routes only
([ADR-0062](/chartgen/decisions/0062-cors-per-route-and-no-framing/)); there is nothing to add at
the proxy for that, and it should not add CORS headers of its own. Two things are the proxy's:

- **`Strict-Transport-Security`.** The server speaks plain HTTP behind the proxy and cannot
  know the public side is HTTPS, so it does not send this header. Send it from the proxy.
- **No storing of answers marked `no-store`.** The sign-in and consent pages are marked so.
  A cache in front must honour it; do not configure one to ignore the origin's
  `Cache-Control`.

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
API every 5 minutes. When `master` has a new head SHA, the `CI` workflow
succeeded for it and the image CI built for it is in the registry, the script
points the Coolify application at that image and calls Coolify's deploy
endpoint, both locally:

```
GET   ${COOLIFY_URL}/api/v1/applications/${COOLIFY_APP_UUID}      (must say "build_pack": "dockerimage")
PATCH ${COOLIFY_URL}/api/v1/applications/${COOLIFY_APP_UUID}
      {"docker_registry_image_name": "${CHARTGEN_IMAGE}", "docker_registry_image_tag": "<sha>"}
POST  ${COOLIFY_URL}/api/v1/deploy?uuid=${COOLIFY_APP_UUID}&force=false
Authorization: Bearer ${COOLIFY_API_TOKEN}
```

The host pulls the image and builds nothing, and what goes out is exactly the
commit that passed CI. The application in Coolify is of the kind **Docker
Image**; the script refuses any other, because a deploy of one that builds
from a Dockerfile would build on the host. A pending CI run, an image that is not published yet and a registry
that refuses or rate-limits are all retried on the next tick; a failed CI run
is not deployed. A failed Coolify call keeps the old state, so the next tick
sets the tag and deploys again. The first run only records the current SHA.

The script logs that a deploy was triggered. It does not wait for it or check
the application: look at the deployment in Coolify. `chartgen-autodeploy.sh
rollback` deploys the image the script triggered before its last one, and
`deploy <sha>` any published one, by its full commit SHA. After either, the
timer leaves `master`'s head of that moment alone and deploys the next
commit. One run works at a time, the timer's or a command by hand.

While a new head is undeployed each tick makes 2 GitHub calls (about 24 per
hour, against the unauthenticated limit of 60); an idle tick makes 1. The
registry is asked on its own host and does not count against that.
Nothing is exposed inbound and the repository holds no Coolify secrets; see
[ADR-0037](/chartgen/decisions/0037-pull-based-deploy/) and
[ADR-0060](/chartgen/decisions/0060-image-built-in-ci/).

Install, pause, rollback and uninstall steps are in
`scripts/autodeploy/README.md`, and so is **moving an existing application from
a Dockerfile build to the image. Follow those steps in their order:** the image
is proven to pull before anything is stopped, the data volume is copied into a
place proven empty, and each copy is checked by what is in the database. An
application that starts on an empty or wrong database has lost its clients,
users, scripts, alerts and refresh tokens. The configuration lives in
`/etc/chartgen-autodeploy.env` on the host:

| Variable | What it is |
|----------|------------|
| `COOLIFY_URL` | Coolify instance root, e.g. `http://coolify.lan:8000`. |
| `COOLIFY_APP_UUID` | UUID of the chartgen application in Coolify, a Docker Image application. |
| `COOLIFY_API_TOKEN` | Coolify API token that may read, change and deploy the application. |
| `GITHUB_REPO` | `owner/name` to poll, default `5queezer/chartgen`. |
| `CHARTGEN_IMAGE` | The image without a tag, default `ghcr.io/5queezer/chartgen`. Its package must be public. |

### The image

The `image` job of `.github/workflows/ci.yml` builds the `Dockerfile` on a
GitHub-hosted arm64 runner after every other CI job passed for the commit, on
each push to `master`. To build the image of another ref, for a staging
server, run the workflow on it: **Actions, CI, Run workflow**, and choose the
ref. The whole of CI runs on that ref first, and the image is pushed under
its commit SHA. The run uses the workflow file of the chosen ref, so the ref
must contain the `image` job. `:master` is moved only from `master`.

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
