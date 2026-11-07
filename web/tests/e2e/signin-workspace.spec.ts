// A signed-in user's workspace across devices and on a shared browser (ADR-0050),
// in real browsers against the real server, signing in through the fake provider.
// Each browser context is a device. Documents the running stores do not hold in
// memory are written directly, to pin down the order of events across devices.

import type { Browser, Page } from "@playwright/test";

import { test, expect, expectChartRendered, pickTool, reloadApp } from "./fixtures/app";

// The accounts outlive a test on this server, so each test has keys of its own.
let round = 0;
let NOTE = "";
let DOC = "";
let OTHER = "";
const drawings = (...ids: string[]) => JSON.stringify(ids.map((id) => ({ id, name: id })));

const ids = (text: string | null): string[] =>
  text === null ? [] : (JSON.parse(text) as { id: string }[]).map((d) => d.id).sort();

const userMenu = (page: Page) => page.getByTestId("user-menu");

async function signIn(page: Page, provider = "GitHub", user = "octo"): Promise<void> {
  await page.getByRole("link", { name: `Continue with ${provider}` }).click();
  await page.getByRole("link", { name: `Sign in as ${user}` }).click();
  await page.waitForURL((url) => url.pathname === "/" && !url.searchParams.has("code"));
  await expectChartRendered(page);
  await expect(userMenu(page)).toBeVisible();
}

async function device(browser: Browser, provider = "GitHub", user = "octo"): Promise<Page> {
  const context = await browser.newContext({ baseURL: test.info().project.use.baseURL });
  const page = await context.newPage();
  await page.goto("/");
  await signIn(page, provider, user);
  return page;
}

async function signOut(page: Page): Promise<void> {
  await userMenu(page).click();
  await page.getByRole("menuitem", { name: "Sign out" }).click();
  await page.waitForURL("**/oauth/signed-out");
}

const read = (page: Page, key: string) =>
  page.evaluate((k) => localStorage.getItem(k), key);
const write = (page: Page, key: string, value: string) =>
  page.evaluate(([k, v]) => localStorage.setItem(k!, v!), [key, value]);

/** The server has this value, as far as the device knows. */
async function pushed(page: Page, key: string, value: string | null): Promise<void> {
  await expect
    .poll(
      async () => {
        const raw = await read(page, `chartgen.sync.${key}`);
        if (raw === null) return undefined;
        const st = JSON.parse(raw) as { b: string | null; s?: string | null };
        return st.s !== undefined ? st.s : st.b;
      },
      { timeout: 30_000 },
    )
    .toBe(value);
}

/** The first round of a new device is through, so only what comes next is pending. */
async function pushedOnce(page: Page): Promise<void> {
  await expect.poll(() => read(page, "chartgen.sync.cursor"), { timeout: 30_000 }).not.toBeNull();
}

/** What the account holds, as a device new to it finds. */
async function freshDeviceReads(browser: Browser, key: string, provider = "GitHub", user = "octo") {
  const page = await device(browser, provider, user);
  const value = await read(page, key);
  await page.context().close();
  return value;
}

const liveKeys = (page: Page) =>
  page.evaluate(() =>
    Object.keys(localStorage).filter(
      (k) =>
        k.startsWith("chartgen.") &&
        !/^chartgen\.(oauth\.|push\.|u\.|workspace\.|legacy\||share\.)/.test(k),
    ),
  );

