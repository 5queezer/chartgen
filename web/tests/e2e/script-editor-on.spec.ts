// The script editor against a real server with CHARTGEN_SCRIPTING=1 and a
// throwaway store: the `scripting` project of playwright.config.ts (ADR-0041).
// Every test gets a fresh browser context, so a fresh OAuth client and no scripts.

import { mkdirSync } from "node:fs";
import { join } from "node:path";

import { expect, gotoApp, reloadApp, test } from "./fixtures/app";
import type { Page } from "@playwright/test";

const code = (page: Page) => page.locator("[data-testid=script-code] .cm-content");
const lines = (page: Page) => page.locator("[data-testid=script-code] .cm-line");

async function openEditor(page: Page): Promise<void> {
  await page.getByTestId("script-editor-open").click();
  await expect(page.getByTestId("script-editor")).toBeVisible({ timeout: 30_000 });
  await expect(code(page)).toContainText("PARAMS");
}

async function setCode(page: Page, text: string): Promise<void> {
  await code(page).click();
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.insertText(text);
}

const script = (body: string, title = "Test script") => `PARAMS = []
META = {"title": ${JSON.stringify(title)}, "overlay": False}

def compute(ohlcv, params, chart):
${body}
`;

/** What the chart holds for an indicator: its first plot's label and last value. */
const plotted = (page: Page, key: string) =>
  page.evaluate((k) => {
    const test = (window as unknown as {
      __chartgenTest: {
        series: () => {
          indicators: Record<string, { lines?: Array<{ label: string | null; values: Array<number | null> }> }>;
        } | null;
      };
    }).__chartgenTest;
    const line = test.series()?.indicators[k]?.lines?.[0];
    return line ? { label: line.label, last: line.values.at(-1) ?? null } : null;
  }, key);

test("create with an error, fix, save, add to chart, edit, delete", async ({ page }) => {
  const requested: string[] = [];
  page.on("request", (r) => requested.push(r.url()));
  await gotoApp(page);
  // Nobody pays for the editor before opening it.
  expect(requested.filter((u) => /codemirror|ScriptEditor\.tsx/.test(u))).toEqual([]);

  await openEditor(page);
  expect(requested.some((u) => u.includes("codemirror"))).toBe(true);
  await expect(page.getByText("No saved indicators yet.")).toBeVisible();
  await expect(page.getByTestId("script-docs")).toHaveAttribute(
    "href",
    /\/chartgen\/guides\/scripted-indicators\/$/,
  );
  await page.getByTestId("script-name").fill("cycle_one");

  // Fails in the dry run, on line 6 of this text.
  await setCode(page, script("    x = 1\n    y = x // 0"));
  await page.getByTestId("script-save").click();
  const errors = page.getByTestId("script-errors");
  await expect(errors).toContainText("Line 6");
  await expect(errors).toContainText("Dry run");
  await expect(page.getByTestId("script-status")).toHaveText("Not saved.");
  // Marked at the line the server named, and on no other.
  const marked = page.locator("[data-testid=script-code] .cm-script-error-line");
  await expect(marked).toHaveCount(1);
  await expect(marked).toHaveText("    y = x // 0");
  await expect(lines(page).nth(5)).toHaveClass(/cm-script-error-line/);
  await expect(page.getByTestId("script-row-cycle_one")).toHaveCount(0);
  await expect(page.getByTestId("script-add")).toBeDisabled();

  await setCode(page, script('    chart.line(100.0, title = "Alpha")'));
  await page.getByTestId("script-save").click();
  await expect(page.getByTestId("script-status")).toHaveText("Saved.");
  await expect(errors).toHaveCount(0);
  await expect(marked).toHaveCount(0);
  await expect(page.getByTestId("script-row-cycle_one")).toContainText("Test script");

  await page.getByTestId("script-add").click();
  await expect.poll(() => plotted(page, "cycle_one"), { timeout: 30_000 }).toEqual({
    label: "Alpha",
    last: 100,
  });
  await expect(page.getByTestId("script-row-cycle_one")).toContainText("on chart");

  // The Add button waits for a save: the chart runs the saved text.
  await setCode(page, script('    chart.line(200.0, title = "Beta")'));
  await expect(page.getByTestId("script-add")).toBeDisabled();
  await expect(page.getByTestId("script-row-cycle_one")).toContainText("•", { timeout: 5_000 });
  await page.keyboard.press("ControlOrMeta+s");
  await expect.poll(() => plotted(page, "cycle_one"), { timeout: 30_000 }).toEqual({
    label: "Beta",
    last: 200,
  });
  await expect(page.getByTestId("script-row-cycle_one")).not.toContainText("•");

  // The picker knows the script as well.
  await page.getByTitle("Indicators", { exact: true }).click();
  await page.getByPlaceholder("Search indicators...").fill("cycle_one");
  await expect(page.getByRole("dialog").getByTitle("cycle_one")).toBeVisible();
  await page.getByRole("button", { name: "Close" }).click();

  // Delete asks first.
  await page.getByTestId("script-delete").click();
  const dialog = page.getByTestId("script-delete-dialog");
  await expect(dialog).toContainText("It also leaves the chart.");
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect(page.getByTestId("script-row-cycle_one")).toBeVisible();

  await page.getByTestId("script-delete").click();
  await page.getByTestId("script-delete-confirm").click();
  await expect(page.getByTestId("script-row-cycle_one")).toHaveCount(0);
  await expect(page.getByTestId("script-status")).toHaveText("Deleted cycle_one.");
  // Gone from the chart, which still loads.
  await expect.poll(() => plotted(page, "cycle_one")).toBeNull();
  await expect(page.locator("text=loaded").first()).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("chart-load-error")).toHaveCount(0);

  await reloadApp(page);
  await openEditor(page);
  await expect(page.getByText("No saved indicators yet.")).toBeVisible();
});

