import { describe, expect, it } from "vitest";

import {
  cellMode,
  firstAtOrAfter,
  footprintMethod,
  footprintSummary,
  gradientIndex,
  sessionCvd,
  shortNumber,
  stackedEnd,
  tpoCellWidth,
  tpoHue,
  tpoMethod,
  tpoSummary,
  volumeCandleWidth,
} from "./orderFlow";
import type { FootprintBar, TpoProfile } from "../types/responses";

const fp = (over: Partial<FootprintBar> = {}): FootprintBar => ({
  t: 0,
  buy: 60,
  sell: 40,
  min_delta: -5,
  max_delta: 25,
  poc: 10,
  vah: 12,
  val: 9,
  rows: [
    [9, 10, 20],
    [10, 50, 20],
  ],
  buy_imbalances: [],
  sell_imbalances: [],
  stacked: [],
  ...over,
});

describe("what fits", () => {
  it("draws candles when bars or rows are too small for cells", () => {
    expect(cellMode(8, 20)).toBe("candles");
    expect(cellMode(50, 1)).toBe("candles");
  });

  it("shows numbers only when both bar and row have room", () => {
    expect(cellMode(30, 20)).toBe("cells");
    expect(cellMode(50, 6)).toBe("cells");
    expect(cellMode(50, 12)).toBe("cells_text");
  });

  it("makes a block fit its profile, up to a letter's width", () => {
    expect(tpoCellWidth(90, 5)).toBe(9);
    expect(tpoCellWidth(30, 10)).toBe(3);
    // Never nothing, however narrow.
    expect(tpoCellWidth(2, 100)).toBe(1);
    expect(tpoCellWidth(30, 0)).toBe(9);
  });
});

describe("gradient", () => {
  it("changes colour at 0.25, 0.5 and 0.75", () => {
    expect([0, 0.24, 0.25, 0.49, 0.5, 0.74, 0.75, 1].map(gradientIndex)).toEqual([
      0, 0, 1, 1, 2, 2, 3, 3,
    ]);
  });
});

describe("numbers in cells", () => {
  it("shortens", () => {
    expect(shortNumber(0)).toBe("0");
    expect(shortNumber(950)).toBe("950");
    expect(shortNumber(1234)).toBe("1.2K");
    expect(shortNumber(1000)).toBe("1K");
    expect(shortNumber(-3_400_000)).toBe("-3.4M");
    expect(shortNumber(0.123)).toBe("0.12");
    expect(shortNumber(250_000)).toBe("250K");
  });
});

describe("stacked imbalance projection", () => {
  const bars = [
    { low: 100, high: 110 },
    { low: 111, high: 120 },
    { low: 115, high: 125 },
    { low: 104, high: 112 },
    { low: 100, high: 101 },
  ];

  it("runs until a later bar touches the zone", () => {
    expect(stackedEnd({ low: 105, high: 108 }, 0, bars)).toBe(3);
  });

  it("does not count the bar it started in", () => {
    expect(stackedEnd({ low: 100, high: 110 }, 3, bars)).toBe(4);
  });

  it("stays open while price has not come back", () => {
    expect(stackedEnd({ low: 200, high: 210 }, 0, bars)).toBeNull();
    expect(stackedEnd({ low: 105, high: 108 }, 4, bars)).toBeNull();
  });

  it("counts a touch of the zone's very edge", () => {
    expect(stackedEnd({ low: 125, high: 130 }, 0, bars)).toBe(2);
  });
});

describe("session CVD", () => {
  const day = 86_400;
  const bars = [
    { t: 0, buy: 10, sell: 4 },
    { t: 3600, buy: 1, sell: 3 },
    { t: day, buy: 5, sell: 0 },
  ];

  it("restarts each UTC day below daily bars", () => {
    const cvd = sessionCvd(bars, true);
    expect([cvd.get(0), cvd.get(3600), cvd.get(day)]).toEqual([6, 4, 5]);
  });

  it("runs on through daily bars", () => {
    expect(sessionCvd(bars, false).get(day)).toBe(9);
  });
});