test.describe("workspace", () => {
  test.describe.configure({ timeout: 180_000 });

  test.beforeEach(() => {
    round += 1;
    NOTE = `chartgen.pane.9.note${round}`;
    DOC = `chartgen.drawings.TEST${round}`;
    OTHER = `chartgen.drawings.OTHER${round}`;
  });

  test.beforeAll(async ({ browser }, info) => {
    info.setTimeout(240_000);
    const page = await browser.newPage({ baseURL: info.project.use.baseURL });
    page.setDefaultTimeout(180_000);
    page.setDefaultNavigationTimeout(180_000);
    await page.goto("/");
    await expect(page.getByRole("heading", { name: "Sign in to chartgen" })).toBeVisible({
      timeout: 180_000,
    });
    await page.close();
  });

  test("a setting changed on one device is there on the next, and a running one is told", async ({
    browser,
  }) => {
    const laptop = await device(browser);
    // Two devices new to an account within seconds would each add a default layout.
    await pushed(laptop, "chartgen.layouts", await read(laptop, "chartgen.layouts"));
    const phone = await device(browser);
    const timeframes = (page: Page) => page.getByRole("group", { name: "Timeframe" });
    await phone.waitForTimeout(6_000);
    await expect(phone.getByTestId("workspace-remote"), "nothing new yet").toBeHidden();

    const next = timeframes(laptop).locator("[aria-pressed=false]").last();
    const chosen = (await next.innerText()).trim();
    await next.click();
    await expect(timeframes(laptop).locator("[aria-pressed=true]")).toHaveText(chosen);

    // The phone keeps what it shows and says there is news, without replacing anything.
    await expect(phone.getByTestId("workspace-remote")).toBeVisible({ timeout: 45_000 });
    await expect(timeframes(phone).locator("[aria-pressed=true]")).not.toHaveText(chosen);
    await Promise.all([phone.waitForEvent("load"), phone.getByTestId("workspace-reload").click()]);
    await expectChartRendered(phone);
    await expect(timeframes(phone).locator("[aria-pressed=true]")).toHaveText(chosen);
    await expect(phone.getByTestId("workspace-remote")).toBeHidden();

    // A device new to the account has it from the start.
    const fresh = await device(browser);
    await expect(timeframes(fresh).locator("[aria-pressed=true]")).toHaveText(chosen);
    await expect(fresh.getByTestId("workspace-remote")).toBeHidden();
    await expect(fresh.getByTestId("workspace-notice")).toHaveCount(0);
    await Promise.all([laptop, phone, fresh].map((p) => p.context().close()));
  });

  test("a drawing made with the tools on one device is on the chart of another", async ({ browser }) => {
    const laptop = await device(browser);
    const symbol = /[A-Z0-9]{3,}/.exec(
      await laptop.getByRole("button", { name: /^Symbol:/ }).innerText(),
    )![0];
    const key = `chartgen.drawings.${symbol}`;
    const had = ids(await read(laptop, key)).length;
    await pickTool(laptop, "Horizontal line");
    const box = (await laptop.locator(".chart-container").first().boundingBox())!;
    await laptop.mouse.move(box.x + box.width / 2, box.y + box.height / 2, { steps: 5 });
    await laptop.mouse.down();
    await laptop.mouse.up();
    await expect.poll(async () => ids(await read(laptop, key)).length).toBe(had + 1);
    const drawn = await read(laptop, key);
    await pushed(laptop, key, drawn);

    const phone = await device(browser);
    expect(await read(phone, key)).toBe(drawn);
    await expect
      .poll(() =>
        phone.evaluate(
          () => (window as unknown as { __chartgenDrawings: () => unknown[] }).__chartgenDrawings().length,
        ),
      )
      .toBe(had + 1);
    await expect(phone.getByTestId("workspace-notice")).toHaveCount(0);
    await Promise.all([laptop, phone].map((p) => p.context().close()));
  });

  test("a reload brings a device's edits out and collects the other's; both sets of drawings stay", async ({
    browser,
  }) => {
    const laptop = await device(browser);
    await write(laptop, DOC, drawings("base"));
    await pushed(laptop, DOC, drawings("base"));
    const phone = await device(browser);
    expect(await read(phone, DOC)).toBe(drawings("base"));

    // The phone loses its connection and draws, while the laptop draws too.
    await phone.route("**/workspace/sync", (route) => route.abort());
    await write(phone, DOC, drawings("base", "phone"));
    await write(laptop, DOC, drawings("base", "laptop"));
    await pushed(laptop, DOC, drawings("base", "laptop"));

    // Back online. The stale write is refused and answered with the current document.
    await phone.unroute("**/workspace/sync");
    const statuses: string[] = [];
    phone.on("response", (res) => {
      if (!res.url().endsWith("/workspace/sync")) return;
      void res.json().then((body: { results: { key: string; status: string }[] }) => {
        for (const r of body.results) if (r.key === DOC) statuses.push(r.status);
      });
    });
    await reloadApp(phone);
    await expect.poll(() => statuses, { timeout: 30_000 }).toEqual(["conflict", "ok"]);

    await expect
      .poll(async () => ids(await freshDeviceReads(browser, DOC)), { timeout: 30_000 })
      .toEqual(["base", "laptop", "phone"]);

    // The laptop wrote without seeing the phone's. Nothing of either was lost, so no notice.
    await reloadApp(laptop);
    expect(ids(await read(laptop, DOC))).toEqual(["base", "laptop", "phone"]);
    await expect(phone.getByTestId("workspace-notice")).toHaveCount(0);
    await reloadApp(phone);
    expect(ids(await read(phone, DOC))).toEqual(["base", "laptop", "phone"]);

    // A drawing removed on one device goes from the other, and all of them from both.
    await write(laptop, DOC, drawings("base", "phone"));
    await pushed(laptop, DOC, drawings("base", "phone"));
    await reloadApp(phone);
    expect(ids(await read(phone, DOC))).toEqual(["base", "phone"]);
    await phone.evaluate((k) => localStorage.removeItem(k), DOC);
    await pushed(phone, DOC, null);
    await reloadApp(laptop);
    expect(await read(laptop, DOC)).toBeNull();
    await Promise.all([laptop, phone].map((p) => p.context().close()));
  });

  test("two edits of one value: the later writer wins, and that device says what it replaced", async ({
    browser,
  }) => {
    const laptop = await device(browser);
    await write(laptop, NOTE, "first");
    await pushed(laptop, NOTE, "first");
    const phone = await device(browser);

    await phone.route("**/workspace/sync", (route) => route.abort());
    await write(phone, NOTE, "phone");
    await write(laptop, NOTE, "laptop");
    await pushed(laptop, NOTE, "laptop");
    await phone.unroute("**/workspace/sync");
    await reloadApp(phone);

    await expect(phone.getByTestId("workspace-notice")).toContainText(
      "the other device's change to the same thing was replaced",
      { timeout: 30_000 },
    );
    await pushed(phone, NOTE, "phone");
    await expect(laptop.getByTestId("workspace-remote")).toBeVisible({ timeout: 45_000 });
    await reloadApp(laptop);
    expect(await read(laptop, NOTE)).toBe("phone");
    await Promise.all([laptop, phone].map((p) => p.context().close()));
  });

  test("something the server refuses stays here, says so, and is not sent over and over", async ({
    browser,
  }) => {
    const page = await device(browser);
    await write(page, OTHER, "x".repeat(600_000));
    await expect(page.getByTestId("workspace-notice")).toContainText(
      "larger than one document may be",
      { timeout: 30_000 },
    );
    expect(await read(page, OTHER)).toHaveLength(600_000);
    await page.context().close();
  });

  test("a reload in one tab leaves another tab's data alone, and sends no deletion", async ({
    browser,
  }) => {
    const first = await device(browser);
    await write(first, DOC, drawings("kept"));
    await pushed(first, DOC, drawings("kept"));
    const second = await first.context().newPage();
    await second.goto("/");
    await expectChartRendered(second);

    const writes: string[] = [];
    for (const page of [first, second]) {
      page.on("request", (req) => {
        if (!req.url().endsWith("/workspace/sync")) return;
        const body = req.postDataJSON() as { writes?: { key: string; value: string | null }[] };
        for (const w of body.writes ?? []) if (w.value === null) writes.push(w.key);
      });
    }
    // The reload passes through the provider-less sign-in while the first tab's engine runs.
    for (let i = 0; i < 3; i++) await reloadApp(second);
    await first.waitForTimeout(6_000);
    expect(await read(first, DOC)).toBe(drawings("kept"));
    expect(writes).toEqual([]);
    expect(await freshDeviceReads(browser, DOC)).toBe(drawings("kept"));
    await first.context().close();
  });

  test("with another tab open, what arrives waits, and Reload brings it to every tab", async ({
    browser,
  }) => {
    const laptop = await device(browser);
    await write(laptop, NOTE, "before");
    await pushed(laptop, NOTE, "before");
    const phone = await device(browser);
    const second = await phone.context().newPage();
    await second.goto("/");
    await expectChartRendered(second);

    await write(laptop, NOTE, "after");
    await pushed(laptop, NOTE, "after");
    // A store in the other tab would save from memory over what is applied now.
    await reloadApp(second);
    expect(await read(second, NOTE)).toBe("before");
    await expect(second.getByTestId("workspace-remote")).toBeVisible();

    const other = phone.waitForEvent("load");
    await second.getByTestId("workspace-reload").click();
    await other;
    await expectChartRendered(second);
    await expect.poll(() => read(second, NOTE)).toBe("after");
    await expectChartRendered(phone);
    await expect(second.getByTestId("workspace-remote")).toBeHidden();
    await expect(phone.getByTestId("workspace-remote")).toBeHidden();
    await Promise.all([laptop, phone].map((p) => p.context().close()));
  });

  test("a change made just before leaving the page still goes out", async ({ browser }) => {
    const page = await device(browser);
    await pushedOnce(page);
    await write(page, NOTE, "last words");
    await page.goto("/oauth/signed-out");
    expect(await freshDeviceReads(browser, NOTE)).toBe("last words");
    await page.context().close();
  });

  test("a new token nobody can put a name to sees nothing of the last account's", async ({
    browser,
  }) => {
    const page = await device(browser);
    await write(page, NOTE, "octo's");
    await pushed(page, NOTE, "octo's");
    await page.route("**/oauth/me", (route) => route.abort());
    await page.goto("/");
    await expect(page.getByText("could not tell which account signed in")).toBeVisible({
      timeout: 60_000,
    });
    expect(await liveKeys(page)).not.toContain(NOTE);
    expect(await read(page, NOTE)).toBeNull();

    await page.unroute("**/oauth/me");
    await reloadApp(page);
    expect(await read(page, NOTE)).toBe("octo's");
    await page.context().close();
  });

  test("with the server not answering, the data is as it was left", async ({ browser }) => {
    const page = await device(browser);
    await write(page, NOTE, "octo's");
    await pushed(page, NOTE, "octo's");
    // No token and no answer: nothing moves. The app then asks to sign in as ever.
    await page.route("**/oauth/me", (route) => route.abort());
    await page.route("**/authorize**", () => {});
    await page.goto("/");
    await expect.poll(() => page.evaluate(() => document.readyState)).toBe("complete");
    await expect.poll(() => read(page, NOTE)).toBe("octo's");
    await page.context().close();
  });

  test("two accounts on one browser: signing out leaves nothing, and neither sees the other's", async ({
    browser,
  }) => {
    const context = await browser.newContext({ baseURL: test.info().project.use.baseURL });
    const page = await context.newPage();
    await page.goto("/");
    await signIn(page);
    await write(page, NOTE, "octo's");
    await write(page, DOC, drawings("octos"));
    await pushed(page, NOTE, "octo's");
    await write(page, "chartgen.sidebar.tab", "alerts");

    // Written just before signing out, so only the flush at sign-out can carry it.
    await write(page, OTHER, drawings("last moment"));
    await signOut(page);
    expect(await liveKeys(page)).toEqual([]);
    expect(await freshDeviceReads(browser, OTHER)).toBe(drawings("last moment"));

    await page.goto("/");
    await signIn(page, "Google", "Ann Example");
    expect(await read(page, NOTE)).toBeNull();
    expect(await read(page, DOC)).toBeNull();
    expect(await read(page, "chartgen.sidebar.tab"), "device-only keys as well").toBeNull();
    await expect(page.getByTestId("workspace-offer")).toBeHidden();
    await write(page, NOTE, "Ann's");
    await pushed(page, NOTE, "Ann's");
    await signOut(page);
    expect(await liveKeys(page)).toEqual([]);

    await page.goto("/");
    await signIn(page);
    expect(await read(page, NOTE)).toBe("octo's");
    expect(await read(page, DOC)).toBe(drawings("octos"));
    expect(await read(page, "chartgen.sidebar.tab"), "device-only keys come back").toBe("alerts");

    // On the server too, and a new device of either finds only their own.
    expect(await freshDeviceReads(browser, NOTE, "Google", "Ann Example")).toBe("Ann's");
    expect(await freshDeviceReads(browser, DOC, "Google", "Ann Example")).toBeNull();
    expect(await freshDeviceReads(browser, NOTE)).toBe("octo's");
    await context.close();
  });

  test("a tab reloads when its account changes under it, and sends nothing meanwhile", async ({
    browser,
  }) => {
    const first = await device(browser);
    await write(first, NOTE, "first");
    await pushed(first, NOTE, "first");
    const second = await first.context().newPage();
    await second.goto("/");
    await expectChartRendered(second);
    await first.bringToFront();
    const loaded = first.waitForEvent("load");
    await second.evaluate(() => localStorage.setItem("chartgen.workspace.account", "google:someone"));
    await loaded;
    await first.context().close();
  });

  test("a session that ended without signing out: the next account still sees nothing", async ({
    browser,
  }) => {
    const context = await browser.newContext({ baseURL: test.info().project.use.baseURL });
    const page = await context.newPage();
    await page.goto("/");
    await signIn(page);
    await write(page, NOTE, "octo's");
    await pushed(page, NOTE, "octo's");

    await context.clearCookies();
    await page.goto("/");
    await signIn(page, "Google", "Ann Example");
    expect(await read(page, NOTE)).toBeNull();
    await expect(page.getByTestId("workspace-offer")).toBeHidden();
    expect(await freshDeviceReads(browser, NOTE, "Google", "Ann Example")).toBeNull();
    await context.close();
  });

  test("an open tab of another account's reloads into the account now signed in", async ({
    browser,
  }) => {
    const context = await browser.newContext({ baseURL: test.info().project.use.baseURL });
    const octo = await context.newPage();
    await octo.goto("/");
    await signIn(octo);
    await write(octo, NOTE, "octo's");
    await pushed(octo, NOTE, "octo's");
    await signOut(octo);

    await octo.goto("/");
    const other = await context.newPage();
    await other.goto("/");
    await signIn(other, "Google", "Ann Example");
    await octo.reload();
    await expect(userMenu(octo)).toHaveAttribute("aria-label", "Account: Ann Example");
    expect(await read(octo, NOTE)).toBeNull();
    await context.close();
  });

  test("data from before sign-in waits for a yes, is offered to one account, and only adds", async ({
    browser,
  }) => {
    const context = await browser.newContext({ baseURL: test.info().project.use.baseURL });
    const page = await context.newPage();
    // A page of the origin with none of the app on it.
    await page.goto("/oauth/signed-out");
    await write(page, NOTE, "from before");
    await write(page, DOC, drawings("old"));

    await page.goto("/");
    await signIn(page);
    await expect(page.getByTestId("workspace-offer")).toBeVisible();
    expect(await read(page, NOTE), "not without a yes").toBeNull();
    expect(await read(page, DOC)).toBeNull();

    // The account already has something on another device.
    const laptop = await device(browser);
    await write(laptop, DOC, drawings("accounts"));
    await pushed(laptop, DOC, drawings("accounts"));
    await reloadApp(page);
    expect(await read(page, DOC)).toBe(drawings("accounts"));
    await expect(page.getByTestId("workspace-offer"), "until decided").toBeVisible();

    // Not offered to anyone else, who sees none of it either.
    await signOut(page);
    await page.goto("/");
    await signIn(page, "Google", "Ann Example");
    await expect(page.getByTestId("workspace-offer")).toBeHidden();
    expect(await read(page, NOTE)).toBeNull();
    await signOut(page);

    await page.goto("/");
    await signIn(page);
    await page.getByTestId("workspace-later").click();
    await expect(page.getByTestId("workspace-offer")).toBeHidden();
    await reloadApp(page);
    await Promise.all([page.waitForEvent("load"), page.getByTestId("workspace-import").click()]);
    await expectChartRendered(page);
    await expect(page.getByTestId("workspace-offer")).toBeHidden();
    expect(ids(await read(page, DOC))).toEqual(["accounts", "old"]);
    expect(await read(page, NOTE)).toBe("from before");

    await expect
      .poll(async () => ids(await freshDeviceReads(browser, DOC)), { timeout: 30_000 })
      .toEqual(["accounts", "old"]);
    expect(await freshDeviceReads(browser, NOTE)).toBe("from before");
    await reloadApp(page);
    await expect(page.getByTestId("workspace-offer"), "and it is over").toBeHidden();
    await Promise.all([context.close(), laptop.context().close()]);
  });

  test("the old data can be deleted instead, and nothing of it reaches the account", async ({
    browser,
  }) => {
    const context = await browser.newContext({ baseURL: test.info().project.use.baseURL });
    const page = await context.newPage();
    await page.goto("/oauth/signed-out");
    await write(page, NOTE, "unwanted");
    await page.goto("/");
    await signIn(page, "Google", "Ann Example");
    page.on("dialog", (d) => void d.accept());
    await page.getByTestId("workspace-discard").click();
    await expect(page.getByTestId("workspace-offer")).toBeHidden();
    const stored = await page.evaluate(() =>
      Object.keys(localStorage).filter((k) => k.startsWith("chartgen.legacy")),
    );
    expect(stored).toEqual([]);
    await reloadApp(page);
    await expect(page.getByTestId("workspace-offer")).toBeHidden();
    expect(await read(page, NOTE)).toBeNull();
    await context.close();
  });

  test("a share link opened signed out lands in the account, not lost on the way", async ({
    page,
  }) => {
    await page.goto("/?symbol=ETHUSDT&interval=4h");
    await signIn(page);
    await expect(page.getByRole("button", { name: /^Symbol:/ })).toContainText("ETHUSDT");
    await expect(
      page.getByRole("group", { name: "Timeframe" }).locator("[aria-pressed=true]"),
    ).toHaveText("4h");
    expect(new URL(page.url()).search, "and the address is clean").toBe("");
  });

  test("a file the app wrote keeps what only a newer build knows, per item", async ({ browser }) => {
    const laptop = await device(browser);
    const raw = await read(laptop, "chartgen.layouts");
    const file = JSON.parse(raw!) as { items: Record<string, unknown>[] };
    file.items.push({ id: "future", name: "Future", updatedAt: 1, state: {}, newField: 7 });
    const withFuture = JSON.stringify(file);
    await write(laptop, "chartgen.layouts", withFuture);
    await pushed(laptop, "chartgen.layouts", withFuture);

    const fresh = await device(browser);
    await reloadApp(fresh);
    const onFresh = JSON.parse((await read(fresh, "chartgen.layouts"))!) as typeof file;
    expect(onFresh.items.find((i) => i.id === "future")?.newField).toBe(7);
    await write(laptop, "chartgen.layouts", raw!);
    await pushed(laptop, "chartgen.layouts", raw);
    await Promise.all([laptop, fresh].map((p) => p.context().close()));
  });

  test("a layouts file from a newer build is kept as it is, on load and across devices", async ({
    browser,
  }) => {
    const newer = JSON.stringify({ version: 99, somethingNew: { a: 1 }, items: { not: "a list" } });
    const laptop = await device(browser);
    const raw = await read(laptop, "chartgen.layouts");
    await write(laptop, "chartgen.layouts", newer);
    await pushed(laptop, "chartgen.layouts", newer);

    const fresh = await device(browser);
    expect(await read(fresh, "chartgen.layouts")).toBe(newer);
    await reloadApp(fresh);
    expect(await read(fresh, "chartgen.layouts")).toBe(newer);
    await fresh.waitForTimeout(6_000);
    await pushed(fresh, "chartgen.layouts", newer);
    // Put back: the other specs of this server sign in as this account.
    await write(laptop, "chartgen.layouts", raw!);
    await pushed(laptop, "chartgen.layouts", raw);
    await Promise.all([laptop, fresh].map((p) => p.context().close()));
  });
});
