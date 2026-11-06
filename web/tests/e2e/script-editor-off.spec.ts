// With scripting off (the default server) the script editor leaves no trace:
// nothing on screen, no scripting tool called, nothing failing (ADR-0041).

import { expect, gotoApp, test } from "./fixtures/app";

test("no UI, no scripting calls and no failed requests", async ({ page }) => {
  const mcpBodies: string[] = [];
  const failed: string[] = [];
  const loaded: string[] = [];
  page.on("request", (r) => {
    loaded.push(r.url());
    if (new URL(r.url()).pathname === "/mcp" && r.method() === "POST") {
      mcpBodies.push(r.postData() ?? "");
    }
  });
  page.on("response", (r) => {
    if (r.status() >= 400) failed.push(`${r.status()} ${r.request().method()} ${r.url()}`);
  });

  await gotoApp(page);

  // The detection did run, and the server did not list the tools.
  await expect.poll(() => mcpBodies.some((b) => b.includes('"tools/list"'))).toBe(true);
  await expect(page.getByTestId("script-editor-open")).toHaveCount(0);
  await expect(page.getByTestId("script-editor")).toHaveCount(0);

  await page.getByTitle("Indicators", { exact: true }).click();
  await expect(page.getByPlaceholder("Search indicators...")).toBeVisible();
  await expect(page.getByTestId("picker-script-editor")).toHaveCount(0);
  await expect(page.getByRole("dialog")).not.toContainText("script editor");

  for (const tool of ["list_my_indicators", "get_my_indicator", "register_indicator", "delete_indicator"]) {
    expect(mcpBodies.filter((b) => b.includes(tool)), tool).toEqual([]);
  }
  expect(loaded.filter((u) => /codemirror|ScriptEditor\.tsx/.test(u))).toEqual([]);
  expect(failed).toEqual([]);
  expect(await page.evaluate(() => localStorage.getItem("chartgen.scripts.drafts"))).toBeNull();
});

test("an unknown indicator fails the chart with scripting off, and says which", async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("chartgen.indicators.active", JSON.stringify(["rsi", "gone_script"]));
  });
  await page.goto("/");
  const error = page.getByTestId("chart-load-error");
  await expect(error).toContainText("unknown indicator gone_script", { timeout: 30_000 });
  await expect(error).toContainText("remove it from the chips");
  await page.getByRole("button", { name: "Remove gone_script" }).click();
  await expect(error).toHaveCount(0, { timeout: 30_000 });
  await expect(page.locator(".pane-header").first()).toBeVisible();
});
