import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  loadDrawings,
  MAX_BYTES,
  MAX_DRAWINGS,
  MAX_SYMBOLS,
  parseRecord,
  parseRecords,
  pointTimestamp,
  sanitizeStyles,
  saveDrawings,
  serialize,
  storageKey,
  timeAxisOf,
  toChartPoint,
  type DrawingRecord,
} from "./drawings";
import { readState } from "../lib/workspaceSync";

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

  it("keeps a valid id (alerts refer to it, ADR-0034)", () => {
    expect(parseRecord({ ...good, id: "overlay_1_1" })).toEqual({ ...good, id: "overlay_1_1" });
    expect(parseRecord({ ...good, id: 5 })).toEqual(good);
    expect(parseRecord({ ...good, id: "x".repeat(65) })).toEqual(good);
    const [rec] = serialize([{ ...good, id: "overlay_2_1" }], null);
    expect(rec?.id).toBe("overlay_2_1");
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
  ])("%s needs exactly %i points", (name, n) => {
    const pts = (k: number) => Array.from({ length: k }, (_, i) => pt(i + 1));
    expect(parseRecord({ ...good, name, points: pts(n) })?.name).toBe(name);
    expect(parseRecord({ ...good, name, points: pts(n - 1) })).toBeNull();
    if (n < 3) expect(parseRecord({ ...good, name, points: pts(n + 1) })).toBeNull();
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
      timestamps: bars,
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

  describe("another tab on the symbol", () => {
    const rec = (id: string): DrawingRecord => parseRecord({ ...good, id })!;
    const stored = (symbol: string) =>
      (JSON.parse(localStorage.getItem(storageKey(symbol))!) as DrawingRecord[]).map((r) => r.id);
    /** Another tab: writes what it holds, with nothing of this page's in between. */
    const otherTabSaves = (symbol: string, ...ids: string[]) =>
      localStorage.setItem(storageKey(symbol), JSON.stringify(ids.map(rec)));

    it("what it added stays through this page's save", () => {
      otherTabSaves("TWO", "mine");
      loadDrawings("TWO");
      otherTabSaves("TWO", "mine", "theirs");
      saveDrawings("TWO", [rec("mine"), rec("new")]);
      expect(stored("TWO")).toEqual(["mine", "new", "theirs"]);
    });

    it("and through saves after that, this page still not having it", () => {
      otherTabSaves("TWO", "mine");
      loadDrawings("TWO");
      otherTabSaves("TWO", "mine", "theirs");
      saveDrawings("TWO", [rec("mine"), rec("new")]);
      saveDrawings("TWO", [rec("mine"), rec("new"), rec("newer")]);
      expect(stored("TWO")).toEqual(["mine", "new", "newer", "theirs"]);
    });

    it("what this page removes goes, and what it removes last keeps the other's", () => {
      otherTabSaves("TWO", "a", "b");
      loadDrawings("TWO");
      otherTabSaves("TWO", "a", "b", "theirs");
      saveDrawings("TWO", [rec("a")]);
      expect(stored("TWO")).toEqual(["a", "theirs"]);
      saveDrawings("TWO", []);
      expect(stored("TWO")).toEqual(["theirs"]);
    });

    it("what it removed stays removed", () => {
      otherTabSaves("TWO", "a", "b");
      loadDrawings("TWO");
      otherTabSaves("TWO", "a");
      saveDrawings("TWO", [rec("a"), rec("b"), rec("new")]);
      expect(stored("TWO")).toEqual(["a", "new"]);
    });

    it("a page alone behaves as ever, removing the last one removes the key", () => {
      saveDrawings("ONE", [rec("a")]);
      saveDrawings("ONE", [rec("b")]);
      expect(stored("ONE")).toEqual(["b"]);
      saveDrawings("ONE", []);
      expect(localStorage.getItem(storageKey("ONE"))).toBeNull();
    });

    it("a copy dropped from storage does not take this page's with it", () => {
      otherTabSaves("TWO", "a");
      loadDrawings("TWO");
      localStorage.removeItem(storageKey("TWO"));
      saveDrawings("TWO", [rec("a"), rec("new")]);
      expect(stored("TWO")).toEqual(["a", "new"]);
    });
  });

  it("dropping a synced symbol from the LRU sends no deletion (ADR-0050)", () => {
    const recs = [good] as DrawingRecord[];
    saveDrawings("FIRST", recs);
    const key = storageKey("FIRST");
    const value = localStorage.getItem(key);
    localStorage.setItem(`chartgen.sync.${key}`, JSON.stringify({ v: 3, b: value }));
    for (let i = 0; i < MAX_SYMBOLS; i++) saveDrawings(`S${i}`, recs);
    expect(localStorage.getItem(key)).toBeNull();
    expect(readState(localStorage, key)).toEqual({ v: 3, b: null, s: value, e: 1 });
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

describe("pointTimestamp on a gapped market", () => {
  // Hourly bars with a 64 h weekend gap after every 5th bar.
  const H = 3_600_000;
  const gapped = Array.from({ length: 30 }, (_, i) => (i + Math.floor(i / 5) * 64) * H);
  const ax = timeAxisOf(gapped)!;

  it("uses the real bar time for an index inside the data", () => {
    expect(pointTimestamp({ dataIndex: 12 }, ax)).toBe(gapped[12]);
    expect(pointTimestamp({ dataIndex: 0 }, ax)).toBe(gapped[0]);
  });

  it("extrapolates only beyond the last bar", () => {
    expect(pointTimestamp({ dataIndex: 31 }, ax)).toBe(gapped[29]! + 2 * H);
    expect(pointTimestamp({ dataIndex: -2 }, ax)).toBe(gapped[0]! - 2 * H);
  });

  it("keeps an explicit timestamp", () => {
    expect(pointTimestamp({ timestamp: 5, dataIndex: 12 }, ax)).toBe(5);
  });
});
