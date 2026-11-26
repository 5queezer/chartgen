---
title: "0091 — A Setups page at /setups, and a Setups tab for the chart's symbol"
description: "The owner's setups get a page of their own at /setups, reached as /admin is: figures, open risk, the open and the closed setups as sortable, filtered tables, the results per day, the track record grouped, and one setup in a drawer with a link to its chart. Every count and sum is the server's, read with the existing tools and the app's own token. The sidebar tab keeps only the setups of the chart's symbol."
sidebar:
  order: 91
---

**Status:** Accepted
**Date:** 2026-11-25
**Supersedes in part:** [ADR-0071](/chartgen/decisions/0071-setups-panel/) (where the whole list,
its filter and the track record's totals are shown; what a click on a setup does is kept)

## Context and Problem Statement

The Setups tab ([ADR-0071](/chartgen/decisions/0071-setups-panel/)) was made for a short list
beside a chart. Since then a setup's record says its result's sign after costs (`net`), the track
record groups by tag prefix, skip reason, playbook version and the skip's bracket anchor
([ADR-0079](/chartgen/decisions/0079-track-record-after-costs/),
[ADR-0087](/chartgen/decisions/0087-a-skips-bracket-from-its-fill/)), and `list_setups` adds up
the open risk by direction and symbol ([ADR-0078](/chartgen/decisions/0078-compact-tool-answers/)).
None of that fits a column a few hundred pixels wide, and the tab could not sort, filter by more
than open or closed, or group at all. The owner reads the record; he does not draw with it.

Where does the record go, what does it show, and what stays beside the chart?

## Decision Drivers

- The owner sees their own setups only, decided by the server as for every other tool call.
- No second computation of the record in the browser: counts and sums are
  `get_track_record`'s.
- No new endpoint and no tool change where the existing tools answer the question.
- A phone loads it: `list_setups` with 500 whole setups is megabytes.
- The app's look, tokens and both themes; no heavy dependency.
- Opening a setup on its chart with its drawings keeps working as it does.

## Considered Options

For where the record lives:

1. A page of its own at `/setups`, reached as `/admin` is.
2. A wider sidebar tab, or a dialog over the chart.
3. Leave it in the tab.

For what stays in the sidebar:

A. The setups of the chart's symbol, open ones and the last few closed, with a link to the page.
B. Nothing: the tab goes.
C. The whole list as before, beside the page.

For a setup's detail:

I. A drawer on the page with what the record says, and a link that opens the setup on the
   app's chart.
II. A second chart embedded in the page.

## Decision Outcome

Chosen: **1, A and I.**

### How the page is reached

Like the operator page ([ADR-0063](/chartgen/decisions/0063-operator-page/)). The server already
answers a browser's request for any path with the app. A tab opened at `/setups` keeps a flag for
the tab (never an address) through the sign-in, which returns to `/`. Then, with a sign-in that
did not fail, it calls `list_setups` with the tab's own token. If the server answers
`enabled: true`, the tab shows the page and the address says `/setups` again. A server without
setups, or a sign-in that failed, leaves the tab as the app at `/`. A server that fails for a
moment shows the page with the error and a Refresh button.

There is no second gate in the browser, because there is nothing for one to protect: the page
calls the tools any MCP client of the owner calls, and the server answers each with the caller's
own setups only. On a server with sign-in, a visitor who is not signed in goes through the
sign-in first, as at `/`. On a server without sign-in, the owner is the browser's own OAuth
client, as for the tab. The sidebar links to the page; the operator page has no link because it is
for one person, and this page is for every owner.

### What the page shows

Its structure and density follow the dashboards of trading bots (open trades, closed trades,
daily results and a breakdown, in panels of a grid). Its look is the app's own. It is read only:
setups are made and changed by the agent's tools.

- **Figures** on top. Open setups, taken and skipped. Open risk long and short, filled, and what
  the filled ones would end at at their stops, from `list_setups`' `open_risk`, over all open takes whatever the filters. Closed setups,
  taken and skipped. For takes and for skips: the total R after costs, how many ended above 0 R
  after costs of how many (`net_gain` of `count`), and the mean R. These figures come from
  `get_track_record` with `decision: take` and `skip`.
- **Filters:** status, take or skip, side, symbol, tag, and a range of days the setups were made
  in. The tables and the chart follow all of them. The figures and the breakdown follow the ones
  `get_track_record` takes (symbol, side, decision, `from`, `to`); status and tag are not among
  them, since the record counts closed setups and groups by tag instead.
- **Open** and **closed** setups as two tables with sortable columns, what a trader scans first
  on the left, so that it shows on a phone before the table scrolls sideways (a fade at the edge
  says it does):
  - closed: symbol and interval, side and `skip`, status, the result (with a warning on a win
    below 0 R and a mark on a result with a gap), then entry (the fill once filled), stop, T1,
    MFE and MAE in R, when it was made and when it ended;
  - open: symbol, side, status, MFE and MAE, when it was made and when it expires, then the
    levels.
- **Results per day:** each closed setup listed, at its own result as the server scored it, on
  the day it ended, as bars, with their running sum as a line. It plots results and is no total:
  it covers the listed setups (the newest 500, and the page says so when there are more), and the
  figures above say what the record counts. Plain SVG: two series in the app's colors need no
  chart library.
- **Open risk by symbol:** `open_risk.by_symbol`, as the server sends it.
- **Track record by** symbol, direction, playbook version, skip reason, tag, feature, probability
  or bracket anchor: one `get_track_record` call with `group_by`, showing per group the closed
  count, total R, mean R, a bar of the total, then won, above and below 0 R after costs, entered,
  mean R entered and gaps, and the total. Tags group by the prefixes the page offers from the listed
  setups: the families (the part of a tag up to its first `:`) or one family's tags, the most
  used first, at most 10, which is what the tool takes. Features group by a name the listed
  setups carry.
- **A setup's drawer.** A click on a row reads the setup whole with `get_setup` and shows its
  thesis, levels, fill and exit, net, MFE and MAE, bars to fill and to exit, the rule that set its
  levels and its anchor, the costs, what the agent stated (playbook version, probability, skip
  reason, tags, features), and its shadow exits. **Open chart** goes to `/?setup=<id>`, where the
  app opens it as the tab does: the setup's symbol at its interval, its drawings joined to the
  symbol's own under ids of the setup's, and a skip's bracket as a position.
- It asks again every minute and on every change of a filter.

### What it reads

- `list_setups` with `limit: 500` and `fields` ([ADR-0078](/chartgen/decisions/0078-compact-tool-answers/)):
  only what the tables, figures and filters show. The thesis, drawings, shadow results and log
  come with `get_setup` for the one setup opened. Measured on a server with made-up setups, see
  the pull request.
- `list_setups` again with `status: open` and the same fields, merged by id: an open take older
  than the newest 500 is in the figures and the open risk, so it is in the open table too.
- `get_track_record` three times: takes, skips, and the chosen grouping.
- A read that fails shows why, with Refresh to try again; the tab is at `/setups` then, and
  leaving it is the app. A later refresh that fails keeps what an earlier one read, and says so.
- No current price: an "R now" for an open setup would score it a second time, and against a
  stop the agent may have moved since the fill, where the server keeps the risk at the fill. MFE
  and MAE are the server's.

No tool and no schema changes. The web app gains one call, `list_setups` with fields, validated
by a Zod schema of those fields picked from the setup's.

### What stays in the sidebar

The tab lists the setups **of the chart's symbol**: its open ones, then the five that ended last,
as rows that look and open as before. It reads `list_setups` for that symbol only, limit 50,
again when the chart switches symbols. Its header links to the page, and so does a line that
says how many more the symbol has. The totals and the Open, Closed and All filter are on the
page now. This is what the tab is for beside a chart: what is set up on this symbol, one click
from its drawings. A second full copy would keep two places to read the record, and no tab at
all would take that click away while drawing.

### Consequences

- Good: the record is read where it fits: sortable, filtered and grouped, with open risk and the
  daily results.
- Good: no new endpoint, and the server still decides whose setups are shown.
- Good: the page loads row fields only; the tab loads one symbol's setups, and no longer calls
  `get_track_record` twice a minute.
- Good: one way to open a setup on the chart, the tab's.
- Bad: the results per day plot at most the newest 500 setups; with more, the line is not the
  record's total, and the page says so.
- Bad: **Open chart** loads the app anew instead of switching in place.
- Bad: the tab no longer shows the totals at a glance beside the chart.
- Neutral: the app's bundle carries the page for everyone; it loads only at `/setups`.

## Confirmation

- **Unit:** the filters and the record's arguments (`web/src/lib/setupsPage.test.ts`), sorting,
  the days plotted, the tag groupings and feature names; reaching the page and taking
  `?setup=` out of the address (`web/src/services/setupsPage.test.ts`); the tab asks for the
  chart's symbol (`web/src/stores/setups.test.ts`).
- **Playwright** (`setups-page.spec.ts`, on the server with setups): the page lists the owner's
  setups and not another browser's, and opens one in the drawer and on its chart with its
  drawing; on answers put in place of the server's, the tables sort, the filters filter, the
  figures are the tool's and the breakdown sends `group_by` and shows the tool's groups.
  `signin-setups.spec.ts` (with sign-in): signed out, `/setups` is the sign-in first, and a
  signed-in owner sees their own setups and not another account's.
  `setups-panel.spec.ts`: the tab lists the chart's symbol's setups only, and links to the page.

## Pros and Cons of the Options

### 1. A page at /setups

- Good: room for tables, a chart and a breakdown, on a phone too.
- Bad: leaving the chart to read the record.

### 2. A wider tab or a dialog

- Bad: covers the chart it sits beside, and a dialog cannot be linked to or reloaded.

### 3. The tab as it is

- Bad: what the record holds now cannot be read there.

### B. No tab

- Bad: opening a setup's drawings while on its chart takes a trip to the page.

### C. The whole list in the tab as well

- Bad: two places that list the record, kept alike by hand.

### II. A chart embedded in the page

- Good: no reload.
- Bad: a second chart with its own data, indicators and drawing store beside the app's, for what
  the app's chart already does.
