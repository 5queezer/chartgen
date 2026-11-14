// Each press below is at literal pixels and must be refused.
await page.mouse.click(600, 200, { button: "right" });
await page.mouse.dblclick(120, 80);
await page.mouse.move(300, 160);
await page.mouse.down();
