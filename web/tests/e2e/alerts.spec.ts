// Price alerts (ADR-0020): create from Alt+A / the Alerts tab, fire on live
// ticks of the sample source (toast + Notification), once vs every-time,
// edit / pause / delete, dragging the chart line, and persistence.
//
// The sample source's forming bar wobbles by ±0.3 % every second, so a
// "crossing" alert at the current price fires within a few ticks.
// Set ALERT_SHOTS_DIR to also write screenshots there.

import type { Page } from "@playwright/test";

import { test, expect } from "./fixtures/app";

const shot = async (page: Page, name: string) => {
  const dir = process.env.ALERT_SHOTS_DIR;
  if (dir) await page.screenshot({ path: `${dir}/${name}.png` });
};

const dialog = (page: Page) => page.getByRole("dialog");
const priceInput = (page: Page) => dialog(page).getByLabel("Price");
const alertsTab = (page: Page) => page.getByRole("tab", { name: /^Alerts/ });
const rows = (page: Page) => page.locator("[data-alert-id]");
const toasts = (page: Page) => page.locator("[data-toast]");

type TestWindow = {
  __chartgenTest: { series: () => { bars: { c: number; h: number }[] } | null };
  __chartgenAlertLines: () => { alertId: string; value: number; y: number }[];
};

/** Last close of the chart's live series. */
async function lastClose(page: Page): Promise<number> {
  await expect
    .poll(() =>
      page.evaluate(() => {
        const s = (window as unknown as TestWindow).__chartgenTest.series();
        return s?.bars.at(-1)?.c ?? null;
      }),
    )
    .not.toBeNull();
  return page.evaluate(
    () => (window as unknown as TestWindow).__chartgenTest.series()!.bars.at(-1)!.c,
  );
}

/**
 * A level inside the forming bar's wobble: the mean of a few ticks. A level
 * taken from one tick can sit at the extreme of the wobble and never be
 * crossed again.
 */
async function wobbleMid(page: Page): Promise<number> {
  const samples: number[] = [];
  for (let i = 0; i < 4; i++) {
    samples.push(await lastClose(page));
    await page.waitForTimeout(2_100);
  }
  return samples.reduce((a, b) => a + b, 0) / samples.length;
}

const lineCount = (page: Page) =>
  page.evaluate(
    () => (window as unknown as TestWindow).__chartgenAlertLines().length,
  );

/** Notifications the page created, with permission pre-granted. */
async function mockNotifications(page: Page) {
  await page.evaluate(() => {
    const w = window as unknown as { __notes: string[] };
    w.__notes = [];
    class FakeNotification {
      static permission = "granted";
      static requestPermission = () => Promise.resolve("granted");
      constructor(_title: string, opts?: { body?: string }) {
        w.__notes.push(opts?.body ?? "");
      }
    }
    Object.defineProperty(window, "Notification", {
      configurable: true,
      value: FakeNotification,
    });
  });
}

async function createAlert(
  page: Page,
  opts: { condition?: string; price?: number; trigger?: string; message?: string },
) {
  await page.keyboard.press("Alt+KeyA");
  await expect(dialog(page)).toBeVisible();
  if (opts.condition) await dialog(page).getByLabel("Condition").selectOption(opts.condition);
  if (opts.price !== undefined) await priceInput(page).fill(String(opts.price));
  if (opts.trigger) await dialog(page).getByLabel("Trigger").selectOption(opts.trigger);
  if (opts.message) await dialog(page).getByLabel("Message").fill(opts.message);
  await dialog(page).getByRole("button", { name: "Create" }).click();
  await expect(dialog(page)).toHaveCount(0);
}

