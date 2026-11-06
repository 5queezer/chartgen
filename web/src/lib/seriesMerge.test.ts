import { describe, expect, it } from "vitest";

import type { SeriesPayload } from "../types/responses";
import { barColor } from "./plotStyle";
import {
  dedupeDivlines,
  mergeOlder,
  mergeTail,
  missingBars,
  sortChannels,
} from "./seriesMerge";

// A hand-made stand-in for the server's sample source: bar i opens at
// BASE + i*STEP and every closed bar is a pure function of i, so
// overlapping requests agree, as ADR-0009 promises. `now` is the index of
// the forming bar, whose close (and the indicator values on it) depend on
// `tick` as the wall clock does on the server.

const BASE = 1_700_000_000;
const STEP = 60;
const tOf = (i: number) => BASE + i * STEP;
const close = (i: number) => 100 + Math.sin(i / 5) * 10;

interface Req {
  /** Number of bars returned. */
  bars: number;
  /** Exclusive upper bound on bar index (`end_time`); default: up to now. */
  end?: number;
  /** Index of the forming bar. */
  now: number;
  /** Wall-clock wobble of the forming bar. */
  tick?: number;
  has_more?: boolean;
}

function serve({ bars, end, now, tick = 0, has_more = true }: Req): SeriesPayload {
  const last = Math.min(end === undefined ? now : end - 1, now);
  const first = Math.max(0, last - bars + 1);
  const idx = Array.from({ length: last - first + 1 }, (_, k) => first + k);
  const c = (i: number) => close(i) + (i === now ? tick : 0);
  const at = (i: number) => idx.indexOf(i);

  // Divergence lines join bar i and i+5 for i ≡ 3 (mod 10). The server
  // only sees bars up to `last`; lines starting before the window are
  // clipped at its first bar, lines ending before it are dropped.
  const divlines = [];
  for (let i = 3; i + 5 <= last; i += 10) {
    const j = i + 5;
    if (j < first) continue;
    const x1 = Math.max(i, first);
    const y1 = c(i) + ((c(j) - c(i)) * (x1 - i)) / (j - i);
    divlines.push({
      x1: at(x1),
      t1: tOf(x1),
      y1,
      x2: at(j),
      t2: tOf(j),
      y2: c(j),
      dashed: i % 20 === 3,
      color: "#abcdef",
    });
  }
  const closes = idx.map(c);
  const mean = closes.reduce((a, b) => a + b, 0) / Math.max(1, closes.length);

  return {
    symbol: "TESTUSDT",
    interval: "1m",
    has_more,
    bars: idx.map((i) => ({
      t: tOf(i),
      o: close(i - 1),
      h: Math.max(close(i - 1), c(i)) + 1,
      l: Math.min(close(i - 1), c(i)) - 1,
      c: c(i),
      v: 1000 + i,
    })),
    indicators: {
      sma: {
        label: "SMA",
        is_overlay: true,
        lines: [{ index: 0, label: "SMA", values: closes.map((v) => v - 1) }],
      },
      macd: {
        label: "MACD",
        is_overlay: false,
        y_range: [null, null],
        hlines: [{ y: 0 }],
        lines: [
          { index: 0, label: "MACD", values: closes.map((v) => v / 10) },
          { index: 1, label: "Signal", values: idx.map((i) => (i % 2 ? null : i)) },
        ],
        histogram: [{ index: 0, values: closes.map((v) => v - 100) }],
        signals: idx
          .filter((i) => i % 7 === 0)
          .map((i) => ({ x: at(i), t: tOf(i), y: c(i), label: "buy" })),
        fills: [
          {
            y1: closes.map((v) => v + 2),
            y2: closes.map((v) => v - 2),
            color: "#00ff00",
            color_below: "#ff0000",
          },
        ],
        divlines,
      },
      // A windowed profile: hbars/hlines depend on the bars in the response.
      vpvr: {
        label: "VPVR",
        is_overlay: true,
        hbars: [{ y: mean, height: 1, width: 0.5, offset: 0, left: false }],
        hlines: [{ y: mean }, { y: Math.max(...closes) }, { y: Math.min(...closes) }],
      },
    },
  };
}

