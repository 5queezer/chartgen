// Keyboard shortcuts must work right after a dialog closes, without a click
// in between (as in TradingView). Each case opens a dialog, closes it with
// Escape or a button, then presses a shortcut and expects its effect.

import type { Page } from "@playwright/test";

import { test, expect } from "./fixtures/app";

const dialog = (page: Page) => page.getByRole("dialog");

interface Case {
  name: string;
  open: (page: Page) => Promise<void>;
  /** Button that closes the dialog; Escape is always tested too. */
  button: string;
  /** Shortcut that must still work afterwards, and the dialog it opens. */
  probe: string;
  probeDialog: RegExp;
}

const cases: Case[] = [
  {
    name: "alert dialog",
    open: (p) => p.keyboard.press("Alt+KeyA"),
    button: "Cancel",
    probe: "Alt+KeyG",
    probeDialog: /go to date/i,
  },
  {
    name: "go to date",
    open: (p) => p.keyboard.press("Alt+KeyG"),
    button: "Cancel",
    probe: "Alt+KeyA",
    probeDialog: /alert/i,
  },
  {
    name: "indicator settings",
    open: async (p) => {
      const row = p.locator(".pane-header", { hasText: "RSI" }).first();
      await row.hover();
      await row.getByRole("button", { name: "Settings for RSI" }).click();
    },
    button: "Cancel",
    probe: "Alt+KeyA",
    probeDialog: /alert/i,
  },
  {
    name: "symbol search",
    open: (p) => p.keyboard.press("KeyE"),
    button: "",
    probe: "Alt+KeyA",
    probeDialog: /alert/i,
  },
];

test.describe("shortcuts after a dialog closes", () => {
  for (const c of cases) {
    for (const how of ["Escape", "button"] as const) {
      if (how === "button" && !c.button) continue;
      test(`${c.name} closed with ${how}`, async ({ bootedApp: page }) => {
        await c.open(page);
        await expect(dialog(page)).toBeVisible();
        if (how === "Escape") await page.keyboard.press("Escape");
        else await dialog(page).getByRole("button", { name: c.button }).click();
        await expect(dialog(page)).toHaveCount(0);

        await page.keyboard.press(c.probe);
        await expect(dialog(page)).toBeVisible();
        await expect(dialog(page)).toHaveAccessibleName(c.probeDialog);
      });
    }
  }
});
