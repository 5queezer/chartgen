// Setups panel (ADR-0071) and page (ADR-0091) on a server that keeps no setups: the tab
// says so, and asks for nothing but the list, which answers `enabled: false`.

import { test, expect, gotoApp, openSidebarTab } from "./fixtures/app";

test("the Setups tab says when the server keeps no setups", async ({ page }) => {
  const asked: string[] = [];
  page.on("request", (r) => {
    if (r.method() !== "POST" || new URL(r.url()).pathname !== "/mcp") return;
    const name = (r.postDataJSON() as { params?: { name?: string } } | null)?.params?.name;
    if (name?.includes("setup") || name === "get_track_record") asked.push(name);
  });
  await gotoApp(page);
  await openSidebarTab(page, "Setups");
  await expect(page.locator("[data-role=setups-off]")).toContainText("keeps no setups");
  await expect(page.locator("[data-setup-id]")).toHaveCount(0);
  expect(asked).toEqual(["list_setups"]);
});

test("/setups on a server without setups is the app at `/` (ADR-0091)", async ({ page }) => {
  await gotoApp(page, "/setups");
  await expect(page).toHaveURL((url) => url.pathname === "/");
  await expect(page.getByTestId("setups-page")).toHaveCount(0);
  // Nothing of the page's request is kept: the next load at `/` is the app as well.
  await gotoApp(page, "/");
  await expect(page.getByTestId("setups-page")).toHaveCount(0);
});