describe("summary tables", () => {
  const fmt = (n: number | null | undefined) => (n == null ? "-" : n.toFixed(2));
  const value = (rows: [string, string][], name: string) => rows.find((r) => r[0] === name)?.[1];

  it("has the footprint's percentages and change", () => {
    const rows = footprintSummary(fp(), fp({ buy: 40, sell: 10 }), 77, 1, fmt);
    expect(value(rows, "Volume")).toBe("100");
    expect(value(rows, "Vol chg")).toBe("100%");
    expect(value(rows, "Buy %")).toBe("60.0%");
    expect(value(rows, "Sell %")).toBe("40.0%");
    expect(value(rows, "Delta")).toBe("20");
    expect(value(rows, "Delta %")).toBe("20.0%");
    expect(value(rows, "Min Δ")).toBe("-5.0");
    expect(value(rows, "Max Δ")).toBe("25");
    expect(value(rows, "CVD")).toBe("77");
    expect(value(rows, "POC")).toBe("10.00");
    // Rows 9 and 10, each 1 high.
    expect(value(rows, "HL range")).toBe("2.00");
  });

  it("has nothing to compare the first bar with", () => {
    expect(value(footprintSummary(fp(), undefined, undefined, 1, fmt), "Vol chg")).toBe("-");
    expect(value(footprintSummary(fp(), undefined, undefined, 1, fmt), "CVD")).toBe("-");
  });

  it("does not divide by no volume", () => {
    const rows = footprintSummary(fp({ buy: 0, sell: 0 }), undefined, 0, 1, fmt);
    expect(value(rows, "Buy %")).toBe("-");
  });

  it("has the profile's ranges and counts", () => {
    const p = {
      high: 110,
      low: 100,
      vah: 108,
      val: 102,
      poc: 105,
      volume: 1500,
      total_blocks: 40,
      blocks_above_poc: 15,
      blocks_below_poc: 20,
      rotation_factor: -3,
      ib_high: 106,
      ib_low: 103,
    } as TpoProfile;
    const rows = tpoSummary(p, (n) => n.toFixed(1));
    expect(value(rows, "HL range")).toBe("10.0");
    expect(value(rows, "VA range")).toBe("6.0");
    expect(value(rows, "Volume")).toBe("1.5K");
    expect(value(rows, "Above POC")).toBe("15");
    expect(value(rows, "Rotation")).toBe("-3");
    expect(value(rows, "IB range")).toBe("3.0");
  });
});

describe("volume candles", () => {
  it("are as wide as their share of the most volume on screen", () => {
    expect(volumeCandleWidth(100, 100, 40)).toBe(40);
    expect(volumeCandleWidth(25, 100, 40)).toBe(10);
  });

  it("never vanish, and never pass their room", () => {
    expect(volumeCandleWidth(0, 100, 40)).toBe(1);
    expect(volumeCandleWidth(1, 1000, 40)).toBe(1);
    expect(volumeCandleWidth(100, 0, 40)).toBe(1);
    expect(volumeCandleWidth(200, 100, 40)).toBe(40);
  });
});

describe("TPO colours and positions", () => {
  it("runs a hue range per letter case, and wraps", () => {
    expect(tpoHue(0)).toBe(190);
    expect(tpoHue(25)).toBe(270);
    expect(tpoHue(26)).toBe(280);
    expect(tpoHue(52)).toBe(tpoHue(0));
  });

  it("finds the first bar at or after a time", () => {
    const bars = [{ timestamp: 10 }, { timestamp: 20 }, { timestamp: 30 }];
    expect(firstAtOrAfter(bars, 5)).toBe(0);
    expect(firstAtOrAfter(bars, 20)).toBe(1);
    expect(firstAtOrAfter(bars, 21)).toBe(2);
    expect(firstAtOrAfter(bars, 31)).toBe(3);
  });
});

describe("the status says how it was made", () => {
  const meta = {
    intrabar_interval: "1m" as string | null,
    side: "taker_volume",
    row_size: 5,
    row_auto: true,
    row_widened: false,
    bars_without_data: 0,
  };

  it("names the data, the side and the row", () => {
    expect(footprintMethod(meta)).toBe("from 1m bars · taker volume · row 5 auto");
  });

  it("is plain about coarser data", () => {
    const text = footprintMethod({
      ...meta,
      intrabar_interval: null,
      side: "close_vs_open",
      row_auto: false,
      row_widened: true,
      bars_without_data: 3,
    });
    expect(text).toBe(
      "from the bars themselves · close vs open · row 5 (widened) · 3 bars without data",
    );
  });

  it("says when a TPO has fewer periods than asked", () => {
    const tpo = { period: "1M", block: "5m", row_size: 2, row_auto: true, row_widened: false };
    expect(tpoMethod({ ...tpo, periods_cut: false })).toBe("1M from 5m blocks · row 2 auto");
    expect(tpoMethod({ ...tpo, periods_cut: true })).toContain("fewer periods");
  });
});
