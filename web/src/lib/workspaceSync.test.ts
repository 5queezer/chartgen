import { beforeEach, describe, expect, it } from "vitest";

import {
  applyAnswer,
  collectWrites,
  discardLegacy,
  enterAccount,
  enterAnonymous,
  forgetEvicted,
  hasLegacyOffer,
  hasRemoteChanges,
  hideLive,
  requestImport,
  runRequestedImport,
  isHidden,
  isScoped,
  isSynced,
  MARKER,
  merge,
  readState,
  settle,
  type SyncAnswer,
} from "./workspaceSync";

/** Keeps its keys in insertion order and counts writes, unlike a browser, on purpose. */
class MemoryStorage implements Storage {
  private map = new Map<string, string>();
  writes = 0;
  get length() {
    return this.map.size;
  }
  key = (i: number) => [...this.map.keys()][i] ?? null;
  getItem = (k: string) => this.map.get(k) ?? null;
  setItem = (k: string, v: string) => {
    this.writes++;
    this.map.set(k, v);
  };
  removeItem = (k: string) => {
    if (this.map.delete(k)) this.writes++;
  };
  clear = () => this.map.clear();
  [name: string]: unknown;
}

const s = new MemoryStorage();
const none = () => false;
const live = () =>
  Object.fromEntries(
    Array.from({ length: s.length }, (_, i) => s.key(i)!)
      .filter(isScoped)
      .map((k) => [k, s.getItem(k)]),
  );
const answer = (a: Partial<SyncAnswer>): SyncAnswer => ({ cursor: 1, results: [], changes: [], ...a });
const drawings = (...ids: string[]) => JSON.stringify(ids.map((id) => ({ id, name: id })));
const DOC = "chartgen.drawings.BTCUSDT";
const OTHER_DOC = "chartgen.drawings.ETHUSDT";

beforeEach(() => s.clear());

describe("which keys", () => {
  it("scopes everything of the app's but the browser's own", () => {
    for (const key of ["chartgen.layouts", "chartgen.sidebar.tab", "chartgen.scripts.drafts", "chartgen.sync.x"]) {
      expect(isScoped(key), key).toBe(true);
    }
    for (const key of ["chartgen.oauth.client_id", "chartgen.push.on", "chartgen.u.a|x", "chartgen.legacy|x", MARKER, "other"]) {
      expect(isScoped(key), key).toBe(false);
    }
  });

  it("syncs the workspace and not what is this device's or private", () => {
    for (const key of ["chartgen.layouts", "chartgen.layout.grid", "chartgen.pane.1.indicators", "chartgen.watchlist.v2", DOC, "chartgen.alerts.list", "chartgen.chartSettings", "chartgen.orderflow", "chartgen.indicators.active"]) {
      expect(isSynced(key), key).toBe(true);
    }
    for (const key of ["chartgen.layouts.bak", "chartgen.drawings.index", "chartgen.sidebar.tab", "chartgen.scripts.drafts", "chartgen.serverAlerts.list", "chartgen.sync.chartgen.layouts"]) {
      expect(isSynced(key), key).toBe(false);
    }
  });
});

