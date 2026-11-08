// The wordmark's load sweep: one band of light across "gen" that ends on the static
// logo, pixel for pixel, and never runs under reduced motion. Also the icons the page
// names, which are the g of the wordmark.

import type { Page } from "@playwright/test";

import { test, expect, gotoApp, reloadApp } from "./fixtures/app";

interface SweepLog {
  /** Animations started on anything inside the wordmark. */
  started: number;
  /** Whether the sweep's overlay was ever in the wordmark. */
  overlay: boolean;
}

/**
 * From the first script on, counts the wordmark's animations and watches for its
 * overlay. With `hold`, each animation is paused as it starts, for the test to drive.
 */
const watchSweep = (page: Page, hold: boolean) =>
  page.addInitScript((hold) => {
    const log: SweepLog = { started: 0, overlay: false };
    const w = window as unknown as { __sweep: SweepLog; __held: Animation[] };
    w.__sweep = log;
    w.__held = [];
    const animate = Element.prototype.animate;
    Element.prototype.animate = function (this: Element, ...args: Parameters<Element["animate"]>) {
      const anim = animate.apply(this, args);
      if (this.closest("header h1")) {
        log.started += 1;
        if (hold) {
          anim.pause();
          w.__held.push(anim);
        }
      }
      return anim;
    };
    new MutationObserver(() => {
      if (document.querySelector("header h1 svg > :not(path)")) log.overlay = true;
    }).observe(document, { childList: true, subtree: true });
  }, hold);

const sweepLog = (page: Page) =>
  page.evaluate(() => (window as unknown as { __sweep: SweepLog }).__sweep);

const wordmark = (page: Page) => page.locator("header h1");
const overlay = (page: Page) => page.locator("header h1 svg > :not(path)");

/** The wordmark's markup, its generated ids made equal. */
const markup = (page: Page) =>
  wordmark(page).evaluate((h1) => h1.innerHTML.replace(/cl-[\w-]+/g, "id"));

/** Two frames, so whatever the last change was is on screen. */
const settle = (page: Page) =>
  page.evaluate(
    () => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))),
  );

test.describe("wordmark sweep", () => {
  test("runs once on load and leaves the static logo's markup", async ({ page }) => {
    await watchSweep(page, false);
    await gotoApp(page);
    await expect(overlay(page)).toHaveCount(0, { timeout: 5_000 });
    expect(await sweepLog(page)).toEqual({ started: 1, overlay: true });
    const swept = await markup(page);

    await page.emulateMedia({ reducedMotion: "reduce" });
    await reloadApp(page);
    expect(await markup(page)).toBe(swept);
  });

  for (const theme of ["dark", "light"] as const) {
    test(`shows mid-run and ends pixel-identical to the static logo, ${theme}`, async ({ page }) => {
      if (theme === "light") await page.emulateMedia({ colorScheme: "light" });
      await page.emulateMedia({ reducedMotion: "reduce" });
      await gotoApp(page);
      await settle(page);
      const still = await wordmark(page).screenshot();

      await page.emulateMedia({ reducedMotion: "no-preference" });
      await watchSweep(page, true);
      await reloadApp(page);
      expect((await sweepLog(page)).started).toBe(1);

      // Halfway the band is on "gen".
      await page.evaluate(() => {
        for (const a of (window as unknown as { __held: Animation[] }).__held) a.currentTime = 650;
      });
      await settle(page);
      expect((await wordmark(page).screenshot()).equals(still)).toBe(false);

      await page.evaluate(() => {
        for (const a of (window as unknown as { __held: Animation[] }).__held) a.play();
      });
      await expect(overlay(page)).toHaveCount(0, { timeout: 5_000 });
      await settle(page);
      const end = await wordmark(page).screenshot();
      await test.info().attach(`still-${theme}`, { body: still, contentType: "image/png" });
      await test.info().attach(`end-${theme}`, { body: end, contentType: "image/png" });
      expect(end.equals(still)).toBe(true);
    });
  }

  test("with reduced motion never animates: no animation, no overlay, the static logo", async ({
    page,
  }) => {
    await page.emulateMedia({ reducedMotion: "reduce" });
    await watchSweep(page, false);
    await gotoApp(page);
    await expect(page.getByRole("img", { name: "chartgen", exact: true })).toBeVisible();
    await page.waitForTimeout(1_500);
    expect(await sweepLog(page)).toEqual({ started: 0, overlay: false });
    await expect(page.locator("header h1 svg > *")).toHaveCount(2);
  });
});

test("every icon the page names is served, with its type", async ({ page }) => {
  await gotoApp(page);
  const linked = await page.evaluate(() =>
    [...document.querySelectorAll<HTMLLinkElement>('link[rel~="icon"], link[rel="apple-touch-icon"]')].map(
      (l) => l.getAttribute("href")!,
    ),
  );
  expect(linked.sort()).toEqual(["/favicon.ico", "/favicon.svg", "/icons/apple-touch-icon.png"]);
  const manifest = (await (await page.request.get("/manifest.webmanifest")).json()) as {
    icons: { src: string }[];
  };
  const types: Record<string, string> = { ico: "image/", svg: "image/svg+xml", png: "image/png" };
  for (const href of [...linked, ...manifest.icons.map((i) => i.src)]) {
    const res = await page.request.get(href);
    expect(res.status(), href).toBe(200);
    expect(res.headers()["content-type"], href).toContain(types[href.split(".").pop()!]);
  }
});
