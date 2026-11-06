// Price-based chart types (ADR-0047): the server builds the bars, the chart draws
// them. Each type is picked from the menu and takes its inputs from its dialog. The
// suite's server serves deterministic sample bars, so no market data is touched.

import { tmpdir } from "node:os";
import { join } from "node:path";

import { devices, type Page, type Request } from "@playwright/test";

import { test, expect, expectChartRendered, gotoApp, reloadApp } from "./fixtures/app";

const SHOTS = join(tmpdir(), "chartgen-shots", "price-bars");

interface ChartArgs {
  bars?: number;
  end_time?: number;
  chart_style?: string;
  chart_params?: Record<string, unknown>;
}

interface Bar {
  t: number;
  o: number;
  h: number;
  l: number;
  c: number;
}

interface Series {
  chart_style?: string;
  chart_params?: { box_size?: number };
  bars: Bar[];
  indicators: Record<string, { lines?: { values: unknown[] }[] }>;
}

function chartArgs(req: Request): ChartArgs | null {
  if (req.method() !== "POST" || !req.url().endsWith("/mcp")) return null;
  try {
    const b = req.postDataJSON() as {
      method?: string;
      params?: { name?: string; arguments?: ChartArgs };
    };
    return b.method === "tools/call" && b.params?.name === "generate_chart"
      ? (b.params.arguments ?? {})
      : null;
  } catch {
    return null;
  }
}

const chartRequest = (page: Page, match: (a: ChartArgs) => boolean) =>
  page.waitForRequest((req) => {
    const a = chartArgs(req);
    return a !== null && match(a);
  });

const series = (page: Page) =>
  page.evaluate(
    () =>
      (window as unknown as { __chartgenTest: { shownSeries: () => unknown } }).__chartgenTest
        .shownSeries() as Series | null,
  );

const drawnBars = (page: Page) =>
  page.evaluate(
    () =>
      (
        window as unknown as {
          __chartgenTest: {
            chart: () => { getDataList: () => Array<{ timestamp: number; close: number }> } | null;
          };
        }
      ).__chartgenTest
        .chart()
        ?.getDataList() ?? [],
  );

async function pickChartType(page: Page, name: string): Promise<void> {
  await page.getByRole("button", { name: /^Chart type/ }).click();
  await page.getByRole("menuitemradio", { name, exact: true }).click();
  await expect(page.getByRole("button", { name: `Chart type: ${name}` })).toBeVisible();
}

/** Wait for this chart's series: the one built by `style`, with the bars on the chart. */
async function expectBuilt(page: Page, style: string): Promise<Series> {
  await expect.poll(async () => (await series(page))?.chart_style).toBe(style);
  await expectChartRendered(page);
  const s = (await series(page))!;
  expect(s.bars.length).toBeGreaterThan(5);
  return s;
}

async function openSettings(page: Page, name: string): Promise<void> {
  await page.getByRole("button", { name: /^Chart type/ }).click();
  await page.getByRole("menuitem", { name: `Settings for ${name}…` }).click();
  await expect(page.getByTestId("chart-type-settings")).toBeVisible();
}

const legendParams = (page: Page) => page.locator(".pane-header-params").first();

/** Non-transparent pixels near each color on the price pane's canvases. */
const pixelsNear = (page: Page, colors: number[][]) =>
  page.evaluate((colors) => {
    const host = document.querySelector(".chart-host");
    const out = colors.map(() => 0);
    if (!host) return out;
    const y = host.getBoundingClientRect().top + 100;
    for (const canvas of document.querySelectorAll<HTMLCanvasElement>(".chart-container canvas")) {
      const rect = canvas.getBoundingClientRect();
      if (rect.width < 200 || y < rect.top || y >= rect.bottom) continue;
      const ctx = canvas.getContext("2d");
      if (!ctx || canvas.width === 0 || canvas.height === 0) continue;
      const d = ctx.getImageData(0, 0, canvas.width, canvas.height).data;
      for (let i = 0; i < d.length; i += 4) {
        if (d[i + 3]! < 120) continue;
        colors.forEach((c, k) => {
          if (
            Math.abs(d[i]! - c[0]!) < 24 &&
            Math.abs(d[i + 1]! - c[1]!) < 24 &&
            Math.abs(d[i + 2]! - c[2]!) < 24
          ) {
            out[k]!++;
          }
        });
      }
    }
    return out;
  }, colors);

