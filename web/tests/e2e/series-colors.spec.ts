// Plot colors on the wire (ADR-0026): lines and histograms draw in the PNG
// renderer's colors, histograms per bar (Volume green/red like TradingView),
// and the Style tab's override still wins. Pixel probes count canvas pixels
// of a color inside one pane, found through its legend row.

import type { Page } from "@playwright/test";

import { test, expect, gotoApp } from "./fixtures/app";

type Rgba = { r: number; g: number; b: number; a: number };

// trend::Volume: 0x26a69a / 0xef5350 at 0.5.
const UP: Rgba = { r: 0x26, g: 0xa6, b: 0x9a, a: 0.5 };
const DOWN: Rgba = { r: 0xef, g: 0x53, b: 0x50, a: 0.5 };
// ta_bridge::EmaStack's EMA 50: 0x2196f3 at 0.9.
const EMA50: Rgba = { r: 0x21, g: 0x96, b: 0xf3, a: 0.9 };

const SHOTS = "/tmp/claude-1000/chartgen-shots/series-colors";

async function openWith(page: Page, active: unknown[]) {
  await page.addInitScript((a) => {
    localStorage.setItem("chartgen.indicators.active", JSON.stringify(a));
  }, active);
  await gotoApp(page);
}

const header = (page: Page, name: string) =>
  page
    .locator(".pane-header")
    .filter({ has: page.locator(".pane-header-label", { hasText: new RegExp(`^${name}$`) }) })
    .first();

/**
 * Pixels per color on the wide canvases of the pane whose legend row is
 * labelled `name`. Lines antialias, so `minAlpha` set matches any pixel at
 * least that opaque by RGB; otherwise alpha must be the color's own.
 */
const paneCounts = (
  page: Page,
  name: string,
  colors: Record<string, Rgba>,
  minAlpha?: number,
) =>
  page.evaluate(
    ({ name, colors, minAlpha }) => {
      const label = [...document.querySelectorAll(".pane-header-label")].find(
        (l) => l.textContent === name,
      );
      const out: Record<string, number> = {};
      for (const k of Object.keys(colors)) out[k] = 0;
      if (!label) return out;
      // Just below the legend row: inside its pane.
      const y = label.getBoundingClientRect().bottom + 20;
      for (const canvas of document.querySelectorAll<HTMLCanvasElement>(
        ".chart-container canvas",
      )) {
        const rect = canvas.getBoundingClientRect();
        if (rect.width < 300 || y < rect.top || y >= rect.bottom) continue;
        const ctx = canvas.getContext("2d");
        if (!ctx || canvas.width === 0 || canvas.height === 0) continue;
        const d = ctx.getImageData(0, 0, canvas.width, canvas.height).data;
        for (const [k, c] of Object.entries(colors)) {
          const want = Math.round(c.a * 255);
          const tol = minAlpha === undefined ? 6 : 12;
          for (let i = 0; i < d.length; i += 4) {
            const alphaOk =
              minAlpha === undefined
                ? Math.abs(d[i + 3]! - want) <= 6
                : d[i + 3]! >= minAlpha;
            if (
              alphaOk &&
              Math.abs(d[i]! - c.r) <= tol &&
              Math.abs(d[i + 1]! - c.g) <= tol &&
              Math.abs(d[i + 2]! - c.b) <= tol
            )
              out[k]!++;
          }
        }
      }
      return out;
    },
    { name, colors, minAlpha },
  );

const legendColors = (page: Page, name: string) =>
  header(page, name)
    .locator(".pane-header-value")
    .evaluateAll((els) => els.map((el) => getComputedStyle(el).color));

