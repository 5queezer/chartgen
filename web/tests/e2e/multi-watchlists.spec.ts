// Multiple named watchlists, sections and color flags (ADR-0031): the list
// switcher, sections with drag and Alt+Arrow reordering, flags and the
// Flagged view, persistence across reload, and migration of the old
// single-list storage.

import type { Page } from "@playwright/test";

import { test, expect, reloadApp } from "./fixtures/app";

const table = (page: Page) => page.getByRole("table", { name: "Watchlist" });
const row = (page: Page, sym: string) =>
  table(page).locator(`[role=row][data-symbol="${sym}"]`);
const rows = (page: Page) => table(page).locator("[role=row][data-symbol]");
const lastCell = (page: Page, sym: string) =>
  row(page, sym).getByRole("cell").nth(1);
const switcher = (page: Page) =>
  page.getByRole("button", { name: /^Watchlist: / });
const listName = (page: Page) => page.getByTestId("watchlist-name");

const NUMBER = /^[+−]?\d+\.\d+%?$/;

/** Row order on screen: symbols and `[section]` markers. */
async function order(page: Page): Promise<string[]> {
  return table(page)
    .locator("[data-symbol], [data-section]")
    .evaluateAll((els) =>
      els.map((el) =>
        el.getAttribute("data-symbol") ??
        `[${el.querySelector("button")?.textContent?.trim().replace(/^[▾▸]\s*/, "")}]`,
      ),
    );
}

async function menu(page: Page, item: string | RegExp) {
  await switcher(page).click();
  await page.getByRole("menuitem", { name: item }).click();
}

async function nameDialog(page: Page, label: string, name: string) {
  const input = page.getByRole("textbox", { name: label });
  await input.fill(name);
  await input.press("Enter");
  await expect(page.getByRole("dialog")).toHaveCount(0);
}

async function addSymbol(page: Page, query: string) {
  await page.getByRole("button", { name: "Add symbol" }).click();
  const input = page.getByRole("dialog").getByRole("combobox");
  await input.fill(query);
  await page.keyboard.press("Enter");
  await expect(page.getByRole("dialog")).toHaveCount(0);
}

async function addSection(page: Page, name: string) {
  await page.getByRole("button", { name: "Add section" }).click();
  await nameDialog(page, "Section name", name);
}

async function flag(page: Page, sym: string, color: string) {
  await row(page, sym).click({ button: "right" });
  await page.getByRole("menuitem", { name: "Flag…" }).click();
  await page.getByRole("menuitem", { name: `Flag ${color}` }).click();
}

