import { beforeEach, describe, expect, it, vi } from "vitest";

const toasts: string[] = [];
vi.mock("./alertEngine", () => ({
  showToast: (t: string) => void toasts.push(t),
}));

function memory() {
  const d = new Map<string, string>();
  return {
    getItem: (k: string) => d.get(k) ?? null,
    setItem: (k: string, v: string) => void d.set(k, v),
    removeItem: (k: string) => void d.delete(k),
  };
}

/** Stub the browser globals for a page at `url`; storage is shared via the args. */
function stub(
  url: string,
  local: ReturnType<typeof memory> = memory(),
  session: ReturnType<typeof memory> = memory(),
) {
  const u = new URL(url);
  const history = { replaceState: vi.fn() };
  vi.stubGlobal("localStorage", local);
  vi.stubGlobal("sessionStorage", session);
  vi.stubGlobal("window", {
    location: {
      search: u.search,
      pathname: u.pathname,
      hash: u.hash,
      origin: u.origin,
    },
    history,
    addEventListener: () => {},
  });
  return { history, local, session };
}

async function load() {
  vi.resetModules();
  const [svc, ind, layout] = await Promise.all([
    import("./shareLink"),
    import("../stores/indicators"),
    import("../stores/layout"),
  ]);
  return { svc, ind, layout };
}

beforeEach(() => {
  vi.unstubAllGlobals();
  toasts.length = 0;
});

describe("applyShareLink", () => {
  it("applies params, cleans only share params and holds the chart query", async () => {
    const { history, local } = stub(
      "http://x/?symbol=ethusdt&interval=4h&ind=rsi,bogus&code=c1&state=s1",
    );
    const { svc, ind, layout } = await load();
    svc.applyShareLink();
    expect(layout.symbol()).toBe("ETHUSDT");
    expect(layout.timeframe()).toBe("4h");
    // Raw entries stay out of the store (and localStorage) until reconciled.
    expect(ind.activeIndicators().map(ind.indicatorName)).not.toContain("bogus");
    expect(local.getItem("chartgen.indicators.active") ?? "").not.toContain(
      "bogus",
    );
    expect(svc.importPending()).toBe(true);
    expect(history.replaceState).toHaveBeenCalledWith(
      {},
      "",
      "/?code=c1&state=s1",
    );
  });

  it("does nothing without share params", async () => {
    const { history } = stub("http://x/?code=c1");
    const { svc, layout } = await load();
    svc.applyShareLink();
    expect(layout.symbol()).toBe("BTCUSDT");
    expect(svc.importPending()).toBe(false);
    expect(history.replaceState).not.toHaveBeenCalled();
  });

  it("reconciles only link names, with a toast, and releases the gate", async () => {
    stub("http://x/?ind=rsi(length:0),bogus_xyz");
    const { svc, ind } = await load();
    svc.applyShareLink();
    svc.reconcileImported([
      {
        name: "rsi",
        configurable_params: [{ name: "length", type: "integer" }],
      },
    ]);
    expect(ind.activeIndicators()).toEqual(["rsi"]);
    expect(svc.importPending()).toBe(false);
    expect(toasts.join("|")).toContain("bogus_xyz");
    expect(toasts.join("|")).toContain("rsi.length");
  });

  it("survives a reload through sessionStorage (OAuth redirect)", async () => {
    const first = stub("http://x/?ind=rsi,bogus_xyz");
    const a = await load();
    a.svc.applyShareLink();
    // New page after the redirect: no share params, same tab storage.
    stub("http://x/?code=c&state=s", first.local, first.session);
    const b = await load();
    expect(b.svc.importPending()).toBe(true);
    b.svc.reconcileImported([{ name: "rsi" }]);
    expect(b.ind.activeIndicators()).toEqual(["rsi"]);
    expect(b.svc.importPending()).toBe(false);
  });

  it("never writes unvalidated entries to localStorage before reconcile", async () => {
    const first = stub("http://x/?ind=rsi(length:0),bogus_xyz");
    const a = await load();
    const before = a.ind.activeIndicators();
    a.svc.applyShareLink();
    expect(a.ind.activeIndicators()).toEqual(before);
    expect(first.local.getItem("chartgen.indicators.active") ?? "").not.toContain(
      "bogus_xyz",
    );
    // A second tab (same localStorage, own sessionStorage) is not gated and
    // sees the previous set only.
    stub("http://x/", first.local);
    const b = await load();
    expect(b.svc.importPending()).toBe(false);
    expect(b.ind.activeIndicators()).toEqual(before);
  });

  it("keeps everything when the registry is unavailable", async () => {
    stub("http://x/?ind=bogus_xyz");
    const { svc, ind } = await load();
    svc.applyShareLink();
    svc.reconcileImported(undefined);
    expect(ind.activeIndicators()).toEqual(["bogus_xyz"]);
  });
});
