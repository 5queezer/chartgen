"""Locust load test for chartgen, modelled on what the web app sends.

Usage and the safety notes are in README.md. In short:

    locust -f scripts/load/locustfile.py --headless -H <base url> --profile smoke --csv out/smoke

Exit codes: 0 clean, 1 final failure ratio above the limit, 2 refused or could not
start (guard, auth, bad arguments), 3 a stop condition tripped.
"""

import base64
import collections
import hashlib
import itertools
import json
import logging
import random
import re
import secrets
import sys
import time
from urllib.parse import parse_qs, urlparse

import gevent
from locust import HttpUser, LoadTestShape, between, events, task
from locust.clients import HttpSession
from locust.runners import MasterRunner

log = logging.getLogger("chartgen-load")

# --- The app's constants (web/src/lib/liveFeed.ts, web/src/queries.ts) -----------
INITIAL_BARS = 500
PAGE_BARS = 500
WARMUP = 300
TAIL_BARS = 2
FAST_POLL_S = 2.0  # 24/7 markets
SLOW_POLL_S = 15.0  # session markets
MAX_BACKOFF_S = 60.0
QUOTES_POLL_S = 5.0
APP_WATCHLIST = "BTCUSDT,ETHUSDT,SOLUSDT,AAPL,MSFT,NVDA,SPY"
CONTINUOUS_SUFFIXES = ("USDT", "USDC", "BUSD", "BTC", "ETH")
# Preferred picks; the ones the server does not list are skipped.
LIGHT_INDICATORS = ["ema_stack", "rsi", "macd", "bbands", "atr", "obv", "supertrend"]
HEAVY_INDICATORS = ["chart_patterns", "volume_profile"]
REQUEST_TIMEOUT_S = 30
# Only in error texts, never in bar data, so a price like 418.2 cannot match.
GATE_RE = re.compile(r"Binance rate limit|HTTP (?:418|429)|\b(?:418|429)\b|too many requests", re.IGNORECASE)
# subscribe_chart errors that mean "poll this chart instead", as the app does.
PUSH_FALLBACKS = ("not pushable", "push limit reached", "feeds busy")

# --- Options -----------------------------------------------------------------------


@events.init_command_line_parser.add_listener
def _add_args(parser):
    g = parser
    a = lambda *n, **k: g.add_argument(*n, include_in_web_ui=False, **k)
    a("--load-profile", choices=["smoke", "ramp", "soak"], default="smoke", env_var="LOAD_PROFILE")
    a("--smoke-seconds", type=int, default=120, env_var="LOAD_SMOKE_SECONDS", help="smoke: 1 to 5 users over this long")
    a("--ramp-users", default="5,10,20,40,80", env_var="LOAD_RAMP_USERS", help="ramp: user count per step")
    a("--step-seconds", type=int, default=180, env_var="LOAD_STEP_SECONDS", help="ramp: seconds per step")
    a("--soak-users", type=int, default=10, env_var="LOAD_SOAK_USERS")
    a("--soak-seconds", type=int, default=1800, env_var="LOAD_SOAK_SECONDS")
    a("--symbols", default="BTCUSDT,ETHUSDT,SOLUSDT", env_var="LOAD_SYMBOLS", help="chart symbols, most popular first")
    a("--intervals", default="15m,1h,4h,1d", env_var="LOAD_INTERVALS")
    a("--watchlist", default=APP_WATCHLIST, env_var="LOAD_WATCHLIST", help="get_quotes symbols (app default)")
    a("--real-data", action="store_true", env_var="LOAD_REAL_DATA", help="allow a server on live Binance/Yahoo data")
    a("--no-png", action="store_true", env_var="LOAD_NO_PNG", help="never render PNGs")
    a("--png-weight", type=int, default=1, env_var="LOAD_PNG_WEIGHT", help="task weight; the others total 15")
    a("--max-streams", type=int, default=8, env_var="LOAD_MAX_STREAMS", help="push streams held at once; 0 turns push off")
    a("--heavy", choices=["auto", "on", "off"], default="auto", env_var="LOAD_HEAVY", help="auto: heavy users iff the server lists the indicators")
    a("--max-error-rate", type=float, default=0.05, env_var="LOAD_MAX_ERROR_RATE")
    a("--error-window", type=int, default=30, env_var="LOAD_ERROR_WINDOW", help="seconds")
    a("--max-p95-ms", type=int, default=10_000, env_var="LOAD_MAX_P95_MS")
    a("--p95-window", type=int, default=60, env_var="LOAD_P95_WINDOW", help="seconds")
    a("--burst-5xx", type=int, default=5, env_var="LOAD_BURST_5XX", help="5xx responses within --burst-window")
    a("--burst-window", type=int, default=10, env_var="LOAD_BURST_WINDOW", help="seconds")
    a("--min-requests", type=int, default=20, env_var="LOAD_MIN_REQUESTS", help="fewer requests in a window never trip a rate")