test("unsaved text survives closing the panel and reloading the page", async ({ page }) => {
  await gotoApp(page);
  await openEditor(page);
  await page.getByTestId("script-name").fill("draft_one");
  await setCode(page, script("    pass  # draft text"));

  await page.getByTestId("script-close").click();
  await expect(page.getByTestId("script-editor")).toHaveCount(0);
  await openEditor(page);
  await expect(code(page)).toContainText("# draft text");
  await expect(page.getByTestId("script-name")).toHaveValue("draft_one");
  await expect(page.getByTestId("script-status")).toHaveText("Unsaved draft restored.");

  // Right after typing, ahead of the debounce: leaving the page flushes it.
  await code(page).click();
  await page.keyboard.press("ControlOrMeta+End");
  await page.keyboard.insertText("# typed last\n");
  await reloadApp(page);
  await openEditor(page);
  await expect(code(page)).toContainText("# typed last");

  // Per script: another script does not touch it, and saving ends the draft.
  await page.getByTestId("script-save").click();
  await expect(page.getByTestId("script-status")).toHaveText("Saved.");
  await setCode(page, script("    pass  # edited after saving"));
  await page.getByTestId("script-new").click();
  await expect(code(page)).toContainText("My EMA");
  await page.getByTestId("script-row-draft_one").click();
  await expect(code(page)).toContainText("# edited after saving");
  await expect(page.getByTestId("script-status")).toHaveText("Unsaved draft restored.");

  // Revert goes back to the saved text, and is itself undoable.
  await page.getByTestId("script-revert").click();
  await expect(code(page)).toContainText("# typed last");
  await expect(page.getByTestId("script-revert")).toHaveCount(0);
  await code(page).click();
  await page.keyboard.press("ControlOrMeta+z");
  await expect(code(page)).toContainText("# edited after saving");
});