const clone = <T>(v: T): T => structuredClone(v);

function older(cur: SeriesPayload, page: SeriesPayload): SeriesPayload {
  const r = mergeOlder(cur, page);
  if (!r.ok) throw new Error(`mergeOlder rejected: ${r.reason}`);
  return r.series;
}

function tail(cur: SeriesPayload, t: SeriesPayload): SeriesPayload {
  const r = mergeTail(cur, t);
  if (!r.ok) throw new Error(`mergeTail rejected: ${r.reason}`);
  return r.series;
}

/** Everything but the windowed profile, which comes from the first load. */
function withoutProfile(s: SeriesPayload) {
  const { vpvr: _vpvr, ...rest } = s.indicators;
  return { ...s, indicators: rest };
}

describe("mergeOlder", () => {
  it("prepends a page and re-indexes signals and divlines", () => {
    const cur = serve({ bars: 30, now: 100 });
    const page = serve({ bars: 30, end: 71, now: 100, has_more: false });
    const r = mergeOlder(cur, page);
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.count).toBe(30);
    expect(r.series.has_more).toBe(false);
    expect(r.series.bars.map((b) => b.t)).toEqual(
      Array.from({ length: 60 }, (_, k) => tOf(41 + k)),
    );
    const macd = r.series.indicators.macd!;
    for (const l of macd.lines!) expect(l.values).toHaveLength(60);
    expect(macd.histogram![0]!.values).toHaveLength(60);
    expect(macd.fills![0]!.y1).toHaveLength(60);
    for (const s of macd.signals!) expect(r.series.bars[s.x]!.t).toBe(s.t);
    for (const d of macd.divlines!) {
      expect(r.series.bars[d.x1]!.t).toBe(d.t1);
      expect(r.series.bars[d.x2]!.t).toBe(d.t2);
    }
  });

  it("keeps the current profile hbars/hlines and y_range", () => {
    const cur = serve({ bars: 30, now: 100 });
    const page = serve({ bars: 30, end: 71, now: 100 });
    const merged = older(cur, page);
    expect(merged.indicators.vpvr!.hbars).toEqual(cur.indicators.vpvr!.hbars);
    expect(merged.indicators.vpvr!.hlines).toEqual(cur.indicators.vpvr!.hlines);
    expect(merged.indicators.macd!.hlines).toEqual(cur.indicators.macd!.hlines);
    expect(merged.indicators.macd!.y_range).toEqual(cur.indicators.macd!.y_range);
  });

  it("does not modify its inputs", () => {
    const cur = serve({ bars: 30, now: 100 });
    const page = serve({ bars: 30, end: 71, now: 100 });
    const [c0, p0] = [clone(cur), clone(page)];
    older(cur, page);
    expect(cur).toEqual(c0);
    expect(page).toEqual(p0);
  });

  it("accepts an empty page and ends the history with it", () => {
    const cur = serve({ bars: 30, now: 30 }); // bars 1..30
    const short = mergeOlder(cur, serve({ bars: 30, end: 1, now: 30 }));
    expect(short.ok && short.count).toBe(1);
    // Even if the page claims more: asking again would loop on it.
    const empty = mergeOlder(cur, serve({ bars: 0, end: 1, now: 30, has_more: true }));
    expect(empty.ok).toBe(true);
    if (empty.ok) {
      expect(empty.count).toBe(0);
      expect(empty.series.bars).toHaveLength(30);
      expect(empty.series.has_more).toBe(false);
    }
  });

  it("rejects a page that overlaps the current bars", () => {
    const cur = serve({ bars: 30, now: 100 });
    const page = serve({ bars: 30, end: 72, now: 100 });
    expect(mergeOlder(cur, page)).toEqual({ ok: false, reason: "overlap" });
  });

  it("rejects a page in the wrong order", () => {
    const cur = serve({ bars: 30, now: 100 });
    const page = serve({ bars: 30, end: 71, now: 100 });
    page.bars.reverse();
    expect(mergeOlder(cur, page)).toEqual({ ok: false, reason: "order" });
  });

  it("rejects another symbol or interval", () => {
    const cur = serve({ bars: 30, now: 100 });
    const page = serve({ bars: 30, end: 71, now: 100 });
    expect(mergeOlder(cur, { ...page, symbol: "ETHUSDT" })).toEqual({
      ok: false,
      reason: "symbol",
    });
    expect(mergeOlder(cur, { ...page, interval: "5m" })).toEqual({
      ok: false,
      reason: "interval",
    });
  });

  it("rejects mixing candles and Heikin Ashi bars", () => {
    const cur = serve({ bars: 30, now: 100 });
    const page = serve({ bars: 30, end: 71, now: 100 });
    const tail = serve({ bars: 2, now: 101 });
    const ha = { chart_style: "heikin_ashi" as const };
    expect(mergeOlder(cur, { ...page, ...ha })).toEqual({ ok: false, reason: "style" });
    expect(mergeTail(cur, { ...tail, ...ha })).toEqual({ ok: false, reason: "style" });
    // Absent means candles; equal styles merge.
    expect(mergeTail(cur, { ...tail, chart_style: "candles" }).ok).toBe(true);
    const haCur = { ...cur, ...ha };
    expect(mergeTail(haCur, { ...tail, ...ha }).ok).toBe(true);
  });

  it("rejects a different indicator set or channel count", () => {
    const cur = serve({ bars: 30, now: 100 });
    const page = serve({ bars: 30, end: 71, now: 100 });
    const { sma: _sma, ...fewer } = page.indicators;
    expect(mergeOlder(cur, { ...page, indicators: fewer })).toEqual({
      ok: false,
      reason: "shape",
    });
    const extraLine = clone(page);
    extraLine.indicators.sma!.lines!.push({ index: 1, label: null, values: [] });
    expect(mergeOlder(cur, extraLine)).toEqual({ ok: false, reason: "shape" });
    const short = clone(page);
    short.indicators.macd!.histogram![0]!.values.pop();
    expect(mergeOlder(cur, short)).toEqual({ ok: false, reason: "shape" });
  });

  it("rejects bars without timestamps", () => {
    const cur = serve({ bars: 30, now: 100 });
    const page = serve({ bars: 30, end: 71, now: 100 });
    page.bars[3] = { ...page.bars[3]!, t: null };
    expect(mergeOlder(cur, page)).toEqual({ ok: false, reason: "untimed" });
  });
});

