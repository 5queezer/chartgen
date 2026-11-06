import { describe, expect, it } from "vitest";

import {
  barsWithoutProfile,
  cellMode,
  firstAtOrAfter,
  footprintMethod,
  footprintSummary,
  footprintTail,
  gradientIndex,
  mergeFootprintMeta,
  mergeTpoMeta,
  profileStart,
  sessionCvd,
  shortNumber,
  stackedEnd,
  tpoCellWidth,
  tpoHue,
  tpoMethod,
  tpoSummary,
  tpoTail,
  volumeCandleWidth,
} from "./orderFlow";
import type { FootprintBar, FootprintMeta, TpoMeta, TpoProfile } from "../types/responses";

const fp = (over: Partial<FootprintBar> = {}): FootprintBar => ({
  t: 0,
  row: 1,
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

  it("says when the block is longer than asked", () => {
    const tpo = { period: "1M", block: "15m", row_size: 2, row_auto: true, row_widened: false, periods_cut: false };
    expect(tpoMethod(tpo)).toBe("1M from 15m blocks · row 2 auto");
    expect(tpoMethod({ ...tpo, block_widened: true })).toContain("1M from 15m blocks (shorter ones cannot cover it)");
  });
});

describe("what a poll keeps of its load", () => {
  const load = {
    symbol: "BTCUSDT",
    interval: "1h",
    intrabar_interval: "1m",
    side: "taker_volume",
    row_size: 5,
    row_auto: true,
    row_widened: true,
    tick: 0.01,
    bars_without_data: 2,
  } as FootprintMeta;

  it("keeps saying the row was auto, though a poll sends it explicitly", () => {
    const later = { ...load, row_auto: false, row_widened: false, bars_without_data: 0 };
    const got = mergeFootprintMeta(load, later);
    expect(got.row_auto).toBe(true);
    expect(got.row_widened).toBe(true);
    expect(got.bars_without_data).toBe(2);
    expect(got.coarser_older).toBe(false);
  });

  it("keeps the load's row size, and says when any reply had to widen its own", () => {
    const quiet = { ...load, row_widened: false };
    const got = mergeFootprintMeta(quiet, { ...quiet, row_size: 10, row_auto: false, row_widened: true });
    expect(got.row_size).toBe(5);
    expect(got.row_widened).toBe(true);
    expect(footprintMethod(got)).toContain("row 5 auto (widened)");
    // A later reply that fits does not take that back.
    expect(mergeFootprintMeta(got, { ...quiet, row_auto: false }).row_widened).toBe(true);
  });

  it("does not call a poll coarser when it read what its load read", () => {
    const got = mergeFootprintMeta(load, { ...load, row_auto: false });
    expect(footprintMethod(got)).not.toContain("coarser");
  });

  it("says when an older page came from coarser data, and goes on saying it", () => {
    const got = mergeFootprintMeta(load, { ...load, intrabar_interval: "1h" });
    expect(got.intrabar_interval).toBe("1m");
    expect(got.coarser_older).toBe(true);
    expect(mergeFootprintMeta(got, load).coarser_older).toBe(true);
    expect(footprintMethod(got)).toContain("older bars from coarser data");
  });

  it("does the same for a TPO", () => {
    const tpo = { row_auto: true, row_widened: false, periods_cut: true } as TpoMeta;
    const got = mergeTpoMeta(tpo, { ...tpo, row_auto: false, periods_cut: false });
    expect(got.row_auto).toBe(true);
    expect(got.periods_cut).toBe(true);
  });

  it("keeps a TPO's row size too, and says when a reply widened its own", () => {
    const tpo = { row_size: 5, row_auto: true, row_widened: false, periods_cut: false } as TpoMeta;
    const got = mergeTpoMeta(tpo, { ...tpo, row_size: 10, row_auto: false, row_widened: true });
    expect(got.row_size).toBe(5);
    expect(got.row_widened).toBe(true);
  });
});

describe("how much a poll asks for", () => {
  it("is the last closed bar and the forming one, when it is on time", () => {
    expect(footprintTail(10, "1m")).toBe(3);
    expect(footprintTail(10, "1h")).toBe(3);
  });

  it("adds the bars of a gap", () => {
    expect(footprintTail(20 * 60, "1m")).toBe(22);
    expect(footprintTail(3 * 3_600, "1h")).toBe(5);
  });

  it("stays at 3 for an interval it cannot read", () => {
    expect(footprintTail(99_999, "nonsense")).toBe(3);
  });

  it("is two periods for a TPO, whose last one may have ended since", () => {
    expect(tpoTail(30, "1D")).toBe(2);
    expect(tpoTail(30, "1M")).toBe(2);
    expect(tpoTail(3 * 86_400, "1D")).toBe(4);
    expect(tpoTail(3 * 86_400, "1W")).toBe(2);
  });
});

describe("where a profile is", () => {
  const profile = (first: number, end: number) => ({ t: first - 3_600, end, block_times: [first] });

  it("starts at its first block; its key is the period's", () => {
    expect(profileStart(profile(5_000, 9_000))).toBe(5_000);
    expect(profileStart({ t: 7, block_times: [] })).toBe(7);
  });

  it("finds the bars no profile covers", () => {
    const opens = [100, 200, 300, 400, 500, 600];
    // The last of `opens` only ends the bar before it.
    expect(barsWithoutProfile(opens, [profile(300, 500)], 5)).toEqual([0, 1, 4]);
    expect(barsWithoutProfile(opens, [], 5)).toEqual([0, 1, 2, 3, 4]);
  });

  it("counts a bar a profile only starts inside as covered", () => {
    // A daily bar stamped at midnight, its session's profile opening hours later.
    expect(barsWithoutProfile([0, 86_400], [profile(50_000, 70_000)], 1)).toEqual([]);
  });

  it("covers the newest bar, which has no next one to end it", () => {
    expect(barsWithoutProfile([100, 200], [profile(200, 300)], 2)).toEqual([0]);
  });

  it("takes profiles in any order, and sees a gap between two", () => {
    const opens = [0, 100, 200, 300, 400, 500];
    const list = [profile(400, 500), profile(0, 100)];
    expect(barsWithoutProfile(opens, list, 5)).toEqual([1, 2, 3]);
  });
});