describe("accounts on one browser", () => {
  it("a second account sees nothing of the first, who finds it all again", () => {
    enterAccount(s, "github:1");
    s.setItem("chartgen.layouts", "mine");
    s.setItem("chartgen.sidebar.tab", "alerts");
    s.setItem("chartgen.oauth.client_id", "shared");

    enterAccount(s, "google:2");
    expect(live()).toEqual({});
    expect(s.getItem("chartgen.oauth.client_id")).toBe("shared");
    s.setItem("chartgen.layouts", "theirs");

    enterAccount(s, "github:1");
    expect(live()).toEqual({ "chartgen.layouts": "mine", "chartgen.sidebar.tab": "alerts" });
    enterAccount(s, "google:2");
    expect(live()).toEqual({ "chartgen.layouts": "theirs" });
  });

  it("hiding leaves nothing live, and the same account gets it back", () => {
    enterAccount(s, "github:1");
    s.setItem("chartgen.layouts", "mine");
    hideLive(s);
    expect(live()).toEqual({});
    expect(s.getItem(MARKER)).toBe("github:1");
    enterAccount(s, "github:1");
    expect(live()).toEqual({ "chartgen.layouts": "mine" });
  });

  it("a stray write after hiding never replaces the stored copy", () => {
    enterAccount(s, "github:1");
    s.setItem("chartgen.layouts", "mine");
    hideLive(s);
    s.setItem("chartgen.layouts", "default from a tab left behind");
    s.setItem("chartgen.sidebar.tab", "new");
    enterAccount(s, "google:2");
    expect(live()).toEqual({});
    enterAccount(s, "github:1");
    expect(live()).toEqual({ "chartgen.layouts": "mine", "chartgen.sidebar.tab": "new" });
  });

  it("an account id cannot reach into another's scope", () => {
    enterAccount(s, "a");
    s.setItem("chartgen.layouts", "a's");
    enterAccount(s, "a|chartgen.x");
    expect(live()).toEqual({});
    s.setItem("chartgen.layouts", "other");
    enterAccount(s, "a");
    expect(live()).toEqual({ "chartgen.layouts": "a's" });
  });

  it("isHidden holds while the data is out of sight or another's", () => {
    enterAccount(s, "github:1");
    expect(isHidden(s, "github:1")).toBe(false);
    s.setItem("chartgen.layouts", "mine");
    hideLive(s);
    expect(isHidden(s, "github:1")).toBe(true);
    enterAccount(s, "github:1");
    expect(isHidden(s, "github:1")).toBe(false);
    enterAccount(s, "google:2");
    expect(isHidden(s, "github:1")).toBe(true);
  });

  it("with no sign-in and nothing from one, nothing is written or moved", () => {
    s.setItem("chartgen.layouts", "local");
    s.writes = 0;
    enterAnonymous(s);
    expect(s.writes).toBe(0);
    expect(live()).toEqual({ "chartgen.layouts": "local" });
  });

  it("sign-in turned off gives the old data back and keeps accounts' copies apart", () => {
    s.setItem("chartgen.layouts", "from before");
    enterAccount(s, "github:1");
    s.setItem("chartgen.layouts", "account's");
    enterAnonymous(s);
    expect(live()).toEqual({ "chartgen.layouts": "from before" });
    expect(s.getItem(MARKER)).toBeNull();
    enterAccount(s, "github:1");
    expect(s.getItem("chartgen.layouts")).toBe("account's");
  });
});

describe("data from before any sign-in", () => {
  beforeEach(() => {
    s.setItem("chartgen.layouts", "local");
    s.setItem(DOC, drawings("old"));
    s.setItem("chartgen.sidebar.tab", "alerts");
  });

  it("is hidden from the account until it asks, and offered to the first account only", () => {
    enterAccount(s, "github:1");
    expect(live()).toEqual({});
    expect(hasLegacyOffer(s, "github:1")).toBe(true);
    enterAccount(s, "google:2");
    expect(hasLegacyOffer(s, "google:2")).toBe(false);
    enterAccount(s, "github:1");
    expect(hasLegacyOffer(s, "github:1")).toBe(true);
  });

  it("is not offered when there is none", () => {
    s.clear();
    enterAccount(s, "github:1");
    expect(hasLegacyOffer(s, "github:1")).toBe(false);
  });

  it("importing adds to what the account has and replaces nothing", () => {
    enterAccount(s, "github:1");
    s.setItem("chartgen.layouts", "account's");
    s.setItem(DOC, drawings("new"));
    requestImport(s, "github:1");
    expect(hasLegacyOffer(s, "github:1"), "asked, so no longer offered").toBe(false);
    expect(s.getItem(DOC), "not under running stores").toBe(drawings("new"));
    runRequestedImport(s, "github:1");
    expect(s.getItem("chartgen.layouts")).toBe("account's");
    expect(s.getItem(DOC)).toBe(drawings("new", "old"));
    expect(s.getItem("chartgen.sidebar.tab")).toBe("alerts");
    expect(hasLegacyOffer(s, "github:1")).toBe(false);
    enterAnonymous(s);
    expect(live(), "none left to come back").toEqual({});
  });

  it("importing keeps old drawings that have no ids", () => {
    const old = JSON.stringify([{ name: "no id", points: [] }]);
    s.setItem(DOC, old);
    enterAccount(s, "github:1");
    s.setItem(DOC, drawings("new"));
    requestImport(s, "github:1");
    runRequestedImport(s, "github:1");
    expect(JSON.parse(s.getItem(DOC)!)).toEqual([
      { id: "new", name: "new" },
      { name: "no id", points: [] },
    ]);
  });

  it("an import asked for by one account does not run for another", () => {
    enterAccount(s, "github:1");
    requestImport(s, "github:1");
    enterAccount(s, "google:2");
    runRequestedImport(s, "google:2");
    expect(live()).toEqual({});
    enterAccount(s, "github:1");
    runRequestedImport(s, "github:1");
    expect(s.getItem(DOC)).toBe(drawings("old"));
  });

  it("discarding touches only the old data", () => {
    enterAccount(s, "github:1");
    s.setItem("chartgen.layouts", "account's");
    discardLegacy(s);
    expect(s.getItem("chartgen.layouts")).toBe("account's");
    expect(hasLegacyOffer(s, "github:1")).toBe(false);
    enterAnonymous(s);
    expect(s.getItem(DOC)).toBeNull();
  });
});