describe("optional fields survive merges", () => {
  const colors = (s: SeriesPayload) => {
    const macd = s.indicators.macd!;
    return {
      fill: [macd.fills![0]!.color, macd.fills![0]!.color_below],
      div: new Set(macd.divlines!.map((d) => d.color)),
    };
  };

  it("mergeOlder keeps fill and divline colors", () => {
    const cur = serve({ bars: 30, now: 100 });
    const page = serve({ bars: 30, end: 71, now: 100 });
    const r = mergeOlder(cur, page);
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    const c = colors(r.series);
    expect(c.fill).toEqual(["#00ff00", "#ff0000"]);
    expect([...c.div]).toEqual(["#abcdef"]);
  });

  it("mergeTail keeps fill and divline colors", () => {
    const cur = serve({ bars: 30, now: 100 });
    const r = mergeTail(cur, serve({ bars: 3, now: 102 }));
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    const c = colors(r.series);
    expect(c.fill).toEqual(["#00ff00", "#ff0000"]);
    expect([...c.div]).toEqual(["#abcdef"]);
  });
});

describe("mergeTail", () => {
  it("replaces the last closed and forming bars in place (no rollover)", () => {
    const cur = serve({ bars: 30, now: 100, tick: 0 });
    const valuesRef = cur.indicators.sma!.lines![0]!.values;
    const r = mergeTail(cur, serve({ bars: 2, now: 100, tick: 3 }));
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.from).toBe(28);
    expect(r.series.bars).toHaveLength(30);
    expect(r.series.bars.at(-1)!.c).toBe(close(100) + 3);
    // Per-bar arrays are updated in place, not copied.
    expect(r.series.indicators.sma!.lines![0]!.values).toBe(valuesRef);
    expect(valuesRef.at(-1)).toBe(close(100) + 3 - 1);
    expect(withoutProfile(r.series)).toEqual(
      withoutProfile(serve({ bars: 30, now: 100, tick: 3 })),
    );
  });

  it("appends the new bar on rollover and finalizes the previous one", () => {
    const cur = serve({ bars: 30, now: 100, tick: 5 });
    const merged = tail(cur, serve({ bars: 2, now: 101, tick: -1 }));
    const want = serve({ bars: 31, now: 101, tick: -1 });
    expect(withoutProfile(merged)).toEqual(withoutProfile(want));
    expect(merged.bars.at(-2)!.c).toBe(close(100));
  });

  it("returns a new top-level object", () => {
    const cur = serve({ bars: 10, now: 100 });
    const r = mergeTail(cur, serve({ bars: 2, now: 100, tick: 1 }));
    expect(r.ok && r.series !== cur).toBe(true);
  });

  it("catches up with a larger tail after missed bars", () => {
    const cur = serve({ bars: 30, now: 100 });
    const gap = serve({ bars: 2, now: 105 });
    expect(mergeTail(cur, gap)).toEqual({ ok: false, reason: "gap" });
    const missing = missingBars(cur.bars.at(-1)!.t!, gap.bars[0]!.t!, STEP);
    expect(missing).toBe(3); // bars 101..103
    const merged = tail(cur, serve({ bars: missing + 2 + 1, now: 105 }));
    expect(withoutProfile(merged)).toEqual(
      withoutProfile(serve({ bars: 35, now: 105 })),
    );
  });

  it("rejects a stale tail", () => {
    const cur = serve({ bars: 30, now: 100 });
    expect(mergeTail(cur, serve({ bars: 2, now: 99 }))).toEqual({
      ok: false,
      reason: "stale",
    });
  });

  it("rejects a tail off the bar grid", () => {
    const cur = serve({ bars: 30, now: 100 });
    const t = serve({ bars: 2, now: 100 });
    t.bars[0] = { ...t.bars[0]!, t: t.bars[0]!.t! + 1 };
    expect(mergeTail(cur, t)).toEqual({ ok: false, reason: "mismatch" });
    const longer = serve({ bars: 3, now: 101 });
    longer.bars[1] = { ...longer.bars[1]!, t: longer.bars[1]!.t! - 1 };
    expect(mergeTail(cur, longer)).toEqual({ ok: false, reason: "mismatch" });
  });

  it("skips tail bars older than our history", () => {
    // Our history is bars 91..100; a catch-up tail of 20 bars (86..105)
    // reaches back past it.
    const cur = serve({ bars: 10, now: 100 });
    const r = mergeTail(cur, serve({ bars: 20, now: 105, tick: 1 }));
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.from).toBe(0);
    const want = serve({ bars: 15, now: 105, tick: 1 });
    // Lines/signals equal the request over our range; divlines may differ
    // only by clipping at our first bar, which the merge keeps from ours.
    expect(r.series.bars).toEqual(want.bars);
    expect(r.series.indicators.macd!.lines).toEqual(want.indicators.macd!.lines);
    expect(r.series.indicators.macd!.signals).toEqual(want.indicators.macd!.signals);
    for (const d of r.series.indicators.macd!.divlines!) {
      expect(r.series.bars[d.x1]!.t).toBe(d.t1);
      expect(r.series.bars[d.x2]!.t).toBe(d.t2);
    }
  });

  it("rejects a mismatched header or shape and leaves current untouched", () => {
    const cur = serve({ bars: 30, now: 100 });
    const before = clone(cur);
    const t = serve({ bars: 2, now: 101 });
    expect(mergeTail(cur, { ...t, symbol: "X" })).toEqual({ ok: false, reason: "symbol" });
    expect(mergeTail(cur, { ...t, interval: "1h" })).toEqual({
      ok: false,
      reason: "interval",
    });
    const bad = clone(t);
    bad.indicators.macd!.fills![0]!.y2.push(1);
    expect(mergeTail(cur, bad)).toEqual({ ok: false, reason: "shape" });
    const reversed = clone(t);
    reversed.bars.reverse();
    expect(mergeTail(cur, reversed)).toEqual({ ok: false, reason: "order" });
    expect(cur).toEqual(before);
  });

  it("rejects untimed (demo) data", () => {
    const cur = serve({ bars: 5, now: 100 });
    cur.bars = cur.bars.map((b) => ({ ...b, t: null }));
    expect(mergeTail(cur, serve({ bars: 2, now: 100 }))).toEqual({
      ok: false,
      reason: "untimed",
    });
  });

  it("keeps the profile channels from the full load", () => {
    const cur = serve({ bars: 30, now: 100 });
    const profile = clone(cur.indicators.vpvr!);
    const merged = tail(cur, serve({ bars: 2, now: 101, tick: 7 }));
    expect(merged.indicators.vpvr).toEqual(profile);
  });

  it("rebuilds only the signals and divlines in the tail window, in place", () => {
    const cur = serve({ bars: 30, now: 99 });
    const signals = cur.indicators.macd!.signals!;
    const divlines = cur.indicators.macd!.divlines!;
    const [oldSignal, oldLine] = [signals[0], divlines[0]];
    const merged = tail(cur, serve({ bars: 2, now: 100 }));
    expect(merged.indicators.macd!.signals).toBe(signals);
    expect(merged.indicators.macd!.divlines).toBe(divlines);
    expect(signals[0]).toBe(oldSignal);
    expect(divlines[0]).toBe(oldLine);
  });

  it("sortChannels orders signals by bar and divlines by end bar", () => {
    const s = serve({ bars: 30, now: 100 });
    const macd = s.indicators.macd!;
    macd.signals!.reverse();
    macd.divlines!.reverse();
    sortChannels(s);
    const xs = macd.signals!.map((g) => g.x);
    expect(xs).toEqual([...xs].sort((a, b) => a - b));
    const ends = macd.divlines!.map((d) => Math.max(d.x1, d.x2));
    expect(ends).toEqual([...ends].sort((a, b) => a - b));
  });

  it("drops a signal that vanished from the forming bar", () => {
    const cur = serve({ bars: 30, now: 98 }); // bar 98 = 7*14 carries a signal
    expect(cur.indicators.macd!.signals!.at(-1)!.t).toBe(tOf(98));
    const t = serve({ bars: 2, now: 98, tick: 1 });
    t.indicators.macd!.signals = [];
    const merged = tail(cur, t);
    expect(merged.indicators.macd!.signals!.some((s) => s.t === tOf(98))).toBe(false);
    expect(merged.indicators.macd!.signals!.some((s) => s.t === tOf(91))).toBe(true);
  });
});

