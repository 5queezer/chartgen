import { beforeEach, describe, expect, it, vi } from "vitest";

import answers from "../../../tests/fixtures/setup_responses.json";
import type { McpSession } from "../mcp";
import { SetupPayloadSchema, SetupsPayloadSchema, type Setup, type SetupsPayload } from "../types/responses";

const opened = vi.hoisted(() => ({ symbols: [] as string[], drawings: [] as string[] }));

vi.mock("./layout", () => ({
  symbol: () => opened.symbols.at(-1) ?? "BTCUSDT",
  setSymbol: (s: string) => opened.symbols.push(s),
  setTimeframe: () => undefined,
}));
vi.mock("./drawings", async (original) => ({
  ...(await original<typeof import("./drawings")>()),
  replaceDrawings: (symbol: string) => {
    opened.drawings.push(symbol);
    return 1;
  },
  announceDrawings: () => undefined,
}));
vi.mock("../services/alertEngine", () => ({ showToast: () => undefined }));

import { openSetup, refreshSetups, setupsState } from "./setups";

/** A promise a test resolves when it chooses. */
function later<T>(): { promise: Promise<T>; resolve: (v: T) => void } {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((r) => (resolve = r));
  return { promise, resolve };
}

const list = SetupsPayloadSchema.parse(answers.list_setups);
const settle = () => new Promise((r) => setTimeout(r, 0));

beforeEach(() => {
  opened.symbols.length = 0;
  opened.drawings.length = 0;
});

describe("refreshing the setups (ADR-0071)", () => {
  it("keeps a newer refresh's answer when an older one answers after it", async () => {
    const older = later<SetupsPayload>();
    const newer = later<SetupsPayload>();
    const lists = [older.promise, newer.promise];
    const session = {
      callListSetups: () => lists.shift()!,
      callGetTrackRecord: () => Promise.reject(new Error("not here")),
    } as unknown as McpSession;
    const first = refreshSetups(session);
    const second = refreshSetups(session);
    const won = list.setups.map((s) => ({ ...s, status: "won" as const }));
    newer.resolve({ ...list, setups: won });
    await second;
    older.resolve(list);
    await first;
    const state = setupsState();
    expect(state.kind).toBe("ready");
    if (state.kind === "ready") expect(state.setups.map((s) => s.status)).toEqual(["won", "won", "won"]);
  });

  it("drops an older refresh's off and error too", async () => {
    const older = later<SetupsPayload>();
    const older2 = later<SetupsPayload>();
    const lists = [older.promise, older2.promise, Promise.resolve(list)];
    const session = {
      callListSetups: () => lists.shift()!,
      callGetTrackRecord: () => Promise.reject(new Error("not here")),
    } as unknown as McpSession;
    const off = refreshSetups(session);
    const failed = refreshSetups(session);
    await refreshSetups(session);
    older.resolve({ enabled: false, setups: [] });
    older2.resolve(Promise.reject(new Error("late")) as unknown as SetupsPayload);
    await Promise.allSettled([off, failed]);
    expect(setupsState().kind).toBe("ready");
  });
});

describe("opening a setup (ADR-0071)", () => {
  it("opens the setup clicked last, whichever read answers first", async () => {
    const a = later<Setup>();
    const b = later<Setup>();
    const reads: Record<string, Promise<Setup>> = { a: a.promise, b: b.promise };
    const session = { callGetSetup: ({ id }: { id: string }) => reads[id]! } as unknown as McpSession;
    const eth = SetupPayloadSchema.parse(answers.cancel_setup);
    const btc = SetupPayloadSchema.parse(answers.create_setup);
    expect([eth.symbol, btc.symbol]).toEqual(["ETHUSDT", "BTCUSDT"]);
    const first = openSetup(session, "a");
    const second = openSetup(session, "b");
    b.resolve(btc);
    await second;
    a.resolve(eth);
    await first;
    await settle();
    expect(opened.symbols).toEqual(["BTCUSDT"]);
    expect(opened.drawings).toEqual(["BTCUSDT"]);
  });
});
