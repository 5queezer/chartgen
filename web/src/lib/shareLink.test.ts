import { describe, expect, it } from "vitest";

import type { ParamSpec } from "../types/responses";
import { DEFAULT_CHART_PARAMS } from "./priceBars";
import {
  coerceParam,
  decodeDrawings,
  decodeIndicators,
  decodeShare,
  encodeDrawings,
  encodeIndicators,
  encodeShare,
  hasShareParams,
  MAX_DRAW_LENGTH,
  MAX_INDICATORS,
  MAX_LINK_DRAWINGS,
  reconcileIndicators,
  stripShareParams,
  type ShareState,
} from "./shareLink";

const state: ShareState = {
  symbol: "ETHUSDT",
  timeframe: "4h",
  chartType: "hollow",
  indicators: ["ema_stack", { name: "rsi", length: 21 }],
  compare: ["BTCUSDT", "SOLUSDT"],
  timeZone: "Europe/Berlin",
};

describe("share links", () => {
  it("round-trips a full chart", () => {
    const q = encodeShare(state);
    expect(q).toBe(
      "symbol=ETHUSDT&interval=4h&type=hollow&ind=ema_stack,rsi(length:21)&compare=BTCUSDT,SOLUSDT&tz=Europe/Berlin",
    );
    // Param values stay raw strings until the registry types them.
    expect(decodeShare(q)).toEqual({
      ...state,
      indicators: ["ema_stack", { name: "rsi", length: "21" }],
    });
  });

  it("round-trips two instances of one indicator", () => {
    const entries = ["rsi", { name: "rsi", id: "rsi#2", length: 21 }];
    const ind = encodeIndicators(entries, { rsi: { length: 14 } });
    expect(ind).toBe("rsi,rsi(length:21)");
    expect(decodeIndicators(ind)).toEqual([
      "rsi",
      { name: "rsi", id: "rsi#2", length: "21" },
    ]);
  });

  it("reconciles each instance on its own", () => {
    const decoded = decodeIndicators("rsi(length:0),rsi(length:21)");
    const r = reconcileIndicators(decoded, ["rsi", "rsi#2"], [
      {
        name: "rsi",
        configurable_params: [{ name: "length", type: "integer" } as never],
      },
    ]);
    expect(r.entries).toEqual(["rsi", { name: "rsi", id: "rsi#2", length: 21 }]);
    expect(r.invalid).toEqual(["rsi.length"]);
  });

  it("omits default params and hidden entries", () => {
    const out = encodeIndicators(
      [
        { name: "rsi", length: 14 },
        { name: "macd", fast: 8, slow: 26 },
        { name: "ema_stack", hidden: true },
      ],
      { rsi: { length: 14 }, macd: { fast: 12, slow: 26 } },
    );
    expect(out).toBe("rsi,macd(fast:8)");
  });

  it("round-trips multiple params, booleans and text as raw strings", () => {
    const text = encodeIndicators([
      { name: "x", a: 1.5, b: true, c: "close", d: "a b,c(1)é" },
    ]);
    expect(text).toBe("x(a:1.5;b:true;c:close;d:a%20b%2Cc%281%29%C3%A9)");
    expect(decodeIndicators(text)).toEqual([
      { name: "x", a: "1.5", b: "true", c: "close", d: "a b,c(1)é" },
    ]);
  });

  it("writes numbers without exponent notation", () => {
    expect(encodeIndicators([{ name: "x", a: 1e-7, b: 1e21 }])).toBe(
      "x(a:0.0000001;b:1000000000000000000000)",
    );
  });

  it("does not take hidden, style or name from a link", () => {
    expect(
      decodeIndicators("rsi(hidden:true;style:x;name:y;length:5)"),
    ).toEqual([{ name: "rsi", length: "5" }]);
    expect(
      encodeIndicators([{ name: "rsi", style: { plots: {} }, length: 5 }]),
    ).toBe("rsi(length:5)");
  });

  it("always writes compare, empty when there is none", () => {
    expect(encodeShare({ ...state, compare: [] })).toContain("&compare=&");
    expect(decodeShare("compare=").compare).toEqual([]);
  });

  it("accepts tz keywords in any case", () => {
    expect(decodeShare("tz=UTC").timeZone).toBe("utc");
    expect(decodeShare("tz=Exchange").timeZone).toBe("exchange");
    expect(decodeShare("tz=Mars/Base").timeZone).toBeUndefined();
  });

  it("caps the size of ind", () => {
    expect(decodeIndicators("a".repeat(2049))).toEqual([]);
    const many = Array.from({ length: 80 }, (_, i) => `i${i}`).join(",");
    expect(decodeIndicators(many)).toHaveLength(MAX_INDICATORS);
  });

  it("parses a 32k-char hostile input quickly", () => {
    const t = performance.now();
    decodeShare(`ind=${" ".repeat(32_000)}x(`);
    decodeShare(`ind=${"(".repeat(32_000)}`);
    decodeShare(`compare=${",".repeat(32_000)}&symbol=${"A".repeat(32_000)}`);
    expect(performance.now() - t).toBeLessThan(200);
  });

  it("keeps an empty ind as no indicators, absent as untouched", () => {
    expect(decodeShare("ind=").indicators).toEqual([]);
    expect(decodeShare("symbol=AAPL").indicators).toBeUndefined();
    expect(decodeShare("ind=nope!!").indicators).toBeUndefined();
  });

  it("does not split on commas inside parentheses", () => {
    expect(decodeIndicators("macd(fast:8;slow:20),rsi")).toEqual([
      { name: "macd", fast: "8", slow: "20" },
      "rsi",
    ]);
  });

  it("is tolerant of bad indicator input", () => {
    expect(decodeIndicators("rsi(length:),,a b,ok")).toEqual(["rsi", "ok"]);
    expect(decodeIndicators("rsi(length:21),rsi(length:5)")).toEqual([
      { name: "rsi", length: "21" },
      { name: "rsi", id: "rsi#2", length: "5" },
    ]);
  });

  it("keeps a __proto__ param as data", () => {
    const [e] = decodeIndicators("x(__proto__:1)");
    expect(Object.getPrototypeOf(e)).toBe(Object.prototype);
    expect(Object.keys(e as object)).toContain("__proto__");
  });

  it("falls back per field on invalid values", () => {
    expect(
      decodeShare("symbol=&interval=7x&type=pie&compare=,,%20&tz=Mars/Base"),
    ).toEqual({ compare: [] });
    expect(decodeShare("symbol=a%20b!&interval=").symbol).toBeUndefined();
  });

  it("carries custom intervals in canonical form and drops bad ones", () => {
    expect(decodeShare("interval=7m").timeframe).toBe("7m");
    expect(decodeShare("interval=2W").timeframe).toBe("2wk");
    expect(decodeShare("interval=120").timeframe).toBe("2h");
    expect(decodeShare("interval=61m").timeframe).toBeUndefined();
  });

  it("normalises symbol, interval and type", () => {
    expect(decodeShare("symbol=ethusdt&interval=240&type=Candles")).toEqual({
      symbol: "ETHUSDT",
      timeframe: "4h",
      chartType: "candles",
    });
  });

  it("detects and strips only share params", () => {
    expect(hasShareParams("?code=1&symbol=X")).toBe(true);
    expect(hasShareParams("?code=1")).toBe(false);
    expect(hasShareParams("")).toBe(false);
    expect(stripShareParams("?symbol=X&code=1&state=s&ind=rsi")).toBe(
      "?code=1&state=s",
    );
    expect(stripShareParams("?symbol=X")).toBe("");
  });

  it("never throws on garbage", () => {
    expect(() => decodeShare("%E0%A4%A&&===&ind=(((")).not.toThrow();
  });

  describe("registry", () => {
    const int: ParamSpec = { name: "length", type: "integer", default: 14 };
    const registry = [
      { name: "rsi", aliases: ["relative"], configurable_params: [int] },
      {
        name: "bb",
        configurable_params: [
          { name: "mult", type: "number", min: 0.5, max: 5 },
          { name: "src", type: "string", enum: ["close", "open"] },
          { name: "fill", type: "boolean" },
        ],
      },
    ];

    it("types params by their spec", () => {
      expect(coerceParam(int, "21")).toBe(21);
      expect(coerceParam(int, "0")).toBeUndefined();
      expect(coerceParam(int, "2.5")).toBeUndefined();
      expect(coerceParam({ ...int, max: 50 }, "51")).toBeUndefined();
      expect(coerceParam({ name: "s", type: "string" }, "007")).toBe("007");
      expect(coerceParam({ name: "b", type: "boolean" }, "yes")).toBeUndefined();
    });

    it("removes unknown imported names and bad params only", () => {
      const r = reconcileIndicators(
        [
          "volume",
          "bogus",
          { name: "rsi", length: "0" },
          { name: "bb", mult: "2.5", src: "high", fill: "true", extra: "1" },
          "mine",
        ],
        ["bogus", "rsi", "bb"],
        registry,
      );
      expect(r.entries).toEqual([
        "volume",
        "rsi",
        { name: "bb", mult: 2.5, fill: true },
        "mine",
      ]);
      expect(r.unknown).toEqual(["bogus"]);
      expect(r.invalid).toEqual(["rsi.length", "bb.src", "bb.extra"]);
    });

    it("matches aliases and keeps everything when there is no registry", () => {
      expect(
        reconcileIndicators([{ name: "relative", length: "9" }], ["relative"], registry)
          .entries,
      ).toEqual([{ name: "relative", length: 9 }]);
      const r = reconcileIndicators(
        [{ name: "zzz", n: "3", on: "true" }],
        ["zzz"],
        undefined,
      );
      expect(r.entries).toEqual([{ name: "zzz", n: 3, on: true }]);
      expect(r.unknown).toEqual([]);
    });
  });
});