describe("divlines", () => {
  it("keeps the unclipped line when a tail repeats it clipped", () => {
    // Line 93→98 ends on the last closed bar; the tail [98, 99] sees it
    // clipped to start at 98 itself.
    const cur = serve({ bars: 30, now: 99 });
    const full = cur.indicators.macd!.divlines!.find((d) => d.t2 === tOf(98))!;
    expect(full.t1).toBe(tOf(93));
    const merged = tail(cur, serve({ bars: 2, now: 99, tick: 2 }));
    const lines = merged.indicators.macd!.divlines!.filter((d) => d.t2 === tOf(98));
    expect(lines).toHaveLength(1);
    expect(lines[0]).toEqual(full);
  });

  it("drops a line in the tail window that the tail no longer has", () => {
    const cur = serve({ bars: 30, now: 99 });
    const t = serve({ bars: 2, now: 99 });
    t.indicators.macd!.divlines = [];
    const merged = tail(cur, t);
    expect(merged.indicators.macd!.divlines!.some((d) => d.t2 === tOf(98))).toBe(false);
    expect(merged.indicators.macd!.divlines!.some((d) => d.t2 === tOf(88))).toBe(true);
  });

  it("keeps a line clipped at a page seam once and in data space", () => {
    // Seam at bar 75: line 73→78 arrives clipped in the newer response and
    // is invisible to the older page, which ends at 74.
    const cur = serve({ bars: 26, now: 100 });
    const page = serve({ bars: 30, end: 75, now: 100 });
    const merged = older(cur, page);
    const seam = merged.indicators.macd!.divlines!.filter((d) => d.t2 === tOf(78));
    expect(seam).toHaveLength(1);
    expect(seam[0]!.t1).toBe(tOf(75));
    expect(merged.bars[seam[0]!.x1]!.t).toBe(tOf(75));
  });

  it("dedupeDivlines keeps the longest line per end bar and style", () => {
    const bars = Array.from({ length: 10 }, (_, i) => ({
      t: tOf(i),
      o: 1,
      h: 1,
      l: 1,
      c: 1,
      v: 1,
    }));
    const d = (x1: number, x2: number, dashed = false) => ({
      x1,
      t1: tOf(x1),
      y1: 0,
      x2,
      t2: tOf(x2),
      y2: 1,
      dashed,
    });
    expect(dedupeDivlines([d(4, 8), d(2, 8), d(6, 8, true), d(1, 3)], bars)).toEqual([
      d(1, 3),
      d(2, 8),
      d(6, 8, true),
    ]);
  });
});