test("line numbers, highlighting, Tab and undo", async ({ page }) => {
  await gotoApp(page);
  await openEditor(page);
  await setCode(page, "x = 1\ny = 2");
  await expect(page.locator("[data-testid=script-code] .cm-lineNumbers .cm-gutterElement").filter({ hasText: /^2$/ })).toBeVisible();

  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.insertText("def compute(a):");
  // Highlighted: `def` has a color of its own, set from the theme's variable.
  const color = (text: string) =>
    lines(page).locator("span", { hasText: new RegExp(`^${text}$`) }).evaluate((el) => getComputedStyle(el).color);
  expect(await color("def")).toBe("rgb(199, 146, 234)");

  // Tab indents and stays in the editor.
  await page.keyboard.press("Home");
  await page.keyboard.press("Tab");
  await expect(lines(page).first()).toHaveText("    def compute(a):");
  await expect(code(page)).toBeFocused();
  await page.keyboard.press("ControlOrMeta+z");
  await expect(lines(page).first()).toHaveText("def compute(a):");

  // Esc then Tab leaves the editor, so the keyboard is not trapped in it.
  await page.keyboard.press("Escape");
  await page.keyboard.press("Tab");
  await expect(code(page)).not.toBeFocused();
  await expect(lines(page).first()).toHaveText("def compute(a):");
});

test("source, titles and server errors render as text", async ({ page }) => {
  const attack = '<img src=x onerror="window.__xss=1">';
  await gotoApp(page);
  await openEditor(page);
  await page.getByTestId("script-name").fill("html_one");

  await setCode(page, `# ${attack}\n` + script(`    d = {}\n    y = d[${JSON.stringify(attack)}]`));
  await page.getByTestId("script-save").click();
  await expect(page.getByTestId("script-errors")).toContainText("<img src=x");

  await setCode(page, `# ${attack}\n` + script("    pass", attack));
  await page.getByTestId("script-save").click();
  await expect(page.getByTestId("script-row-html_one")).toContainText(attack);
  await expect(code(page)).toContainText(attack);

  await expect(page.locator("[data-testid=script-editor] img")).toHaveCount(0);
  expect(await page.evaluate(() => (window as unknown as { __xss?: number }).__xss)).toBeUndefined();
});

test("a script that fails on the real chart shows its error at the line", async ({ page }) => {
  await gotoApp(page);
  await openEditor(page);
  await page.getByTestId("script-name").fill("fails_late");
  // The dry run has 100 bars, so this registers and then fails on the chart.
  await setCode(page, script("    if len(ohlcv.close) > 100:\n        y = 1 // 0\n    chart.line(ohlcv.close)"));
  await page.getByTestId("script-save").click();
  await expect(page.getByTestId("script-status")).toHaveText("Saved.");
  await page.getByTestId("script-add").click();

  const errors = page.getByTestId("script-errors");
  await expect(errors).toContainText("Line 6", { timeout: 30_000 });
  await expect(errors).toContainText("On the chart");
  await expect(page.locator("[data-testid=script-code] .cm-script-error-line")).toHaveText("        y = 1 // 0");
  // Only this indicator failed.
  await expect.poll(() => plotted(page, "ema_stack")).not.toBeNull();
});

test("a new script cannot overwrite a saved one by name", async ({ page }) => {
  await gotoApp(page);
  await openEditor(page);
  await page.getByTestId("script-name").fill("dup");
  await setCode(page, script("    pass  # the original"));
  await page.getByTestId("script-save").click();
  await expect(page.getByTestId("script-status")).toHaveText("Saved.");

  await page.getByTestId("script-new").click();
  await page.getByTestId("script-name").fill("dup");
  await page.getByTestId("script-save").click();
  await expect(page.getByTestId("script-errors")).toContainText("already have an indicator named dup");

  await page.getByTestId("script-row-dup").click();
  await expect(code(page)).toContainText("# the original");
});