const BULL = [0x26, 0xa6, 0x9a];
const BEAR = [0xef, 0x53, 0x50];

const TYPES = [
  { label: "Renko", style: "renko", legend: /Renko \d/ },
  { label: "Line break", style: "line_break", legend: /Line Break 3/ },
  { label: "Kagi", style: "kagi", legend: /Kagi \d/ },
  { label: "Point & figure", style: "point_figure", legend: /P&F \d[\d.]* x 3/ },
  { label: "Range", style: "range", legend: /Range \d/ },
];

test.describe("price-based chart types", () => {
  for (const { label, style, legend } of TYPES) {
    test(`${label} builds on the server, draws, and runs indicators on its bars`, async ({
      bootedApp: page,
    }, info) => {
      const load = chartRequest(page, (a) => a.chart_style === style && a.end_time === undefined);
      await pickChartType(page, label);
      const args = chartArgs(await load)!;
      expect(args.chart_params).toBeTruthy();

      const s = await expectBuilt(page, style);
      await expect(legendParams(page)).toHaveText(legend);

      // Times increase and are not the hour grid of the source.
      const times = s.bars.map((b) => b.t);
      expect(times.every((t, i) => i === 0 || t > times[i - 1]!)).toBe(true);
      expect(new Set(times.slice(1).map((t, i) => t - times[i]!)).size).toBeGreaterThan(1);

      // Every indicator has a value slot per bar of these bars.
      const lines = Object.values(s.indicators).flatMap((i) => i.lines ?? []);
      expect(lines.length).toBeGreaterThan(0);
      for (const l of lines) expect(l.values).toHaveLength(s.bars.length);

      // The chart holds those bars, and paints both directions on the price pane.
      await expect.poll(async () => (await drawnBars(page)).length).toBeGreaterThan(5);
      await expect
        .poll(async () => Math.min(...(await pixelsNear(page, [BULL, BEAR]))))
        .toBeGreaterThan(40);
      await page.screenshot({ path: `${SHOTS}/${style}.png` });
      info.annotations.push({ type: "shot", description: `${SHOTS}/${style}.png` });
    });
  }

  test("Renko bricks are the box tall, and the dialog changes the box", async ({
    bootedApp: page,
  }) => {
    await pickChartType(page, "Renko");
    const atr = await expectBuilt(page, "renko");
    const size = atr.chart_params!.box_size!;
    expect(size).toBeGreaterThan(0);
    for (const b of atr.bars.slice(0, -1)) {
      expect(Math.abs(b.o - b.c)).toBeCloseTo(size, 6);
    }

    const fixed = size * 2;
    await openSettings(page, "Renko");
    const dialog = page.getByTestId("chart-type-settings");
    await dialog.getByLabel("Box assignment").selectOption("fixed");
    // It starts from the box in use.
    await expect(dialog.getByLabel("Size")).toHaveValue(String(size));
    await dialog.getByLabel("Size").fill("-1");
    await expect(dialog.getByRole("alert")).toContainText("above 0");
    await expect(dialog.getByRole("button", { name: "OK" })).toBeDisabled();
    await dialog.getByLabel("Size").fill(String(fixed));

    const load = chartRequest(
      page,
      (a) => a.chart_params?.box === "fixed" && a.chart_params?.box_size === fixed,
    );
    await dialog.getByRole("button", { name: "OK" }).click();
    await load;
    await expect(legendParams(page)).toHaveText(new RegExp(`Renko ${fixed}`));
    await expect.poll(async () => (await series(page))?.chart_params?.box_size).toBe(fixed);
    const s = (await series(page))!;
    for (const b of s.bars.slice(0, -1)) expect(Math.abs(b.o - b.c)).toBeCloseTo(fixed, 6);
    await page.screenshot({ path: `${SHOTS}/renko-fixed.png` });

    // Too small for its prices: refused by the server. The chart falls back and says so.
    await openSettings(page, "Renko");
    await dialog.getByLabel("Size").fill("0.0000001");
    await dialog.getByRole("button", { name: "OK" }).click();
    await expect(page.getByTestId("toasts")).toContainText(/too small.*Using ATR/);
    await expect(legendParams(page)).toHaveText(/Renko \d/);
    await expect.poll(async () => (await series(page))?.chart_params?.box).toBe("atr");
  });

  test("each type's dialog shows its own inputs and sends them", async ({ bootedApp: page }) => {
    await pickChartType(page, "Point & figure");
    await expectBuilt(page, "point_figure");
    await openSettings(page, "Point & figure");
    const dialog = page.getByTestId("chart-type-settings");
    await page.screenshot({ path: `${SHOTS}/pnf-dialog.png` });
    await dialog.getByLabel("Box assignment").selectOption("percent");
    await dialog.getByLabel("Percent of last price").fill("2");
    await dialog.getByLabel("Reversal, in boxes").fill("2");
    const pnf = chartRequest(
      page,
      (a) => a.chart_params?.percent === 2 && a.chart_params?.reversal === 2,
    );
    await dialog.getByRole("button", { name: "OK" }).click();
    await pnf;
    await expect(legendParams(page)).toHaveText(/P&F \d[\d.]* x 2/);

    await pickChartType(page, "Line break");
    await expectBuilt(page, "line_break");
    await openSettings(page, "Line break");
    await expect(dialog.getByLabel("Box assignment")).toHaveCount(0);
    await dialog.getByLabel("Line count").fill("5");
    const lb = chartRequest(page, (a) => a.chart_style === "line_break" && a.chart_params?.lines === 5);
    await dialog.getByRole("button", { name: "OK" }).click();
    await lb;
    await expect(legendParams(page)).toHaveText(/Line Break 5/);

    // Cancel leaves them alone.
    await openSettings(page, "Line break");
    await dialog.getByLabel("Line count").fill("7");
    await dialog.getByRole("button", { name: "Cancel" }).click();
    await expect(legendParams(page)).toHaveText(/Line Break 5/);
  });

  test("a bar replay and compare are refused, and come back on candles", async ({
    bootedApp: page,
  }) => {
    const replay = page.getByTestId("toolbar-replay");
    const compare = page.getByRole("button", { name: "Compare symbol" });
    await expect(replay).toBeEnabled();
    await expect(page.getByTestId("bar-countdown")).toBeVisible();
    await compare.click();
    await page.getByRole("dialog").getByRole("combobox").fill("ETHUSDT");
    await expect(page.getByRole("option").first()).toContainText("ETHUSDT");
    await page.keyboard.press("Enter");
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await expect(page.locator(".pane-header", { hasText: "ETHUSDT" })).toBeVisible();

    await pickChartType(page, "Renko");
    await expectBuilt(page, "renko");
    await expect(replay).toBeDisabled();
    await expect(replay).toHaveAttribute("title", /time grid/);
    await expect(compare).toBeDisabled();
    await expect(compare).toHaveAttribute("title", /time grid/);
    // The compare line is gone from this chart, not from the list.
    await expect(page.locator(".pane-header", { hasText: "ETHUSDT" })).toHaveCount(0);

    await pickChartType(page, "Candles");
    await expect(replay).toBeEnabled();
    await expect(compare).toBeEnabled();
    await expect(page.locator(".pane-header", { hasText: "ETHUSDT" })).toBeVisible();

    // Picking a price-based type during a replay ends the replay.
    await replay.click();
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            (
              window as unknown as { __chartgenTest: { replay: () => { mode: string } } }
            ).__chartgenTest.replay().mode,
        ),
      )
      .toBe("select");
    await pickChartType(page, "Kagi");
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            (
              window as unknown as { __chartgenTest: { replay: () => { mode: string } } }
            ).__chartgenTest.replay().mode,
        ),
      )
      .toBe("off");
  });

  test("a drawing lives on a price-based chart, through a reload and back to candles", async ({
    bootedApp: page,
  }) => {
    await pickChartType(page, "Renko");
    await expectBuilt(page, "renko");
    const overlays = () =>
      page.evaluate(() =>
        (
          window as unknown as {
            __chartgenDrawings: () => { points: { timestamp?: number; value?: number }[] }[];
          }
        ).__chartgenDrawings(),
      );
    const box = (await page.locator(".chart-host").boundingBox())!;
    await page.getByRole("button", { name: "Trend line" }).click();
    await page.mouse.click(box.x + box.width * 0.4, box.y + box.height * 0.2);
    await page.mouse.click(box.x + box.width * 0.6, box.y + box.height * 0.3);
    await expect.poll(overlays).toHaveLength(1);
    const [drawn] = await overlays();
    expect(drawn!.points).toHaveLength(2);
    // Its times are the bars', keyed like on any chart.
    const times = new Set((await series(page))!.bars.map((b) => b.t * 1000));
    for (const p of drawn!.points) expect(times.has(p.timestamp!)).toBe(true);

    await reloadApp(page);
    await expectBuilt(page, "renko");
    await expect.poll(overlays).toHaveLength(1);
    const at = (points: { timestamp?: number; value?: number }[]) =>
      points.map((p) => [p.timestamp, p.value]);
    expect(at((await overlays())[0]!.points)).toEqual(at(drawn!.points));
    await page.screenshot({ path: `${SHOTS}/renko-drawing.png` });

    await pickChartType(page, "Candles");
    await expect.poll(async () => (await series(page))?.chart_style).toBe("candles");
    await expect.poll(overlays).toHaveLength(1);
  });

  test("the type and its inputs survive a reload and a saved layout", async ({
    bootedApp: page,
  }) => {
    await pickChartType(page, "Line break");
    await expectBuilt(page, "line_break");
    await openSettings(page, "Line break");
    const dialog = page.getByTestId("chart-type-settings");
    await dialog.getByLabel("Line count").fill("4");
    await dialog.getByRole("button", { name: "OK" }).click();
    await expect(legendParams(page)).toHaveText(/Line Break 4/);

    await reloadApp(page);
    await expect(legendParams(page)).toHaveText(/Line Break 4/);

    await saveLayoutAs(page, "Breaks");
    // Other inputs and another type, saved too so nothing prompts, then back to the layout.
    await openSettings(page, "Line break");
    await dialog.getByLabel("Line count").fill("6");
    await dialog.getByRole("button", { name: "OK" }).click();
    await expect(legendParams(page)).toHaveText(/Line Break 6/);
    await pickChartType(page, "Candles");
    await saveLayoutAs(page, "Candles");
    await page.getByRole("button", { name: /^Layout:/ }).click();
    await page.getByRole("menuitem", { name: /^Load layout/ }).click();
    await page.getByRole("button", { name: /^Breaks/ }).click();
    await expect(page.getByRole("button", { name: "Chart type: Line break" })).toBeVisible();
    await expect(legendParams(page)).toHaveText(/Line Break 4/);
  });

  test("a share link carries the type and its inputs", async ({ page }) => {
    await gotoApp(page, "/?symbol=ETHUSDT&interval=4h&type=kagi&tp=box:percent;pct:2&ind=rsi");
    await expect(page.getByRole("button", { name: "Chart type: Kagi" })).toBeVisible();
    const s = await expectBuilt(page, "kagi");
    expect(s.chart_params).toMatchObject({ box: "percent", percent: 2 });

    // A link without inputs means the defaults, whatever was stored.
    await gotoApp(page, "/?symbol=ETHUSDT&interval=4h&type=kagi&ind=rsi");
    await expect.poll(async () => (await series(page))?.chart_params).toMatchObject({
      box: "atr",
      atr_length: 14,
    });
    // Nonsense in a link falls back without an error.
    await gotoApp(page, "/?symbol=ETHUSDT&type=renko&tp=box:fixed;size:-3;len:x&ind=rsi");
    await expect.poll(async () => (await series(page))?.chart_params).toMatchObject({ box: "atr" });
  });

  test("Copy link writes the type and only what differs from the defaults", async ({
    bootedApp: page,
    context,
  }) => {
    await context.grantPermissions(["clipboard-read", "clipboard-write"]);
    await pickChartType(page, "Renko");
    await expectBuilt(page, "renko");
    await openSettings(page, "Renko");
    const dialog = page.getByTestId("chart-type-settings");
    await dialog.getByLabel("Box assignment").selectOption("fixed");
    await dialog.getByLabel("Size").fill("120");
    await dialog.getByRole("button", { name: "OK" }).click();
    await expect(legendParams(page)).toHaveText(/Renko 120/);
    await page.getByRole("button", { name: "Copy link to chart" }).click();
    const copied = await page.evaluate(() => navigator.clipboard.readText());
    expect(copied).toContain("type=renko");
    expect(copied).toContain("tp=box:fixed;size:120");
  });
});

