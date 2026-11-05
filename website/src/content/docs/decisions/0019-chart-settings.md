---
title: "0019 — Chart settings: per-theme overrides over the palette"
description: "How the chart settings dialog stores its values and merges them with the theme palette."
sidebar:
  order: 19
---

**Status:** Accepted
**Date:** 2026-11-05
**Amends:** [ADR-0016](./0016-light-theme/) (adds a user layer on top of the palettes; 0016 stays in force)

## Context and Problem Statement

TradingView's chart settings dialog lets users recolor candles, hide the grid,
move the price scale and trim the legends. ADR-0016 fixed two palettes in
`lib/theme.ts` and the canvas styles are built from the active one. Where do
user-chosen values live, how do they interact with the two themes, and what is
persisted?

## Decision Drivers

- The palette stays the single source of truth; "reset" must mean "the palette".
- A color chosen for the dark theme is usually wrong on the light one.
- Stored data is untrusted: validated on load, never trusted into canvas styles.
- Live preview with Cancel, without writing storage on every keystroke.
- The data-flow code in `Chart.tsx` should not learn about the dialog.

## Considered Options

1. A full copy of the palette per theme, edited in place.
2. Sparse per-theme overrides over the palette, plus theme-independent options.
3. One global set of overrides for both themes.

## Decision Outcome

Chosen option: 2.

Persisted under `chartgen.chartSettings`:

```json
{
  "version": 1,
  "themes": {
    "dark": { "bodyUp": "#ffeb3b", "wickOn": false },
    "light": {}
  },
  "options": {
    "gridVertical": true, "gridHorizontal": true,
    "crosshairStyle": "dashed", "scalePlacement": "right",
    "lastPriceLine": true, "lastPriceLabel": true, "highLowMarks": false,
    "legendOhlc": true, "legendChange": true,
    "legendVolume": true, "legendIndicators": true
  }
}
```

- `themes.<name>` is sparse. Color keys (`bodyUp`, `bodyDown`, `borderUp`,
  `borderDown`, `wickUp`, `wickDown`, `line`, `background`, `grid`,
  `crosshair`) hold a color the store accepts (`#rgb`, `#rrggbb`,
  `#rrggbbaa`, `rgba()`); `bodyOn`, `borderOn` and `wickOn` are booleans.
  A missing key means "use the palette", so a palette tweak still reaches
  every user who did not override that value, and resetting a value is
  deleting its key. Border and wick colors default to the effective body color.
- `options` is complete and shared by both themes: booleans, the crosshair
  style and the price scale placement.
- `stores/chartSettings.ts` validates everything on load and on every write;
  unknown keys, wrong types and invalid colors fall back to the defaults.
  `previewChartSettings` updates the live signal only, `saveChartSettings`
  also persists, so the dialog previews edits and Cancel restores the value
  that was live when it opened.
- `lib/chartStyle.ts` is a pure function from palette, overrides, options and
  chart type to a klinecharts styles object. `Chart.tsx` applies it with
  `setStyles` right after the palette styles, relying on klinecharts' deep
  merge, in the same effect that handles theme and chart type.
- The background is not a klinecharts style (the canvas is transparent); it
  is set on the chart host element and used for screenshots.
- Settings are not part of named layouts (ADR-0015): they describe the
  viewer, not the analysis, and a layout switch should not recolor the chart.

### Consequences

- Good: "Reset settings" clears the active theme's overrides and restores the
  default options; there is no second copy of the palette to keep in sync.
- Good: switching theme switches the overrides with it.
- Good: the merge is a pure function with unit tests.
- Bad: a user override hides later palette improvements for that value.
- Bad: options such as the grid are shared, so turning the grid off in dark
  also turns it off in light.
- Bad: indicator colors, drawings and server-sent colors are not covered.