test.describe("price alerts", () => {
  test("Alt+A opens the dialog prefilled with the last price", async ({
    bootedApp: page,
  }) => {
    const last = await lastClose(page);
    await page.keyboard.press("Alt+KeyA");
    await expect(dialog(page)).toBeVisible();
    await expect(dialog(page)).toContainText("Create alert");
    const value = Number(await priceInput(page).inputValue());
    expect(Math.abs(value - last) / last).toBeLessThan(0.01);
    await expect(dialog(page).getByLabel("Condition")).toHaveValue("crossing");
    await expect(dialog(page).getByLabel("Trigger")).toHaveValue("once");
    await shot(page, "create-dialog");

    // Invalid price is rejected in place.
    await priceInput(page).fill("0");
    await dialog(page).getByRole("button", { name: "Create" }).click();
    await expect(dialog(page).getByRole("alert")).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(dialog(page)).toHaveCount(0);

    // Not while typing in a field.
    await page.getByRole("button", { name: /^Symbol:/ }).click();
    await page.keyboard.press("Alt+KeyA");
    await expect(dialog(page).getByText("Create alert")).toHaveCount(0);
  });

  test("a once alert fires with a toast and a notification, then deactivates", async ({
    bootedApp: page,
  }) => {
    test.setTimeout(120_000);
    await mockNotifications(page);
    await createAlert(page, { price: await wobbleMid(page), message: "level hit" });
    await expect(toasts(page).filter({ hasText: "Alert created" })).toBeVisible();
    await expect.poll(() => lineCount(page)).toBe(1);
    await shot(page, "alert-line");

    await alertsTab(page).click();
    await expect(rows(page)).toHaveCount(1);
    await expect(rows(page)).toHaveAttribute("data-status", "active");
    await shot(page, "alerts-tab");

    await expect(toasts(page).filter({ hasText: /^Alert: .*level hit/ })).toBeVisible({
      timeout: 60_000,
    });
    await shot(page, "toast");
    await expect(rows(page)).toHaveAttribute("data-status", "triggered");
    await expect.poll(() => lineCount(page)).toBe(0);
    const notes = await page.evaluate(
      () => (window as unknown as { __notes: string[] }).__notes,
    );
    expect(notes).toHaveLength(1);
    expect(notes[0]).toContain("level hit");

    // Once: it does not fire again.
    await page.waitForTimeout(5_000);
    await expect(toasts(page).filter({ hasText: /^Alert: / })).toHaveCount(0);
  });

  test("an every-time alert keeps firing and stays active", async ({
    bootedApp: page,
  }) => {
    test.setTimeout(150_000);
    await createAlert(page, { price: await wobbleMid(page), trigger: "every" });
    await alertsTab(page).click();
    await expect(toasts(page).filter({ hasText: /^Alert: / }).first()).toBeVisible({
      timeout: 60_000,
    });
    await expect(rows(page)).toHaveAttribute("data-status", "active");
    await expect.poll(() => lineCount(page)).toBe(1);
    // It fires again on a later cross: lastTriggeredAt advances.
    const triggeredAt = () =>
      page.evaluate(
        () =>
          (JSON.parse(localStorage.getItem("chartgen.alerts.list")!) as {
            lastTriggeredAt: number;
          }[])[0]!.lastTriggeredAt,
      );
    const first = await triggeredAt();
    await expect.poll(triggeredAt, { timeout: 60_000 }).toBeGreaterThan(first);
  });

  test("edit, pause, resume and delete from the Alerts tab", async ({
    bootedApp: page,
  }) => {
    const far = Math.round((await lastClose(page)) * 3);
    await alertsTab(page).click();
    await page.getByRole("button", { name: "Create alert" }).click();
    await expect(dialog(page)).toBeVisible();
    await dialog(page).getByLabel("Condition").selectOption("greater");
    await priceInput(page).fill(String(far));
    await dialog(page).getByRole("button", { name: "Create" }).click();
    await expect(rows(page)).toHaveCount(1);
    await expect(rows(page)).toContainText("Greater than");

    await rows(page).getByRole("button", { name: "Edit alert" }).click();
    await expect(dialog(page)).toContainText("Edit alert");
    await expect(priceInput(page)).toHaveValue(String(far));
    await dialog(page).getByLabel("Condition").selectOption("less");
    await priceInput(page).fill(String(far + 1));
    await dialog(page).getByLabel("Trigger").selectOption("every");
    await dialog(page).getByRole("button", { name: "Save" }).click();
    await expect(rows(page)).toContainText("Less than");
    await expect(rows(page)).toContainText(String(far + 1));
    await expect(rows(page)).toContainText("Every time");

    await rows(page).getByRole("button", { name: "Pause alert" }).click();
    await expect(rows(page)).toHaveAttribute("data-status", "paused");
    await expect.poll(() => lineCount(page)).toBe(0);
    await rows(page).getByRole("button", { name: "Resume alert" }).click();
    await expect(rows(page)).toHaveAttribute("data-status", "active");
    await expect.poll(() => lineCount(page)).toBe(1);

    await rows(page).getByRole("button", { name: "Delete alert" }).click();
    await expect(rows(page)).toHaveCount(0);
    await expect.poll(() => lineCount(page)).toBe(0);
  });

  test("alerts and the selected tab persist across a reload", async ({
    bootedApp: page,
  }) => {
    const far = Math.round((await lastClose(page)) * 3);
    await createAlert(page, { condition: "greater", price: far, message: "far away" });
    await alertsTab(page).click();
    await expect(rows(page)).toHaveCount(1);

    await page.reload();
    await expect(alertsTab(page)).toHaveAttribute("aria-selected", "true", {
      timeout: 30_000,
    });
    await expect(rows(page)).toHaveCount(1);
    await expect(rows(page)).toContainText("far away");
    await expect(rows(page)).toHaveAttribute("data-status", "active");
    await expect.poll(() => lineCount(page), { timeout: 20_000 }).toBe(1);
  });

  test("dragging an alert line moves its level; clicking a row switches symbol", async ({
    bootedApp: page,
  }) => {
    const level = await page.evaluate(() => {
      const bars = (window as unknown as TestWindow).__chartgenTest.series()!.bars;
      return Math.max(...bars.slice(-30).map((b) => b.h));
    });
    await createAlert(page, { condition: "greater", price: +level.toFixed(2) });
    await alertsTab(page).click();
    await expect.poll(() => lineCount(page)).toBe(1);

    const [line] = await page.evaluate(
      () => (window as unknown as TestWindow).__chartgenAlertLines(),
    );
    const box = await page.evaluate(() =>
      (
        window as unknown as {
          __chartgenTest: { chart: () => { getDom: () => HTMLElement } };
        }
      ).__chartgenTest
        .chart()
        .getDom()
        .getBoundingClientRect()
        .toJSON(),
    );
    const x = box.x + box.width * 0.5;
    const y = box.y + line!.y;
    await page.mouse.move(x, y);
    await page.mouse.down();
    await page.mouse.move(x, y + 20, { steps: 8 });
    await page.mouse.up();

    await expect
      .poll(async () => (await page.evaluate(
        () => (window as unknown as TestWindow).__chartgenAlertLines()[0]?.value,
      ))!)
      .toBeLessThan(line!.value);
    // The store followed the overlay.
    const stored = JSON.parse(
      (await page.evaluate(() => localStorage.getItem("chartgen.alerts.list")))!,
    ) as { price: number }[];
    expect(stored[0]!.price).toBeLessThan(line!.value);
    expect(stored[0]!.price).toBeGreaterThan(0);

    await page.getByRole("tab", { name: "Watchlist" }).click();
    await page.getByRole("button", { name: "Show ETHUSDT on the chart" }).click();
    await expect(page.getByRole("button", { name: /^Symbol:.*ETHUSDT/ })).toBeVisible();
    await expect.poll(() => lineCount(page)).toBe(0);
    await alertsTab(page).click();
    await rows(page).click();
    await expect(page.getByRole("button", { name: /^Symbol:.*BTCUSDT/ })).toBeVisible();
    await expect.poll(() => lineCount(page)).toBe(1);
  });
});