async function saveLayoutAs(page: Page, name: string): Promise<void> {
  await page.getByRole("button", { name: /^Layout:/ }).click();
  await page.getByRole("menuitem", { name: /Save layout as|Save as/ }).click();
  const input = page.getByRole("textbox", { name: "Layout name" });
  await input.fill(name);
  await input.press("Enter");
  await expect(page.getByRole("dialog")).toHaveCount(0);
}

test.describe("a box the server refuses", () => {
  // The review's values, on a sample price of about 246. The last is under the floor by
  // far, 0.00001 by a little; 0.001 is allowed, and must not build its marks to refuse them.
  for (const size of ["0.00001", "0.0000001", "0.000000001"]) {
    test(`a link to a Point & Figure box of ${size} recovers, with a message`, async ({ page }) => {
      const errors: string[] = [];
      page.on("pageerror", (e) => errors.push(e.message));
      await gotoApp(page, `/?symbol=BTCUSDT&type=point_figure&tp=box:fixed;size:${size}&ind=rsi`);
      await expect(page.getByTestId("toasts")).toContainText(/too small.*Using ATR/);
      const s = await expectBuilt(page, "point_figure");
      expect(s.chart_params).toMatchObject({ box: "atr" });
      expect(errors).toEqual([]);
      // The fallback is what is saved, so a reload is fine too.
      const stored = await page.evaluate(() => localStorage.getItem("chartgen.layout.chartParams"));
      expect(JSON.parse(stored!)).toMatchObject({ box: "atr" });
      await reloadApp(page);
      expect((await expectBuilt(page, "point_figure")).chart_params).toMatchObject({ box: "atr" });
      expect(errors).toEqual([]);
    });
  }

  test("a bad box already saved recovers on the next load", async ({ page }) => {
    await page.addInitScript(() => {
      localStorage.setItem("chartgen.layout.chartType", JSON.stringify("kagi"));
      localStorage.setItem(
        "chartgen.layout.chartParams",
        JSON.stringify({ box: "fixed", boxSize: 1e-9, atrLength: 14, percent: 1, reversal: 3, lines: 3 }),
      );
    });
    await gotoApp(page, "/?ind=rsi");
    await expect(page.getByTestId("toasts")).toContainText(/too small.*Using ATR/);
    expect((await expectBuilt(page, "kagi")).chart_params).toMatchObject({ box: "atr" });
  });

  test("a small box the server allows draws without stalling", async ({ page }) => {
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));
    await gotoApp(page, "/?symbol=BTCUSDT&type=point_figure&tp=box:fixed;size:0.001&ind=rsi");
    await expectBuilt(page, "point_figure");
    await expect
      .poll(async () => Math.min(...(await pixelsNear(page, [BULL, BEAR]))))
      .toBeGreaterThan(40);
    const frames = await page.evaluate(
      () =>
        new Promise<number>((resolve) => {
          let n = 0;
          const start = performance.now();
          const tick = () => {
            n++;
            if (performance.now() - start < 1000) requestAnimationFrame(tick);
            else resolve(n);
          };
          requestAnimationFrame(tick);
        }),
    );
    expect(frames).toBeGreaterThan(10);
    expect(errors).toEqual([]);
  });
});

