import { test, expect, gotoApp, disablePush } from "./fixtures/app";

// Observe actual canvas paths, not just the wire flags or legend values.
test("SAR draws per-bar circles and Donchian uses orthogonal steps", async ({ page }, info) => {
  await disablePush(page);
  await page.addInitScript(() => {
    localStorage.setItem("chartgen.indicators.active", JSON.stringify(["sar", "donchian", "vwap", "heikin_ashi"]));
    const counts = { sar: 0, steps: 0, diagonal: 0 };
    Object.assign(window, { presentationCounts: counts });
    const proto = CanvasRenderingContext2D.prototype;
    const begin = proto.beginPath;
    const move = proto.moveTo;
    const line = proto.lineTo;
    const stroke = proto.stroke;
    const arc = proto.arc;
    const paths = new WeakMap<CanvasRenderingContext2D, number[][]>();
    proto.beginPath = function () { paths.set(this, []); return begin.call(this); };
    proto.moveTo = function (x, y) { paths.set(this, [[x, y]]); return move.call(this, x, y); };
    proto.lineTo = function (x, y) { paths.get(this)?.push([x, y]); return line.call(this, x, y); };
    proto.arc = function (...args) {
      const color = String(this.fillStyle).replaceAll(" ", "").toLowerCase();
      if (["#e6c84f", "#b39ddb"].includes(color)) counts.sar++;
      return arc.apply(this, args);
    };
    proto.stroke = function (...args: Parameters<typeof stroke>) {
      const color = String(this.strokeStyle).replaceAll(" ", "").toLowerCase();
      if (color === "#26c6da" || color.startsWith("rgba(38,198,218,")) {
        const path = paths.get(this) ?? [];
        for (let i = 1; i < path.length; i++) {
          const a = path[i - 1]!;
          const b = path[i]!;
          counts.steps++;
          if (a[0] !== b[0] && a[1] !== b[1]) counts.diagonal++;
        }
      }
      return stroke.apply(this, args);
    };
  });
  await gotoApp(page);
  const counts = () => page.evaluate(() => (window as unknown as { presentationCounts: { sar: number; steps: number; diagonal: number } }).presentationCounts);
  await expect.poll(async () => (await counts()).sar).toBeGreaterThan(20);
  await expect.poll(async () => (await counts()).steps).toBeGreaterThan(20);
  expect((await counts()).diagonal).toBe(0);
  const ha = page.locator(".pane-header").filter({ has: page.locator(".pane-header-label", {hasText: /^HA$/}) });
  await expect(ha).toContainText("HA Open");
  await expect(ha).toContainText("HA Close");
  await page.screenshot({path: info.outputPath("indicator-presentation.png")});
});