test.describe("multiple watchlists", () => {
  test("create a second list, add a symbol and switch lists", async ({
    bootedApp: page,
  }) => {
    await expect(listName(page)).toHaveText("Watchlist");
    await expect(rows(page)).toHaveCount(7);

    await menu(page, "Create new list…");
    await nameDialog(page, "List name", "Crypto");
    await expect(listName(page)).toHaveText("Crypto");
    await expect(rows(page)).toHaveCount(0);

    await addSymbol(page, "doge");
    await expect(rows(page)).toHaveCount(1);
    await expect(lastCell(page, "DOGEUSDT")).toHaveText(NUMBER, {
      timeout: 15_000,
    });

    await menu(page, "Watchlist");
    await expect(listName(page)).toHaveText("Watchlist");
    await expect(rows(page)).toHaveCount(7);
    await expect(row(page, "DOGEUSDT")).toHaveCount(0);
    await expect(lastCell(page, "AAPL")).toHaveText(NUMBER, {
      timeout: 15_000,
    });

    await reloadApp(page);
    await expect(listName(page)).toHaveText("Watchlist");
    await menu(page, "Crypto");
    await expect(rows(page)).toHaveCount(1);
    await expect(row(page, "DOGEUSDT")).toHaveCount(1);
    await reloadApp(page);
    await expect(listName(page)).toHaveText("Crypto");
  });

  test("rename, duplicate and delete with confirmation; the last list stays", async ({
    bootedApp: page,
  }) => {
    await switcher(page).click();
    await expect(page.getByRole("menuitem", { name: "Delete…" })).toHaveAttribute(
      "data-disabled",
      "",
    );
    await page.keyboard.press("Escape");

    await menu(page, "Rename…");
    await nameDialog(page, "List name", "Main");
    await expect(listName(page)).toHaveText("Main");

    await menu(page, "Duplicate");
    await expect(listName(page)).toHaveText("Main copy");
    await expect(rows(page)).toHaveCount(7);

    await menu(page, "Delete…");
    await page.getByRole("button", { name: "Cancel" }).click();
    await expect(listName(page)).toHaveText("Main copy");
    await menu(page, "Delete…");
    await page.getByRole("button", { name: "Delete", exact: true }).click();
    await expect(listName(page)).toHaveText("Main");
  });

  test("sections: add, move a symbol under it by drag and Alt+Arrow, collapse, persist", async ({
    bootedApp: page,
  }) => {
    await addSection(page, "Tech");
    await expect.poll(() => order(page)).toEqual([
      "BTCUSDT",
      "ETHUSDT",
      "SOLUSDT",
      "AAPL",
      "MSFT",
      "NVDA",
      "SPY",
      "[Tech]",
    ]);

    // Drag SPY above the section header: stays above it.
    // Drag AAPL below the header: joins the section.
    const grip = row(page, "AAPL").getByRole("button", {
      name: "Drag AAPL to reorder",
    });
    await row(page, "AAPL").hover();
    const g = (await grip.boundingBox())!;
    const header = (await page
      .locator("[data-section]")
      .boundingBox())!;
    await page.mouse.move(g.x + g.width / 2, g.y + g.height / 2);
    await page.mouse.down();
    await page.mouse.move(header.x + 40, header.y + header.height - 2, {
      steps: 8,
    });
    await page.mouse.up();
    await expect.poll(() => order(page)).toEqual([
      "BTCUSDT",
      "ETHUSDT",
      "SOLUSDT",
      "MSFT",
      "NVDA",
      "SPY",
      "[Tech]",
      "AAPL",
    ]);

    // Keyboard: Alt+ArrowUp on the focused row moves it over the header.
    await row(page, "AAPL").focus();
    await page.keyboard.press("Alt+ArrowUp");
    await expect.poll(async () => (await order(page)).slice(-2)).toEqual(["AAPL", "[Tech]"]);
    await expect(row(page, "AAPL")).toBeFocused();
    await page.keyboard.press("Alt+ArrowDown");
    await expect.poll(async () => (await order(page)).slice(-2)).toEqual(["[Tech]", "AAPL"]);

    // Collapse hides the section's symbols; persists across reload.
    await page.getByRole("button", { name: "Section Tech", exact: true }).click();
    await expect(row(page, "AAPL")).toHaveCount(0);
    await expect(rows(page)).toHaveCount(6);
    await reloadApp(page);
    await expect.poll(() => order(page)).toEqual([
      "BTCUSDT",
      "ETHUSDT",
      "SOLUSDT",
      "MSFT",
      "NVDA",
      "SPY",
      "[Tech]",
    ]);
    await page.getByRole("button", { name: "Section Tech", exact: true }).click();
    await expect(row(page, "AAPL")).toHaveCount(1);

    // Rename and remove (symbols stay).
    await page.locator("[data-section]").hover();
    await page.getByRole("button", { name: "Rename section Tech" }).click();
    await nameDialog(page, "Section name", "Stocks");
    await page.locator("[data-section]").hover();
    await page.getByRole("button", { name: "Remove section Stocks" }).click();
    await expect(page.locator("[data-section]")).toHaveCount(0);
    await expect(rows(page)).toHaveCount(7);
  });

  test("flags: set from the context menu, Flagged view lists them across lists", async ({
    bootedApp: page,
  }) => {
    await flag(page, "BTCUSDT", "red");
    await expect(row(page, "BTCUSDT")).toHaveAttribute("data-flag", "red");
    await flag(page, "AAPL", "blue");

    // Same symbol in a second list shows the same flag.
    await menu(page, "Create new list…");
    await nameDialog(page, "List name", "Other");
    await addSymbol(page, "aapl");
    await expect(row(page, "AAPL")).toHaveAttribute("data-flag", "blue");

    await menu(page, "Flagged");
    await expect(listName(page)).toHaveText("Flagged");
    await expect.poll(() => order(page)).toEqual(["BTCUSDT", "AAPL"]);
    await expect(lastCell(page, "BTCUSDT")).toHaveText(NUMBER, {
      timeout: 15_000,
    });
    // Read-only: no adding, no sections, no remove or drag handles.
    await expect(page.getByRole("button", { name: "Add symbol" })).toBeDisabled();
    await expect(page.getByRole("button", { name: "Add section" })).toBeDisabled();
    await expect(
      page.getByRole("button", { name: /^Remove BTCUSDT/ }),
    ).toHaveCount(0);

    await reloadApp(page);
    await expect(listName(page)).toHaveText("Flagged");
    await expect.poll(() => order(page)).toEqual(["BTCUSDT", "AAPL"]);

    // Clearing a flag drops the row from the view.
    await flag(page, "AAPL", "red");
    await row(page, "AAPL").click({ button: "right" });
    await page.getByRole("menuitem", { name: "Flag…" }).click();
    await page.getByRole("menuitem", { name: "No flag" }).click();
    await expect.poll(() => order(page)).toEqual(["BTCUSDT"]);
  });

  test("Escape cancels a drag; the flag menu is keyboard-operable and returns focus", async ({
    bootedApp: page,
  }) => {
    const before = await order(page);
    await row(page, "AAPL").hover();
    const g = (await row(page, "AAPL")
      .getByRole("button", { name: "Drag AAPL to reorder" })
      .boundingBox())!;
    const top = (await row(page, "BTCUSDT").boundingBox())!;
    await page.mouse.move(g.x + g.width / 2, g.y + g.height / 2);
    await page.mouse.down();
    await page.mouse.move(top.x + 40, top.y + 2, { steps: 6 });
    await expect(row(page, "BTCUSDT")).toHaveClass(/wl-drop-before/);
    await page.keyboard.press("Escape");
    await page.mouse.up();
    await expect(row(page, "BTCUSDT")).not.toHaveClass(/wl-drop-before/);
    expect(await order(page)).toEqual(before);

    await row(page, "ETHUSDT").focus();
    await row(page, "ETHUSDT").click({ button: "right" });
    await expect(page.getByRole("menu", { name: "ETHUSDT actions" })).toBeFocused();
    await page.keyboard.press("ArrowDown");
    await page.keyboard.press("ArrowDown");
    await page.keyboard.press("ArrowDown");
    await expect(page.getByRole("menuitem", { name: "Flag…" })).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(page.getByRole("menuitem", { name: "Flag red" })).toBeFocused();
    await page.keyboard.press("ArrowRight");
    await expect(page.getByRole("menuitem", { name: "Flag orange" })).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(row(page, "ETHUSDT")).toHaveAttribute("data-flag", "orange");
    await expect(row(page, "ETHUSDT")).toBeFocused();
  });

  test("the old single-list storage migrates into 'Watchlist'", async ({
    bootedApp: page,
  }) => {
    await page.evaluate(() => {
      localStorage.removeItem("chartgen.watchlist.v2");
      localStorage.setItem(
        "chartgen.watchlist.symbols",
        JSON.stringify(["msft", "ETHUSDT"]),
      );
    });
    await reloadApp(page);
    await expect(listName(page)).toHaveText("Watchlist");
    await expect.poll(() => order(page)).toEqual(["MSFT", "ETHUSDT"]);
    await expect(lastCell(page, "MSFT")).toHaveText(NUMBER, {
      timeout: 15_000,
    });
    await addSymbol(page, "doge");
    await reloadApp(page);
    await expect.poll(() => order(page)).toEqual(["MSFT", "ETHUSDT", "DOGEUSDT"]);
  });
});
