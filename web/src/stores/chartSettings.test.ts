import { beforeEach, describe, expect, it, vi } from "vitest";

function stubStorage(initial: Record<string, string> = {}) {
  const data = new Map(Object.entries(initial));
  vi.stubGlobal("localStorage", {
    getItem: (k: string) => data.get(k) ?? null,
    setItem: (k: string, v: string) => void data.set(k, v),
  });
  return data;
}

// The store reads localStorage at import time, so each test imports a
// fresh copy of the module after stubbing storage.
async function load() {
  vi.resetModules();
  return import("./chartSettings");
}

const KEY = "chartgen.chartSettings";

beforeEach(() => {
  vi.unstubAllGlobals();
});

describe("chart settings store", () => {
  it("uses defaults when storage is empty or corrupt", async () => {
    stubStorage();
    const s = await load();
    expect(s.chartSettings()).toEqual(s.defaultSettings());
    stubStorage({ [KEY]: "{not json" });
    expect((await load()).chartSettings()).toEqual(s.defaultSettings());
  });

  it("restores valid values and drops invalid ones", async () => {
    stubStorage({
      [KEY]: JSON.stringify({
        version: 1,
        themes: {
          dark: {
            bodyUp: "#112233",
            bodyDown: "red",
            wickOn: false,
            borderOn: "no",
            bogus: "#fff",
          },
          light: "nope",
        },
        options: {
          gridVertical: false,
          lastPriceLine: 1,
          crosshairStyle: "dotted",
          scalePlacement: "middle",
        },
      }),
    });
    const s = await load();
    expect(s.chartSettings().themes.dark).toEqual({
      bodyUp: "#112233",
      wickOn: false,
    });
    expect(s.chartSettings().themes.light).toEqual({});
    expect(s.chartSettings().options).toEqual({
      ...s.DEFAULT_OPTIONS,
      gridVertical: false,
      crosshairStyle: "dotted",
    });
  });

  it("accepts rgba colors and rejects css injection", async () => {
    stubStorage();
    const s = await load();
    const out = s.sanitizeSettings({
      themes: {
        dark: {
          background: "rgba(1, 2, 3, 0.5)",
          grid: "red; background: url(x)",
        },
      },
    });
    expect(out.themes.dark).toEqual({ background: "rgba(1, 2, 3, 0.5)" });
  });

  it("previews without persisting and saves with persisting", async () => {
    const data = stubStorage();
    const s = await load();
    const next = s.defaultSettings();
    next.options.gridHorizontal = false;
    s.previewChartSettings(next);
    expect(s.chartSettings().options.gridHorizontal).toBe(false);
    expect(data.has(KEY)).toBe(false);
    s.saveChartSettings(next);
    expect(JSON.parse(data.get(KEY)!).options.gridHorizontal).toBe(false);
  });

  it("sanitizes what it is given", async () => {
    stubStorage();
    const s = await load();
    s.saveChartSettings({ options: { crosshairStyle: "zigzag" } } as never);
    expect(s.chartSettings()).toEqual(s.defaultSettings());
  });
});