def _split(s):
    return [x.strip() for x in s.split(",") if x.strip()]


# --- Authentication: everything that changes with sign-in (ADR-0044) is here -------

_CLIENT = {}  # the one OAuth client of this process, registered on first use


def authenticate(http, base):
    """A bearer token for one simulated user.

    The first call registers the process's single OAuth client (the per-IP registration
    limit is 20, and production's client table gains one row per run); every call then
    goes through the authorization-code flow with PKCE, which auto-approves.

    When sign-in lands this is the only function to change: it must return whatever the
    server then accepts as `Authorization: Bearer ...`. A test account's credentials
    would come from the environment, never from the repo.
    """
    redirect = base.rstrip("/") + "/"
    if "id" not in _CLIENT:
        body = {
            "client_name": "chartgen-load-test",
            "redirect_uris": [redirect],
            "grant_types": ["authorization_code"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none",
        }
        r = http.post("/register", json=body, name="oauth register", timeout=REQUEST_TIMEOUT_S)
        if r.status_code not in (200, 201):
            raise RuntimeError(f"registration failed: HTTP {r.status_code} {r.text[:120]}")
        _CLIENT["id"] = r.json()["client_id"]
    verifier = secrets.token_urlsafe(32)
    challenge = base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest()).rstrip(b"=").decode()
    params = {
        "response_type": "code",
        "client_id": _CLIENT["id"],
        "redirect_uri": redirect,
        "code_challenge": challenge,
        "code_challenge_method": "S256",
        "state": secrets.token_urlsafe(12),
    }
    r = http.get("/authorize", params=params, allow_redirects=False, name="oauth authorize", timeout=REQUEST_TIMEOUT_S)
    code = parse_qs(urlparse(r.headers.get("location", "")).query).get("code", [None])[0]
    if code is None:
        raise RuntimeError(f"authorize gave no code: HTTP {r.status_code}")
    form = {
        "grant_type": "authorization_code",
        "code": code,
        "redirect_uri": redirect,
        "client_id": _CLIENT["id"],
        "code_verifier": verifier,
    }
    r = http.post("/token", data=form, name="oauth token", timeout=REQUEST_TIMEOUT_S)
    if r.status_code != 200:
        raise RuntimeError(f"token exchange failed: HTTP {r.status_code} {r.text[:120]}")
    return r.json()["access_token"]


# --- MCP over Streamable HTTP --------------------------------------------------------

_ids = itertools.count(1)


def mcp(http, token, sid, method, params=None, *, name, notify=False, ok_errors=()):
    """POST one JSON-RPC message. Returns (result, error_text, response headers).

    A tool error is a failure unless its text starts with one of `ok_errors`. A text that
    shows Binance's gate (or a 429/418) also trips the stop condition.
    """
    body = {"jsonrpc": "2.0", "method": method}
    if params is not None:
        body["params"] = params
    if not notify:
        body["id"] = next(_ids)
    headers = {
        "Authorization": f"Bearer {token}",
        "Accept": "application/json, text/event-stream",
        "Content-Type": "application/json",
        "MCP-Protocol-Version": "2025-03-26",
    }
    if sid:
        headers["Mcp-Session-Id"] = sid
    with http.post("/mcp", data=json.dumps(body), headers=headers, name=name, catch_response=True, timeout=REQUEST_TIMEOUT_S) as r:
        if r.status_code not in (200, 202):
            r.failure(f"HTTP {r.status_code}")
            return None, f"HTTP {r.status_code}", r.headers
        if notify:
            r.success()
            return {}, None, r.headers
        try:
            msg = r.json()
        except ValueError:
            r.failure("response is not JSON")
            return None, "not JSON", r.headers
        err = msg.get("error", {}).get("message") if "error" in msg else None
        result = msg.get("result") or {}
        if err is None and result.get("isError"):
            err = " ".join(c.get("text", "") for c in result.get("content", []))
        if err is not None:
            WATCH.scan_gate(err, name)
            if err.startswith(ok_errors) and ok_errors:
                r.success()
            else:
                r.failure(err[:120])
            return None, err, r.headers
        r.success()
        return result, None, r.headers


