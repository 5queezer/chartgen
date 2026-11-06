import { describe, expect, it } from "vitest";

import type { ParamSpec } from "../types/responses";
import {
  buildFields,
  defaultValues,
  initialValues,
  mergeParams,
  validate,
} from "./paramForm";

const specs: ParamSpec[] = [
  { name: "length", type: "integer", default: 14, description: "period" },
  { name: "mult", type: "number", default: 2.0, min: 0.5, max: 5 },
  { name: "dots", type: "boolean", default: true },
  { name: "mode", type: "string", default: "strict" },
  { name: "bars", type: "integer", default: null },
];

describe("buildFields", () => {
  it("maps types, steps and bounds", () => {
    const f = buildFields(specs);
    expect(f.map((x) => x.kind)).toEqual([
      "integer",
      "number",
      "boolean",
      "string",
      "integer",
    ]);
    expect(f[0]).toMatchObject({ min: 1, max: Number.MAX_SAFE_INTEGER, step: 1, description: "period" });
    expect(f[1]).toMatchObject({ min: 0.5, max: 5, step: "any" });
    expect(f[4]!.defaultValue).toBeNull();
  });

  it("treats unknown types as numbers and undefined specs as empty", () => {
    expect(buildFields([{ name: "x", type: "float" }])[0]!.kind).toBe("number");
    expect(buildFields(undefined)).toEqual([]);
  });
});

describe("values", () => {
  const fields = buildFields(specs);

  it("defaultValues stringifies numbers and blanks nulls", () => {
    expect(defaultValues(fields)).toEqual({
      length: "14",
      mult: "2",
      dots: true,
      mode: "strict",
      bars: "",
    });
  });

  it("initialValues prefers current params over defaults", () => {
    const v = initialValues(fields, { length: 21, dots: false });
    expect(v).toMatchObject({ length: "21", dots: false, mult: "2" });
  });
});

describe("validate", () => {
  const fields = buildFields(specs);
  const ok = defaultValues(fields);

  it("accepts defaults and omits unset optionals", () => {
    const r = validate(fields, ok);
    expect(r.errors).toEqual({});
    expect(r.params).toEqual({
      length: 14,
      mult: 2,
      dots: true,
      mode: "strict",
    });
  });

  it("parses edited numbers", () => {
    const r = validate(fields, { ...ok, length: " 21 ", bars: "500" });
    expect(r.params).toMatchObject({ length: 21, bars: 500 });
  });

  it("rejects NaN, empty required, non-integers and out-of-range values", () => {
    const r = validate(fields, {
      ...ok,
      length: "abc",
      mult: "9",
    });
    expect(r.errors.length).toBe("Must be a number");
    expect(r.errors.mult).toBe("Must be at most 5");

    expect(validate(fields, { ...ok, length: "" }).errors.length).toBe(
      "Required",
    );
    expect(validate(fields, { ...ok, length: "1.5" }).errors.length).toBe(
      "Must be a whole number",
    );
    expect(validate(fields, { ...ok, length: "0" }).errors.length).toBe(
      "Must be at least 1",
    );
    expect(validate(fields, { ...ok, length: "1e300" }).errors.length).toBe(
      "Must be at most 9007199254740991",
    );
    expect(validate(fields, { ...ok, mult: "0.1" }).errors.mult).toBe(
      "Must be at least 0.5",
    );
    expect(validate(fields, { ...ok, mult: "Infinity" }).errors.mult).toBe(
      "Must be a number",
    );
  });
});

describe("mergeParams", () => {
  const fields = buildFields(specs);

  it("keeps params outside the schema and drops default-valued ones", () => {
    const merged = mergeParams(
      fields,
      { length: 21, legacy: "x", mult: 3 },
      { length: 14, mult: 4, dots: false, mode: "strict" },
    );
    expect(merged).toEqual({ legacy: "x", mult: 4, dots: false });
  });

  it("clearing an optional param removes the stored value", () => {
    expect(mergeParams(fields, { bars: 50 }, {})).toEqual({});
  });
});

describe("params with allowed values", () => {
  it("carries them to the field, for a select", () => {
    const [f, g] = buildFields([
      { name: "pattern", type: "string", default: "all", enum: ["all", "hammer"] },
      { name: "note", type: "string", default: "" },
    ]);
    expect(f!.options).toEqual(["all", "hammer"]);
    expect(g!.options).toBeUndefined();
  });
});
