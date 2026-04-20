// Indicator picker flow — covers ADR-0005 behavior:
//   - modal open/close via Kobalte Dialog
//   - search filter expanding the accordion
//   - add / remove toggles the chip row and triggers a chart refetch
//   - localStorage persistence survives a full reload
//
// Known limitation (tracked in fix/web-dynamic-indicators-and-layout):
// the current Chart component only renders RSI; other indicators show
// up as chips and are fetched by the server, but aren't drawn. So we
// scope assertions to observable app state (chips, network POST,
// localStorage) — not to chart-pane visuals for non-RSI indicators.

import type { Response } from "@playwright/test";

import { test, expect, expectChartRendered } from "./fixtures/app";

/**
 * Match only POST /mcp responses that carry a `tools/call` for
 * `generate_chart`. Anything else (list_indicators, initialize,
 * notifications/*) is ignored so waiters don't accidentally resolve on
 * the wrong MCP frame.
 */
function isGenerateChartCall(res: Response): boolean {
  if (res.request().method() !== "POST") return false;
  if (!res.url().endsWith("/mcp")) return false;
  if (res.status() >= 400) return false;
  let body: unknown;
  try {
    body = res.request().postDataJSON();
  } catch {
    return false;
  }
  const b = body as
    | { method?: string; params?: { name?: string } }
    | null
    | undefined;
  return b?.method === "tools/call" && b?.params?.name === "generate_chart";
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
    // isn't already in the default active set so the chip row changes.
    const search = dialog.getByPlaceholder("Search indicators...");
    await search.fill("rsi");

    const items = dialog.locator("li button[type='button']");
    await expect(items.first()).toBeVisible();

    // Filter to items that are NOT already active. Active items render
    // an "added" label next to the name.
    const available = items.filter({ hasNot: page.locator("text=added") });
    await expect(available.first()).toBeVisible();

    // Capture the indicator's name before clicking so we can assert the
    // chip and persistence later. The button's first monospaced span is
    // the canonical name.
    const newIndicatorName = (
      await available.first().locator(".font-mono").first().innerText()
    ).trim();
    expect(newIndicatorName.length).toBeGreaterThan(0);

    // Intercept the refetch that should fire once the active set
    // changes. The active set drives the TanStack query key, so any
    // change causes a POST /mcp for `generate_chart` specifically.
    const refetchAfterAdd = page.waitForResponse(isGenerateChartCall, {
      timeout: 30_000,
    });

    await available.first().click();

    // Modal stays open after add (multi-add design). Close it
    // explicitly via the × button.
    await expect(dialog).toBeVisible();
    await dialog.getByRole("button", { name: "Close" }).click();
    await expect(dialog).toBeHidden();

    // The chip row renders the indicator name; its remove button is
    // `aria-label="Remove <name>"` (see IndicatorChips.tsx).
    const removeButton = page.getByRole("button", {
      name: `Remove ${newIndicatorName}`,
    });
    await expect(removeButton).toBeVisible();

    // Refetch fires from the active-set change.
    await refetchAfterAdd;
    await expectChartRendered(page);

    // Reload and confirm localStorage persisted the chip. Active-set
    // restoration happens synchronously at module load, so the chip
    // row shows before the first network request returns. Wait
    // directly for the positive "remove chip" assertion — that implies
    // both the page has rendered and the active set rehydrated.
    await page.reload();
    await expect(
      page.getByRole("button", { name: `Remove ${newIndicatorName}` }),
    ).toBeVisible({ timeout: 30_000 });

    // Click Load to seed a fresh query, then remove the chip and
    // expect another refetch.
    await page.getByRole("button", { name: "Load" }).click();
    await expectChartRendered(page);

    const refetchAfterRemove = page.waitForResponse(isGenerateChartCall, {
      timeout: 30_000,
    });
    await page
      .getByRole("button", { name: `Remove ${newIndicatorName}` })
      .click();
    await refetchAfterRemove;

    await expect(
      page.getByRole("button", { name: `Remove ${newIndicatorName}` }),
    ).toHaveCount(0);
  });
});