def tool(http, token, sid, name, args, *, label, ok_errors=(), parse=True):
    """Call a tool; returns its JSON payload (or the raw result when parse is false)."""
    result, err, _ = mcp(http, token, sid, "tools/call", {"name": name, "arguments": args}, name=label, ok_errors=ok_errors)
    if result is None or not parse:
        return result if not parse else (None if err is None else {"_error": err})
    try:
        return json.loads(result["content"][0]["text"])
    except (KeyError, IndexError, ValueError):
        return None


# --- Stop conditions -----------------------------------------------------------------


class Watch:
    """Rolling windows over the request events, checked once a second."""

    def __init__(self):
        self.reset()

    def reset(self):
        self.samples = collections.deque()  # (time, ms, failed)
        self.fivexx = collections.deque()
        self.gate = None
        self.started = None
        self.reason = None

    def scan_gate(self, text, where=""):
        if self.gate is None and GATE_RE.search(text or ""):
            self.gate = f"{where}: {text[:100]}"

    def record(self, request_type, name, ms, failed, status):
        if request_type == "PUSH":  # held streams are not requests
            return
        now = time.time()
        self.started = self.started or now
        self.samples.append((now, ms, failed))
        if status >= 500:
            self.fivexx.append(now)
        if status in (418, 429) and self.gate is None:
            self.gate = f"{name}: HTTP {status}"

    def check(self, o):
        now = time.time()
        horizon = now - max(o.error_window, o.p95_window)
        while self.samples and self.samples[0][0] < horizon:
            self.samples.popleft()
        while self.fivexx and self.fivexx[0] < now - o.burst_window:
            self.fivexx.popleft()
        if self.gate:
            return f"Binance gate or rate limit shown: {self.gate}"
        if len(self.fivexx) >= o.burst_5xx:
            return f"5xx burst: {len(self.fivexx)} in {o.burst_window}s"
        win = [s for s in self.samples if s[0] >= now - o.error_window]
        if len(win) >= o.min_requests:
            rate = sum(s[2] for s in win) / len(win)
            if rate > o.max_error_rate:
                return f"error rate {rate:.1%} over {o.error_window}s (limit {o.max_error_rate:.1%}, {len(win)} requests)"
        win = [s[1] for s in self.samples if s[0] >= now - o.p95_window]
        if self.started and now - self.started >= o.p95_window and len(win) >= o.min_requests:
            p95 = sorted(win)[int(len(win) * 0.95) - 1 if len(win) > 1 else 0]
            if p95 > o.max_p95_ms:
                return f"p95 {p95:.0f} ms over {o.p95_window}s (limit {o.max_p95_ms} ms)"
        return None


WATCH = Watch()
_watcher = {}


@events.request.add_listener
def _on_request(request_type, name, response_time, response_length, exception=None, response=None, **kw):
    status = getattr(response, "status_code", 0) or 0
    WATCH.record(request_type, name, response_time, exception is not None, status)


def _watch_loop(environment):
    o = environment.parsed_options
    while True:
        gevent.sleep(1)
        reason = WATCH.check(o)
        if reason:
            WATCH.reason = reason
            log.error("STOP CONDITION: %s", reason)
            environment.process_exit_code = 3
            environment.runner.quit()
            return


@events.test_start.add_listener
def _test_start(environment, **kw):
    if isinstance(environment.runner, MasterRunner):
        return
    WATCH.reset()
    _watcher["g"] = gevent.spawn(_watch_loop, environment)


@events.test_stop.add_listener
def _test_stop(environment, **kw):
    if "g" in _watcher:
        _watcher.pop("g").kill(block=False)


# --- Startup: guard, auth, discovery ---------------------------------------------------

AVAILABLE = []  # indicator names the server lists
STREAMS = {"open": 0, "ticks": 0}


def _die(msg):
    log.error(msg)
    sys.exit(2)