test("opening another script during a save leaves that script alone", async ({ page }) => {
  await gotoApp(page);
  await openEditor(page);
  for (const name of ["race_a", "race_b"]) {
    await page.getByTestId("script-new").click();
    await page.getByTestId("script-name").fill(name);
    await setCode(page, script(`    pass  # ${name} saved`));
    await page.getByTestId("script-save").click();
    await expect(page.getByTestId(`script-row-${name}`)).toBeVisible();
  }

  await page.getByTestId("script-row-race_a").click();
  await expect(page.getByTestId("script-name")).toHaveValue("race_a");
  await expect(code(page)).toContainText("# race_a saved");
  await setCode(page, script("    pass  # race_a edited"));

  // Hold the save until the other script is open.
  let release = () => {};
  const held = new Promise<void>((resolve) => (release = resolve));
  let saving = () => {};
  const saveSent = new Promise<void>((resolve) => (saving = resolve));
  await page.route("**/mcp", async (route) => {
    if (route.request().postData()?.includes('"register_indicator"')) {
      saving();
      await held;
    }
    await route.continue();
  });
  await page.getByTestId("script-save").click();
  await saveSent;
  await page.getByTestId("script-row-race_b").click();
  await expect(code(page)).toContainText("# race_b saved");
  const answered = page.waitForResponse((r) => r.request().postData()?.includes('"register_indicator"') ?? false);
  release();
  await answered;
  await expect(page.getByText("Saved race_a.")).toBeVisible();

  // Still race_b, by name and text, and saving it now writes race_b.
  await expect(page.getByTestId("script-name")).toHaveValue("race_b");
  await expect(code(page)).toContainText("# race_b saved");
  await expect(page.getByTestId("script-row-race_b")).toHaveAttribute("aria-current", "true");
  await page.getByTestId("script-row-race_a").click();
  await expect(code(page)).toContainText("# race_a edited");
  await expect(page.getByTestId("script-revert")).toHaveCount(0);
});

test("an error on line 401 does not read as an expired sign-in", async ({ page }) => {
  await gotoApp(page);
  await openEditor(page);
  await page.getByTestId("script-name").fill("long_one");
  const oauth: string[] = [];
  page.on("request", (r) => {
    if (/\/(token|authorize|register)/.test(r.url())) oauth.push(r.url());
  });
  await setCode(page, `${"# pad\n".repeat(396)}${script("    y = 1 // 0")}`);
  await page.getByTestId("script-save").click();
  await expect(page.getByTestId("script-errors")).toContainText("Line 401");
  await expect(page.getByTestId("script-status")).toHaveText("Not saved.");
  expect(oauth).toEqual([]);
  await expect(code(page)).toContainText("y = 1 // 0");
});

test("a failed chunk load says so and leaves the app working", async ({ page }) => {
  await gotoApp(page);
  await page.route("**/ScriptEditor.tsx*", (route) => route.abort());
  await page.getByTestId("script-editor-open").click();
  const failed = page.getByTestId("script-editor-failed");
  await expect(failed).toContainText("Could not load the script editor");
  await failed.getByRole("button", { name: "Close" }).click();
  await expect(failed).toHaveCount(0);
  await expect(page.locator(".chart-host")).toBeVisible();
});

/** Answer every call of `tool` with an error, as the server does; `page.unroute` ends it. */
async function failTool(page: Page, tool: string, text: string): Promise<void> {
  await page.route("**/mcp", async (route) => {
    const body = route.request().postData();
    if (!body?.includes(`"name":"${tool}"`)) return route.fallback();
    const { id } = JSON.parse(body) as { id: unknown };
    await route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({
        jsonrpc: "2.0",
        id,
        result: { content: [{ type: "text", text }], isError: true },
      }),
    });
  });
}

/** Hold the next calls of `tool` until `release`; `sent` resolves once one is out. */
async function holdTool(page: Page, tool: string) {
  let release = () => {};
  const held = new Promise<void>((resolve) => (release = resolve));
  let markSent = () => {};
  const sent = new Promise<void>((resolve) => (markSent = resolve));
  await page.route("**/mcp", async (route) => {
    if (route.request().postData()?.includes(`"name":"${tool}"`)) {
      markSent();
      await held;
    }
    await route.fallback();
  });
  return { sent, release: () => release() };
}

