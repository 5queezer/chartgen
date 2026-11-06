---
title: "0041 — Script editor in the web app"
description: "A CodeMirror 6 panel, loaded on first open, for Starlark indicators on servers that list the scripting tools; a get_my_indicator tool returns the stored source; drafts stay in localStorage."
sidebar:
  order: 41
---

**Status:** Accepted
**Date:** 2026-11-06
**Amends:** [ADR-0025](/chartgen/decisions/0025-user-indicator-registry/) (adds `get_my_indicator`)

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
indicator picker and the dock. A failed `tools/list` reads as "not listed".

**`get_my_indicator(name)`** answers `{name, source, updated_at}` for the caller's
own script and otherwise the same error as `delete_indicator`. It is listed and
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
sign-in. A new script may not take the name of a saved one, because
`register_indicator` replaces silently. A saved script's name cannot be edited.

**Updating the chart.** Saving invalidates the `generate_chart` queries whose
indicators include the script: the visible chart reloads, other cached ones refetch
when next shown, and a chart without the script keeps its view. Deleting removes
the script's instances from the chart unless a built-in of that name is back,
since an unknown name fails the whole chart request. What a saved script reports on
the chart (`indicator_errors`) shows in the panel while its text is unchanged.

**Layout.** A dock below the bottom bar shrinks the chart, so the chart stays
visible while editing; below 768 px, plain CSS turns it into a full-screen panel.

### Consequences

- Good: normal users download 2 kB more and make one successful extra request.
- Good: the editor needs no server support beyond one read tool.
- Good: drafts survive a closed tab, a crash and a reload.
- Bad: drafts live in one browser, and a draft holding text for a script deleted
  elsewhere is only offered when that script was the open one.
- Bad: the server reports a line but no column, so errors have no column.
- Bad: testing the panel needs a second server and dev server, because scripting
  is fixed at server start. Every e2e run starts and warms them too.
- Neutral: the security position of [ADR-0024](/chartgen/decisions/0024-starlark-script-runtime/)
  and ADR-0025 is unchanged: scripting remains for trusted or single-user
  deployments, and an editor does not make it safer to enable.
