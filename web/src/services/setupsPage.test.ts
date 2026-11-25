import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  enterSetupsPage,
  forgetSetupsPage,
  noteSetupsPath,
  setupChartHref,
  pendingSetup,
  setupOpened,
  wantsSetupsPage,
} from "./setupsPage";

/** A tab's address and session storage, as much of them as the page uses. */
function tab(address: string) {
  const location = new URL(address, "https://example.test");
  const store = new Map<string, string>();
  vi.stubGlobal("window", {
    location,
    history: {
      replaceState: (_: unknown, __: string, to: string) => {
        const next = new URL(to, location);
        location.pathname = next.pathname;
        location.search = next.search;
        location.hash = next.hash;
      },
    },
  });
  vi.stubGlobal("sessionStorage", {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => store.set(k, v),
    removeItem: (k: string) => store.delete(k),
  });
  return { location, store };
}

beforeEach(() => vi.unstubAllGlobals());

describe("reaching /setups (ADR-0091)", () => {
  it("keeps a flag through the sign-in, and the address once the page shows", () => {
    const t = tab("/setups");
    noteSetupsPath();
    // The sign-in comes back to `/`.
    t.location.pathname = "/";
    expect(wantsSetupsPage()).toBe(true);
    enterSetupsPage();
    expect(t.location.pathname).toBe("/setups");
    expect(wantsSetupsPage()).toBe(false);
  });

  it("is the app at `/` when the server keeps none for this tab", () => {
    const t = tab("/setups?x=1");
    noteSetupsPath();
    forgetSetupsPage();
    expect(t.location.pathname + t.location.search).toBe("/?x=1");
    expect(wantsSetupsPage()).toBe(false);
  });

  it("notes nothing at another address", () => {
    tab("/");
    noteSetupsPath();
    expect(wantsSetupsPage()).toBe(false);
  });
});

describe("opening a setup's chart from the page", () => {
  it("takes the setup out of the address, and keeps it through a redirect until it opened", () => {
    const t = tab(`${setupChartHref("setup-000042")}&symbol=ETHUSDT#x`);
    expect(pendingSetup()).toBe("setup-000042");
    expect(t.location.search).toBe("?symbol=ETHUSDT");
    expect(t.location.hash).toBe("#x");
    // Back from the redirect, at `/?code=...`.
    t.location.search = "?code=c";
    expect(pendingSetup()).toBe("setup-000042");
    setupOpened();
    expect(pendingSetup()).toBeNull();
  });

  it("ignores an empty one", () => {
    tab("/?setup=");
    expect(pendingSetup()).toBeNull();
  });
});