describe("merge", () => {
  it("one side changed takes that side, a deletion included", () => {
    expect(merge("chartgen.chartSettings", "b", "l", "b")).toEqual({ value: "l", conflict: false });
    expect(merge("chartgen.chartSettings", "b", "b", "r")).toEqual({ value: "r", conflict: false });
    expect(merge("chartgen.chartSettings", "b", null, "b")).toEqual({ value: null, conflict: false });
    expect(merge("chartgen.chartSettings", "b", "b", null)).toEqual({ value: null, conflict: false });
  });

  it("two edits: the later writer, and it is said", () => {
    expect(merge("chartgen.chartSettings", "b", "l", "r")).toEqual({ value: "l", conflict: true });
  });

  it("a new device's value gives way to the account's, and it is said", () => {
    expect(merge("chartgen.chartSettings", null, "default", "account")).toEqual({ value: "account", conflict: true });
  });

  it("an edit beats a deletion from either side", () => {
    expect(merge("chartgen.chartSettings", "b", null, "r")).toEqual({ value: "r", conflict: true });
    expect(merge("chartgen.chartSettings", "b", "l", null)).toEqual({ value: "l", conflict: true });
  });

  it("drawings from two devices are both kept, and neither wipes the other", () => {
    const m = merge(DOC, drawings("a"), drawings("a", "here"), drawings("a", "there"));
    expect(m).toEqual({ value: drawings("a", "here", "there"), conflict: false });
  });

  it("a stale save takes away only what it had seen", () => {
    const m = merge(DOC, drawings("a", "b"), drawings("a"), drawings("a", "b", "new"));
    expect(m).toEqual({ value: drawings("a", "new"), conflict: false });
  });

  it("removing every item here keeps what another device added meanwhile", () => {
    expect(merge(DOC, drawings("a"), null, drawings("a", "there"))).toEqual({
      value: drawings("there"),
      conflict: false,
    });
    expect(merge(DOC, drawings("a"), null, "[]")).toEqual({ value: null, conflict: false });
  });

  it("drawings from before they had ids are kept and removed by what they are", () => {
    const old = (...names: string[]) => JSON.stringify(names.map((name) => ({ name, points: [] })));
    expect(merge(DOC, old("a"), old("a", "here"), old("a", "there"))).toEqual({
      value: old("a", "here", "there"),
      conflict: false,
    });
    expect(merge(DOC, old("a", "b"), old("a"), old("a", "b", "there")).value).toBe(old("a", "there"));
  });

  it("one item edited on both: this one, and said; deleted against edited: the edit", () => {
    const item = (name: string) => JSON.stringify([{ id: "a", name }]);
    expect(merge(DOC, item("b"), item("l"), item("r"))).toEqual({ value: item("l"), conflict: true });
    expect(merge(DOC, item("b"), "[]", item("r"))).toEqual({ value: item("r"), conflict: true });
    expect(merge(DOC, item("b"), item("l"), "[]")).toEqual({ value: item("l"), conflict: true });
  });

  it("merges layouts by item and field, keeping what it does not know", () => {
    const file = (active: string, items: object[], extra = {}) =>
      JSON.stringify({ version: 1, active, items, ...extra });
    const a = { id: "a", name: "A" };
    const m = merge(
      "chartgen.layouts",
      file("a", [a]),
      file("l", [a, { id: "l", name: "L" }]),
      file("r", [a, { id: "r", name: "R", newField: 7 }], { future: { x: 1 } }),
    );
    expect(JSON.parse(m.value!)).toEqual({
      version: 1,
      active: "l",
      items: [a, { id: "l", name: "L" }, { id: "r", name: "R", newField: 7 }],
      future: { x: 1 },
    });
    expect(m.conflict).toBe(false);
  });

  it("merges watchlists and browser alerts by id", () => {
    const lists = (...names: string[]) =>
      JSON.stringify({ version: 2, lists: names.map((id) => ({ id, name: id, items: [] })), active: "a" });
    expect(merge("chartgen.watchlist.v2", lists("a"), lists("a", "l"), lists("a", "r")).value).toBe(
      lists("a", "l", "r"),
    );
    expect(merge("chartgen.alerts.list", drawings("a"), drawings("a", "l"), drawings("r")).value).toBe(
      drawings("l", "r"),
    );
  });

  it("a map both sides changed is said to be lost, which entry is active is not", () => {
    const file = (active: string, flags: object) =>
      JSON.stringify({ version: 2, lists: [], flags, active });
    expect(merge("chartgen.watchlist.v2", file("b", {}), file("l", {}), file("r", {}))).toEqual({
      value: file("l", {}),
      conflict: false,
    });
    expect(
      merge("chartgen.watchlist.v2", file("b", {}), file("b", { X: "red" }), file("b", { Y: "blue" })),
    ).toEqual({ value: file("b", { X: "red" }), conflict: true });
  });

  it("a shape it cannot read falls back to the whole document, never to garbling it", () => {
    const m = merge("chartgen.layouts", '{"version":1,"items":[]}', '{"version":1,"items":[]} ', '{"version":9,"items":{"x":1}}');
    expect(m).toEqual({ value: '{"version":1,"items":[]} ', conflict: true });
    expect(merge(DOC, "[]", "not json", drawings("a"))).toEqual({ value: "not json", conflict: true });
    const twice = JSON.stringify([{ id: "a" }, { id: "a" }]);
    expect(merge(DOC, "[]", twice, drawings("b"))).toEqual({ value: twice, conflict: true });
  });
});

