// The script editor against a real server with CHARTGEN_SCRIPTING=1 and a
// throwaway store: the `scripting` project of playwright.config.ts (ADR-0041).
// Every test gets a fresh browser context, so a fresh OAuth client and no scripts.

import { mkdirSync } from "node:fs";
import { join } from "node:path";

import { expect, gotoApp, reloadApp, test } from "./fixtures/app";
import { devices, type Page } from "@playwright/test";

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

  test("a clean panel closing in another tab leaves a new-script draft alone", async ({ page }) => {
    await gotoApp(page);
    await openEditor(page);
    const other = await secondTab(page);

    await page.getByTestId("script-name").fill("precious");
    await setCode(page, script("    pass  # precious text"));
    await page.getByTestId("script-close").click();
    expect(await storedDraft(page, "")).toContain("# precious text");

    // The other tab has been clean on New all along.
    await other.getByTestId("script-close").click();
    expect(await storedDraft(other, "")).toContain("# precious text");

    await openEditor(other);
    await expect(code(other)).toContainText("# precious text");
    await expect(other.getByTestId("script-name")).toHaveValue("precious");
  });

  test("an untouched tab reloading leaves another tab's draft of the same script alone", async ({ page }) => {
    await gotoApp(page);
    await openEditor(page);
    await saveNew(page, "shared_two", script("    pass  # saved"));
    const other = await secondTab(page);
    await other.getByTestId("script-row-shared_two").click();
    await expect(code(other)).toContainText("# saved");
    await expect(other.getByTestId("script-revert")).toHaveCount(0);

    await setCode(page, script("    pass  # drafted"));
    await page.getByTestId("script-close").click();
    expect(await storedDraft(page, "shared_two")).toContain("# drafted");

    await reloadApp(other);
    expect(await storedDraft(other, "shared_two")).toContain("# drafted");
    await openEditor(other);
    await expect(code(other)).toContainText("# drafted");
  });

  test("Revert and Save still end this panel's own draft", async ({ page }) => {
    await gotoApp(page);
    await openEditor(page);
    await saveNew(page, "own_draft", script("    pass  # saved"));

    await setCode(page, script("    pass  # edited"));
    await expect.poll(() => storedDraft(page, "own_draft")).toContain("# edited");
    await page.getByTestId("script-revert").click();
    expect(await storedDraft(page, "own_draft")).toBeNull();

    await setCode(page, script("    pass  # edited again"));
    await expect.poll(() => storedDraft(page, "own_draft")).toContain("# edited again");
    await page.getByTestId("script-save").click();
    await expect(page.getByTestId("script-status")).toHaveText("Saved.");
    expect(await storedDraft(page, "own_draft")).toBeNull();
    await expect(page.getByTestId("script-row-own_draft")).not.toContainText("•");

    // A draft restored from an earlier visit is this panel's to end, too.
    await setCode(page, script("    pass  # from before"));
    await expect.poll(() => storedDraft(page, "own_draft")).toContain("# from before");
    await reloadApp(page);
    await openEditor(page);
    await expect(code(page)).toContainText("# from before");
    await page.getByTestId("script-revert").click();
    expect(await storedDraft(page, "own_draft")).toBeNull();
  });

  test("a stored draft equal to the saved text is cleared on opening", async ({ page }) => {
    await gotoApp(page);
    await openEditor(page);
    await saveNew(page, "same_text", script("    pass  # same"));
    const text = script("    pass  # same");
    await page.getByTestId("script-close").click();
    await page.evaluate((source) => {
      localStorage.setItem(
        "chartgen.scripts.drafts",
        JSON.stringify({ selected: null, drafts: { same_text: { name: "same_text", source, base: source } } }),
      );
    }, text);
    await openEditor(page);
    await page.getByTestId("script-row-same_text").click();
    await expect(code(page)).toContainText("# same");
    await expect.poll(() => storedDraft(page, "same_text")).toBeNull();
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
    await expect(page.getByTestId("script-conflict")).toHaveCount(0);
    await expect(page.getByTestId("script-revert")).toHaveCount(0);
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

/** Start with these indicators on the chart. Call before the first navigation. */
const seedIndicators = (page: Page, entries: unknown[]) =>
  page.addInitScript((value) => {
    if (localStorage.getItem("chartgen.indicators.active") === null) {
      localStorage.setItem("chartgen.indicators.active", JSON.stringify(value));
    }
  }, entries);

const errorChip = (page: Page) => page.locator("[data-chip-error]");

test.describe("names and params the server cannot run", () => {
  test("a name nobody knows fails only its chip, and removing it clears the error", async ({ page }) => {
    // A script of a deleted OAuth client, or deleted from another browser.
    await seedIndicators(page, ["rsi", "gone_one"]);
    await gotoApp(page);
    await expect(page.getByTestId("chart-load-error")).toHaveCount(0);
    await expect.poll(() => plotted(page, "rsi")).not.toBeNull();
    await expect(errorChip(page)).toHaveCount(1);
    await expect(errorChip(page).getByTitle(/Unknown indicator: gone_one/)).toBeVisible();
    // The desktop has the chips: the compact row stays out of the way.
    await expect(page.getByTestId("indicator-errors")).toBeHidden();

    await page.getByRole("button", { name: "Remove gone_one" }).click();
    await expect(errorChip(page)).toHaveCount(0, { timeout: 15_000 });
  });

  test("a script deleted elsewhere leaves the running chart working", async ({ page }) => {
    await gotoApp(page);
    await openEditor(page);
    await saveNew(page, "doomed_two", script('    chart.line(7.0, title = "Doomed")'));
    await page.getByTestId("script-add").click();
    await expect.poll(() => plotted(page, "doomed_two"), { timeout: 30_000 }).not.toBeNull();

    const other = await secondTab(page);
    await other.getByTestId("script-row-doomed_two").click();
    await other.getByTestId("script-delete").click();
    await other.getByTestId("script-delete-confirm").click();
    await expect(other.getByTestId("script-status")).toHaveText("Deleted doomed_two.");

    // The next tail reports it; the chart keeps polling, nothing blanks.
    await expect(errorChip(page)).toHaveCount(1, { timeout: 15_000 });
    await expect(page.getByTestId("chart-load-error")).toHaveCount(0);
    await reloadApp(page);
    await expect(page.getByTestId("chart-load-error")).toHaveCount(0);
    await expect.poll(() => plotted(page, "rsi")).not.toBeNull();
  });

  test("a saved layout holding a deleted script still loads", async ({ page }) => {
    const layoutMenu = async (item: RegExp) => {
      await page.getByRole("button", { name: /^Layout:/ }).click();
      await page.getByRole("menuitem", { name: item }).click();
    };
    const createLayout = async (name: string) => {
      await layoutMenu(/^Create new layout/);
      const input = page.getByRole("textbox", { name: "Layout name" });
      await input.fill(name);
      await input.press("Enter");
      await expect(page.getByTestId("layout-name")).toHaveText(name);
    };
    await gotoApp(page);
    await createLayout("Scripted");
    await openEditor(page);
    await saveNew(page, "in_layout", script('    chart.line(9.0, title = "Held")'));
    await page.getByTestId("script-add").click();
    await expect.poll(() => plotted(page, "in_layout"), { timeout: 30_000 }).not.toBeNull();
    // Ctrl+S on the chart saves the layout, with the script in it.
    await page.locator(".chart-container").click({ position: { x: 300, y: 60 } });
    await page.keyboard.press("Control+s");
    await expect(page.getByTestId("layout-dirty")).toHaveCount(0);

    await createLayout("Plain");
    await page.getByTestId("script-delete").click();
    await page.getByTestId("script-delete-confirm").click();
    await expect(page.getByTestId("script-status")).toHaveText("Deleted in_layout.");

    await layoutMenu(/^Load layout/);
    await page.getByRole("dialog").getByRole("button", { name: /^Scripted/ }).click();
    await expect(page.getByTestId("layout-name")).toHaveText("Scripted");
    await expect(errorChip(page)).toHaveCount(1, { timeout: 15_000 });
    await expect(page.getByTestId("chart-load-error")).toHaveCount(0);
  });

  test("share links keep known scripts and leave out unknown names", async ({ page }) => {
    await gotoApp(page);
    await openEditor(page);
    await saveNew(page, "linked", script('    chart.line(3.0, title = "Linked")'));

    await gotoApp(page, "/?ind=linked,gone_link");
    await expect(page.getByText("Unknown indicator(s) left out: gone_link")).toBeVisible();
    await expect.poll(() => plotted(page, "linked"), { timeout: 30_000 }).toEqual({ label: "Linked", last: 3 });
    await expect(errorChip(page)).toHaveCount(0);
  });

  test("params a saved script no longer allows show in the panel, not over the chart", async ({ page }) => {
    await seedIndicators(page, ["rsi", { name: "narrow_me", length: 300 }]);
    await gotoApp(page);
    await openEditor(page);
    const withMax = (max: number) => `PARAMS = [{"name": "length", "type": "int", "default": 5, "max": ${max}}]
META = {"title": "Narrow", "overlay": False}

def compute(ohlcv, params, chart):
    chart.line(5.0, title = "Narrow")
`;
    await saveNew(page, "narrow_me", withMax(500));
    await expect.poll(() => plotted(page, "narrow_me"), { timeout: 30_000 }).not.toBeNull();

    await setCode(page, withMax(200));
    await page.getByTestId("script-save").click();
    await expect(page.getByTestId("script-status")).toHaveText("Saved.");
    const errors = page.getByTestId("script-errors");
    await expect(errors).toContainText("Parameters on the chart", { timeout: 30_000 });
    await expect(errors).toContainText("length must be <= 200");
    await expect(page.getByTestId("chart-load-error")).toHaveCount(0);
    await expect(errorChip(page)).toHaveCount(1);
    await expect.poll(() => plotted(page, "rsi")).not.toBeNull();
  });

  test("a chart reload that fails after a save shows in the panel", async ({ page }) => {
    await gotoApp(page);
    await openEditor(page);
    await saveNew(page, "reload_me", script('    chart.line(1.0, title = "One")'));
    await page.getByTestId("script-add").click();
    await expect.poll(() => plotted(page, "reload_me"), { timeout: 30_000 }).not.toBeNull();

    await failTool(page, "generate_chart", "upstream is down");
    await setCode(page, script('    chart.line(2.0, title = "Two")'));
    await page.getByTestId("script-save").click();
    await expect(page.getByTestId("script-status")).toHaveText("Saved, but the chart did not reload.", {
      timeout: 15_000,
    });
    await expect(page.getByTestId("script-errors")).toContainText("Chart reload");
    await expect(page.getByTestId("script-errors")).toContainText("upstream is down");
  });
});

test.describe("live updates of a chart with a script", () => {
  test("a script shadowing a built-in keeps its values, and the chart polls", async ({ page }) => {
    const subscribes: string[] = [];
    page.on("request", (r) => {
      if (r.postData()?.includes('"name":"subscribe_chart"')) subscribes.push(r.url());
    });
    await gotoApp(page);
    await openEditor(page);
    await saveNew(
      page,
      "rsi",
      `PARAMS = [{"name": "length", "type": "int", "default": 14}]
META = {"title": "Mine", "overlay": False}

def compute(ohlcv, params, chart):
    chart.line(12345.0, title = "Mine")
`,
    );
    await expect(page.getByTestId("script-status")).toContainText("replaces the built-in rsi");
    await expect.poll(() => plotted(page, "rsi"), { timeout: 30_000 }).toEqual({ label: "Mine", last: 12345 });

    // Pushed tails came about once a second and carried the built-in's values.
    const atStart = subscribes.length;
    for (let i = 0; i < 16; i++) {
      await page.waitForTimeout(500);
      expect(await plotted(page, "rsi")).toEqual({ label: "Mine", last: 12345 });
    }
    // "not pushable" ends the attempts: no retry for this chart (first retry after 2 s).
    expect(subscribes.length).toBe(atStart);
    await expect(page.getByText("polling").first()).toBeVisible();
  });

  /** Big requests of generate_chart: full loads, not tail polls. */
  const countFullLoads = (page: Page) => {
    const loads = { count: 0 };
    page.on("request", (r) => {
      const body = r.postData();
      if (body?.includes('"name":"generate_chart"') && body.includes('"bars":500')) loads.count++;
    });
    return loads;
  };

  for (const [title, condition] of [
    ["only a full load", "len(ohlcv.close) > 1000"],
    ["only a tail", "len(ohlcv.close) > 100 and len(ohlcv.close) < 1000"],
  ] as const) {
    test(`a script failing on ${title} does not reload the chart over and over`, async ({ page }) => {
      await gotoApp(page);
      await openEditor(page);
      await saveNew(
        page,
        "one_window",
        script(`    if ${condition}:\n        y = 1 // 0\n    chart.line(1.0, title = "Window")`),
      );
      const loads = countFullLoads(page);
      await page.getByTestId("script-add").click();

      // The panel knows, on either window.
      const errors = page.getByTestId("script-errors");
      await expect(errors).toContainText("On the chart", { timeout: 30_000 });
      await expect(errors).toContainText("Line 6");

      await page.waitForTimeout(8000);
      expect(loads.count).toBeLessThanOrEqual(1);
      await expect(page.getByTestId("chart-load-error")).toHaveCount(0);
    });
  }
});

test.describe("panel size", () => {
  /** Whether a click at the center of the element would reach it. */
  const reachable = (page: Page, id: string) =>
    page.getByTestId(id).evaluate((el) => {
      const r = el.getBoundingClientRect();
      const hit = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
      return hit === el || el.contains(hit);
    });

  for (const [width, height] of [[1280, 700], [1280, 600], [1366, 650], [844, 390]] as const) {
    test(`full screen when the window is short: ${width}x${height}`, async ({ page }) => {
      await gotoApp(page);
      await page.setViewportSize({ width, height });
      await openEditor(page);
      const panel = page.getByTestId("script-editor");
      expect(await panel.boundingBox()).toEqual({ x: 0, y: 0, width, height });
      for (const id of ["script-new", "script-name", "script-save", "script-add", "script-close"]) {
        expect(await reachable(page, id), id).toBe(true);
      }
      expect((await page.getByTestId("script-code").boundingBox())!.height).toBeGreaterThan(100);
      expect(await panel.evaluate((el) => el.scrollHeight <= el.clientHeight)).toBe(true);

      // Closing brings the chart back at its usual height.
      await page.getByTestId("script-close").click();
      await expect(panel).toHaveCount(0);
      await expect(page.locator(".chart-host")).toBeVisible();
    });
  }

  for (const [width, height] of [[1280, 720], [1440, 780], [1920, 950]] as const) {
    test(`docked with room for the price pane: ${width}x${height}`, async ({ page }) => {
      await page.setViewportSize({ width, height });
      await gotoApp(page);
      await openEditor(page);
      const panel = (await page.getByTestId("script-editor").boundingBox())!;
      expect(panel.height).toBeLessThanOrEqual(height * 0.45 + 1);
      expect(panel.y + panel.height).toBeLessThanOrEqual(height);
      // Nothing of the chart's side reaches into the panel.
      const main = (await page.locator("main").boundingBox())!;
      expect(main.y + main.height).toBeLessThanOrEqual(panel.y);
      // A mouse keeps the compact controls and the editor's own text size.
      expect((await page.getByTestId("script-save").boundingBox())!.height).toBeLessThan(40);
      expect(await code(page).evaluate((el) => getComputedStyle(el).fontSize)).toBe("13px");
      // Docked, the chart may shrink; the rule is scoped away from the compact shell.
      expect(await page.locator(".chart-host").evaluate((el) => getComputedStyle(el).minHeight)).toBe("120px");
      // The drawing toolbar still fits: its last button is not cut off.
      const last = (await page.getByRole("button", { name: "Remove all drawings" }).boundingBox())!;
      expect(last.y + last.height).toBeLessThanOrEqual(main.y + main.height);
      for (const id of ["script-new", "script-name", "script-save", "script-close"]) {
        expect(await reachable(page, id), id).toBe(true);
      }
      await expect
        .poll(
          () =>
            page.evaluate(() => {
              const t = (window as unknown as {
                __chartgenTest: {
                  chart: () => { getSize: (pane: string) => { height: number } | null } | null;
                };
              }).__chartgenTest;
              return t.chart()?.getSize("candle_pane")?.height ?? 0;
            }),
          { timeout: 5_000 },
        )
        .toBeGreaterThanOrEqual(60);
    });
  }
});

test("a failed tools/list at boot does not hide the editor for the session", async ({ page }) => {
  let failures = 0;
  await page.route("**/mcp", async (route) => {
    if (route.request().postData()?.includes('"tools/list"') && failures === 0) {
      failures++;
      return route.abort();
    }
    return route.fallback();
  });
  await gotoApp(page);
  await expect(page.getByTestId("script-editor-open")).toBeVisible({ timeout: 10_000 });
  expect(failures).toBe(1);
});

/** A device's context options; the suite only runs Chromium. */
const device = (name: string) => {
  const { defaultBrowserType: _browser, ...options } = devices[name]!;
  return options;
};

/** Boot at a size where the status text is hidden: wait for the panes instead. */
async function bootPhone(page: Page): Promise<void> {
  const exchanged = page.waitForResponse(
    (res) => res.request().method() === "POST" && new URL(res.url()).pathname === "/token",
  );
  await page.goto("/");
  expect((await exchanged).ok()).toBe(true);
  await page.waitForURL((url) => !url.searchParams.has("code"));
  await expect(page.locator(".pane-header").first()).toBeVisible({ timeout: 20_000 });
}

/** Open the panel the only way a phone can: from the indicator picker. */
async function openOnPhone(page: Page): Promise<void> {
  await expect(page.getByTestId("script-editor-open")).toBeHidden();
  await page.getByTitle("Indicators", { exact: true }).tap();
  await page.getByTestId("picker-script-editor").tap();
  await expect(page.getByTestId("script-editor")).toBeVisible({ timeout: 30_000 });
  await expect(code(page)).toContainText("PARAMS");
}

const inViewport = async (page: Page, id: string) => {
  const box = (await page.getByTestId(id).boundingBox())!;
  const { width, height } = page.viewportSize()!;
  return box.x >= 0 && box.y >= 0 && box.x + box.width <= width && box.y + box.height <= height;
};

test.describe("on the compact shell", () => {
  const DEVICES = [
    { name: "iPhone 13", use: device("iPhone 13"), minCode: 300 },
    { name: "iPhone 13 landscape", use: device("iPhone 13 landscape"), minCode: 180 },
    { name: "Pixel 7", use: device("Pixel 7"), minCode: 300 },
    { name: "Pixel 7 landscape", use: device("Pixel 7 landscape"), minCode: 180 },
    {
      name: "360x640 phone",
      use: { viewport: { width: 360, height: 640 }, isMobile: true, hasTouch: true },
      minCode: 250,
    },
    // Wide and tall: only `compact` makes these two full screen, no size query would.
    { name: "iPad Mini", use: device("iPad Mini"), minCode: 500 },
    {
      name: "1000x800 touch tablet",
      use: { viewport: { width: 1000, height: 800 }, isMobile: true, hasTouch: true },
      minCode: 400,
    },
  ];

  for (const { name, use, minCode } of DEVICES) {
    test.describe(name, () => {
      test.use(use);

      test("the panel fills the screen and works by touch", async ({ page }) => {
        await bootPhone(page);
        expect(await page.evaluate(() => document.documentElement.classList.contains("compact"))).toBe(true);
        await openOnPhone(page);

        const { width, height } = page.viewportSize()!;
        const panel = page.getByTestId("script-editor");
        expect(await panel.boundingBox()).toEqual({ x: 0, y: 0, width, height });
        expect(await panel.evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(true);
        expect(await panel.evaluate((el) => el.scrollHeight <= el.clientHeight)).toBe(true);
        for (const id of ["script-new", "script-name", "script-save", "script-add", "script-close", "script-docs"]) {
          expect(await inViewport(page, id), id).toBe(true);
        }
        expect((await page.getByTestId("script-code").boundingBox())!.height).toBeGreaterThan(minCode);
        for (const id of ["script-new", "script-save", "script-add", "script-close"]) {
          const box = (await page.getByTestId(id).boundingBox())!;
          expect(Math.min(box.width, box.height), id).toBeGreaterThanOrEqual(40);
        }
        expect(await code(page).evaluate((el) => getComputedStyle(el).fontSize)).toBe("16px");
        // The compact shell's own rule stands: nothing forces the chart taller.
        expect(await page.locator(".chart-host").evaluate((el) => getComputedStyle(el).minHeight)).toBe("0px");
        // Nothing of the shell is on top of the panel.
        expect(
          await page.getByTestId("script-close").evaluate((el) => {
            const r = el.getBoundingClientRect();
            return el.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2));
          }),
        ).toBe(true);

        await page.getByTestId("script-name").fill("touch_one");
        if (process.env.SCRIPT_EDITOR_SHOTS) {
          await setCode(page, script("    y = 1 // 0"));
          await page.getByTestId("script-save").tap();
          await expect(page.getByTestId("script-errors")).toBeVisible();
          await page.screenshot({
            path: join(process.env.SCRIPT_EDITOR_SHOTS, `phone-${name.replace(/\W+/g, "-")}.png`),
          });
          await setCode(page, script('    chart.line(1.0, title = "Shot")'));
        }
        await page.getByTestId("script-save").tap();
        await expect(page.getByTestId("script-status")).toHaveText("Saved.");
        await page.getByTestId("script-add").tap();
        await expect.poll(() => plotted(page, "touch_one"), { timeout: 30_000 }).not.toBeNull();

        // Delete asks in a dialog that fits, then the chart is back.
        await page.getByTestId("script-delete").tap();
        expect(await inViewport(page, "script-delete-confirm")).toBe(true);
        expect((await page.getByTestId("script-delete-confirm").boundingBox())!.height).toBeGreaterThanOrEqual(40);
        await page.getByTestId("script-delete-confirm").tap();
        await expect(page.getByTestId("script-status")).toHaveText("Deleted touch_one.");
        await page.getByTestId("script-close").tap();
        await expect(panel).toHaveCount(0);
        await expect(page.locator(".chart-host")).toBeVisible();
      });
    });
  }

  for (const name of ["iPhone 13", "iPhone 13 landscape"]) {
    test.describe(`${name}, failed indicators`, () => {
      test.use(device(name));
      const row = (page: Page) => page.getByTestId("indicator-errors");

      test("an unknown name shows its reason as text, and can be removed", async ({ page }) => {
        await seedIndicators(page, ["rsi", "gone_x"]);
        await bootPhone(page);
        await expect(row(page)).toBeVisible({ timeout: 15_000 });
        await expect(row(page)).toContainText("Unknown indicator: gone_x");
        // Only what failed: this is not the chip row.
        await expect(row(page).locator("li")).toHaveCount(1);
        await expect(row(page)).not.toContainText("RSI");
        const box = (await row(page).boundingBox())!;
        expect(box.y + box.height).toBeLessThanOrEqual(page.viewportSize()!.height);

        await page.getByRole("button", { name: "Remove gone_x" }).tap();
        await expect(row(page)).toHaveCount(0, { timeout: 15_000 });
        expect(await plotted(page, "rsi")).not.toBeNull();
      });

      test("with nothing else on the chart it is still explained", async ({ page }) => {
        await seedIndicators(page, ["gone_x"]);
        await bootPhone(page);
        await expect(row(page)).toContainText("Unknown indicator: gone_x", { timeout: 15_000 });
      });

      test("a script failing at runtime shows its message", async ({ page }) => {
        await bootPhone(page);
        await openOnPhone(page);
        await page.getByTestId("script-name").fill("fails_late");
        await setCode(page, script("    if len(ohlcv.close) > 100:\n        y = 1 // 0\n    chart.line(ohlcv.close)"));
        await page.getByTestId("script-save").tap();
        await expect(page.getByTestId("script-status")).toHaveText("Saved.");
        await page.getByTestId("script-add").tap();
        await expect(page.getByTestId("script-errors")).toContainText("On the chart", { timeout: 30_000 });
        await page.getByTestId("script-close").tap();
        await expect(row(page)).toContainText("Floor division by zero");
      });
    });
  }

  test.describe("iPhone 13 with its insets and keyboard", () => {
    test.use(device("iPhone 13"));

    test("stays clear of the notch and the home indicator", async ({ page }) => {
      await bootPhone(page);
      const cdp = await page.context().newCDPSession(page);
      await cdp.send("Emulation.setSafeAreaInsetsOverride", {
        insets: { top: 47, bottom: 34, left: 0, right: 0 },
      });
      await openOnPhone(page);
      const { height } = page.viewportSize()!;
      const close = (await page.getByTestId("script-close").boundingBox())!;
      expect(close.y).toBeGreaterThanOrEqual(47);
      const editor = (await page.getByTestId("script-code").boundingBox())!;
      expect(editor.y + editor.height).toBeLessThanOrEqual(height - 34);
    });

    test("fits what the on-screen keyboard leaves visible", async ({ page }) => {
      await bootPhone(page);
      await openOnPhone(page);
      // What lib/viewport.ts sets from the visual viewport when a keyboard opens:
      // the emulator has no real keyboard to open.
      await page.evaluate(() => {
        document.documentElement.style.setProperty("--vvh", "400px");
        document.documentElement.style.setProperty("--vvt", "20px");
      });
      const panel = (await page.getByTestId("script-editor").boundingBox())!;
      expect(panel.y).toBe(20);
      expect(panel.height).toBe(400);

      await page.getByTestId("script-name").fill("kb_one");
      await setCode(page, script("    y = 1 // 0"));
      await page.getByTestId("script-save").tap();
      const errors = page.getByTestId("script-errors");
      await expect(errors).toContainText("Line 5");
      // The error and the code above it stay above the keyboard.
      const list = (await errors.boundingBox())!;
      expect(list.y + list.height).toBeLessThanOrEqual(420);
      const editor = (await page.getByTestId("script-code").boundingBox())!;
      expect(editor.height).toBeGreaterThan(80);
      await expect(page.locator("[data-testid=script-code] .cm-script-error-line")).toBeVisible();
    });
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
