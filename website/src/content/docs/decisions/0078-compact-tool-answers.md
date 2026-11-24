---
title: "0078 — Compact tool answers"
description: "list_setups takes fields, so an agent reads a few fields of many setups instead of whole setups, and adds up the risk of the open takes in open_risk. Tool answers are written as compact JSON instead of indented JSON. list_indicators takes detail: compact for names and parameter defaults. Every default answer keeps its data, so existing clients are unaffected."
sidebar:
  order: 78
---

**Status:** Accepted
**Date:** 2026-11-24
**Amends:** [ADR-0059](/chartgen/decisions/0059-setups-paper-scored/) (`list_setups` takes
`fields` and answers `open_risk`)

## Context and Problem Statement

An agent client that trades on paper decides with a small budget of tool calls, and every
character of an answer is a token it pays for. Three answers cost far more than what it reads
from them:

- **`list_setups`** returns each setup whole: thesis, drawings, features, the three shadow
  results, excursions and durations, a few KB each. An agent that looks over its recent setups
  before a decision wants their id, status, result, tags and date. Measured on a client's own
  history, its answers averaged 11.5k characters and reached 46k; 100 setups are over 200 KB.
- **Every tool answer** is indented JSON (`to_string_pretty`). The indentation carries no data;
  in a list of setups it is about a third of the text.
- **`list_indicators`** is about 59 KB: every indicator with a description, its category, its
  aliases and every parameter's type, range, enum and description. To call `get_indicators` an
  agent needs the names and the parameters with their defaults.

The same agent adds up by hand how much risk its open takes carry per direction, to keep
correlated positions (several shorts on symbols that move together) under its own limit. The
server holds every number for that.

The web app reads `list_setups` (the Setups panel shows each setup's levels, tags, thesis as a
tooltip, status and result) and `list_indicators`. The agent client's instructions cannot be
changed together with the server, and a merge deploys at once, so an answer it relies on must
not lose data without the client asking for it.

## Decision Drivers

- An agent must be able to get what it needs for far fewer tokens.
- A client that sends what it sends today receives the same data: no field disappears from a
  default answer.
- The web app keeps working without a change of its own.
- One way to ask, in the tool's own schema, that an agent finds from `tools/list`.
- The server reports risk; whether it is too much is the agent's rule, not the server's.

## Considered Options

For `list_setups`:

1. **A `fields` argument**: only the named fields of each setup, and its id.
2. A `summary` mode with a fixed set of fields.
3. Compact by default, with the whole setup left to `get_setup`.

For the text of answers:

1. **Compact JSON everywhere** a tool answers JSON.
2. An argument per tool, or a server setting, to choose indented or compact.
3. Keep indented JSON.

For `list_indicators`:

1. **A `detail` argument**: `full` (default) as today, or `compact`.
2. Compact by default.
3. A `names` filter that returns full entries for a few indicators.

For the open risk: a sum in `list_setups`, or in `get_track_record`.

## Decision Outcome

### `list_setups` takes `fields`

Option 1. `fields` is a list of a setup's fields, the keys `get_setup` answers without `log`,
given as an enum in the tool's schema. Each setup then has those fields and its `id`, which is
always there so that a row can be read again with `get_setup`. A field a setup does not have
(`probability` where none was stated, a stock's `market` on a Binance setup) stays out, as it
does in the whole setup. A name that is not a field, `log` and `owner` among them, is refused
with the list of fields. Without `fields` every setup is whole, exactly as before.

`{"fields": ["status", "result_r", "tags", "created_at"]}` gives an agent its id, status,
result, strategy tag and date. A fixed summary (option 2) would be the right set for one
client and the wrong one for the next. A compact default (option 3) would take the thesis,
levels and tags from every client that does not ask for them, the web app's Setups panel and the
agent's own instructions among them, the moment it is deployed.

### `list_setups` answers `open_risk`

`list_setups` adds up the caller's open takes, whatever the call's filters and limit:

```json
"open_risk": {
  "long":  { "takes": 1, "risk_r": 1, "filled": 1, "filled_at_stop_r": -1.045 },
  "short": { "takes": 3, "risk_r": 3, "filled": 0, "filled_at_stop_r": 0.0 },
  "by_symbol": [ { "symbol": "BTCUSDT", "direction": "long", "takes": 1, ... }, ... ]
}
```

Each open take risks 1 R: R is its own risk from fill to invalidation, and its levels are fixed
from the fill on (ADR-0059), so `risk_r` is the number of open takes, watching or triggered.
`filled_at_stop_r` is what the filled ones would end at together, after costs, if each went out
at its invalidation now: a little below −1 R each, by its own costs. A watching take has no fill
price yet, so it is in `risk_r` and not in `filled_at_stop_r`. Skips are not trades and count
nowhere. `by_symbol` has an entry for each symbol and direction with an open take.

It is in `list_setups`, the answer about what is open; `get_track_record` counts closed setups
only. It reports and the server enforces nothing: a correlation limit is the agent's rule, and
which symbols move together is its judgement.

### Tool answers are compact JSON

Option 1. Every tool answer that is JSON, success or error (`setup_errors`, `drawing_errors`,
a script's errors, `indicator_errors`), is written without indentation. The data is the same;
only the whitespace between tokens goes. Every consumer in this repository parses the text as
JSON (the web app, the load test, the tests and the fixtures written from parsed answers), and
JSON readers ignore that whitespace, so no client reads the layout. Two kinds of test did: a
golden file of `generate_chart`'s summary compares its text byte for byte and is rewritten, and
the tests of the call budget (ADR-0052) rely on an unread answer being large, so they ask for
indicators whose compact answer is still large enough. The budget holds an unread answer at its
size, so the same answers now hold less of it. An argument or a setting
(option 2) would be one more thing to choose for no gain: nobody reads these answers by eye
except through a client that can indent them. Files a person edits, such as the persisted
alerts and the generated drawing-tools document, stay indented.

### `list_indicators` takes `detail`

Option 1. `detail: "compact"` returns each indicator as its name and its parameters with their
defaults, `{"name": "rsi", "params": {"length": 14, ...}}`, the user's scripts included;
`detail: "full"`, the default, returns what it does today. A compact default (option 2) would
take the descriptions, categories and parameter ranges from the web app's indicator picker and
from any agent that reads them. A `names` filter (option 3) does not shrink the call an agent
makes to learn what exists.

### Consequences

- Good: an agent asks for what it reads. The size of each answer, before and after, on a local
  build is in the pull requests that implement this.
- Good: no default answer loses a field, so the deployed agent client and the web app keep
  working unchanged; they gain only `open_risk` and the dropped whitespace.
- Neutral: the text of every JSON answer changes layout. A client that matched the indented text
  instead of parsing it would break; none in this repository does.
- Bad: a compact `list_indicators` drops the parameter ranges and enums; an agent that passes
  a value out of range is refused by `get_indicators` and reads the full entry then.
- Bad: `open_risk` counts each take as 1 R. An agent that sizes positions differently, or counts
  a watching take as no risk, adds up its own way from `takes` and `filled`.

## More Information

- [ADR-0059](/chartgen/decisions/0059-setups-paper-scored/): setups, R and the fixed levels of
  a filled setup.
- [Setups guide](/chartgen/guides/setups/#listing-them) and the
  [MCP reference](/chartgen/reference/mcp/).