describe("a round", () => {
  it("sends only what changed, with the version it was based on", () => {
    s.setItem("chartgen.chartSettings", "a");
    s.setItem("chartgen.sidebar.tab", "device only");
    const first = collectWrites(s, none);
    expect(first.writes.map((w) => [w.key, w.value, w.base_version])).toEqual([["chartgen.chartSettings", "a", 0]]);
    applyAnswer(s, 0, first.writes, answer({ cursor: 1, results: [{ key: "chartgen.chartSettings", status: "ok", version: 1 }] }));
    expect(collectWrites(s, none).writes).toEqual([]);
    s.setItem("chartgen.chartSettings", "b");
    expect(collectWrites(s, none).writes[0]).toMatchObject({ value: "b", base_version: 1 });
  });

  it("a user's deletion goes out as one", () => {
    s.setItem(DOC, drawings("a"));
    const first = collectWrites(s, none);
    applyAnswer(s, 0, first.writes, answer({ results: [{ key: DOC, status: "ok", version: 1 }] }));
    s.removeItem(DOC);
    expect(collectWrites(s, none).writes[0]).toMatchObject({ key: DOC, value: null, base_version: 1 });
  });

  it("the LRU dropping a copy sends no deletion, and it returns", () => {
    s.setItem(DOC, drawings("a"));
    const first = collectWrites(s, none);
    applyAnswer(s, 0, first.writes, answer({ results: [{ key: DOC, status: "ok", version: 1 }] }));
    s.removeItem(DOC);
    forgetEvicted(s, DOC);
    expect(collectWrites(s, none).writes).toEqual([]);
    expect(hasRemoteChanges(s)).toBe(false);
    settle(s);
    expect(s.getItem(DOC)).toBe(drawings("a"));
  });

  it("forgetting a key never synced leaves no state behind", () => {
    forgetEvicted(s, DOC);
    expect(s.length).toBe(0);
  });

  it("a conflict's answer is merged and sent again, keeping both sides", () => {
    s.setItem(DOC, drawings("a"));
    const first = collectWrites(s, none);
    applyAnswer(s, 0, first.writes, answer({ results: [{ key: DOC, status: "ok", version: 1 }] }));
    s.setItem(DOC, drawings("a", "here"));
    const stale = collectWrites(s, none);
    applyAnswer(s, 1, stale.writes, answer({ cursor: 2, results: [{ key: DOC, status: "conflict", version: 2, value: drawings("a", "there") }] }));
    const retry = collectWrites(s, none);
    expect(retry.writes[0]).toMatchObject({ value: drawings("a", "here", "there"), base_version: 2, conflict: false });
    expect(s.getItem(DOC), "the stores' value stays until a reload").toBe(drawings("a", "here"));
    applyAnswer(s, 2, retry.writes, answer({ cursor: 3, results: [{ key: DOC, status: "ok", version: 3 }] }));
    expect(collectWrites(s, none).writes).toEqual([]);
    expect(hasRemoteChanges(s)).toBe(true);
    settle(s);
    expect(s.getItem(DOC)).toBe(drawings("a", "here", "there"));
    expect(hasRemoteChanges(s)).toBe(false);
    expect(collectWrites(s, none).writes).toEqual([]);
  });

  it("remote changes wait: nothing live moves until settled, and a local edit is not overwritten", () => {
    s.setItem("chartgen.chartSettings", "here");
    s.setItem("chartgen.layout.grid", "here");
    const first = collectWrites(s, none);
    applyAnswer(s, 0, first.writes, answer({ results: first.writes.map((w, i) => ({ key: w.key, status: "ok" as const, version: i + 1 })) }));
    applyAnswer(s, 2, [], answer({ cursor: 4, changes: [
      { key: "chartgen.chartSettings", value: "there", version: 3 },
      { key: "chartgen.layout.grid", value: "there", version: 4 },
    ]}));
    expect(s.getItem("chartgen.chartSettings")).toBe("here");
    expect(hasRemoteChanges(s)).toBe(true);
    s.setItem("chartgen.layout.grid", "edited since");
    settle(s);
    expect(s.getItem("chartgen.chartSettings")).toBe("there");
    expect(s.getItem("chartgen.layout.grid")).toBe("edited since");
  });

  it("a remote deletion removes the key when settled and sends nothing back", () => {
    s.setItem(DOC, drawings("a"));
    const first = collectWrites(s, none);
    applyAnswer(s, 0, first.writes, answer({ results: [{ key: DOC, status: "ok", version: 1 }] }));
    applyAnswer(s, 1, [], answer({ cursor: 2, changes: [{ key: DOC, value: null, version: 2 }] }));
    settle(s);
    expect(s.getItem(DOC)).toBeNull();
    expect(collectWrites(s, none).writes).toEqual([]);
  });

  it("a new device's default gives way, and the settled value is not sent back", () => {
    applyAnswer(s, 0, [], answer({ cursor: 5, changes: [{ key: "chartgen.chartSettings", value: "account", version: 5 }] }));
    s.setItem("chartgen.chartSettings", "default");
    const round = collectWrites(s, none);
    expect(round.writes).toEqual([]);
    expect(round.gaveWay).toEqual(["chartgen.chartSettings"]);
    settle(s);
    expect(s.getItem("chartgen.chartSettings")).toBe("account");
    expect(collectWrites(s, none)).toEqual({ writes: [], gaveWay: [], tooLarge: [] });
  });

  it("a refused value is reported and not sent again until it changes", () => {
    s.setItem("chartgen.chartSettings", "big");
    const round = collectWrites(s, none);
    const applied = applyAnswer(s, 0, round.writes, answer({ results: [{ key: "chartgen.chartSettings", status: "quota" }] }));
    expect(applied.refused).toEqual([{ key: "chartgen.chartSettings", live: "big", status: "quota" }]);
    const refused = (k: string, v: string | null) => k === "chartgen.chartSettings" && v === "big";
    expect(collectWrites(s, refused).writes).toEqual([]);
    s.setItem("chartgen.chartSettings", "small");
    expect(collectWrites(s, refused).writes).toHaveLength(1);
  });

  it("a document past the limit is reported without being sent", () => {
    s.setItem("chartgen.chartSettings", "x".repeat(512 * 1024 + 1));
    expect(collectWrites(s, none)).toEqual({ writes: [], gaveWay: [], tooLarge: ["chartgen.chartSettings"] });
  });

  it("a round stays under what a request may hold, and the rest go next", () => {
    for (let i = 0; i < 60; i++) s.setItem(`chartgen.pane.${i}.x`, "v");
    expect(collectWrites(s, none).writes).toHaveLength(50);
    s.clear();
    for (let i = 0; i < 3; i++) s.setItem(`chartgen.pane.${i}.x`, "v".repeat(100));
    expect(collectWrites(s, none, 250).writes).toHaveLength(2);
  });

  it("keys the app does not sync are ignored when the server sends them", () => {
    applyAnswer(s, 0, [], answer({ changes: [{ key: "chartgen.oauth.client_id", value: "evil", version: 1 }] }));
    expect(s.getItem("chartgen.oauth.client_id")).toBeNull();
    expect(s.getItem("chartgen.sync.chartgen.oauth.client_id")).toBeNull();
  });

  it("a restored server's lower versions are believed", () => {
    s.setItem(DOC, drawings("a"));
    const first = collectWrites(s, none);
    applyAnswer(s, 0, first.writes, answer({ cursor: 90, results: [{ key: DOC, status: "ok", version: 90 }] }));
    applyAnswer(s, 90, [], answer({ cursor: 3, changes: [{ key: DOC, value: drawings("older"), version: 3 }] }));
    expect(readState(s, DOC)).toMatchObject({ v: 3, s: drawings("older") });
  });

  it("a server that lost a document gets it again, and takes nothing for a deletion", () => {
    s.setItem(DOC, drawings("a", "b"));
    s.setItem(OTHER_DOC, drawings("untouched"));
    const first = collectWrites(s, none);
    applyAnswer(s, 0, first.writes, answer({ cursor: 90, results: [
      { key: DOC, status: "ok", version: 89 },
      { key: OTHER_DOC, status: "ok", version: 90 },
    ] }));

    // Rows removed by hand: a device with an edit is told, as a version 0 conflict.
    s.setItem(DOC, drawings("a", "b", "new"));
    const edit = collectWrites(s, none);
    expect(edit.writes[0]).toMatchObject({ key: DOC, base_version: 89 });
    applyAnswer(s, 90, edit.writes, answer({ cursor: 0, results: [
      { key: DOC, status: "conflict", version: 0, value: null },
    ] }));
    expect(s.getItem(DOC)).toBe(drawings("a", "b", "new"));

    const again = collectWrites(s, none);
    expect(again.writes.map((w) => [w.key, w.value, w.base_version]).sort()).toEqual([
      [DOC, drawings("a", "b", "new"), 0],
      [OTHER_DOC, drawings("untouched"), 0],
    ]);
    expect(again.writes.every((w) => !w.conflict)).toBe(true);
  });

  it("a restored server's copy of a doc is kept, and only what it lacks goes back as new", () => {
    s.setItem(DOC, drawings("a"));
    s.setItem(OTHER_DOC, drawings("since the backup"));
    const first = collectWrites(s, none);
    applyAnswer(s, 0, first.writes, answer({ cursor: 90, results: [
      { key: DOC, status: "ok", version: 80 },
      { key: OTHER_DOC, status: "ok", version: 90 },
    ] }));
    applyAnswer(s, 90, [], answer({ cursor: 7, changes: [{ key: DOC, value: drawings("older"), version: 7 }] }));
    expect(readState(s, DOC)).toMatchObject({ v: 7, s: drawings("older") });
    expect(collectWrites(s, none).writes.map((w) => [w.key, w.base_version])).toEqual([[OTHER_DOC, 0]]);
  });

  it("an older change arriving late does not go back", () => {
    s.setItem(DOC, drawings("a"));
    const first = collectWrites(s, none);
    applyAnswer(s, 0, first.writes, answer({ cursor: 5, results: [{ key: DOC, status: "ok", version: 5 }] }));
    applyAnswer(s, 2, [], answer({ cursor: 5, changes: [{ key: DOC, value: drawings("older"), version: 3 }] }));
    expect(readState(s, DOC)).toEqual({ v: 5, b: drawings("a") });
  });
});
