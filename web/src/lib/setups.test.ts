import { describe, expect, it } from "vitest";

import answers from "../../../tests/fixtures/setup_responses.json";
import { SetupPayloadSchema, SetupsPayloadSchema, type Setup } from "../types/responses";
import {
  bracketDrawing,
  entryText,
  filterSetups,
  formatR,
  setupDrawings,
  sortSetups,
  wonAtALoss,
} from "./setups";

// The server's own answers (src/setup_service/tests.rs writes them): three closed
// setups, a cancelled take, a skip that won and a take that was stopped.
const list = SetupsPayloadSchema.parse(answers.list_setups).setups;
// The same setup as setup-000001 when it was made, still watching, under an id of its own.
const open = { ...SetupPayloadSchema.parse(answers.create_setup), id: "setup-000008" };
const byId = (id: string): Setup => {
  const s = list.find((x) => x.id === id);
  if (!s) throw new Error(id);
  return s;
};
const price = (p: number) => p.toFixed(2);

describe("the setups list", () => {
  it("puts open setups first, newest first, then closed ones by when they ended", () => {
    const later = { ...open, id: "setup-000009", created_at: open.created_at + 1 };
    const ids = sortSetups([...list, open, later]).map((s) => s.id);
    // Of the closed: the stop at 14:15, the cancel at 12:01, the skip's win at 10:37.
    expect(ids).toEqual([
      "setup-000009",
      open.id,
      "setup-000001",
      "setup-000003",
      "setup-000002",
    ]);
  });

  it("filters open and closed, and keeps all", () => {
    const all = [...list, open];
    expect(filterSetups(all, "open").map((s) => s.status)).toEqual(["watching"]);
    expect(filterSetups(all, "closed")).toHaveLength(3);
    expect(filterSetups(all, "all")).toHaveLength(4);
  });
});

describe("a row", () => {
  it("shows the zone until the fill, then the fill", () => {
    expect(entryText(byId("setup-000003"), price)).toBe("2997.24–3003.23");
    expect(entryText(byId("setup-000001"), price)).toBe("fill 60196.41");
    // A skip's entry is one level.
    expect(entryText({ ...byId("setup-000002"), fill: null }, price)).toBe("3072.31");
  });

  it("writes R with its sign", () => {
    expect([formatR(1.7659), formatR(-1.1547), formatR(0), formatR(null)]).toEqual([
      "+1.77R",
      "−1.15R",
      "0.00R",
      "–",
    ]);
  });

  it("says when a win is below 0 R, and only then", () => {
    const won = byId("setup-000002");
    expect(wonAtALoss(won)).toBe(false);
    // As a skip filled past its target scores: won, at a loss after costs.
    expect(wonAtALoss({ ...won, result_r: -0.12 })).toBe(true);
    expect(wonAtALoss({ ...byId("setup-000001") })).toBe(false);
    expect(wonAtALoss({ ...won, status: "invalidated", result_r: -1 })).toBe(false);
  });
});

describe("what a click draws", () => {
  it("puts a take's stored drawings on the chart as the user's own", () => {
    const [line, ...rest] = setupDrawings(open);
    expect(rest).toEqual([]);
    const stored = open.drawings[0]!;
    expect(line).toEqual({
      name: "horizontalStraightLine",
      points: [{ timestamp: stored.points[0]!.timestamp, value: stored.points[0]!.price }],
      lock: false,
      visible: true,
    });
  });

  it("draws a skip's bracket as the server scores it, from the fill to the exit", () => {
    const skip = byId("setup-000002");
    expect(setupDrawings(skip)).toEqual([bracketDrawing(skip)]);
    expect(bracketDrawing(skip)).toEqual({
      name: "cgShort",
      points: [
        { timestamp: skip.fill!.bar_time, value: skip.fill!.price },
        { timestamp: skip.exit!.bar_time, value: skip.targets[0] },
        { timestamp: skip.exit!.bar_time, value: skip.invalidation },
      ],
      lock: false,
      visible: true,
    });
  });

  it("draws an unfilled skip from when it was made to its expiry", () => {
    const made = SetupPayloadSchema.parse(answers.create_setup_skip);
    expect(made.fill).toBeNull();
    expect(bracketDrawing(made)?.points).toEqual([
      { timestamp: made.created_at, value: made.entry.low },
      { timestamp: made.expires_at, value: made.targets[0] },
      { timestamp: made.expires_at, value: made.invalidation },
    ]);
    // A long is a cgLong.
    expect(bracketDrawing({ ...made, direction: "long" })?.name).toBe("cgLong");
  });

  it("leaves out a stored drawing the web app does not know", () => {
    const odd = { ...open, drawings: [{ ...open.drawings[0]!, tool: "noSuchTool" }] };
    expect(setupDrawings(odd)).toEqual([]);
  });
});