describe("price-based chart types in a link", () => {
  const state: ShareState = {
    symbol: "BTCUSDT",
    timeframe: "1h",
    chartType: "renko",
    chartParams: { ...DEFAULT_CHART_PARAMS, box: "fixed", boxSize: 50 },
    indicators: [],
    compare: [],
    timeZone: "local",
  };

  it("round-trips the type and its inputs", () => {
    const q = encodeShare(state);
    expect(q).toContain("type=renko");
    expect(q).toContain("tp=box:fixed;size:50");
    expect(decodeShare(q).chartParams).toEqual(state.chartParams);
  });

  it("says nothing more than before for the other types", () => {
    const q = encodeShare({ ...state, chartType: "candles" });
    expect(q).not.toContain("tp=");
    expect(decodeShare(q).chartParams).toBeUndefined();
    expect(decodeShare("?type=candles&tp=box:fixed;size:5").chartParams).toBeUndefined();
  });

  it("means the defaults when a price-based link has none", () => {
    expect(encodeShare({ ...state, chartParams: DEFAULT_CHART_PARAMS })).not.toContain("tp=");
    expect(decodeShare("?type=kagi").chartParams).toEqual(DEFAULT_CHART_PARAMS);
  });

  it("a link's params are its own to strip", () => {
    expect(hasShareParams("?tp=box:atr")).toBe(true);
    expect(stripShareParams("?type=kagi&tp=rev:2&code=x")).toBe("?code=x");
  });
});

