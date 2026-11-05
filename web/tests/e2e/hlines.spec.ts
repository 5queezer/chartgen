// hlines (ADR-0012): panes must draw their reference lines as full-width
// 1px horizontal lines in the server's colors. The probe scans canvas rows
// for a row whose pixels (almost) all carry the line color, which pins both
// the color and that the line spans the pane width.

import { test, expect, gotoApp, type Page } from "./fixtures/app";

type Rgba = { r: number; g: number; b: number; a: number };

async function openWith(page: Page, active: unknown[]) {
  await page.addInitScript((a) => {
    localStorage.setItem("chartgen.indicators.active", JSON.stringify(a));
  }, active);
  await gotoApp(page);
}

// Returns, per line, the widest fraction of any canvas row whose pixels
// match the color (un-premultiplied RGB is imprecise at low alpha).
const probe = (page: Page, lines: Record<string, Rgba>) =>
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
  }, lines);

test("rsi pane draws its 70/30 hlines in the wire colors", async ({ page }) => {
  // 0xe13e3e @0.3 for 70, 0x3ee145 @0.3 for 30.
  const lines = {
    overbought: { r: 0xe1, g: 0x3e, b: 0x3e, a: 0.3 },
    oversold: { r: 0x3e, g: 0xe1, b: 0x45, a: 0.3 },
  };
  await openWith(page, ["rsi"]);
  await expect
    .poll(async () => (await probe(page, lines)).overbought, { timeout: 15_000 })
    .toBeGreaterThan(0.6);
  expect((await probe(page, lines)).oversold).toBeGreaterThan(0.6);
});

test("hvn_lvn (hlines only) is registered and draws its node levels", async ({
  page,
}) => {
  // HVN levels are 0x26a69a @0.75, LVN 0xef5350 @0.55.
  const lines = {
    hvn: { r: 0x26, g: 0xa6, b: 0x9a, a: 0.75 },
    lvn: { r: 0xef, g: 0x53, b: 0x50, a: 0.55 },
  };
  await openWith(page, ["hvn_lvn"]);
  await expect
    .poll(
      async () => {
        const r = await probe(page, lines);
        return Math.max(r.hvn!, r.lvn!);
      },
      { timeout: 15_000 },
    )
    .toBeGreaterThan(0.6);
});