describe("paging then live", () => {
  it("three pages and a tail equal one big request over the same range", () => {
    const now = 400;
    let s = serve({ bars: 100, now, tick: 4 });
    for (let page = 0; page < 3; page++) {
      const end = (s.bars[0]!.t! - BASE) / STEP;
      s = older(s, serve({ bars: 50, end, now, has_more: page < 2 }));
    }
    expect(s.has_more).toBe(false);
    s = tail(s, serve({ bars: 2, now: now + 1, tick: -2 }));

    const big = serve({ bars: 251, now: now + 1, tick: -2 });
    expect(withoutProfile(s)).toEqual({ ...withoutProfile(big), has_more: false });
    // Strictly increasing, no duplicates.
    const ts = s.bars.map((b) => b.t!);
    expect(ts.every((t, i) => i === 0 || t > ts[i - 1]!)).toBe(true);
  });

  it("a page loaded after a tail joins as well", () => {
    const now = 400;
    let s = serve({ bars: 100, now });
    s = tail(s, serve({ bars: 2, now: now + 1 }));
    s = tail(s, serve({ bars: 2, now: now + 2, tick: 1 }));
    const end = (s.bars[0]!.t! - BASE) / STEP;
    s = older(s, serve({ bars: 50, end, now: now + 2 }));
    expect(withoutProfile(s)).toEqual(
      withoutProfile(serve({ bars: 152, now: now + 2, tick: 1 })),
    );
  });
});

