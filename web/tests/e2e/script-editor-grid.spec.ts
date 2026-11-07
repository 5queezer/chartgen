// The script editor in a chart grid (ADR-0041, ADR-0043), on the `scripting` project
// of playwright.config.ts. Each effect belongs to its chart: a saved script reloads
// the charts using it, the rest keep their view, errors follow the active chart.

import type { Page } from "@playwright/test";

import { expect, gotoApp, test } from "./fixtures/app";

test.use({ viewport: { width: 1600, height: 1100 } });

interface PaneInfo {
  pane: number;
  timeframe: string;
  indicators: unknown[];
  bars: number;
  candleHeight: number | null;
  stackHeight: number | null;
  staleCanvases: number;
}

const panes = (page: Page): Promise<PaneInfo[]> =>
  page.evaluate(() =>
    (window as unknown as { __chartgenTest: { panes: () => PaneInfo[] } }).__chartgenTest.panes(),
  );

const paneCell = (page: Page, n: number) =>
  page.locator(`[data-testid=chart-pane][data-pane="${n}"]`);

async function activate(page: Page, n: number): Promise<void> {
  const box = await paneCell(page, n).boundingBox();
  if (!box) throw new Error(`pane ${n} not laid out`);
  await page.mouse.click(box.x + box.width * 0.4, box.y + box.height * 0.35);
  await expect(paneCell(page, n)).toHaveAttribute("data-active", "");
}

async function chooseGrid(page: Page, id: string, count: number): Promise<void> {
  await page.getByTestId("grid-menu").click();
  await page.getByTestId(`grid-option-${id}`).click();
  await expect
    .poll(async () => {
      const list = await panes(page);
      return list.length === count && list.every((p) => p.bars > 0);
    }, { timeout: 30_000 })
    .toBe(true);
}

/**
 * A value of the first plot of `key`, ten bars back. Live tails update the newest
 * bars by themselves, so only a full reload changes this one.
 */
const loadedValue = (page: Page, pane: number, key: string) =>
  page.evaluate(
    ([n, k]) => {
      const test = (window as unknown as {
        __chartgenTest: {
          paneSeries: (p: number) => {
            indicators: Record<string, { lines?: Array<{ values: Array<number | null> }> }>;
          } | null;
        };
      }).__chartgenTest;
      return test.paneSeries(n as number)?.indicators[k as string]?.lines?.[0]?.values.at(-10) ?? null;
    },
    [pane, key] as const,
  );

const script = (body: string) => `PARAMS = []
META = {"title": "Test script", "overlay": False}

def compute(ohlcv, params, chart):
${body}
`;

const code = (page: Page) => page.locator("[data-testid=script-code] .cm-content");

async function openEditor(page: Page): Promise<void> {
  await page.getByTestId("script-editor-open").click();
  await expect(page.getByTestId("script-editor")).toBeVisible({ timeout: 30_000 });
  await expect(code(page)).toContainText("PARAMS");
}

async function saveScript(page: Page, name: string, body: string): Promise<void> {
  await page.getByTestId("script-name").fill(name);
  await setCode(page, body);
  await page.getByTestId("script-save").click();
  await expect(page.getByTestId("script-status")).toHaveText("Saved.");
}

async function setCode(page: Page, body: string): Promise<void> {
  await code(page).click();
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.insertText(script(body));
}

async function pageBack(page: Page, n: number): Promise<number> {
  const box = await paneCell(page, n).boundingBox();
  if (!box) throw new Error(`pane ${n} not laid out`);
  const y = box.y + box.height * 0.2;
  const start = (await panes(page))[n]!.bars;
  await expect
    .poll(
      async () => {
        await page.mouse.move(box.x + 40, y);
        await page.mouse.down();
        await page.mouse.move(box.x + box.width * 0.8, y, { steps: 6 });
        await page.mouse.up();
        return (await panes(page))[n]!.bars;
      },
      { timeout: 25_000 },
    )
    .toBeGreaterThan(start);
  return (await panes(page))[n]!.bars;
}