@events.init.add_listener
def _setup(environment, **kw):
    if isinstance(environment.runner, MasterRunner):
        return
    o = environment.parsed_options
    if not environment.host:
        _die("no host: pass -H <base url>")
    if not _split(o.symbols) or not _split(o.intervals):
        _die("--symbols and --intervals must not be empty")
    http = HttpSession(base_url=environment.host, request_event=environment.events.request, user=None)
    try:
        token = authenticate(http, environment.host)
    except Exception as e:  # noqa: BLE001
        _die(f"authentication failed: {e}")
    # Data-source guard. One get_quotes call; on a live server that is one Binance
    # request through the server's own gate, before any load.
    q = tool(http, token, None, "get_quotes", {"symbols": ["BTCUSDT"]}, label="guard get_quotes")
    sources = {e.get("source") for e in (q or {}).get("quotes", [])}
    if sources == {"sample"}:
        log.info("server reports sample data")
    elif not o.real_data:
        _die(f"REFUSED: server does not report sample data (get_quotes source: {sources or q}). Pass --real-data to load a live server.")
    else:
        log.warning("--real-data: this run reaches Binance/Yahoo through the server")
    listed = tool(http, token, None, "list_indicators", {}, label="discover list_indicators")
    AVAILABLE[:] = [i["name"] for i in (listed or {}).get("indicators", [])]
    if not AVAILABLE:
        _die("server lists no indicators")
    heavy = [n for n in HEAVY_INDICATORS if n in AVAILABLE]
    if o.heavy == "off" or not heavy:
        if o.heavy == "on":
            log.warning("--heavy on, but the server lists none of %s: no heavy users", HEAVY_INDICATORS)
        environment.user_classes = [c for c in environment.user_classes if c is not HeavyUser]
    # Task weights: PNG by option, the others fixed.
    tasks = [(WebUser.switch_chart, 4), (WebUser.scroll_back, 6), (WebUser.search, 2), (WebUser.open_app, 3)]
    if not o.no_png and o.png_weight > 0:
        tasks.append((WebUser.render_png, o.png_weight))
    WebUser.tasks = HeavyUser.tasks = [fn for fn, w in tasks for _ in range(w)]


# --- Shapes -------------------------------------------------------------------------------