describe("drawings in a link (ADR-0054)", () => {
  // What the MCP server writes (src/drawings.rs, the_share_param_is_compact).
  const SERVER =
    "segment(1760227200:100.5;1760230800:103;color:%232962ff;width:2),cgText(1760227200:110;text:Break%2C%20out%3B%20100%25)";

  it("reads what the server writes", () => {
    expect(decodeDrawings(SERVER)).toEqual([
      {
        name: "segment",
        points: [
          { timestamp: 1760227200000, value: 100.5 },
          { timestamp: 1760230800000, value: 103 },
        ],
        style: { color: "#2962ff", width: 2 },
        lock: false,
        visible: true,
      },
      {
        name: "cgText",
        points: [{ timestamp: 1760227200000, value: 110 }],
        extendData: "Break, out; 100%",
        lock: false,
        visible: true,
      },
    ]);
  });

  it("writes what the server writes, and reads it back", () => {
    const records = decodeDrawings(SERVER);
    expect(encodeDrawings(records)).toBe(SERVER);
    const q = encodeShare({ ...state, drawings: records });
    expect(decodeShare(q).drawings).toEqual(records);
    expect(encodeShare({ ...state, drawings: [] })).not.toContain("draw=");
  });

  it("round-trips every style field, fractional seconds and a rgba fill", () => {
    const rec = {
      name: "cgRect" as const,
      points: [
        { timestamp: 1760227200500, value: 0.00001234 },
        { timestamp: -86400000, value: -2 },
      ],
      style: { color: "#ff0000", fill: "rgba(8, 153, 129, 0.2)", width: 4, dash: "dotted" as const },
      lock: false,
      visible: true,
    };
    expect(decodeDrawings(encodeDrawings([rec])!)).toEqual([rec]);
  });

  it("carries a text of the full 200 characters, prose or not", () => {
    for (const text of ["a few words, ".repeat(20).slice(0, 200), "€".repeat(200), "📈".repeat(100)]) {
      const rec = { name: "cgText" as const, points: [{ timestamp: 1000, value: 1 }], extendData: text, lock: false, visible: true };
      expect(decodeDrawings(encodeDrawings([rec])!)).toEqual([rec]);
    }
    const tooLong = { name: "cgText" as const, points: [{ timestamp: 1000, value: 1 }], extendData: "x".repeat(201), lock: false, visible: true };
    expect(decodeDrawings(encodeDrawings([tooLong])!)).toEqual([]);
  });

  it("drops what a stored drawing would not be", () => {
    const bad = [
      "nope(1:1;2:2)",
      "segment(1:1)",
      "cgLong(1:1;2:2)",
      "cgText(1:1)",
      "segment(1:1;2:x)",
      "segment(1:1;2:2",
      "horizontalStraightLine(1:1;2:2)",
    ];
    expect(decodeDrawings(bad.join(","))).toEqual([]);
    // Unknown or invalid style fields only lose themselves.
    expect(decodeDrawings("segment(1:1;2:2;color:red;width:9;bogus:1)")[0]?.style).toBeUndefined();
    // Text only on cgText.
    expect(decodeDrawings("simpleTag(1:1;text:x)")[0]?.extendData).toBeUndefined();
  });

  it("caps its size and count", () => {
    const one = "horizontalStraightLine(1:1)";
    expect(decodeDrawings(Array(MAX_LINK_DRAWINGS + 5).fill(one).join(","))).toHaveLength(MAX_LINK_DRAWINGS);
    expect(decodeDrawings(one + ",".repeat(MAX_DRAW_LENGTH))).toEqual([]);
    const many = decodeDrawings(one);
    expect(encodeDrawings(Array(MAX_LINK_DRAWINGS + 1).fill(many[0]))).toBeNull();
  });

  it("is a share param, and garbage in it leaves nothing", () => {
    expect(hasShareParams("?draw=x")).toBe(true);
    expect(stripShareParams("?draw=x&code=y")).toBe("?code=y");
    expect(decodeShare("?draw=%%%(((").drawings).toBeUndefined();
  });
});