/** Save a new script under `name`, in the panel that is open. */
async function saveNew(page: Page, name: string, source: string): Promise<void> {
  await page.getByTestId("script-new").click();
  await page.getByTestId("script-name").fill(name);
  await setCode(page, source);
  await page.getByTestId("script-save").click();
  await expect(page.getByTestId("script-status")).toHaveText(/^Saved/);
}

/** A second tab of the same browser: same OAuth client, same drafts. */
async function secondTab(page: Page): Promise<Page> {
  const tab = await page.context().newPage();
  await gotoApp(tab);
  await openEditor(tab);
  return tab;
}

/** Wait until the draft debounce has written the text in `key`. */
const storedDraft = (page: Page, key: string) =>
  page.evaluate((k) => {
    const raw = localStorage.getItem("chartgen.scripts.drafts");
    const drafts = raw ? (JSON.parse(raw) as { drafts: Record<string, { source: string }> }).drafts : {};
    return Object.hasOwn(drafts, k) ? drafts[k]!.source : null;
  }, key);

test.describe("code is never lost", () => {
  test("a tab with a stale list cannot overwrite a script saved in another tab", async ({ page }) => {
    await gotoApp(page);
    await openEditor(page);
    await expect(page.getByText("No saved indicators yet.")).toBeVisible();

    const other = await secondTab(page);
    await saveNew(other, "foo", script("    pass  # from the other tab"));

    // This tab's list still says it has no `foo`.
    await expect(page.getByTestId("script-row-foo")).toHaveCount(0);
    await page.getByTestId("script-name").fill("foo");
    await setCode(page, script("    pass  # from this tab"));
    await page.getByTestId("script-save").click();
    await expect(page.getByTestId("script-errors")).toContainText("already have an indicator named foo");
    await expect(page.getByTestId("script-status")).toHaveText("Not saved.");
    await expect(code(page)).toContainText("# from this tab");

    await other.getByTestId("script-new").click();
    await other.getByTestId("script-row-foo").click();
    await expect(code(other)).toContainText("# from the other tab");
  });

  test("Save waits until the list has loaded", async ({ page }) => {
    await gotoApp(page);
    await failTool(page, "list_my_indicators", "The indicator store is unavailable");
    await openEditor(page);
    await expect(page.getByTestId("script-list-retry")).toBeVisible();
    await page.getByTestId("script-name").fill("blocked");
    await expect(page.getByTestId("script-save")).toBeDisabled();
    await code(page).click();
    await page.keyboard.press("ControlOrMeta+s");
    await expect(page.getByTestId("script-status")).toContainText("have not loaded yet");

    await page.unroute("**/mcp");
    await page.getByTestId("script-list-retry").click();
    await expect(page.getByTestId("script-save")).toBeEnabled();
    await page.getByTestId("script-save").click();
    await expect(page.getByTestId("script-status")).toHaveText("Saved.");
  });

  test("a script changed elsewhere is not replaced without asking", async ({ page }) => {
    await gotoApp(page);
    await openEditor(page);
    await saveNew(page, "shared", script("    pass  # version one"));
    await setCode(page, script("    pass  # my draft"));
    await expect.poll(() => storedDraft(page, "shared")).toContain("# my draft");

    // Another machine or MCP client saves over it.
    const other = await secondTab(page);
    await other.getByTestId("script-row-shared").click();
    await expect(other.getByTestId("script-conflict")).toHaveCount(0);
    await other.getByTestId("script-revert").click();
    await setCode(other, script("    pass  # version two"));
    await other.getByTestId("script-save").click();
    await expect(other.getByTestId("script-status")).toHaveText("Saved.");
    await other.close();

    // Saving what was already open is caught...
    await page.getByTestId("script-save").click();
    await expect(page.getByTestId("script-conflict")).toBeVisible();
    await expect(page.getByTestId("script-status")).toHaveText("Not saved: this script changed elsewhere.");
    await expect(page.getByTestId("script-save")).toBeDisabled();

    // ...and so is a draft reopened later, which says so instead of "restored".
    await page.getByTestId("script-close").click();
    await openEditor(page);
    await expect(code(page)).toContainText("# my draft");
    await expect(page.getByTestId("script-conflict")).toBeVisible();
    await expect(page.getByTestId("script-status")).not.toHaveText("Unsaved draft restored.");
    await expect(page.getByTestId("script-save")).toBeDisabled();

    await page.getByTestId("script-use-saved").click();
    await expect(code(page)).toContainText("# version two");
    await expect(page.getByTestId("script-conflict")).toHaveCount(0);
    await expect(page.getByTestId("script-revert")).toHaveCount(0);

    // The saved version replaced the text as one undoable edit: the draft is
    // still there, and Save now replaces knowingly.
    await code(page).click();
    await page.keyboard.press("ControlOrMeta+z");
    await expect(code(page)).toContainText("# my draft");
    await page.getByTestId("script-save").click();
    await expect(page.getByTestId("script-status")).toHaveText("Saved.");
    await reloadApp(page);
    await openEditor(page);
    await expect(code(page)).toContainText("# my draft");
    await expect(page.getByTestId("script-conflict")).toHaveCount(0);
  });

  test("Keep my draft lets Save replace the newer version", async ({ page }) => {
    await gotoApp(page);
    await openEditor(page);
    await saveNew(page, "kept", script("    pass  # one"));
    await setCode(page, script("    pass  # mine"));
    const other = await secondTab(page);
    await other.getByTestId("script-row-kept").click();
    await other.getByTestId("script-revert").click();
    await setCode(other, script("    pass  # two"));
    await other.getByTestId("script-save").click();
    await expect(other.getByTestId("script-status")).toHaveText("Saved.");

    await page.getByTestId("script-save").click();
    await page.getByTestId("script-keep-draft").click();
    await expect(page.getByTestId("script-conflict")).toHaveCount(0);
    await expect(code(page)).toContainText("# mine");
    await page.getByTestId("script-save").click();
    await expect(page.getByTestId("script-status")).toHaveText("Saved.");
    await other.getByTestId("script-new").click();
    await other.getByTestId("script-row-kept").click();
    await expect(code(other)).toContainText("# mine");
  });

  test("two tabs keep each other's drafts", async ({ page }) => {
    await gotoApp(page);
    await openEditor(page);
    await saveNew(page, "one", script("    pass  # one"));
    await saveNew(page, "two", script("    pass  # two"));
    const other = await secondTab(page);

    await page.getByTestId("script-row-one").click();
    await expect(code(page)).toContainText("# one");
    await setCode(page, script("    pass  # one drafted"));
    await expect.poll(() => storedDraft(page, "one")).toContain("# one drafted");

    await other.getByTestId("script-row-two").click();
    await expect(code(other)).toContainText("# two");
    await setCode(other, script("    pass  # two drafted"));
    await expect.poll(() => storedDraft(other, "two")).toContain("# two drafted");

    // Both are stored, and stay after the tabs flush on close.
    expect(await storedDraft(page, "one")).toContain("# one drafted");
    await other.close({ runBeforeUnload: true });
    await page.getByTestId("script-close").click();
    expect(await storedDraft(page, "one")).toContain("# one drafted");
    expect(await storedDraft(page, "two")).toContain("# two drafted");
  });

  test("the draft of a vanished script keeps its name and spares the new-script draft", async ({ page }) => {
    await gotoApp(page);
    await openEditor(page);
    await page.getByTestId("script-name").fill("precious_new");
    await setCode(page, script("    pass  # precious"));
    await expect.poll(() => storedDraft(page, "")).toContain("# precious");

    const other = await secondTab(page);
    await other.getByTestId("script-new").click();
    await other.getByTestId("script-name").fill("doomed");
    await setCode(other, script("    pass  # doomed saved"));
    await other.getByTestId("script-save").click();
    await expect(other.getByTestId("script-status")).toHaveText("Saved.");
    await page.getByTestId("script-close").click();
    await openEditor(page);
    await page.getByTestId("script-row-doomed").click();
    await expect(code(page)).toContainText("# doomed saved");
    await setCode(page, script("    pass  # doomed edited"));
    await expect.poll(() => storedDraft(page, "doomed")).toContain("# doomed edited");

    await other.getByTestId("script-row-doomed").click();
    await expect(other.getByTestId("script-delete")).toBeVisible();
    await other.getByTestId("script-delete").click();
    await other.getByTestId("script-delete-confirm").click();
    await expect(other.getByTestId("script-status")).toHaveText("Deleted doomed.");
    await other.close();

    await reloadApp(page);
    await openEditor(page);
    await expect(page.getByTestId("script-status")).toContainText("doomed is no longer saved");
    await expect(code(page)).toContainText("# doomed edited");
    expect(await storedDraft(page, "")).toContain("# precious");

    // Both reachable: the new script's draft, and the row of the vanished one.
    await page.getByTestId("script-new").click();
    await expect(code(page)).toContainText("# precious");
    await expect(page.getByTestId("script-name")).toHaveValue("precious_new");
    await expect(page.getByTestId("script-row-doomed")).toContainText("Draft, not saved");
    await page.getByTestId("script-row-doomed").click();
    await expect(code(page)).toContainText("# doomed edited");

    // Saving creates it again, and ends only its own draft.
    await page.getByTestId("script-save").click();
    await expect(page.getByTestId("script-status")).toHaveText("Saved.");
    expect(await storedDraft(page, "doomed")).toBeNull();
    expect(await storedDraft(page, "")).toContain("# precious");
  });

  test("a failed read is not taken for a deleted script", async ({ page }) => {
    await gotoApp(page);
    await openEditor(page);
    await saveNew(page, "flaky", script("    pass  # flaky saved"));
    await setCode(page, script("    pass  # flaky edited"));
    await expect.poll(() => storedDraft(page, "flaky")).toContain("# flaky edited");
    await page.getByTestId("script-close").click();

    await failTool(page, "get_my_indicator", "Could not read the indicator: disk I/O error");
    await openEditor(page);
    await expect(page.getByTestId("script-errors")).toContainText("disk I/O error");
    await expect(page.getByTestId("script-status")).not.toContainText("no longer saved");
    expect(await storedDraft(page, "flaky")).toContain("# flaky edited");

    await page.unroute("**/mcp");
    await page.getByTestId("script-row-flaky").click();
    await expect(code(page)).toContainText("# flaky edited");
    await expect(page.getByTestId("script-status")).toHaveText("Unsaved draft restored.");
  });

  test("a script called constructor has no phantom draft", async ({ page }) => {
    await gotoApp(page);
    await openEditor(page);
    await saveNew(page, "constructor", script("    pass  # odd name"));
    await page.getByTestId("script-new").click();
    await page.getByTestId("script-row-constructor").click();
    await expect(code(page)).toContainText("# odd name");
    await expect(page.getByTestId("script-status")).toHaveText("");
  });

  test("the last keystrokes survive opening another script", async ({ page }) => {
    await gotoApp(page);
    await openEditor(page);
    await saveNew(page, "first", script("    pass  # first"));
    await saveNew(page, "second", script("    pass  # second"));
    await page.getByTestId("script-row-first").click();
    await expect(code(page)).toContainText("# first");

    const held = await holdTool(page, "get_my_indicator");
    await page.getByTestId("script-row-second").click();
    await held.sent;
    await setCode(page, script("    pass  # typed while it loaded"));
    held.release();
    await expect(code(page)).toContainText("# second");
    await page.unroute("**/mcp");
    await page.getByTestId("script-row-first").click();
    await expect(code(page)).toContainText("# typed while it loaded");
  });

  test("Save during a pending save says so", async ({ page }) => {
    await gotoApp(page);
    await openEditor(page);
    await page.getByTestId("script-name").fill("slow_save");
    const held = await holdTool(page, "register_indicator");
    await page.getByTestId("script-save").click();
    await held.sent;
    await code(page).click();
    await page.keyboard.press("ControlOrMeta+s");
    await expect(page.getByTestId("script-status")).toHaveText("Still saving the last change…");
    held.release();
    await expect(page.getByTestId("script-status")).toHaveText("Saved.");
  });
});

