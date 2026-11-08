// Indicator picker flow — covers ADR-0005 behavior:
//   - modal open/close via Kobalte Dialog
//   - search filter expanding the accordion
//   - add / remove toggles the legend row and triggers a chart refetch
//   - localStorage persistence survives a full reload

import type { Response } from "@playwright/test";

import {
  test,
  expect,
  expectChartRendered,
  reloadApp,
} from "./fixtures/app";

/**
 * Match only a successful full chart load (`generate_chart` with no
 * `end_time` and more than the 2 bars of a live tail poll) whose
 * indicator list does or does not include `name`. Live tail polls run
 * every 2 s and carry the old list, so a bare `generate_chart` match
 * would resolve on one of them.
 */
function isFullLoad(name: string, included: boolean) {
  return (res: Response): boolean => {
    if (res.request().method() !== "POST") return false;
    if (!res.url().endsWith("/mcp")) return false;
    if (res.status() >= 400) return false;
    try {
      const b = res.request().postDataJSON() as {
        method?: string;
        params?: {
          name?: string;
          arguments?: {
            bars?: number;
            end_time?: number;
            indicators?: Array<string | { name: string }>;
          };
        };
      } | null;
      const a = b?.params?.arguments;
      if (
        b?.method !== "tools/call" ||
        b.params?.name !== "generate_chart" ||
        a?.end_time !== undefined ||
        (a?.bars ?? 0) <= 2
      ) {
        return false;
      }
      const names = (a?.indicators ?? []).map((e) =>
        typeof e === "string" ? e : e.name,
      );
      return names.includes(name) === included;
    } catch {
      return false;
    }
  };
}

test.describe("indicator picker", () => {
  test("add + remove + persistence round-trip", async ({ bootedApp: page }) => {
    // Kobalte Dialog hasn't mounted yet — clicking the header button
    // renders it into the portal.
    await page.getByRole("button", { name: /Indicators/ }).click();

    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible();

    // Search for "rsi" — several matching items appear (RSI, Stoch RSI,
    // Cipher B divergences, etc.). We intentionally pick an item that
    // isn't already in the default active set so a legend row appears.
    const search = dialog.getByPlaceholder("Search indicators...");
    await search.fill("rsi");

    const items = dialog.locator("li button[type='button']");
    await expect(items.first()).toBeVisible();

    // Filter to items that are NOT already active. Active items render
    // an "added" label next to the name.
    const available = items.filter({ hasNot: page.locator("text=added") });
    await expect(available.first()).toBeVisible();

    // Capture the indicator's name before clicking so we can assert the
    // legend row and persistence later. The button's first monospaced span is
    // the canonical name.
    const newIndicatorName = (
      await available.first().locator(".font-mono").first().innerText()
    ).trim();
    expect(newIndicatorName.length).toBeGreaterThan(0);

    // Intercept the refetch that should fire once the active set
    // changes. The active set drives the TanStack query key, so the
    // change causes a full `generate_chart` load with the new indicator.
    const refetchAfterAdd = page.waitForResponse(
      isFullLoad(newIndicatorName, true),
    );

    await available.first().click();

    // Modal stays open after add (multi-add design). Close it
    // explicitly via the × button.
    await expect(dialog).toBeVisible();
    await dialog.getByRole("button", { name: "Close" }).click();
    await expect(dialog).toBeHidden();

    // Its legend row carries the remove button.
    const removeButton = page
      .getByTestId(`indicator-legend-${newIndicatorName}`)
      .getByRole("button", { name: /^Remove / });
    await expect(removeButton).toBeVisible();

    // Refetch fires from the active-set change.
    await refetchAfterAdd;
    await expectChartRendered(page);

    // Reload and confirm localStorage persisted the indicator. The chart
    // auto-loads after reload; once it has rendered, remove it and
    // expect another refetch.
    await reloadApp(page);
    await expect(removeButton).toBeVisible();

    const refetchAfterRemove = page.waitForResponse(
      isFullLoad(newIndicatorName, false),
    );
    await removeButton.click();
    await refetchAfterRemove;

    await expect(removeButton).toHaveCount(0);
  });
});