test.describe("series colors", () => {
  test("the default Volume pane draws green and red bars", async ({ bootedApp: page }) => {
    await expect
      .poll(async () => {
        const n = await paneCounts(page, "Volume", { up: UP, down: DOWN });
        return Math.min(n.up!, n.down!);
      }, { timeout: 15_000 })
      .toBeGreaterThan(100);
    // The legend names the histogram after the indicator, not "hist0".
    await expect(header(page, "Volume")).toContainText("Volume MA");
    await expect(header(page, "Volume").locator(".pane-header-value").last()).toContainText(
      /^Volume /,
    );
    await expect(header(page, "Volume")).not.toContainText("hist0");
  });

  test("EMA lines and the legend use the server's colors", async ({ bootedApp: page }) => {
    await expect
      .poll(() => legendColors(page, "EMA"), { timeout: 15_000 })
      .toEqual([
        "rgba(255, 235, 59, 0.9)",
        "rgba(255, 152, 0, 0.9)",
        "rgba(33, 150, 243, 0.9)",
        "rgba(244, 67, 54, 0.9)",
      ]);
    const n = await paneCounts(page, "EMA", { ema50: EMA50 }, 150);
    expect(n.ema50).toBeGreaterThan(50);
  });

  test("a Style tab color overrides the server's per-bar colors", async ({
    bootedApp: page,
  }) => {
    const row = header(page, "Volume");
    await row.hover();
    await row.getByRole("button", { name: "Settings for Volume" }).click();
    const dialog = page.getByRole("dialog");
    await dialog.getByRole("tab", { name: "Style" }).click();
    await expect(dialog.getByTestId("style-row-hist0")).toContainText("Volume");
    // Volume has an up and a down palette color: recolor both.
    for (const k of [1, 2]) {
      await dialog.getByRole("button", { name: `Volume color ${k}`, exact: true }).click();
      // Presets keep the current opacity: the bars' 0.5.
      await page.getByLabel("Color #9c27b0").click();
      await dialog.getByRole("tab", { name: "Style" }).click();
    }
    await dialog.getByRole("button", { name: "OK" }).click();
    await expect(dialog).toHaveCount(0);

    const purple = { r: 0x9c, g: 0x27, b: 0xb0, a: 0.5 };
    await expect
      .poll(async () => (await paneCounts(page, "Volume", { purple })).purple, {
        timeout: 15_000,
      })
      .toBeGreaterThan(200);
    const left = await paneCounts(page, "Volume", { up: UP, down: DOWN });
    expect(left.up! + left.down!).toBeLessThan(20);
    await expect(row.locator(".pane-header-value").last()).toHaveCSS(
      "color",
      "rgba(156, 39, 176, 0.5)",
    );
  });

  test("Volume, MACD and EMA together (screenshot)", async ({ page }) => {
    await openWith(page, ["ema_stack", "volume", "macd"]);
    await expect
      .poll(async () => {
        const n = await paneCounts(page, "MACD", {
          up: { r: 0x26, g: 0xa6, b: 0x9a, a: 0.8 },
          down: { r: 0xef, g: 0x53, b: 0x50, a: 0.8 },
        });
        return Math.min(n.up!, n.down!);
      }, { timeout: 15_000 })
      .toBeGreaterThan(20);
    await expect(header(page, "MACD").locator(".pane-header-value").last()).toContainText(
      /^Histogram /,
    );
    await page.screenshot({ path: `${SHOTS}/volume-macd-ema.png` });
  });

  test("per-bar plots draw as lines: Supertrend, SAR, HA, KVF", async ({ page }) => {
    // Their per-bar points used to be unlabeled dots, which the series
    // wire drops: the chart showed no plot at all.
    await openWith(page, ["supertrend", "sar", "heikin_ashi", "kalman_volume"]);
    // [legend name, line labels, lines with a value at the last bar]
    const plots: Array<[string, string[], number]> = [
      ["Supertrend", ["Up Trend", "Down Trend"], 1],
      ["SAR", ["SAR Up", "SAR Down"], 1],
      ["HA", ["HA Close"], 1],
      ["KVF", ["KVF", "Signal"], 2],
    ];
    for (const [name, labels, valued] of plots) {
      // Lines come first in the legend (KVF's volume histograms follow).
      const values = header(page, name).locator(".pane-header-value");
      await expect(values.nth(labels.length - 1)).toBeVisible({ timeout: 15_000 });
      const texts = (await values.allTextContents()).slice(0, labels.length);
      const read = texts.map((t, i) => {
        expect(t.startsWith(`${labels[i]} `), t).toBe(true);
        return t.slice(labels[i]!.length + 1);
      });
      // At rest the legend reads the last bar: of a trend pair only the
      // active line has a value there, the other shows none, not a stale one.
      const numbers = read.filter((v) => /\d/.test(v)).length;
      expect(numbers, `${name}: ${texts.join(" | ")}`).toBe(valued);
      if (numbers < read.length) expect(read).toContain("—");
    }
    // The KVF line is drawn in its pane (cyan, opaque; the fills are faint).
    await expect
      .poll(
        async () =>
          (await paneCounts(page, "KVF", { kvf: { r: 0x00, g: 0xbc, b: 0xd4, a: 0.9 } }, 150)).kvf,
        { timeout: 15_000 },
      )
      .toBeGreaterThan(50);
  });
});
