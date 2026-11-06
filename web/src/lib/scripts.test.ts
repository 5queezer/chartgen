import { describe, expect, it } from "vitest";

import {
  loadDrafts,
  saveDrafts,
  scriptingEnabled,
  scriptRejection,
  usesIndicator,
} from "./scripts";
import { activeIndicatorsKey } from "../stores/indicators";

const all = ["generate_chart", "list_my_indicators", "get_my_indicator", "register_indicator", "delete_indicator"];

describe("scriptingEnabled", () => {
  it("needs every tool the editor calls", () => {
    expect(scriptingEnabled(all)).toBe(true);
    expect(scriptingEnabled(all.filter((t) => t !== "get_my_indicator"))).toBe(false);
    expect(scriptingEnabled(["generate_chart", "list_indicators"])).toBe(false);
  });

  it("is off until the list is known", () => {
    expect(scriptingEnabled(undefined)).toBe(false);
  });
});

describe("scriptRejection", () => {
  it("reads the server's validation errors", () => {
    const text = JSON.stringify({
      ok: false,
      errors: [{ step: "dry_run", line: 4, message: "boom" }],
    });
    expect(scriptRejection(text).errors).toEqual([{ step: "dry_run", line: 4, message: "boom" }]);
  });

  it("keeps a plain refusal as a message without a line", () => {
    const text = "You have 50 indicators, the limit; delete one first.";
    expect(scriptRejection(text).errors).toEqual([{ step: "register", line: null, message: text }]);
  });

  it("keeps JSON of another shape as text", () => {
    expect(scriptRejection('{"a":1}').errors[0]?.message).toBe('{"a":1}');
  });
});

describe("usesIndicator", () => {
  const key = activeIndicatorsKey(["ema_stack", { name: "mine", length: 3, id: "mine#2" }]);

  it("finds bare names and object entries", () => {
    expect(usesIndicator(key, "ema_stack")).toBe(true);
    expect(usesIndicator(key, "mine")).toBe(true);
  });

  it("does not match a param, an id or part of a name", () => {
    expect(usesIndicator(key, "length")).toBe(false);
    expect(usesIndicator(key, "mine#2")).toBe(false);
    expect(usesIndicator(key, "ema")).toBe(false);
  });

  it("is false for a key that is not a list", () => {
    expect(usesIndicator("not json", "mine")).toBe(false);
    expect(usesIndicator('{"name":"mine"}', "mine")).toBe(false);
    expect(usesIndicator("[null]", "mine")).toBe(false);
  });
});

describe("drafts", () => {
  const memory = () => {
    const m = new Map<string, string>();
    return {
      getItem: (k: string) => m.get(k) ?? null,
      setItem: (k: string, v: string) => void m.set(k, v),
    };
  };

  it("round-trips", () => {
    const s = memory();
    const value = { selected: "mine", drafts: { mine: { name: "mine", source: "x = 1" } } };
    expect(saveDrafts(value, s)).toBe(true);
    expect(loadDrafts(s)).toEqual(value);
  });

  it("starts empty when nothing or garbage is stored", () => {
    const s = memory();
    expect(loadDrafts(s)).toEqual({ selected: null, drafts: {} });
    s.setItem("chartgen.scripts.drafts", "{not json");
    expect(loadDrafts(s)).toEqual({ selected: null, drafts: {} });
    s.setItem("chartgen.scripts.drafts", JSON.stringify({ selected: 3, drafts: [] }));
    expect(loadDrafts(s)).toEqual({ selected: null, drafts: {} });
  });

  it("reports a refused write", () => {
    const s = {
      ...memory(),
      setItem: () => {
        throw new Error("QuotaExceededError");
      },
    };
    expect(saveDrafts({ selected: null, drafts: {} }, s)).toBe(false);
  });
});
