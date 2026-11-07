// With no sign-in on the server (ADR-0050) the workspace leaves no trace: the app
// keeps using plain localStorage, and nothing new is written or sent.

import { test, expect, expectChartRendered, gotoApp, reloadApp } from "./fixtures/app";

test("no sign-in configured: this browser's data stays where it is, and nothing else appears", async ({
  page,
}) => {
  const sync: string[] = [];
  page.on("request", (req) => {
    if (req.url().includes("/workspace/")) sync.push(req.url());
  });
  await gotoApp(page);
  await expectChartRendered(page);
  await page.evaluate(() => {
    localStorage.setItem("chartgen.pane.9.note", "mine");
    localStorage.setItem("chartgen.drawings.TESTSYM", JSON.stringify([{ id: "a" }]));
  });
  await reloadApp(page);

  const keys = await page.evaluate(() => Object.keys(localStorage));
  expect(keys.filter((k) => /^chartgen\.(workspace\.|u\.|sync\.|legacy)/.test(k))).toEqual([]);
  expect(await page.evaluate(() => localStorage.getItem("chartgen.pane.9.note"))).toBe("mine");
  expect(await page.evaluate(() => localStorage.getItem("chartgen.drawings.TESTSYM"))).toBe(
    JSON.stringify([{ id: "a" }]),
  );
  await expect(page.getByTestId("user-menu")).toHaveCount(0);
  expect(sync).toEqual([]);
});

test("with no sign-in, /oauth/me failing does not stop a working chart", async ({ page }) => {
  await page.route("**/oauth/me", (route) => route.fulfill({ status: 500, body: "broken" }));
  await gotoApp(page);
  await expectChartRendered(page);
  await reloadApp(page);
  await expect(page.getByTestId("user-menu")).toHaveCount(0);
  await expect(page.getByText("sign-in failed")).toHaveCount(0);
});

test("the page says something while it boots, and that goes when the app comes", async ({ page }) => {
  let release = () => {};
  const held = new Promise<void>((done) => (release = done));
  await page.route("**/oauth/me", async (route) => {
    await held;
    await route.continue();
  });
  const opening = gotoApp(page);
  await expect(page.locator("#root")).toHaveText("Signing in...");
  release();
  await opening;
  await expect(page.getByText("Signing in...")).toHaveCount(0);
});
