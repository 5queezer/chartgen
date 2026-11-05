import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  loadDrawings,
  MAX_BYTES,
  MAX_DRAWINGS,
  MAX_SYMBOLS,
  parseRecord,
  parseRecords,
  sanitizeStyles,
  saveDrawings,
  serialize,
  storageKey,
  timeAxisOf,
  toChartPoint,
  type DrawingRecord,
} from "./drawings";

const good = {
  name: "segment",
  points: [
    { timestamp: 1000, value: 10 },
    { timestamp: 2000, value: 20 },
  ],
  lock: false,
  visible: true,
};

describe("parseRecord", () => {
  it("accepts a well-formed record", () => {
    expect(parseRecord(good)).toEqual(good);
  });

  it("defaults flags and drops extra fields", () => {
    const r = parseRecord({ ...good, lock: "yes", visible: undefined, x: 1 });
    expect(r).toEqual({ ...good, lock: false, visible: true });
  });

  it("keeps whitelisted styles and extendData", () => {
    const styles = { polygon: { color: "rgba(41, 98, 255, 0.15)", borderColor: "#2962ff" } };
    const r = parseRecord({ ...good, styles, extendData: "hello" });
    expect(r?.styles).toEqual(styles);
    expect(r?.extendData).toBe("hello");
  });

  it("drops invalid styles and oversized text but keeps the record", () => {
    const r = parseRecord({ ...good, styles: [1], extendData: "x".repeat(500) });
    expect(r).toEqual(good);
  });

  it.each([
    ["non-object", 5],
    ["unknown name", { ...good, name: "evil" }],
    ["no points", { ...good, points: [] }],
    ["too many points", { ...good, points: Array(4).fill(good.points[0]) }],
    ["missing value", { ...good, points: [{ timestamp: 1 }] }],
    ["NaN value", { ...good, points: [{ timestamp: 1, value: NaN }] }],
    ["string timestamp", { ...good, points: [{ timestamp: "1", value: 1 }] }],
  ])("rejects %s", (_, input) => {
    expect(parseRecord(input)).toBeNull();
  });
});

describe("parseRecord: multi-point tools", () => {
  const pt = (i: number) => ({ timestamp: 1000 * i, value: 10 * i });
  it.each([
    ["cgPriceRange", 2],
    ["cgDateRange", 2],
    ["cgDatePriceRange", 2],
    ["cgFibExt", 3],
    ["cgPitchfork", 3],
    ["cgLong", 3],
    ["cgShort", 3],
  ])("%s needs %i points", (name, n) => {
    const pts = (k: number) => Array.from({ length: k }, (_, i) => pt(i + 1));
    expect(parseRecord({ ...good, name, points: pts(n) })?.name).toBe(name);
    expect(parseRecord({ ...good, name, points: pts(n - 1) })).toBeNull();
  });
});

describe("sanitizeStyles", () => {
  it("strips unknown keys and bad values", () => {
    expect(
      sanitizeStyles({
        line: { color: "red" },
        polygon: {
          style: "evil",
          color: "url(javascript:1)",
          borderColor: "#fff",
          borderSize: 99,
          extra: 1,
        },
      }),
    ).toEqual({ polygon: { borderColor: "#fff" } });
  });

  it("returns undefined when nothing is whitelisted", () => {
    expect(sanitizeStyles({ line: { color: "#fff" } })).toBeUndefined();
  });
});

describe("parseRecords", () => {
  it("drops malformed entries and keeps the rest", () => {
    expect(parseRecords([good, null, { name: "x" }, good])).toHaveLength(2);
  });

  it("returns [] for a non-array", () => {
    expect(parseRecords({ a: 1 })).toEqual([]);
  });

  it("caps the list on load", () => {
    expect(parseRecords(Array(MAX_DRAWINGS + 10).fill(good))).toHaveLength(
      MAX_DRAWINGS,
    );
  });
});

// Bars at t = 1000, 2000, ..., 10000 (interval 1000).
const bars = Array.from({ length: 10 }, (_, i) => (i + 1) * 1000);
const axis = timeAxisOf(bars)!;

describe("timeAxisOf", () => {
  it("derives range and interval", () => {
    expect(axis).toEqual({
      firstTs: 1000,
      lastTs: 10000,
      count: 10,
      intervalMs: 1000,
    });
  });

  it("uses the most common spacing despite gaps", () => {
    const withGap = [...bars.slice(0, 5), ...bars.slice(5).map((t) => t + 5000)];
    expect(timeAxisOf(withGap)?.intervalMs).toBe(1000);
  });

  it("is null for fewer than two bars", () => {
    expect(timeAxisOf([1000])).toBeNull();
    expect(timeAxisOf([])).toBeNull();
  });
});

