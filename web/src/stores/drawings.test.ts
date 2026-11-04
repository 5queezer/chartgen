import { beforeEach, describe, expect, it } from "vitest";

import {
  loadDrawings,
  parseRecord,
  parseRecords,
  saveDrawings,
  serialize,
  storageKey,
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

  it("keeps valid styles and extendData", () => {
    const r = parseRecord({
      ...good,
      styles: { line: { color: "#fff" } },
      extendData: "hello",
    });
    expect(r?.styles).toEqual({ line: { color: "#fff" } });
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

describe("parseRecords", () => {
  it("drops malformed entries and keeps the rest", () => {
    expect(parseRecords([good, null, { name: "x" }, good])).toHaveLength(2);
  });

  it("returns [] for a non-array", () => {
    expect(parseRecords({ a: 1 })).toEqual([]);
  });
});

describe("serialize", () => {
  it("strips dataIndex and drops incomplete points", () => {
    const out = serialize([
      {
        name: "segment",
        points: [
          { timestamp: 1, value: 2, dataIndex: 7 } as never,
          { timestamp: 3, value: 4 },
        ],
        lock: true,
        visible: false,
      },
      { name: "segment", points: [{ value: 1 }], lock: false, visible: true },
    ]);
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
});

describe("storage", () => {
  beforeEach(() => {
    const store = new Map<string, string>();
    globalThis.localStorage = {
      getItem: (k: string) => store.get(k) ?? null,
      setItem: (k: string, v: string) => void store.set(k, v),
      removeItem: (k: string) => void store.delete(k),
    } as Storage;
  });

  it("round-trips per symbol", () => {
    const recs = [good] as DrawingRecord[];
    saveDrawings("BTCUSDT", recs);
    expect(localStorage.getItem(storageKey("BTCUSDT"))).not.toBeNull();
    expect(loadDrawings("BTCUSDT")).toEqual(recs);
    expect(loadDrawings("ETHUSDT")).toEqual([]);
  });

  it("removes the key when the list is empty", () => {
    saveDrawings("BTCUSDT", [good] as DrawingRecord[]);
    saveDrawings("BTCUSDT", []);
    expect(localStorage.getItem(storageKey("BTCUSDT"))).toBeNull();
  });

  it("survives corrupt JSON", () => {
    localStorage.setItem(storageKey("BTCUSDT"), "{not json");
    expect(loadDrawings("BTCUSDT")).toEqual([]);
  });
});