test.describe("compare on a price-based chart", () => {
  test("leaves no percent axis or list entry, and returns on candles", async ({ page }) => {
    const scale = () =>
      page.evaluate(() => JSON.parse(localStorage.getItem("chartgen.layout.scaleMode") ?? "null"));
    await gotoApp(page, "/?symbol=BTCUSDT&type=renko&compare=SOLUSDT&ind=rsi");
    await expectBuilt(page, "renko");
    expect(await scale()).not.toBe("percentage");
    await page.getByRole("tab", { name: "Object Tree" }).click();
    await expect(page.getByTestId("tree-compare-SOLUSDT")).toHaveCount(0);

    await pickChartType(page, "Candles");
    await expect(page.getByTestId("tree-compare-SOLUSDT")).toBeVisible();
    await expect.poll(scale).toBe("percentage");

    await pickChartType(page, "Kagi");
    await expect(page.getByTestId("tree-compare-SOLUSDT")).toHaveCount(0);
    await expect.poll(scale).not.toBe("percentage");
  });
});

test.describe("live price-based bars", () => {
  test.use({ push: false });

  test("a tail of a new build keeps the chart updating, with its bars and history", async ({
    bootedApp: page,
  }) => {
    await pickChartType(page, "Renko");
    const loaded = await expectBuilt(page, "renko");
    expect(typeof (loaded as unknown as { anchor: unknown }).anchor).toBe("number");

    // Every tail from now on comes from a build that started later and has not reached our
    // newest bar: the shape that froze a chart until the new build caught up.
    let n = 0;
    let fullLoads = 0;
    page.on("request", (req) => {
      const a = chartArgs(req);
      if (a && a.bars !== 64 && a.end_time === undefined) fullLoads++;
    });
    await page.route("**/mcp", async (route) => {
      const a = chartArgs(route.request());
      if (!a || a.bars !== 64) return route.continue();
      const res = await route.fetch();
      const body = (await res.json()) as {
        result: { content: { text: string }[] };
      };
      const tail = JSON.parse(body.result.content[0]!.text) as Series & { anchor: number };
      tail.anchor += 1_000_000;
      const keep = tail.bars.length - 3;
      tail.bars = tail.bars.slice(0, keep);
      for (const ind of Object.values(tail.indicators) as Array<Record<string, unknown>>) {
        for (const key of ["lines", "histogram"]) {
          for (const l of (ind[key] ?? []) as Array<Record<string, unknown[]>>) {
            l.values = l.values!.slice(0, keep);
            if (l.color_idx) l.color_idx = l.color_idx.slice(0, keep);
          }
        }
        for (const f of (ind.fills ?? []) as Array<Record<string, unknown[]>>) {
          f.y1 = f.y1!.slice(0, keep);
          f.y2 = f.y2!.slice(0, keep);
        }
      }
      tail.bars.at(-1)!.c = 1000 + ++n;
      body.result.content[0]!.text = JSON.stringify(tail);
      return route.fulfill({ response: res, json: body });
    });

    const drawnClose = async () => (await drawnBars(page)).at(-1)?.close;
    await expect.poll(drawnClose, { timeout: 15_000 }).toBeGreaterThan(1000);
    const first = (await drawnClose())!;
    await expect.poll(drawnClose, { timeout: 15_000 }).toBeGreaterThan(first);

    const s = (await series(page))!;
    expect(s.bars.length).toBeLessThan(loaded.bars.length);
    expect(s.bars[0]!.t).toBe(loaded.bars[0]!.t);
    expect(s.bars.every((b, i) => i === 0 || b.t > s.bars[i - 1]!.t)).toBe(true);
    expect((await drawnBars(page)).length).toBe(s.bars.length);
    expect(fullLoads).toBe(0);
    await expect(page.getByTestId("status-text")).not.toContainText(/keeps not lining up/);
    await page.unroute("**/mcp");
  });

  test("polls keep the load's box and leave its confirmed bars alone", async ({
    bootedApp: page,
  }) => {
    const firstTail = chartRequest(
      page,
      (a) => a.chart_style === "renko" && a.bars === 64 && a.end_time === undefined,
    );
    await pickChartType(page, "Renko");
    const loaded = await expectBuilt(page, "renko");
    const size = loaded.chart_params!.box_size!;
    // An ATR box, sent back so the tail builds on it.
    const tail = chartArgs(await firstTail)!;
    expect(tail.chart_params).toMatchObject({ box: "atr", box_size: size });
    await chartRequest(page, (a) => a.chart_style === "renko" && a.bars === 64);

    const now = (await series(page))!;
    expect(now.chart_params!.box_size).toBe(size);
    const n = loaded.bars.length - 1;
    // Same bars up to the open one: a poll merged in place, with no reload.
    expect(now.bars.slice(0, n).map((b) => [b.t, b.o, b.c])).toEqual(
      loaded.bars.slice(0, n).map((b) => [b.t, b.o, b.c]),
    );
  });

  test("history pages keep the box and go in front", async ({ bootedApp: page }) => {
    await pickChartType(page, "Renko");
    const loaded = await expectBuilt(page, "renko");
    const size = loaded.chart_params!.box_size!;
    const page2 = chartRequest(page, (a) => a.end_time !== undefined && a.chart_style === "renko");
    await page.evaluate(
      () =>
        (
          window as unknown as {
            __chartgenTest: { chart: () => { scrollToDataIndex: (i: number) => void } | null };
          }
        ).__chartgenTest
          .chart()
          ?.scrollToDataIndex(0),
    );
    const args = chartArgs(await page2)!;
    expect(args.chart_params).toMatchObject({ box_size: size });
    await expect
      .poll(async () => (await series(page))!.bars.length)
      .toBeGreaterThan(loaded.bars.length);
    const s = (await series(page))!;
    expect(s.bars.every((b, i) => i === 0 || b.t > s.bars[i - 1]!.t)).toBe(true);
  });
});