describe("histogram colors (ADR-0026)", () => {
  const UP = "rgba(38,166,154,0.5)";
  const DOWN = "rgba(239,83,80,0.5)";
  const NEW = "rgba(0,0,255,1)";
  // Bar i's color is a function of i (none every 11th bar, NEW from bar
  // 101 on). The palette lists colors in first-appearance order, as the
  // server's does, so it differs between responses.
  const colorOf = (i: number) =>
    i % 11 === 0 ? null : i >= 101 ? NEW : i % 5 < 2 ? DOWN : UP;
  function colored(s: SeriesPayload): SeriesPayload {
    const h = s.indicators.macd!.histogram![0]!;
    const palette: string[] = [];
    h.color_idx = s.bars.map((b) => {
      const c = colorOf((b.t! - BASE) / STEP);
      if (c === null) return null;
      if (!palette.includes(c)) palette.push(c);
      return palette.indexOf(c);
    });
    h.palette = palette;
    return s;
  }
  const barColors = (s: SeriesPayload) => {
    const h = s.indicators.macd!.histogram![0]!;
    return s.bars.map((_, i) => barColor(h, i) ?? null);
  };

  it("mergeOlder remaps the page's indices into the current palette", () => {
    const cur = colored(serve({ bars: 30, now: 100 })); // bars 71..100
    const page = colored(serve({ bars: 29, end: 71, now: 100 })); // bars 42..70
    expect(cur.indicators.macd!.histogram![0]!.palette).toEqual([DOWN, UP]);
    expect(page.indicators.macd!.histogram![0]!.palette).toEqual([UP, DOWN]);
    const [c0, p0] = [clone(cur), clone(page)];
    const merged = older(cur, page);
    expect(merged.indicators.macd!.histogram![0]!.palette).toEqual([DOWN, UP]);
    expect(barColors(merged)).toEqual(barColors(colored(serve({ bars: 59, now: 100 }))));
    expect(cur).toEqual(c0);
    expect(page).toEqual(p0);
  });

  it("mergeTail overwrites colors in place and grows the palette", () => {
    const cur = colored(serve({ bars: 30, now: 100 }));
    const idx = cur.indicators.macd!.histogram![0]!.color_idx;
    const merged = tail(cur, colored(serve({ bars: 2, now: 101 })));
    const h = merged.indicators.macd!.histogram![0]!;
    expect(h.color_idx).toBe(idx);
    expect(h.color_idx).toHaveLength(31);
    expect(h.palette).toEqual([DOWN, UP, NEW]);
    expect(barColors(merged)).toEqual(barColors(colored(serve({ bars: 31, now: 101 }))));
  });

  it("pages, tails and a fresh load agree on every bar's color", () => {
    let s = colored(serve({ bars: 50, now: 100 })); // bars 51..100
    s = tail(s, colored(serve({ bars: 3, now: 102 })));
    const end = (s.bars[0]!.t! - BASE) / STEP;
    s = older(s, colored(serve({ bars: 40, end, now: 102 })));
    expect(s.bars).toHaveLength(92);
    expect(barColors(s)).toEqual(barColors(colored(serve({ bars: 92, now: 102 }))));
  });

  it("rejects colors that are not one per bar", () => {
    const cur = colored(serve({ bars: 30, now: 100 }));
    const t = colored(serve({ bars: 2, now: 101 }));
    t.indicators.macd!.histogram![0]!.color_idx!.push(0);
    expect(mergeTail(cur, t)).toEqual({ ok: false, reason: "shape" });
  });

  it("reads a side without colors (an older server) as uncolored bars", () => {
    const cur = colored(serve({ bars: 30, now: 100 }));
    const merged = older(cur, serve({ bars: 10, end: 71, now: 100 }));
    expect(barColors(merged).slice(0, 10)).toEqual(new Array(10).fill(null));
    expect(barColors(merged).slice(10)).toEqual(barColors(cur));
  });
});

