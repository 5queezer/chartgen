---
title: "0041 — Script editor in the web app"
description: "A CodeMirror 6 panel, loaded on first open, for Starlark indicators on servers that list the scripting tools; a get_my_indicator tool returns the stored source; create_only protects saved scripts; a name the server cannot run fails only its key."
sidebar:
  order: 41
---

**Status:** Accepted
**Date:** 2026-11-06
**Amends:** [ADR-0025](/chartgen/decisions/0025-user-indicator-registry/) (adds `get_my_indicator` and `create_only`; unknown names and bad script params fail per key; charts with scripts are not pushable)

## Context and Problem Statement

[ADR-0025](/chartgen/decisions/0025-user-indicator-registry/) lets a user
register Starlark indicators over MCP, behind `CHARTGEN_SCRIPTING=1`. The web
app can chart them but cannot write them. Someone running with scripting on
needs a Pine-editor-like panel, and everyone else must not see it or pay for
it. Four things had to be decided: how the app knows scripting is on, how it
reads a saved script back, which editor to use, and where unsaved text lives.

## Decision Drivers

- With scripting off: no trace in the UI, no failing request, no download.
- The server stays the only judge of a script and the only owner of the list.
- The main bundle does not grow by an editor.
- Unsaved work survives an accidental close.
- Script source and server error text are never rendered as HTML.

## Considered Options

**Detection.**

1. **The names in `tools/list`.** Already answered by every server, always 200.
2. Call `list_my_indicators` and treat "Unknown tool" as off: a failing request
   on every boot of every normal user.
3. A flag in the health response or a build setting: a second source of truth
   for what `tools/list` already says.

**Reading a script back.** `list_my_indicators` returns metadata, no source.

1. **A new `get_my_indicator(name)` tool**, scoped to the caller like the others.
2. Add `source` to `list_my_indicators`: up to 50 scripts of 64 KiB each on every
   list, for one script's text.
3. Keep source only in the browser: a script registered elsewhere, or from
   another machine with the same client, could not be opened.

**Editor.**

1. **CodeMirror 6, from its packages** (`view`, `state`, `commands`, `language`,
   `lang-python`), loaded on first open.
2. A plain `<textarea>` with a line gutter. It has no highlighting without a
   second overlay kept in sync by hand, and inserting indentation by script goes
   around the browser's undo stack, so Tab and undo do not work together.
3. Monaco: a multi-megabyte download and worker setup for features nobody asked
   for here.
4. Prism-based mini editors: smaller, but they still overlay a textarea, with
   its limits on long files, on touch and on marking lines.

**Unsaved text.**

1. **A draft per script in localStorage**, restored without asking.
2. A "discard changes?" prompt on close: it interrupts, and does nothing for a
   crash or a closed tab.

## Decision Outcome

Option 1 of each.

**Detection.** The app reads `tools/list` once after sign-in. Only when every tool
the editor calls is listed does it render the toolbar button, the entry in the
indicator picker and the dock. A failed `tools/list` is tried twice more, then reads as "not listed" until the next reload.

**`get_my_indicator(name)`** answers `{name, source, updated_at}` for the caller's
own script. For none it answers `{ok: false, error: "not_found"}`, told apart from a
failed read, because only the first means the script is gone. It is listed and
served only with `CHARTGEN_SCRIPTING=1`, like the rest of the family.

**Editor and bundle.** `ScriptEditor.tsx` and CodeMirror sit behind `lazy()`, so
they load when the panel first opens:

| | JS | gzip |
|---|---|---|
| Lazy chunk (panel + CodeMirror) | 375 kB | 128 kB |
| Added to the main chunk | 8 kB | 2 kB |

Almost all of the chunk is CodeMirror's `view` and `state`; `lang-python` and the
`autocomplete` code it brings along are a small part, so dropping them would not
change the picture. Starlark is highlighted with the Python grammar. Colors come
from CSS variables, so they follow the theme. The app has no CSP today; if one
is added, CodeMirror's injected styles need `style-src` to allow them.

**Saving.** Save calls `register_indicator`, which validates. A refusal is a
result, not an exception: its errors are listed under the code and their lines
marked. `isError` text is an `McpToolError`, which no longer counts as an auth
failure, because a script error on line 401 used to clear the token and restart
sign-in. A new script saves with `create_only`, enforced by the store's insert,
because a client's list can always be stale; Save also waits for the first list.
A saved script's name cannot be edited.

**Other writers.** A draft stores the saved text it started from, compared as text
because `updated_at` has a resolution of one second. If the server's text differs,
on reopening or in a read just before saving, the panel asks which version to keep.
That read leaves a small window: the tool has no compare-and-set.

**Updating the chart.** Saving invalidates the `generate_chart` queries whose
indicators include the script: the visible chart reloads, other cached ones refetch
when next shown, and a chart without the script keeps its view. Deleting removes
the script's instances from the chart unless a built-in of that name is back. What a
saved script reports on the chart (`indicator_errors`) shows in the panel while its
text is unchanged, and on the indicator's chip.

**What the server cannot run fails per key.** A chart may name a script the server
no longer has: deleted from another client, held by a saved layout, or saved under
an OAuth client since replaced (ADR-0036). A name nobody knows is indistinguishable
from a mistyped one, so for an authenticated client with scripting on, every
unknown name is reported under `indicator_errors` with `step: "unknown"`, and the
same goes for params a script's new schema refuses (`step: "params"`). Without a user
the old behaviour stays: an unknown name fails the request. So does one on a server
with scripting turned off, where the load error names it. This keeps share links as
they were, since they are checked against `list_indicators` first.

**Push.** Pushed tails are computed without a user, so a script would be missing or,
worse, replaced by the built-in it shadows. `subscribe_chart` therefore resolves names
for the caller and answers `not pushable` when a script is involved, and such a chart
polls. User-aware tails remain a possible later step.

**Windows that disagree.** The same script can fail on a full load and work on a
tail, or the reverse, because they differ in bar count and time budget. The shape
check skips keys listed under `indicator_errors`, an indicator that failed on one
side stays aligned with empty bars, and tail errors merge into the series. As a
reload starts a new generation, which resets the poll backoff, reloads are also
capped per chart: 3 in 5 minutes.

**Layout.** A dock below the bottom bar shrinks the chart, so the chart stays
visible while editing, at up to 45% of the window. Below 768 px of width or 720 px
of height there is no room for both, and plain CSS makes it a full-screen panel.

### Consequences

- Good: normal users download 2 kB more and make one successful extra request.
- Good: the editor needs no server support beyond one read tool.
- Good: drafts survive a closed tab, a crash and a reload.
- Good: a script gone from the server costs a red chip, not a blank chart.
- Bad: for such a client a mistyped indicator name no longer fails the request; the
  caller must read `indicator_errors`.
- Bad: drafts live in one browser, and two tabs editing the same script share one
  draft, last writer wins.
- Bad: a chart with a script has no sub-second updates.
- Bad: the server reports a line but no column, so errors have no column.
- Bad: testing the panel needs a second server and dev server, because scripting
  is fixed at server start. Every e2e run starts and warms them too.
- Neutral: the security position of [ADR-0024](/chartgen/decisions/0024-starlark-script-runtime/)
  and ADR-0025 is unchanged: scripting remains for trusted or single-user
  deployments, and an editor does not make it safer to enable.
