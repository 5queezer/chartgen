// hlines (ADR-0012): the RSI pane must draw its 70/30 reference lines as
// full-width 1px horizontal lines in the server's colors (0xe13e3e @0.3
// for 70, 0x3ee145 @0.3 for 30). The probe scans canvas rows for a row
// whose pixels (almost) all carry the line color, which pins both the
// color and that the line spans the pane width.

import { test, expect, expectChartRendered } from "./fixtures/app";

const LINES = {
  overbought: { r: 0xe1, g: 0x3e, b: 0x3e, a: 0.3 },
  oversold: { r: 0x3e, g: 0xe1, b: 0x45, a: 0.3 },
};

test("rsi pane draws its 70/30 hlines in the wire colors", async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("chartgen.indicators.active", JSON.stringify(["rsi"]));
  });
  await page.goto("/");
  await expect(page.locator("text=authenticating...")).toHaveCount(0, {
    timeout: 30_000,
  });
  await expectChartRendered(page);

  // Returns, per line, the widest fraction of any canvas row whose pixels
  // match the color (un-premultiplied RGB is imprecise at 30% alpha).
  const probe = () =>
    page.evaluate((lines) => {
      const out: Record<string, number> = {};
      for (const k of Object.keys(lines)) out[k] = 0;
      for (const canvas of document.querySelectorAll<HTMLCanvasElement>(
        ".chart-container canvas",
      )) {
        const ctx = canvas.getContext("2d");
        if (!ctx || canvas.width === 0 || canvas.height === 0) continue;
        const { width, height } = canvas;
        const d = ctx.getImageData(0, 0, width, height).data;
        for (const [name, c] of Object.entries(lines)) {
          const want = Math.round(c.a * 255);
          for (let y = 0; y < height; y++) {
            let n = 0;
            for (let x = 0; x < width; x++) {
              const i = (y * width + x) * 4;
              // Antialiasing across two rows halves the alpha and makes
              // RGB imprecise, so match faint pixels by dominant channel.
              if (
                d[i + 3]! > 0 &&
                d[i + 3]! <= want + 10 &&
                Math.abs(d[i]! - c.r) <= 70 &&
                Math.abs(d[i + 1]! - c.g) <= 70 &&
                Math.abs(d[i + 2]! - c.b) <= 70
              )
                n++;
            }
            out[name] = Math.max(out[name]!, n / width);
          }
        }
      }
      return out;
    }, LINES);

  await expect
    .poll(async () => (await probe()).overbought, { timeout: 15_000 })
    .toBeGreaterThan(0.6);
  expect((await probe()).oversold).toBeGreaterThan(0.6);
});
