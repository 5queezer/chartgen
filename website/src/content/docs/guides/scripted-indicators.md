---
title: Scripted indicators
description: Write your own indicators in Starlark (a Python dialect with Pine-like series math), register them over MCP and chart them like built-ins.
---

chartgen can run indicators you write yourself, the way TradingView runs
Pine Script. Scripts are written in [Starlark](https://github.com/bazelbuild/starlark/blob/master/spec.md),
a small, deterministic dialect of Python. You register a script over MCP
or write it in the web app's
[script editor](/chartgen/guides/web/#script-editor),
and from then on `generate_chart` accepts its name like any built-in. It
is sandboxed: a script cannot read files, use the network or load other
code. Design records: [ADR-0024](/chartgen/decisions/0024-starlark-script-runtime/)
(runtime) and [ADR-0025](/chartgen/decisions/0025-user-indicator-registry/)
(registry and MCP tools).

## Enabling it

:::caution[Trusted or single-user deployments only]
Scripts run inside the server process, and their owner is an OAuth client
registration, not an authenticated person: anyone who learns a client's
`client_id` can act as it, because `/authorize` approves automatically.
Enable scripting only where you trust everyone who can reach the server
(your own machine, a private or single-user deployment). A public,
multi-user host needs real user login and the worker-subprocess sandbox
recorded in [ADR-0024](/chartgen/decisions/0024-starlark-script-runtime/#required-before-public-hosting-a-worker-subprocess)
first.
:::

Scripting is off by default. Start the HTTP server with it on:

```bash
CHARTGEN_SCRIPTING=1 chartgen --serve
```

| Variable | Default | Purpose |
|---|---|---|
| `CHARTGEN_SCRIPTING` | unset (off) | `1` lists and serves the scripting tools, and makes `generate_chart` and `list_indicators` include your scripts. |
| `CHARTGEN_DB` | `~/.chartgen/chartgen.db` | SQLite file for scripts and registered OAuth clients. Created and migrated at startup. |

Scripts belong to the **OAuth client** that registered them, that is, the
`client_id` your MCP client got from `/register`. Another client, even
your own second client, does not see them. The stdio transport (`--mcp`)
has no user: it lists the tools but cannot store scripts, and it charts
built-ins only.

## A first script

```python
PARAMS = [
    {"name": "length", "type": "int", "default": 14, "min": 2, "max": 200, "desc": "RSI length"},
    {"name": "smooth", "type": "int", "default": 5, "min": 1, "max": 50, "desc": "Signal EMA"},
    {"name": "ob", "type": "float", "default": 70.0, "desc": "Overbought"},
    {"name": "os", "type": "float", "default": 30.0, "desc": "Oversold"},
]
META = {"title": "RSI + signal", "overlay": False, "description": "RSI with an EMA signal and band crosses"}

def compute(ohlcv, params, chart):
    r = ta.rsi(ohlcv.close, params["length"])
    sig = ta.ema(r, params["smooth"])
    chart.line(r, title = "RSI", color = "purple")
    chart.line(sig, title = "Signal", color = "orange", width = 1)
    chart.fill(r, sig, color = "green", color_below = "red", alpha = 0.15)
    chart.dot(ta.crossover(r, params["os"]), r, color = "lime", size = 5, label = "rsi_oversold_exit")
    chart.dot(ta.crossunder(r, params["ob"]), r, color = "red", size = 5, label = "rsi_overbought_exit")
    chart.hline(params["ob"], color = "gray")
    chart.hline(params["os"], color = "gray")
    chart.set_y_range(0, 100)
```

Register it, then chart it next to built-ins and override its params like
any other indicator:

```json
{ "name": "register_indicator", "arguments": { "name": "rsi_bands", "source": "PARAMS = [...]..." } }
{ "name": "generate_chart", "arguments": {
    "ticker": "BTCUSDT", "format": "series",
    "indicators": [{ "name": "rsi_bands", "length": 21 }, "macd"] } }
```

`register_indicator` answers with the normalised parameter schema:

```json
{
  "ok": true, "name": "rsi_bands", "title": "RSI + signal", "overlay": false,
  "replaced": false, "shadows_builtin": false,
  "configurable_params": [
    { "name": "length", "type": "integer", "default": 14, "min": 2, "max": 200, "description": "RSI length" },
    ...
  ]
}
```

## Script shape

A script defines three things at the top level:

- **`PARAMS`**: a list of parameter specs. Each entry has `name`,
  `type` (`int`, `float`, `bool` or `string`), a `default` of that type,
  and optional `min`, `max`, `desc`, and `options` (a list of allowed
  strings, for string params). Chart requests are validated against it,
  like the built-ins' parameters. Use `PARAMS = []` for none. A param
  cannot be called `name`, `id`, `hidden` or `style`: those are keys of an
  indicator entry on the wire and in the web app
  ([ADR-0028](/chartgen/decisions/0028-multiple-indicator-instances/)).
- **`META`**: a dict with `title` (the panel label, default: the name),
  `overlay` (`True` draws on the price chart, default `False`) or
  `category` (`"overlay"` / `"panel"`), and an optional `description`.
- **`def compute(ohlcv, params, chart)`**: draws the indicator and returns
  nothing. `params` is a dict of the resolved values: request overrides
  over defaults, with `float` params always floats.

Module-level constants and helper `def`s are allowed. A top-level name
must not shadow a builtin (`ta`, `len`, `range`, ...).

## Series

Every price input and every `ta.*` result is a **series**: one float per
bar, aligned with the chart bars. `nan` is Pine's `na` (warmup, division by
zero).

| Expression | Meaning |
|---|---|
| `ohlcv.open` `.high` `.low` `.close` `.volume` | Price series |
| `ohlcv.time` | Bar open time, unix seconds (`nan` for the synthetic demo data) |
| `ohlcv.hl2` `.hlc3` `.ohlc4` `.hlcc4` | Composite price sources |
| `a + b`, `a - b`, `a * b`, `a / b`, `-a` | Element-wise; either side may be a number (`1 - close`, `100 / close`). `x / 0` is `nan`. |
| `close[1]`, `close[k]` | The series `k` bars ago, as in Pine (`nan` for the first `k` bars) |
| `a > b`, `<`, `>=`, `<=`, `==`, `!=` | A **bool series** (1.0 / 0.0). A comparison with `nan` is false, as in Pine. |
| `c1 & c2`, `c1 \| c2`, `c1 ^ c2`, `~c` | Element-wise and / or / xor / not on bool series |
| `len(s)` | Number of bars |
| `s.get(i)` | The value at bar `i` as a float (negative counts from the end) |
| `s.list()` | All values as a list of floats, for per-bar loops |
| `ta.series([...])` | A series from a list of numbers (`None` is `nan`) |

`and`, `or`, `not` and `if` work on plain values only: a series is always
truthy, so `if close > open:` does not test each bar. Use a bool series
with `chart.dot(cond, ...)`, or loop over `.list()`:

```python
def compute(ohlcv, params, chart):
    out = []
    streak = 0
    closes = ohlcv.close.list()
    for i in range(len(closes)):
        if i > 0 and closes[i] > closes[i - 1]:
            streak += 1
        else:
            streak = 0
        out.append(streak)
    chart.bars(ta.series(out), color = "teal")
```

Loops run in the interpreter and are slower than `ta.*`, which is native,
vectorized code. Prefer series math where you can.

## `ta.*` functions

They use the same Pine v5 code as the built-ins
([ADR-0018](/chartgen/decisions/0018-pine-semantics-for-indicators/)):
SMA-seeded EMA/RMA, warmup `nan`, and windows that skip `nan`. Lengths
must be between 1 and 10 000.

| Function | Pine | Notes |
|---|---|---|
| `ta.sma(src, length)` | `ta.sma` | |
| `ta.ema(src, length)` | `ta.ema` | Seeded with the SMA of the first `length` values |
| `ta.wma(src, length)` | `ta.wma` | |
| `ta.rma(src, length)` | `ta.rma` | Wilder's smoothing |
| `ta.rsi(src, length)` | `ta.rsi` | |
| `ta.stoch(src, high, low, length)` | `ta.stoch` | Zero range is `nan` |
| `ta.tr(ohlcv)` | `ta.tr(true)` | True range of the chart bars |
| `ta.atr(ohlcv, length)` | `ta.atr` | |
| `ta.macd(src, fast=12, slow=26, signal=9)` | `ta.macd` | Returns `(macd, signal, hist)` |
| `ta.highest(src, length)`, `ta.lowest(src, length)` | `ta.highest`, `ta.lowest` | |
| `ta.stdev(src, length)` | `ta.stdev` | Population (biased) |
| `ta.change(src, length=1)` | `ta.change` | `src - src[length]` |
| `ta.roc(src, length)` | `ta.roc` | Percent |
| `ta.crossover(a, b)`, `ta.crossunder(a, b)` | `ta.crossover`, `ta.crossunder` | Bool series; `b` may be a number |
| `ta.pivot_high(src, left, right)`, `ta.pivot_low(...)` | `ta.pivothigh`, `ta.pivotlow` | Value on the confirming bar, `nan` elsewhere. Tie rule unverified against TradingView. |
| `ta.nz(src, replacement=0)`, `ta.na(src)`, `ta.fixnan(src)` | `nz`, `na`, `fixnan` | |
| `ta.add/sub/mul/div(a, b)` and `*_scalar` | — | Function forms of the operators |
| `ta.max(a, b)`, `ta.min(a, b)`, `ta.pow(a, b)` | `math.max`, `math.min`, `math.pow` | `nan` if either side is `nan` |
| `ta.abs(x)`, `ta.sqrt(x)`, `ta.log(x)`, `ta.exp(x)` | `math.*` | Work on series and numbers |
| `ta.hl2(ohlcv)`, `ta.hlc3(ohlcv)`, `ta.ohlc4(ohlcv)` | — | Same as `ohlcv.hlc3` etc. |
| `ta.nan` | `na` | A float `nan` |

## Drawing: the `chart` object

Each method adds one element to the indicator's output (the same
`PanelResult` the built-ins produce), so PNG, `summary` and `series` all
show it. Series arguments may also be plain numbers (a constant line).

| Method | Draws |
|---|---|
| `chart.line(s, title="", color="#2962ff", width=2, alpha=0.9)` | A line |
| `chart.fill(a, b, color="#2962ff", alpha=0.2, color_below=None)` | A shaded band; `color_below` applies where `a < b` |
| `chart.bars(s, color="#26a69a", colors=None, alpha=0.9, bottom=0.0)` | A histogram; `colors` is one color per bar |
| `chart.dot(x, y, color="#2962ff", size=4, label=None, alpha=1.0)` | A dot at bar `x` (int) and value `y`, or, with `x` a bool series, a dot on every true bar at `y` (series or number). `label` is a signal name. |
| `chart.hline(y, color="#787b86", alpha=0.5)` | A horizontal reference line |
| `chart.hbar(y, height, width, offset=0.0, color="#2962ff", alpha=0.5, left=False)` | A horizontal bar at a price (volume-profile style); `width` and `offset` are fractions of the chart width |
| `chart.divline(x1, y1, x2, y2, color="#787b86", alpha=0.9, dashed=False)` | A straight segment between two bars |
| `chart.set_y_range(lo, hi)` | Fixed y-axis range |
| `chart.set_label(text)` | Panel label (default `META["title"]`) |

Colors are `"#rrggbb"`, `"#rrggbbaa"` (its alpha overrides `alpha`) or a
Pine color name: `aqua`, `black`, `blue`, `fuchsia`, `gray`, `green`,
`lime`, `maroon`, `navy`, `olive`, `orange`, `purple`, `red`, `silver`,
`teal`, `white`, `yellow`.

## Forking a built-in

`fork_indicator` returns the Starlark source of a built-in that has a
reference implementation. Edit it and register it. Keep the built-in's
name to replace it for your account only (other users still get the
built-in), or pick a new name.

```json
{ "name": "fork_indicator", "arguments": { "builtin_name": "wavetrend" } }
→ { "name": "wavetrend", "source": "PARAMS = [...] ...", "configurable_params": [...] }
```

Forkable today: `wavetrend`. Its source,
`src/scripting/refimpl/wavetrend.star`, is tested to match the Rust
indicator within 1e-9. It is a good example of Pine-style series code:

```python
ap = ohlcv.hlc3
esa = ta.ema(ap, params["ch_len"])
d = ta.ema(ta.abs(ap - esa), params["ch_len"])
ci = (ap - esa) / (0.015 * d)
wt1 = ta.ema(ci, params["avg_len"])
wt2 = ta.sma(wt1, 3)
up = (wt1[1] < wt2[1]) & (wt1 >= wt2)
chart.dot(up & (wt2 <= -53), wt2, color = "#3fff00", size = 6, label = "cross_up_os")
```

## Errors

`register_indicator` checks a script in four steps and stops at the first
failure. The error comes back with `isError: true`:

```json
{ "ok": false, "errors": [{ "step": "dry_run", "line": 4, "message": "..." }] }
```

| `step` | Checks |
|---|---|
| `parse` | Syntax; source size (64 KiB) |
| `static` | No `load()`; no shadowed builtins; no banned methods; every name defined; the top level runs |
| `presence` | `PARAMS`, `META` and `compute` exist and are well-formed |
| `dry_run` | `compute` runs on 100 synthetic bars, with defaults, within 200 ms |

A script that passes registration can still fail on a real chart, for
example on more bars or other params. Then only that indicator fails: it
is left out of `indicators`, the rest of the chart renders, and the
response carries
`indicator_errors: {"<name>": {"step": "runtime", "line": 7, "message": "..."}}`.

For an authenticated client with scripting on, two more cases fail only their
indicator, with `line: null`: a name that is neither your script nor a built-in
(`"step": "unknown"`: it may be a script deleted since, or saved under an
earlier client), and params your script no longer accepts (`"step": "params"`).
Bad params of a built-in still fail the whole request, and so does an unknown
name on the stdio server or with scripting off.

A chart that uses your scripts is never pushed: `subscribe_chart` answers
`not pushable`, and clients poll `generate_chart`, which runs your scripts.
See the [MCP reference](/chartgen/reference/mcp/#scripted-indicators-chartgen_scripting1).

## Limits

Each `compute` call runs under these limits; a detected
violation returns an indicator error. Limits are best effort, not a
guarantee against crashes (see [ADR-0024](/chartgen/decisions/0024-starlark-script-runtime/)):

| Limit | Value |
|---|---|
| Wall clock | 1 s per chart request, shared by all its scripts (200 ms for the register dry run) |
| Scripted indicators per chart | 3 |
| Steps (function calls + loop iterations) | 20 million |
| Call depth (recursion) | 64 |
| Script heap | 64 MiB |
| Memory held by one evaluation (heap, dicts, series, strings) | 256 MiB |
| One string, list, tuple, dict or `range` | 20 000 elements |
| A series | exactly one value per bar (`ta.series` included) |
| Integers | 64-bit; overflow and larger literals are errors |
| Top-level values (module constants) | 1 MiB |
| Concurrent evaluations | 4 on the server, 1 per user (others wait, then "server busy") |
| Plot objects | 64 lines, fills, bar series or hlines; 10 000 dots, hbars or divlines |
| Scripts per user | 50; names `^[a-z][a-z0-9_]{0,47}$` |

Not available: `load`, `print`, file or network access, `getattr`, and
the methods `.join`, `.format`, `.replace`, `.extend`, `.update` and
`.setdefault`, whose output can outgrow their input many times over in one
step. Build strings with `+` or `%`, lists with `+` or `.append`, and dicts
with `d[k] = v`. Augmented assignment (`+=` etc.) needs a plain name:
write `x[i] = x[i] + 1`, and assign list or dict items one at a time.

`if`, `and`, `or`, `not`, `x if c else y` and comprehension filters are
rejected when applied directly to a series (`if ohlcv.close > ohlcv.open:`),
because a series is always truthy. The check only sees expressions it can
recognise as series; one stored in a variable first is not detected.

The limits make runaway scripts fail cleanly. They are not a security
boundary against deliberately hostile code
([ADR-0024](/chartgen/decisions/0024-starlark-script-runtime/#sandbox)), so
even a trusted deployment should run chartgen under an OS memory limit
(cgroup `MemoryMax`, a container limit).

## Managing scripts

| Tool | Does |
|---|---|
| `register_indicator(name, source)` | Validate and store, or replace a script with the same name |
| `list_my_indicators()` | Your scripts with params and timestamps |
| `get_my_indicator(name)` | The stored source of one of your scripts |
| `delete_indicator(name)` | Remove one; a built-in it shadowed is visible again |
| `fork_indicator(builtin_name)` | The source of a forkable built-in |

`list_indicators` lists your scripts next to the built-ins, tagged
`"source": "user"`.