describe("projection past the last bar (ADR-0033)", () => {
  const project = (s: SeriesPayload, v: number): SeriesPayload => {
    const last = s.bars.at(-1)!.t!;
    s.indicators.sma!.future = {
      bars: 2,
      t: [last + STEP, last + 2 * STEP],
      lines: [{ index: 0, values: [v, v + 1] }],
      fills: [],
    };
    return s;
  };

  it("takes the tail's projection, which follows the forming bar", () => {
    const cur = project(serve({ bars: 30, now: 100 }), 1);
    const merged = tail(cur, project(serve({ bars: 2, now: 101 }), 7));
    expect(merged.indicators.sma!.future).toEqual({
      bars: 2,
      t: [tOf(102), tOf(103)],
      lines: [{ index: 0, values: [7, 8] }],
      fills: [],
    });
  });

  it("drops ours when the tail has none", () => {
    const cur = project(serve({ bars: 30, now: 100 }), 1);
    const merged = tail(cur, serve({ bars: 2, now: 100, tick: 1 }));
    expect(merged.indicators.sma!.future).toBeUndefined();
  });

  it("keeps ours over an older page's", () => {
    const cur = project(serve({ bars: 30, now: 100 }), 1);
    const merged = older(cur, project(serve({ bars: 20, end: 71, now: 100 }), 9));
    expect(merged.indicators.sma!.future!.lines[0]!.values).toEqual([1, 2]);
  });
});