describe("toChartPoint", () => {
  it("passes in-range points through", () => {
    expect(toChartPoint(axis, { timestamp: 5000, value: 1 })).toEqual({
      timestamp: 5000,
      value: 1,
    });
  });

  it("maps future points to a data index", () => {
    expect(toChartPoint(axis, { timestamp: 13000, value: 1 })).toEqual({
      dataIndex: 12,
      value: 1,
    });
  });

  it("returns null before the first bar", () => {
    expect(toChartPoint(axis, { timestamp: 500, value: 1 })).toBeNull();
  });
});

describe("serialize", () => {
  const overlay = (points: object[]) => ({
    name: "segment",
    points,
    lock: false,
    visible: true,
  });

  it("strips dataIndex and drops incomplete points", () => {
    const out = serialize(
      [
        {
          ...overlay([
            { timestamp: 1, value: 2, dataIndex: 7 },
            { timestamp: 3, value: 4 },
          ]),
          lock: true,
          visible: false,
        },
        overlay([{ value: 1 }]),
      ],
      null,
    );
    expect(out).toEqual([
      {
        name: "segment",
        points: [
          { timestamp: 1, value: 2 },
          { timestamp: 3, value: 4 },
        ],
        lock: true,
        visible: false,
      },
    ]);
  });

  it("persists future points as lastBarTs + n * interval", () => {
    const out = serialize(
      [overlay([{ timestamp: 5000, value: 1 }, { dataIndex: 14, value: 2 }])],
      axis,
    );
    expect(out[0]?.points).toEqual([
      { timestamp: 5000, value: 1 },
      { timestamp: 15000, value: 2 },
    ]);
  });

  it("round-trips a future point through toChartPoint", () => {
    const [rec] = serialize([overlay([{ dataIndex: 12, value: 2 }])], axis);
    expect(toChartPoint(axis, rec!.points[0]!)).toEqual({
      dataIndex: 12,
      value: 2,
    });
  });

  it("extrapolates left of the first bar", () => {
    const out = serialize([overlay([{ dataIndex: -2, value: 2 }])], axis);
    expect(out[0]?.points[0]?.timestamp).toBe(-1000);
  });

  it("drops index-only points when there is no time axis", () => {
    expect(serialize([overlay([{ dataIndex: 3, value: 1 }])], null)).toEqual([]);
  });
});

describe("storage", () => {
  let store: Map<string, string>;
  let failWrites: boolean;

  beforeEach(() => {
    store = new Map();
    failWrites = false;
    globalThis.localStorage = {
      getItem: (k: string) => store.get(k) ?? null,
      setItem: (k: string, v: string) => {
        if (failWrites) throw new Error("QuotaExceededError");
        store.set(k, v);
      },
      removeItem: (k: string) => void store.delete(k),
    } as Storage;
  });

  it("round-trips per symbol", () => {
    const recs = [good] as DrawingRecord[];
    expect(saveDrawings("BTCUSDT", recs)).toBe(true);
    expect(store.has(storageKey("BTCUSDT"))).toBe(true);
    expect(loadDrawings("BTCUSDT")).toEqual(recs);
    expect(loadDrawings("ETHUSDT")).toEqual([]);
  });

  it("removes the key when the list is empty", () => {
    saveDrawings("BTCUSDT", [good] as DrawingRecord[]);
    saveDrawings("BTCUSDT", []);
    expect(store.has(storageKey("BTCUSDT"))).toBe(false);
  });

  it("survives corrupt JSON", () => {
    store.set(storageKey("BTCUSDT"), "{not json");
    expect(loadDrawings("BTCUSDT")).toEqual([]);
  });

  it("reports quota failures and warns", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    failWrites = true;
    expect(saveDrawings("BTCUSDT", [good] as DrawingRecord[])).toBe(false);
    expect(warn).toHaveBeenCalled();
    warn.mockRestore();
  });

  it("refuses to save beyond the per-symbol size cap", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const big = Array(MAX_BYTES / 100).fill(good) as DrawingRecord[];
    expect(saveDrawings("BTCUSDT", big)).toBe(false);
    expect(store.has(storageKey("BTCUSDT"))).toBe(false);
    warn.mockRestore();
  });

  it("evicts least recently used symbols beyond the limit", () => {
    const recs = [good] as DrawingRecord[];
    for (let i = 0; i < MAX_SYMBOLS; i++) saveDrawings(`S${i}`, recs);
    loadDrawings("S0"); // S0 becomes most recent, S1 the oldest
    saveDrawings("NEW", recs);
    expect(store.has(storageKey("S1"))).toBe(false);
    expect(store.has(storageKey("S0"))).toBe(true);
    expect(store.has(storageKey("NEW"))).toBe(true);
  });
});
