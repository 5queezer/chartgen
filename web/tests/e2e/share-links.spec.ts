// Shareable chart links (ADR-0029): boot from a link, clean the URL, copy a
// link that reproduces the chart in a fresh browser profile.

import type { Page } from "@playwright/test";

import { test, expect, gotoApp } from "./fixtures/app";

const activeTimeframe = (page: Page) =>
  page.getByRole("group", { name: "Timeframe" }).locator("[aria-pressed=true]");
const symbolButton = (page: Page) =>
  page.getByRole("button", { name: /^Symbol:/ });
const pane = (page: Page, text: string) =>
  page.locator(".pane-header", { hasText: text });

test.describe("share links", () => {
  test("a link reproduces the chart and the URL is cleaned", async ({
    page,
  }) => {
    await gotoApp(page, "/?symbol=ETHUSDT&interval=4h&ind=rsi");

    await expect(symbolButton(page)).toContainText("ETHUSDT");
    await expect(activeTimeframe(page)).toHaveText("4h");
    await expect(pane(page, "RSI").first()).toBeVisible();
    // The default set has a Volume pane; the link's list replaced it.
    await expect(pane(page, "Volume")).toHaveCount(0);
    expect(new URL(page.url()).pathname + new URL(page.url()).search).toBe("/");

    // It persisted like a manual change.
    await page.reload();
    await expect(symbolButton(page)).toContainText("ETHUSDT");
  });

  test("an empty ind= clears the default indicators", async ({ page }) => {
    await gotoApp(page, "/?symbol=BTCUSDT&ind=");
    await expect(symbolButton(page)).toContainText("BTCUSDT");
    await expect(page.locator("[data-testid^=indicator-legend]")).toHaveCount(0);
  });

  test("a repeated name in ind makes two instances", async ({ page }) => {
    await gotoApp(page, "/?ind=rsi,rsi(length:21)");

    await expect(pane(page, "RSI")).toHaveCount(2);
    await expect(page.getByTestId("indicator-legend-rsi")).toContainText("(14)");
    await expect(page.getByTestId("indicator-legend-rsi#2")).toContainText("(21)");
  });

  test("share params are stripped before the sign-in redirect", async ({
    page,
  }) => {
    // The OAuth redirect would also end on `/`, so look at what the app
    // itself wrote to the address bar before leaving for sign-in.
    await page.addInitScript(() => {
      const real = history.replaceState.bind(history);
      history.replaceState = (data, unused, url) => {
        const log = JSON.parse(sessionStorage.getItem("rs-log") ?? "[]");
        log.push([location.search, String(url)]);
        sessionStorage.setItem("rs-log", JSON.stringify(log));
        real(data, unused, url);
      };
    });
    await gotoApp(page, "/?symbol=ETHUSDT&interval=4h");
    const log = await page.evaluate(() =>
      JSON.parse(sessionStorage.getItem("rs-log") ?? "[]"),
    );
    expect(log[0]).toEqual(["?symbol=ETHUSDT&interval=4h", "/"]);
  });

  test("unknown indicators are dropped after sign-in with a toast", async ({
    page,
  }) => {
    await gotoApp(page, "/?symbol=ETHUSDT&ind=rsi,bogus_xyz");

    await expect(pane(page, "RSI").first()).toBeVisible();
    await expect(page.getByText(/^error:/)).toHaveCount(0);
    await expect(
      page.locator("[data-toast]", { hasText: "bogus_xyz" }),
    ).toBeVisible();
    await expect(pane(page, "Volume")).toHaveCount(0);
  });

  test("invalid parameter values fall back to the default", async ({
    page,
  }) => {
    await gotoApp(page, "/?ind=rsi(length:0)");

    await expect(pane(page, "RSI").first()).toBeVisible();
    await expect(page.getByText(/^error:/)).toHaveCount(0);
    await expect(
      page.locator("[data-toast]", { hasText: "rsi.length" }),
    ).toBeVisible();
  });

  test("invalid values fall back to the stored state", async ({ page }) => {
    await gotoApp(page, "/?symbol=&interval=7x&type=pie&ind=nope!!");

    await expect(symbolButton(page)).toContainText("BTCUSDT");
    await expect(activeTimeframe(page)).toHaveText("1h");
    expect(new URL(page.url()).search).toBe("");
  });

  test("Copy link reproduces the chart in a fresh profile", async ({
    browser,
    context,
    baseURL,
  }) => {
    await context.grantPermissions(["clipboard-read", "clipboard-write"], {
      origin: baseURL,
    });
    const page = await context.newPage();
    await gotoApp(page, "/?symbol=ETHUSDT&interval=4h&ind=rsi(length:21)");

    await page.getByRole("button", { name: "Copy link to chart" }).click();
    await expect(page.getByText("Link copied")).toBeVisible();
    const link = await page.evaluate(() => navigator.clipboard.readText());
    expect(link).toContain("symbol=ETHUSDT");
    expect(link).toContain("interval=4h");
    expect(link).toContain("ind=rsi(length:21)");

    // The context menu offers the same action.
    await page.locator(".chart-host").click({ button: "right" });
    await expect(
      page.getByRole("menuitem", { name: "Copy link to chart" }),
    ).toBeVisible();
    await page.keyboard.press("Escape");

    const fresh = await browser.newContext({ baseURL, colorScheme: "dark" });
    try {
      const other = await fresh.newPage();
      await gotoApp(other, new URL(link).pathname + new URL(link).search);
      await expect(symbolButton(other)).toContainText("ETHUSDT");
      await expect(activeTimeframe(other)).toHaveText("4h");
      await expect(pane(other, "RSI").first()).toContainText("21");
    } finally {
      await fresh.close();
    }
  });

  test("Copy link falls back when the clipboard API is missing", async ({
    page,
  }) => {
    await page.addInitScript(() => {
      Object.defineProperty(navigator, "clipboard", { value: undefined });
      let copied = "";
      document.addEventListener("copy", () => {
        copied = (document.activeElement as HTMLInputElement).value;
        (window as unknown as { __copied: string }).__copied = copied;
      });
      document.execCommand = () => {
        document.dispatchEvent(new Event("copy"));
        return true;
      };
    });
    await gotoApp(page);
    await page.getByRole("button", { name: "Copy link to chart" }).click();
    await expect(page.getByText("Link copied")).toBeVisible();
    const copied = await page.evaluate(
      () => (window as unknown as { __copied: string }).__copied,
    );
    expect(copied).toContain("symbol=BTCUSDT");
  });
});
