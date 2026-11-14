---
title: "0071 — A Setups panel in the web app"
description: "The web app's sidebar has a Setups tab: the signed-in owner's setups, read through the existing tools with the app's own token, open ones first, with an open/closed filter and the track record's totals for takes and skips. A click opens the setup's chart with its drawings, and a skip's bracket as a position. A win below 0 R keeps its status and gets a hint on its row."
sidebar:
  order: 71
---

**Status:** Proposed
**Date:** 2026-11-14
**Amends:** [ADR-0059](/chartgen/decisions/0059-setups-paper-scored/) (the web app shows setups)

## Context and Problem Statement

Setups ([ADR-0059](/chartgen/decisions/0059-setups-paper-scored/)) appear nowhere in the web app.
The person sees a setup only through the `open_url` links Hermes builds from `generate_chart`, and
learns its status and R only by asking Hermes. They asked, through Hermes, for a panel like the
Alerts panel.

One case confuses a list. A skip ("TRX 5b2b25f1") reads `won` with a negative R. Its fill came
at the open, past its target, so reaching the target still left less than the costs. The status
says the first target was reached, which is what `won` means for the agent.

## Decision Drivers

- Only the signed-in owner's setups, through the same authenticated path as every other tool:
  no new endpoint.
- No second scorer: what the server scores is shown as it is, totals included.
- No change to the status values or the tool contract, which Hermes relies on.
- The wire contract rules: generated input types and Zod response schemas.

## Decision Outcome

- **Where.** A fifth sidebar tab, **Setups**, between Alerts and Data Window. On a phone it is a
  tab of the sheet, as the others are.
- **Data.** The tab calls the existing tools with the app's MCP session and bearer token, so the
  server answers with the caller's own setups only:
  - `list_setups` (limit 500) for the list;
  - `get_track_record` with `decision: take`, and again with `skip`, for the totals;
  - `get_setup` when a row is clicked.
  - Polling: as soon as the app is signed in with the tab open (also after a reload with the
    tab remembered), then every 60 s while it shows, since setups move on closed 1m bars.
    Nothing is kept in storage.
  - On a server without setups, `list_setups` answers `enabled: false` and the tab says so.
  - No tool schema changes. `mcp.ts` gains three calls, validated with the existing Zod schemas.
- **Sort and filter.**
  - Open setups (watching, triggered) first, newest first. Then closed ones, the most recently
    ended first: by exit, else by the last bar scored.
  - The filter is Open, Closed or All, with counts. Open is the default, and the choice is
    remembered.
- **A row** shows:
  - first line: symbol, interval, direction, `skip` for a skip, status, and R once closed;
  - then: the entry zone (or the fill once filled), the stop, T1 and the expiry (in the chart's
    time zone, without the year when it is this year);
  - the tags;
  - the thesis as the row's tooltip;
  - and a note when `gap` is set.
- **Totals.** A small grid with Taken and Skipped rows: closed count, won, total R, mean R and
  open count, exactly as `get_track_record` answers them for each decision. The browser adds
  nothing up.
- **Click.**
  - The chart switches to the setup's symbol and interval.
  - The setup's stored drawings, which `generate_chart` resolved with times and prices, join the
    symbol's drawings as the user's own, as an `open_url` link's do (ADR-0054). They get ids of
    the setup's own, `setup-<id>-<n>`, and a click first removes the symbol's drawings with that
    prefix: a setup that moved on since the last click, filled or revised, is drawn once, as it
    is now. A chart already on the symbol reloads its drawings.
  - **For a skip, the server's bracket is drawn as a `cgLong` or `cgShort` position**, from the
    levels the server scores:
    - its left edge and entry: from the fill, or, unfilled, from when the skip was made, at its
      entry level;
    - its right edge: the expiry while it is open; once closed, its exit, else the last bar
      scored; at least one bar of its interval from the left edge, so a skip stopped in its fill
      bar, or invalidated at its first, is a box of one bar;
    - its target and stop: T1 and the invalidation.
  - A take gets only its own drawings. Its levels are the agent's, and the agent draws what it
    means.
- **A win below 0 R.** The status and the tool contract stay as they are. The row keeps `won` and
  its R, adds a line in the danger color, "⚠ Won below 0 R: the costs were more than the move to
  T1", and a tooltip that says why. Only a `won` row with R below 0 gets it.

### Considered and not chosen

- **A new status, or a field such as `won_net`.** This would change what Hermes reads, or add a
  field it does not need. The record already holds `status` and `result_r`, and the panel can
  say what they mean together.
- **Drawings as a temporary layer** that a click replaces. It would need a new overlay layer and
  its own rules for editing and saving. Joining the symbol's drawings is what an `open_url` link
  already does, and the user can delete them like any other.
- **Summing the totals in the browser** from the list. They would differ from `get_track_record`
  over truncation (500), gaps and setups without a result, and the panel would carry a second
  definition of the track record.

## Consequences

- Good: the person sees every setup, its levels and the track record without asking Hermes, and
  one click shows a setup on its chart.
- Good: the panel holds no scoring logic. It shows what the tools answer.
- Bad: a setup's drawings stay on its symbol after a click, one set per setup opened. Opening
  many setups on one symbol fills its chart; the Object Tree removes them.
- Bad: a skip's position box spans to the expiry while it is open, which on a long expiry is
  past the chart's last bar.
- Bad: three tool calls a minute while the tab is open.

## Testing

- **Unit (`web/src/lib/setups.test.ts`),** on the server's own answers
  (`tests/fixtures/setup_responses.json`):
  - sort and filter;
  - the entry text;
  - R formatting;
  - the hint, only on a win below 0 R;
  - a take's drawings;
  - a skip's bracket, filled and unfilled;
  - a stored drawing the web app does not know is left out.
- **Playwright (`setups-panel.spec.ts`),** on the server-alerts server, which now runs with
  `CHARTGEN_SETUPS=1`:
  - setups made through `/mcp` with the app's token are listed;
  - one made from another browser (another OAuth client, so another owner) is not;
  - the newest come first, with tags, status and the totals of open ones;
  - a click opens ETHUSDT 1h with the stored line, which a second click does not add again;
  - a click on the skip opens BTCUSDT 1h with a `cgShort` at the server's levels, listed in the
    Object Tree;
  - with the server's answers replaced, a win below 0 R shows the hint, the totals are the tool's,
    and the filter works.
- **`setups-panel-off.spec.ts`:** on a server without setups, the tab says so and asks for nothing
  but the list.