test.describe("at 390 px", () => {
  test("the panel fits and works from the picker", async ({ page }) => {
    await gotoApp(page);
    await page.setViewportSize({ width: 390, height: 844 });
    await expect(page.getByTestId("script-editor-open")).toBeHidden();
    await page.getByTitle("Indicators", { exact: true }).click();
    await page.getByTestId("picker-script-editor").click();
    const panel = page.getByTestId("script-editor");
    await expect(panel).toBeVisible({ timeout: 30_000 });
    await expect(code(page)).toContainText("PARAMS");

    expect(await panel.boundingBox()).toEqual({ x: 0, y: 0, width: 390, height: 844 });
    expect(await panel.evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(true);
    for (const id of ["script-new", "script-name", "script-save", "script-add", "script-close", "script-docs"]) {
      const box = await page.getByTestId(id).boundingBox();
      expect(box, id).not.toBeNull();
      expect(box!.x, id).toBeGreaterThanOrEqual(0);
      expect(box!.x + box!.width, id).toBeLessThanOrEqual(390);
    }
    // The code keeps most of the height.
    expect((await page.getByTestId("script-code").boundingBox())!.height).toBeGreaterThan(400);

    await page.getByTestId("script-name").fill("narrow_one");
    await page.getByTestId("script-save").click();
    await expect(page.getByTestId("script-status")).toHaveText("Saved.");
    await page.getByTestId("script-add").click();
    await expect.poll(() => plotted(page, "narrow_one"), { timeout: 30_000 }).not.toBeNull();
    await page.getByTestId("script-close").click();
    await expect(panel).toHaveCount(0);
    await expect(page.locator(".chart-host")).toBeVisible();
  });
});

test.describe("screenshots", () => {
  const dir = process.env.SCRIPT_EDITOR_SHOTS;
  test.skip(!dir, "set SCRIPT_EDITOR_SHOTS to a directory to write them");

  const shoot = async (page: Page, name: string) => {
    await gotoApp(page);
    await openEditor(page);
    await page.getByTestId("script-name").fill("rsi_bands");
    await setCode(
      page,
      `# RSI with a signal line
PARAMS = [
    {"name": "length", "type": "int", "default": 14, "min": 2, "max": 200, "desc": "RSI length"},
]
META = {"title": "RSI + signal", "overlay": False}

def compute(ohlcv, params, chart):
    r = ta.rsi(ohlcv.close, params["length"])
    chart.line(r, title = "RSI", color = "purple")
    chart.hline(70.0, color = "gray")
    y = 1 // 0
`,
    );
    await page.getByTestId("script-save").click();
    await expect(page.getByTestId("script-errors")).toBeVisible();
    // klinecharts redraws its resized canvases on a later frame.
    await page.waitForTimeout(1000);
    mkdirSync(dir!, { recursive: true });
    await page.screenshot({ path: join(dir!, `${name}.png`) });
  };

  test("dark", async ({ page }) => {
    await shoot(page, "script-editor-dark");
  });

  test.describe("light", () => {
    test.use({ colorScheme: "light" });
    test("light", async ({ page }) => {
      await shoot(page, "script-editor-light");
    });
  });

  test.describe("narrow", () => {
    test("narrow", async ({ page }) => {
      await gotoApp(page);
      await page.setViewportSize({ width: 390, height: 844 });
      await page.getByTitle("Indicators", { exact: true }).click();
      await page.getByTestId("picker-script-editor").click();
      await expect(code(page)).toContainText("PARAMS", { timeout: 30_000 });
      await page.getByTestId("script-name").fill("narrow_shot");
      await page.getByTestId("script-save").click();
      await expect(page.getByTestId("script-status")).toHaveText("Saved.");
      mkdirSync(dir!, { recursive: true });
      await page.screenshot({ path: join(dir!, "script-editor-390.png") });
    });
  });
});
