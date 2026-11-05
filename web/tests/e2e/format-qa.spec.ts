// Number, time and pane-order formatting found by QA: overlay legends in the
// symbol's price precision, date-only crosshair labels for daily bars, K/M/B
// volume axes and a stable pane order across reloads.

import type { Page } from "@playwright/test";

import { test, expect, reloadApp } from "./fixtures/app";

type Store = {
  getCustomApi(): {
    formatDate(dtf: Intl.DateTimeFormat, ts: number, fmt: string, type: number): string;
    formatBigNumber(v: string | number): string;
  };
};
type TestWindow = {
  __chartgenTest: {
    chart: () => {
      getChartStore(): Store;
      getIndicatorByPaneId(
        paneId: string,
        name?: string,
      ): Map<string, { shouldFormatBigNumber: boolean }> | { shouldFormatBigNumber: boolean } | null;
    } | null;
  };
};

const legend = (page: Page, key: string) => page.getByTestId(`indicator-legend-${key}`);

/** Top of each indicator legend in px, to compare their order. */
async function tops(page: Page, keys: string[]): Promise<number[]> {
  const out: number[] = [];
  for (const k of keys) {
    const box = await legend(page, k).boundingBox();
    if (!box) throw new Error(`no legend for ${k}`);
    out.push(box.y);
  }
  return out;
}

const ascending = (xs: number[]) => xs.every((x, i) => i === 0 || x > xs[i - 1]!);

test.describe("format qa", () => {
  test("overlay legend values use the price precision, not K", async ({ bootedApp: page }) => {
    const close = (await page.getByTestId("legend-c").textContent())!;
    const decimals = close.split(".")[1]!.length;
    const ema = legend(page, "ema_stack").locator(".pane-header-value");
    await expect(ema.first()).toBeVisible();
    const texts = await ema.allTextContents();
    expect(texts.length).toBeGreaterThan(0);
    for (const t of texts) {
      expect(t).toMatch(new RegExp(`^EMA \\d+ -?\\d+\\.\\d{${decimals}}$`));
    }
  });

  test("daily crosshair label has no time; intraday keeps it", async ({ bootedApp: page }) => {
    const label = (p: Page) =>
      p.evaluate(() => {
        const c = (window as unknown as TestWindow).__chartgenTest.chart()!;
        const api = c.getChartStore().getCustomApi();
        const dtf = new Intl.DateTimeFormat("en", { timeZone: "UTC", hour12: false, year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" });
        return api.formatDate(dtf, Date.UTC(2026, 2, 13, 1, 0), "YYYY-MM-DD HH:mm", 1);
      });
    expect(await label(page)).toMatch(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}$/);
    await page.getByLabel("Chart toolbar").getByRole("button", { name: "1D", exact: true }).click();
    await expect(page.getByTestId("legend-time")).toHaveText(/^\d{4}-\d{2}-\d{2}$/);
    expect(await label(page)).toBe("2026-03-13");
  });

  test("volume pane axis is K/M/B abbreviated", async ({ bootedApp: page }) => {
    const r = await page.evaluate(() => {
      const c = (window as unknown as TestWindow).__chartgenTest.chart()!;
      const api = c.getChartStore().getCustomApi();
      return {
        big: api.formatBigNumber("180000000.00"),
        small: api.formatBigNumber("4200.00"),
      };
    });
    expect(r.big).toBe("180M");
    expect(r.small).toBe("4.2K");
    await expect(legend(page, "volume")).toBeVisible();
  });

  test("pane order follows the stored order, also after a reload", async ({ bootedApp: page }) => {
    await page.evaluate(() =>
      localStorage.setItem(
        "chartgen.indicators.active",
        JSON.stringify(["ema_stack", "rsi", "volume"]),
      ),
    );
    await reloadApp(page);
    await expect(legend(page, "volume")).toBeVisible();

    await page.getByRole("button", { name: /Indicators/ }).click();
    const dialog = page.getByRole("dialog");
    await dialog.getByPlaceholder("Search indicators...").fill("macd");
    await dialog
      .locator("li button[type='button']")
      .filter({ has: page.locator('[title="macd"]') })
      .first()
      .click();
    await dialog.getByRole("button", { name: "Close" }).click();
    await expect(dialog).toBeHidden();

    const keys = ["rsi", "volume", "macd"];
    await expect(legend(page, "macd")).toBeVisible();
    await expect.poll(async () => ascending(await tops(page, keys))).toBe(true);

    await reloadApp(page);
    await expect(legend(page, "macd")).toBeVisible();
    await expect.poll(async () => ascending(await tops(page, keys))).toBe(true);
  });
});