test.describe("price-based bars over push", () => {
  test("the subscription carries the type and the load's box", async ({ page }) => {
    const subscribed = page.waitForRequest((req) => {
      if (req.method() !== "POST" || !req.url().endsWith("/mcp")) return false;
      const b = req.postDataJSON() as {
        params?: { name?: string; arguments?: { chart_style?: string } };
      } | null;
      return b?.params?.name === "subscribe_chart" && b.params.arguments?.chart_style === "range";
    });
    await gotoApp(page, "/?symbol=BTCUSDT&interval=1h&type=range&ind=rsi");
    const req = await subscribed;
    const args = (req.postDataJSON() as { params: { arguments: ChartArgs } }).params.arguments;
    const s = await expectBuilt(page, "range");
    expect(args.chart_params).toMatchObject({ box: "atr", box_size: s.chart_params!.box_size });
    // A pushed tail merges: still the range bars, on the same box, with no error shown.
    await expect(page.getByText(/Live|push/i).first()).toBeVisible();
    expect((await series(page))!.chart_params!.box_size).toBe(s.chart_params!.box_size);
  });
});

test.describe("price-based bars in a grid", () => {
  test("two charts of a grid hold their own type and box", async ({ bootedApp: page }) => {
    await page.getByTestId("grid-menu").click();
    await page.getByTestId("grid-option-2h").click();
    const pane = (n: number) => page.locator(`[data-testid=chart-pane][data-pane="${n}"]`);
    await expect(pane(1)).toBeVisible();
    await pane(0).locator(".chart-host").click({ position: { x: 100, y: 100 } });
    await pickChartType(page, "Renko");
    await expect(pane(0).locator(".pane-header-params").first()).toHaveText(/Renko/);
    await pane(1).locator(".chart-host").click({ position: { x: 100, y: 100 } });
    await pickChartType(page, "Point & figure");
    await expect(pane(1).locator(".pane-header-params").first()).toHaveText(/P&F/);
    await expect(pane(0).locator(".pane-header-params").first()).toHaveText(/Renko/);
    await page.screenshot({ path: `${SHOTS}/grid.png` });
  });
});

