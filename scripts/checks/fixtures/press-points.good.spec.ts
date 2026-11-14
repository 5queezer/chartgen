// Measured points, moves away from the chart, a release point and a reasoned exception pass.
const size = await chart.getSize("main");
await page.mouse.move(size.width / 2, size.top + 40);
await page.mouse.down();
await page.mouse.move(600, 20, { steps: 3 });
await page.mouse.up();
await page.mouse.move(0, 0);
await page.mouse.wheel(0, -300);
await page.mouse.click(600, 200, { button: "right" }); // e2e-fixed-point: the viewport is set to 260 px for this
// e2e-fixed-point: and a reason on the line before
await page.mouse.dblclick(10, 10);