test("saving a script reloads the charts using it and no other", async ({ page }) => {
  await gotoApp(page);
  await chooseGrid(page, "4", 4);
  await openEditor(page);
  await saveScript(page, "shared_one", '    chart.line(100.0, title = "Alpha")');

  // Two charts on one request, a third on another interval, a fourth without it.
  await activate(page, 0);
  await page.getByTestId("script-add").click();
  await activate(page, 1);
  await page.getByTestId("script-add").click();
  await activate(page, 2);
  await page.getByRole("button", { name: "15m", exact: true }).click();
  await page.getByTestId("script-add").click();
  for (const n of [0, 1, 2]) {
    await expect.poll(() => loadedValue(page, n, "shared_one"), { timeout: 30_000 }).toBe(100);
  }
  const untouched = await pageBack(page, 3);

  await setCode(page, '    chart.line(200.0, title = "Beta")');
  await page.keyboard.press("ControlOrMeta+s");
  for (const n of [0, 1, 2]) {
    await expect.poll(() => loadedValue(page, n, "shared_one"), { timeout: 30_000 }).toBe(200);
  }
  await page.waitForTimeout(500);
  expect((await panes(page))[3]!.bars).toBe(untouched);

  // A key refreshed while no chart was on it: back on it, the chart gets the new result.
  await activate(page, 2);
  await page.getByRole("button", { name: "4h", exact: true }).click();
  await expect.poll(() => loadedValue(page, 2, "shared_one"), { timeout: 30_000 }).toBe(200);
  await setCode(page, '    chart.line(300.0, title = "Gamma")');
  await page.keyboard.press("ControlOrMeta+s");
  await expect.poll(() => loadedValue(page, 2, "shared_one"), { timeout: 30_000 }).toBe(300);
  await page.getByRole("button", { name: "15m", exact: true }).click();
  await expect.poll(() => loadedValue(page, 2, "shared_one"), { timeout: 30_000 }).toBe(300);
});

test("deleting a script takes it off every chart", async ({ page }) => {
  await gotoApp(page);
  await chooseGrid(page, "2h", 2);
  await openEditor(page);
  await saveScript(page, "doomed_one", '    chart.line(100.0, title = "Alpha")');
  await activate(page, 0);
  await page.getByTestId("script-add").click();
  await activate(page, 1);
  await page.getByTestId("script-add").click();
  for (const n of [0, 1]) {
    await expect.poll(() => loadedValue(page, n, "doomed_one"), { timeout: 30_000 }).toBe(100);
  }

  await page.getByTestId("script-delete").click();
  await page.getByTestId("script-delete-confirm").click();
  const has = (p: PaneInfo) => JSON.stringify(p.indicators).includes("doomed_one");
  await expect.poll(async () => (await panes(page)).map(has)).toEqual([false, false]);
  await expect(page.getByTestId("chart-load-error")).toHaveCount(0);
});

test("a failing script marks the chips and the editor of the chart it fails on", async ({ page }) => {
  await gotoApp(page);
  await chooseGrid(page, "2h", 2);
  await openEditor(page);
  // The same indicator on both charts, failing only on 1h bars. The dry run has 100
  // bars, so it registers.
  await saveScript(
    page,
    "fails_late",
    "    t = ohlcv.time.list()\n    if len(t) > 100 and t[-1] - t[-2] == 3600:\n        y = 1 // 0\n    chart.line(ohlcv.close)",
  );
  await activate(page, 0);
  await page.getByTestId("script-add").click();
  await activate(page, 1);
  await page.getByRole("button", { name: "4h", exact: true }).click();
  await page.getByTestId("script-add").click();
  await expect.poll(() => loadedValue(page, 1, "fails_late"), { timeout: 30_000 }).not.toBeNull();

  const failedChips = page.locator("[data-chip-error]");
  const errors = page.getByTestId("script-errors");
  // Healthy on this chart: a legend row with its controls, and no chip.
  const row = page.locator(".chart-pane[data-active]").getByTestId("indicator-legend-fails_late");
  await expect(row.getByRole("button", { name: /^Remove / })).toBeVisible();
  await expect(failedChips).toHaveCount(0);
  await expect(errors).toHaveCount(0);

  await activate(page, 0);
  await expect(failedChips).toHaveCount(1, { timeout: 30_000 });
  await expect(errors).toContainText("On the chart");
  expect(await loadedValue(page, 0, "fails_late")).toBeNull();

  await activate(page, 1);
  await expect(failedChips).toHaveCount(0);
  await expect(errors).toHaveCount(0);
});

test("the dock in a grid: charts fit, and closing it keeps them fitted to their cells", async ({ page }) => {
  // Short enough that sub-panes at their full height would crush the price pane.
  await page.setViewportSize({ width: 1280, height: 720 });
  await gotoApp(page);
  const fullCandle = (await panes(page))[0]!.candleHeight;
  await chooseGrid(page, "2v", 2);
  const healthy = async () =>
    (await panes(page)).map(
      (p) => p.staleCanvases === 0 && (p.candleHeight ?? 0) >= (p.stackHeight ?? 1) * 0.4,
    );

  await openEditor(page);
  await expect.poll(healthy).toEqual([true, true]);
  await page.getByTestId("script-editor-open").click();
  await expect(page.getByTestId("script-editor")).toHaveCount(0);
  await page.waitForTimeout(300);
  await expect.poll(healthy).toEqual([true, true]);

  // The dock did not cost the chart its usual heights.
  await chooseGrid(page, "1", 1);
  await expect.poll(async () => (await panes(page))[0]!.candleHeight).toBe(fullCandle);
});