test.describe("price-based bars in the compact layout", () => {
  const { defaultBrowserType: _browser, ...iphone } = devices["iPhone 13"]!;
  test.use(iphone);

  test("the menu and the dialog fit a phone, and the chart draws", async ({ page }) => {
    // The phone hides the status text the other tests' boot waits for.
    await page.goto("/?type=point_figure&ind=rsi");
    await expect.poll(async () => (await series(page).catch(() => null))?.chart_style, {
      timeout: 20_000,
    }).toBe("point_figure");
    await expect(page.getByRole("button", { name: "Chart type: Point & figure" })).toBeVisible();
    await expect(legendParams(page)).toHaveText(/P&F/);
    await expect
      .poll(async () => Math.min(...(await pixelsNear(page, [BULL, BEAR]))))
      .toBeGreaterThan(40);
    await page.screenshot({ path: `${SHOTS}/phone-pnf.png` });
    await openSettings(page, "Point & figure");
    const dialog = page.getByTestId("chart-type-settings");
    const viewport = page.viewportSize()!;
    const rect = (await dialog.boundingBox())!;
    expect(rect.x).toBeGreaterThanOrEqual(0);
    expect(rect.x + rect.width).toBeLessThanOrEqual(viewport.width);
    expect(rect.y + rect.height).toBeLessThanOrEqual(viewport.height);
    await page.screenshot({ path: `${SHOTS}/phone-dialog.png` });
    await dialog.getByRole("button", { name: "Cancel" }).click();
    await expect(dialog).toHaveCount(0);
  });
});

test.describe("seconds intervals", () => {
  test("a Binance symbol takes 5s, and a stock's menu offers no seconds", async ({ page }) => {
    await gotoApp(page, "/?symbol=BTCUSDT&interval=5s&ind=rsi");
    await expect.poll(async () => (await series(page))?.bars.length ?? 0).toBeGreaterThan(5);
    const s = (await series(page))!;
    expect(s.bars.every((b) => b.t % 5 === 0)).toBe(true);
    await expect(legendParams(page)).toHaveText(/5s/);
    await page.screenshot({ path: `${SHOTS}/seconds.png` });
    await page.getByRole("button", { name: /^(Timeframe|Interval)|More timeframes/ }).first().click();
    await expect(page.getByRole("menuitem", { name: "15s", exact: true })).toBeVisible();
    await page.keyboard.press("Escape");

    await gotoApp(page, "/?symbol=AAPL&interval=1h&ind=rsi");
    await page.getByRole("button", { name: /^(Timeframe|Interval)|More timeframes/ }).first().click();
    await expect(page.getByRole("menuitem", { name: "30m", exact: true })).toBeVisible();
    await expect(page.getByRole("menuitem", { name: "15s", exact: true })).toHaveCount(0);
  });
});
