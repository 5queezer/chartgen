// Server alerts in a chart grid (ADR-0042, ADR-0043), on the server that runs them.
// They belong to no chart: one poll however many charts show, a fire whatever the
// charts show, and clicking one acts on the active chart alone.

import type { Page } from "@playwright/test";

import { test, expect } from "./fixtures/app";

interface PaneInfo {
  pane: number;
  symbol: string;
  bars: number;
}

const panes = (page: Page): Promise<PaneInfo[]> =>
  page.evaluate(() =>
    (window as unknown as { __chartgenTest: { panes: () => PaneInfo[] } }).__chartgenTest.panes(),
  );

const paneCell = (page: Page, n: number) =>
  page.locator(`[data-testid=chart-pane][data-pane="${n}"]`);

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

async function activate(page: Page, n: number): Promise<void> {
  const box = await paneCell(page, n).boundingBox();
  if (!box) throw new Error(`pane ${n} not laid out`);
  await page.mouse.click(box.x + box.width * 0.4, box.y + box.height * 0.35);
  await expect(paneCell(page, n)).toHaveAttribute("data-active", "");
}

async function pickSymbol(page: Page, sym: string): Promise<void> {
  await page.getByRole("tab", { name: "Watchlist" }).click();
  await page
    .getByRole("table", { name: "Watchlist" })
    .locator(`[role=row][data-symbol="${sym}"]`)
    .click();
}

const dialog = (page: Page) => page.getByRole("dialog");
const rows = (page: Page) => page.locator('[data-alert-id][data-server="true"]');
const toasts = (page: Page) => page.locator("[data-toast]").filter({ hasText: "Alert:" });

/** On the active chart's symbol, which is what the dialog takes. */
async function createServerAlert(page: Page, condition: string): Promise<void> {
  await page.getByRole("tab", { name: /^Alerts/ }).click();
  await page.getByRole("button", { name: "Create alert" }).click();
  await expect(dialog(page)).toBeVisible();
  await dialog(page).getByLabel("Run on server").check();
  await dialog(page).getByLabel("Condition").selectOption(condition);
  await dialog(page).getByLabel("Price").fill("0.01");
  await dialog(page).getByRole("button", { name: "Create" }).click();
  await expect(dialog(page)).toBeHidden();
}

test("one poll of the server's alerts, however many charts show", async ({ bootedApp: page }) => {
  let polls = 0;
  page.on("request", (r) => {
    if ((r.postData() ?? "").includes('"list_server_alerts"')) polls++;
  });
  await chooseGrid(page, "4", 4);
  polls = 0;
  // Every 5 s from the app, so at most 3 in 11 s; 4 charts each polling would be 8 or more.
  await page.waitForTimeout(11_000);
  expect(polls).toBeGreaterThan(0);
  expect(polls).toBeLessThanOrEqual(3);

  // Changing the grid adds none and stops none.
  await chooseGrid(page, "2h", 2);
  polls = 0;
  await page.waitForTimeout(11_000);
  expect(polls).toBeGreaterThan(0);
  expect(polls).toBeLessThanOrEqual(3);
});

test("a server alert stays listed and fires with its symbol on no chart", async ({ bootedApp: page }) => {
  await chooseGrid(page, "2h", 2);
  await activate(page, 1);
  await pickSymbol(page, "ETHUSDT");
  await expect.poll(async () => (await panes(page))[1]!.symbol).toBe("ETHUSDT");

  // Not reached at this level, so it stays listed through what follows.
  await createServerAlert(page, "less");
  const row = rows(page);
  await expect(row).toHaveCount(1);
  await expect(row).not.toContainText("inactive");

  // On a chart that is not active, then on no chart at all.
  await activate(page, 0);
  await expect(row).toHaveCount(1);
  await expect(row).not.toContainText("inactive");
  page.once("dialog", (d) => void d.accept());
  await chooseGrid(page, "1", 1);
  expect((await panes(page))[0]!.symbol).toBe("BTCUSDT");
  await expect(row).toHaveCount(1);
  await expect(row).not.toContainText("inactive");

  // A level the price is below at once: the server fires it, with no chart on the symbol.
  await row.hover();
  await row.getByRole("button", { name: "Edit alert" }).click();
  await dialog(page).getByLabel("Price").fill("1000000");
  await dialog(page).getByRole("button", { name: "Save" }).click();
  await expect(toasts(page).filter({ hasText: "ETHUSDT" })).toHaveCount(1, { timeout: 20_000 });
  expect((await panes(page))[0]!.symbol).toBe("BTCUSDT");
});

test("clicking a server alert puts its symbol on the active chart alone", async ({ bootedApp: page }) => {
  await chooseGrid(page, "2h", 2);
  await activate(page, 1);
  await pickSymbol(page, "SOLUSDT");
  await expect.poll(async () => (await panes(page))[1]!.symbol).toBe("SOLUSDT");
  await createServerAlert(page, "less");
  await pickSymbol(page, "ETHUSDT");
  await expect.poll(async () => (await panes(page))[1]!.symbol).toBe("ETHUSDT");

  await page.getByRole("tab", { name: /^Alerts/ }).click();
  await rows(page).click();
  await expect.poll(async () => (await panes(page))[1]!.symbol).toBe("SOLUSDT");
  expect((await panes(page))[0]!.symbol).toBe("BTCUSDT");
});