describe("indicators that fail on one side (indicator_errors)", () => {
  const STEP_BARS = 5;
  const error = { step: "runtime", line: 7, message: "boom" };
  const barsOf = (idx: number[]) =>
    idx.map((i) => ({ t: tOf(i), o: 1, h: 1, l: 1, c: 1, v: 1 }));
  const mine = (idx: number[]) => ({
    label: "Mine",
    is_overlay: false,
    lines: [{ index: 0, label: null, values: idx.map((i) => i * 2) }],
    histogram: [{ index: 0, values: idx.map(() => 1), color_idx: idx.map(() => 0), palette: ["#fff"] }],
    fills: [{ y1: idx.map(() => 1), y2: idx.map(() => 2), color: "#fff" }],
    signals: [{ x: 1, y: 1, label: "s", color: "#fff" }],
  });
  const range5 = (first: number, n = STEP_BARS) => Array.from({ length: n }, (_, k) => first + k);
  const series = (
    idx: number[],
    indicators: SeriesPayload["indicators"],
    indicator_errors?: SeriesPayload["indicator_errors"],
  ) =>
    ({
      symbol: "X",
      interval: "1m",
      has_more: true,
      bars: barsOf(idx),
      indicators,
      ...(indicator_errors ? { indicator_errors } : {}),
    }) as unknown as SeriesPayload;

  it("a key missing without an error is still a shape change", () => {
    const cur = series(range5(0), { mine: mine(range5(0)) } as never);
    const r = mergeTail(cur, series(range5(3), {}));
    expect(r).toEqual({ ok: false, reason: "shape" });
    const added = mergeTail(series(range5(0), {}), series(range5(3), { mine: mine(range5(3)) } as never));
    expect(added).toEqual({ ok: false, reason: "shape" });
  });

  it("a tail that failed keeps the values and adds empty bars", () => {
    const cur = series(range5(0), { mine: mine(range5(0)) } as never);
    const r = mergeTail(cur, series(range5(3), {}, { mine: error }));
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    const ind = r.series.indicators.mine!;
    expect(r.series.bars).toHaveLength(8);
    expect(ind.lines![0]!.values).toEqual([0, 2, 4, 6, 8, null, null, null]);
    expect(ind.histogram![0]!.values).toHaveLength(8);
    expect(ind.histogram![0]!.color_idx).toHaveLength(8);
    expect(ind.fills![0]!.y1).toHaveLength(8);
    expect(r.series.indicator_errors).toEqual({ mine: error });
  });

  it("a tail that works clears the error of a tail that failed", () => {
    const cur = series(range5(0), { mine: mine(range5(0)) } as never, { mine: error });
    const r = mergeTail(cur, series(range5(3), { mine: mine(range5(3)) } as never));
    expect(r.ok && r.series.indicator_errors).toBeUndefined();
    expect(r.ok && r.series.indicators.mine!.lines![0]!.values.at(-1)).toBe(14);
  });

  it("a full load that failed ignores tail values and keeps its error", () => {
    const cur = series(range5(0), {}, { mine: error });
    const r = mergeTail(cur, series(range5(3), { mine: mine(range5(3)) } as never));
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.series.indicators.mine).toBeUndefined();
    expect(r.series.indicator_errors).toEqual({ mine: error });
  });

  it("an older page that failed pads the indicator so it stays aligned", () => {
    const cur = series(range5(5), { mine: mine(range5(5)) } as never);
    const r = mergeOlder(cur, series(range5(0), {}, { mine: error }));
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    const ind = r.series.indicators.mine!;
    expect(r.series.bars).toHaveLength(10);
    expect(ind.lines![0]!.values.slice(0, 6)).toEqual([null, null, null, null, null, 10]);
    expect(ind.histogram![0]!.values).toHaveLength(10);
    expect(ind.histogram![0]!.color_idx).toHaveLength(10);
    expect(ind.fills![0]!.y2).toHaveLength(10);
    expect(ind.signals![0]!.x).toBe(6);
    expect(r.series.indicator_errors).toEqual({ mine: error });
  });
});
