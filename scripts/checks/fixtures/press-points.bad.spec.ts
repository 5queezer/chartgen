// Each press below is at literal pixels and must be refused.
await page.mouse.click(600, 200, { button: "right" });
await page.mouse.dblclick(120, 80);
await page.mouse.move(300, 160);
await page.mouse.down();
await page.mouse.click(600, 200); // e2e-fixed-point: this one is allowed, the next line is not
await page.mouse.click(10, 10);
