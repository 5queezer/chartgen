// Overlay geometry z-order guard (ADR-0010).
//
// Fills are painted with `destination-over` so they sit behind the
// candles. That only works while nothing opaque (e.g. a background
// fill) is painted on the pane canvas before the indicator's draw
// callback runs: with an opaque layer already there, every fill pixel
// would land beneath it and vanish without an error. This spec loads
// Bollinger Bands and asserts that the band fill really shows on the
// canvas and differs from the page background.

import { test, expect, expectChartRendered } from "./fixtures/app";

// fillTheme("bbands", 0) in web/src/components/geometry.ts.
const FILL = { r: 33, g: 150, b: 243, a: 0.1 };

test("bbands fill is visible above the background (destination-over z-order)", async ({
  page,
}) => {
  await page.addInitScript(() => {
    localStorage.setItem("chartgen.indicators.active", JSON.stringify(["bbands"]));
  });
  await page.goto("/");
  await expect(page.locator("text=authenticating...")).toHaveCount(0, {
    timeout: 30_000,
  });
  await expectChartRendered(page);

  const probe = () =>
    page.evaluate((fill) => {
      // Effective page background behind the transparent chart canvases.
      const parse = (css: string) => css.match(/[\d.]+/g)!.map(Number);
      let el: HTMLElement | null = document.querySelector(".chart-container");
      let bg = [0, 0, 0, 0];
      while (el) {
        bg = parse(getComputedStyle(el).backgroundColor);
        if ((bg[3] ?? 1) > 0) break;
        el = el.parentElement;
      }

      // Pixels whose colour is the fill's own (translucent) colour. At
      // ~10% alpha un-premultiplied RGB is only accurate to ~±10.
      let count = 0;
      let sample: number[] | null = null;
      const want = Math.round(fill.a * 255);
      for (const canvas of document.querySelectorAll<HTMLCanvasElement>(
        ".chart-container canvas",
      )) {
        const ctx = canvas.getContext("2d");
        if (!ctx || canvas.width === 0 || canvas.height === 0) continue;
        const d = ctx.getImageData(0, 0, canvas.width, canvas.height).data;
        for (let i = 0; i < d.length; i += 4) {
          if (
            Math.abs(d[i + 3]! - want) <= 8 &&
            Math.abs(d[i]! - fill.r) <= 20 &&
            Math.abs(d[i + 1]! - fill.g) <= 20 &&
            Math.abs(d[i + 2]! - fill.b) <= 20
          ) {
            count++;
            sample ??= [d[i]!, d[i + 1]!, d[i + 2]!, d[i + 3]!];
          }
        }
      }
      // What the user sees at that pixel: canvas composited over bg.
      const seen = sample
        ? [0, 1, 2].map((k) => {
            const a = sample![3]! / 255;
            return Math.round(sample![k]! * a + bg[k]! * (1 - a));
          })
        : null;
      return { count, seen, bg: bg.slice(0, 3) };
    }, FILL);

  await expect.poll(async () => (await probe()).count, { timeout: 15_000 })
    .toBeGreaterThan(500);
  const { seen, bg } = await probe();
  expect(seen).not.toBeNull();
  expect(seen).not.toEqual(bg);
});