class Profile(LoadTestShape):
    """smoke / ramp / soak, chosen by --profile."""

    def tick(self):
        o = self.runner.environment.parsed_options
        t = self.get_run_time()
        if o.load_profile == "smoke":
            if t >= o.smoke_seconds:
                return None
            users = 1 if t < o.smoke_seconds * 0.25 else 3 if t < o.smoke_seconds * 0.6 else 5
            return users, 2
        if o.load_profile == "ramp":
            steps = [int(x) for x in _split(o.ramp_users)]
            i = int(t // o.step_seconds)
            return None if i >= len(steps) else (steps[i], max(2, steps[i] // 4))
        return None if t >= o.soak_seconds else (o.soak_users, 1)


# --- Users ------------------------------------------------------------------------------------

ASSET_RE = re.compile(r'(?:src|href)="(/[^"]+)"')


class WebUser(HttpUser):
    """One browser tab on the web app: open, load a chart, then poll and click about."""

    weight = 10
    wait_time = between(5, 20)

    # -- helpers
    def call(self, name, args, label, **kw):
        return tool(self.client, self.token, self.sid, name, args, label=label, **kw)

    def pick_indicators(self):
        light = [n for n in LIGHT_INDICATORS if n in AVAILABLE] or AVAILABLE[:3]
        return random.sample(light, k=min(len(light), random.choice([2, 3])))

    def chart_args(self, **extra):
        a = {
            "ticker": self.symbol,
            "timeframe": self.interval,
            "format": "series",
            "indicators": self.indicators,
            "chart_style": "candles",
        }
        a.update(extra)
        return a

    def continuous(self):
        return self.symbol.endswith(CONTINUOUS_SUFFIXES)

    def load_chart(self, label="load"):
        p = self.call("generate_chart", self.chart_args(bars=INITIAL_BARS, warmup=WARMUP), f"generate_chart {label}")
        bars = (p or {}).get("bars") or []
        self.oldest = bars[0]["t"] if bars and bars[0].get("t") else None
        self.has_more = bool((p or {}).get("has_more"))
        return p

    # -- lifecycle
    def on_start(self):
        o = self.environment.parsed_options
        self.symbols, self.intervals = _split(o.symbols), _split(o.intervals)
        self.watchlist = _split(o.watchlist)
        self.token = authenticate(self.client, self.host)
        self.sid = None
        self.sub = None
        self.stream = None
        self.bg = []
        self.symbol = self.pick_symbol()
        self.interval = random.choice(self.intervals)
        self.indicators = self.pick_indicators()
        self.open_app()
        _, _, h = mcp(
            self.client, self.token, None, "initialize",
            {"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "chartgen-web", "version": "0.1.0"}},
            name="mcp initialize",
        )
        self.sid = h.get("Mcp-Session-Id")
        mcp(self.client, self.token, self.sid, "notifications/initialized", name="mcp initialized", notify=True)
        mcp(self.client, self.token, self.sid, "tools/list", {}, name="mcp tools/list")
        self.call("list_indicators", {}, "list_indicators")
        self.quotes()
        self.load_chart("initial")
        if STREAMS["open"] < o.max_streams:
            STREAMS["open"] += 1
            self.stream = gevent.spawn(self.hold_stream)
        self.bg = [gevent.spawn(self.poll_loop), gevent.spawn(self.quotes_loop)]

    def on_stop(self):
        gevent.killall(self.bg + ([self.stream] if self.stream else []), block=False)
        if self.stream:
            STREAMS["open"] -= 1

    def pick_symbol(self, avoid=None):
        pool = [s for s in self.symbols if s != avoid] or self.symbols
        # most popular first: weight 1/(rank+1)
        return random.choices(pool, weights=[1 / (i + 1) for i in range(len(pool))])[0]

    # -- background: the app's own timers
    def poll_loop(self):
        failures = 0
        gevent.sleep(random.uniform(0, FAST_POLL_S))
        while True:
            period = FAST_POLL_S if self.continuous() else SLOW_POLL_S
            if self.sub is None:  # a push subscription replaces polling
                p = self.call("generate_chart", self.chart_args(bars=TAIL_BARS, warmup=WARMUP), "generate_chart tail")
                failures = failures + 1 if p is None or "_error" in p else 0
            period = min(period * 2**failures, MAX_BACKOFF_S)
            gevent.sleep(period * random.uniform(0.95, 1.05))

    def quotes(self):
        q = self.call("get_quotes", {"symbols": self.watchlist}, "get_quotes watchlist")
        for e in (q or {}).get("quotes", []):
            WATCH.scan_gate(e.get("error", ""), "get_quotes " + e.get("symbol", ""))

    def quotes_loop(self):
        gevent.sleep(random.uniform(0, QUOTES_POLL_S))
        while True:
            self.quotes()
            gevent.sleep(QUOTES_POLL_S * random.uniform(0.95, 1.05))

    # -- push: one held SSE stream per user, up to --max-streams
    def hold_stream(self):
        headers = {"Authorization": f"Bearer {self.token}", "Accept": "text/event-stream", "Mcp-Session-Id": self.sid or ""}
        try:
            with self.client.get("/mcp", headers=headers, stream=True, name="sse connect", catch_response=True, timeout=(10, 75)) as r:
                if r.status_code != 200:
                    r.failure(f"HTTP {r.status_code}")
                    return
                r.success()
                self.subscribe()
                last = time.time()
                for line in r.iter_lines(decode_unicode=True):
                    if not line.startswith("data:"):
                        continue
                    try:
                        if json.loads(line[5:]).get("method") != "notifications/chart_tick":
                            continue
                    except ValueError:
                        continue
                    now = time.time()
                    STREAMS["ticks"] += 1
                    events.request.fire(request_type="PUSH", name="chart_tick gap", response_time=(now - last) * 1000, response_length=len(line), response=None, context={}, exception=None)
                    last = now
            events.request.fire(request_type="PUSH", name="stream closed", response_time=0, response_length=0, response=None, context={}, exception=RuntimeError("server closed the stream"))
        except gevent.GreenletExit:
            raise
        except Exception as e:  # noqa: BLE001
            events.request.fire(request_type="PUSH", name="stream closed", response_time=0, response_length=0, response=None, context={}, exception=e)
        finally:
            self.sub = None  # polling resumes, as the app's watchdog does

    def subscribe(self):
        p = self.call(
            "subscribe_chart",
            {"ticker": self.symbol, "timeframe": self.interval, "indicators": self.indicators, "chart_style": "candles", "warmup": WARMUP},
            "subscribe_chart", ok_errors=PUSH_FALLBACKS,
        )
        self.sub = p.get("subscription_id") if p and "_error" not in p else None

    def unsubscribe(self):
        sub, self.sub = self.sub, None
        if sub:
            self.call("unsubscribe_chart", {"subscription_id": sub}, "unsubscribe_chart")

    # -- foreground tasks (weights are set in _setup)
    @task
    def switch_chart(self):
        """Pick another symbol (3 in 4) or another interval."""
        self.unsubscribe()
        if random.random() < 0.75:
            self.symbol = self.pick_symbol(avoid=self.symbol)
        else:
            self.interval = random.choice([i for i in self.intervals if i != self.interval] or self.intervals)
        self.load_chart("switch")
        if self.stream:
            self.subscribe()

    @task
    def scroll_back(self):
        """Page back through history, one or two pages at a time."""
        for _ in range(random.randint(1, 2)):
            if not (self.oldest and self.has_more):
                return
            args = self.chart_args(bars=PAGE_BARS, warmup=WARMUP, end_time=int(self.oldest * 1000))
            p = self.call("generate_chart", args, "generate_chart page")
            bars = (p or {}).get("bars") or []
            if not bars:
                return
            self.oldest = bars[0]["t"]
            self.has_more = bool(p.get("has_more"))
            gevent.sleep(random.uniform(0.5, 2))

    @task
    def search(self):
        q = random.choice(["btc", "eth", "sol", "aapl", "spy", "nvda"])
        self.call("search_symbols", {"query": q, "limit": 30}, "search_symbols")

    @task
    def open_app(self):
        """A visit with a cold cache: the HTML, then everything it links."""
        r = self.client.get("/", headers={"Accept": "text/html"}, name="app html")
        for path in list(dict.fromkeys(ASSET_RE.findall(r.text if r.ok else "")))[:25]:
            self.client.get(path, name="app asset" if path.startswith("/assets/") else "app favicon")

    @task
    def render_png(self):
        """Rare: CPU-heavy server-side rendering (what an assistant asks for, not the web app)."""
        args = {"ticker": self.symbol, "timeframe": self.interval, "format": "png", "bars": 200, "width": 1280, "height": 720, "indicators": self.indicators}
        self.call("generate_chart", args, "generate_chart png", parse=False)


class HeavyUser(WebUser):
    """A user on the heavy indicators; dropped at startup unless the server lists them."""

    weight = 1

    def pick_indicators(self):
        return [random.choice([n for n in HEAVY_INDICATORS if n in AVAILABLE]), "rsi"]


# --- Summary -----------------------------------------------------------------------------------


@events.quitting.add_listener
def _summary(environment, **kw):
    if isinstance(environment.runner, MasterRunner):
        return
    o = environment.parsed_options
    total = environment.stats.total
    http_total = sum(e.num_requests for k, e in environment.stats.entries.items() if k[1] != "PUSH")
    http_fail = sum(e.num_failures for k, e in environment.stats.entries.items() if k[1] != "PUSH")
    ratio = http_fail / http_total if http_total else 0
    if environment.process_exit_code is None:
        environment.process_exit_code = 1 if ratio > o.max_error_rate else 0
    rows = sorted(environment.stats.entries.values(), key=lambda e: e.name)
    w = max([len(e.name) for e in rows] + [10])
    out = ["", f"== chartgen load test: profile {o.load_profile}, {environment.host}, {total.num_requests} events, {time.time() - (total.start_time or time.time()):.0f} s =="]
    out.append(f"{'name':<{w}} {'n':>7} {'fail':>6} {'p50':>7} {'p95':>7} {'p99':>7} {'max':>7}   (ms)")
    for e in rows:
        out.append(
            f"{e.name:<{w}} {e.num_requests:>7} {e.num_failures:>6} {e.get_response_time_percentile(0.5) or 0:>7.0f} "
            f"{e.get_response_time_percentile(0.95) or 0:>7.0f} {e.get_response_time_percentile(0.99) or 0:>7.0f} {e.max_response_time:>7.0f}"
        )
    out.append(f"requests {http_total}, failed {http_fail} ({ratio:.2%}); push ticks {STREAMS['ticks']}; PUSH rows are gaps between ticks, not request times")
    out.append(f"stop condition: {WATCH.reason or 'none'}; exit code {environment.process_exit_code}")
    print("\n".join(out), file=sys.stderr)
