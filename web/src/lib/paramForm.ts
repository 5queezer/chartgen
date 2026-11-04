// Pure mapping from an indicator's parameter schema (`configurable_params`)
// to settings-dialog fields, plus validation of the edited values. No DOM or
// Solid imports so it can be unit-tested directly.

import type { ParamSpec } from "../types/responses";

export type FieldKind = "integer" | "number" | "boolean" | "string";

export interface ParamField {
  name: string;
  kind: FieldKind;
  description?: string;
  /** Schema default; null means the param is optional and unset by default. */
  defaultValue: unknown;
  min?: number;
  max?: number;
  /** `step` attribute for the number input. */
  step: number | "any";
}

/** Form state: numbers and strings as text, booleans as booleans. */
export type FormValues = Record<string, string | boolean>;

export interface ValidationResult {
  errors: Record<string, string>;
  /** Parsed params, valid only when `errors` is empty. Unset optionals are omitted. */
  params: Record<string, unknown>;
}

function kindOf(type: string): FieldKind {
  switch (type) {
    case "integer":
      return "integer";
    case "boolean":
      return "boolean";
    case "string":
      return "string";
    default:
      return "number";
  }
}

/**
 * Map the schema to fields. Integer params are counts/lengths on the server
 * (u64) and 0 can panic it, so integers default to min 1 and max
 * Number.MAX_SAFE_INTEGER unless the schema says otherwise.
 */
export function buildFields(
  specs: readonly ParamSpec[] | undefined,
): ParamField[] {
  return (specs ?? []).map((spec) => {
    const kind = kindOf(spec.type);
    return {
      name: spec.name,
      kind,
      description: spec.description,
      defaultValue: spec.default ?? null,
      min: spec.min ?? (kind === "integer" ? 1 : undefined),
      max: spec.max ?? (kind === "integer" ? Number.MAX_SAFE_INTEGER : undefined),
      step: kind === "integer" ? 1 : "any",
    };
  });
}

function toFormValue(field: ParamField, value: unknown): string | boolean {
  if (field.kind === "boolean") return value === true;
  return value === null || value === undefined ? "" : String(value);
}

/** Form values for the schema defaults. */
export function defaultValues(fields: readonly ParamField[]): FormValues {
  const out: FormValues = {};
  for (const f of fields) out[f.name] = toFormValue(f, f.defaultValue);
  return out;
}

/** Form values from the entry's current params, falling back to defaults. */
export function initialValues(
  fields: readonly ParamField[],
  current: Record<string, unknown>,
): FormValues {
  const out: FormValues = {};
  for (const f of fields) {
    out[f.name] = toFormValue(
      f,
      f.name in current ? current[f.name] : f.defaultValue,
    );
  }
  return out;
}

/**
 * Params to store after an edit: entries outside the schema survive, schema
 * params are replaced by `validated`, and values equal to the schema default
 * are dropped (an entry only carries non-default params).
 */
export function mergeParams(
  fields: readonly ParamField[],
  current: Record<string, unknown>,
  validated: Record<string, unknown>,
): Record<string, unknown> {
  const out: Record<string, unknown> = { ...current };
  for (const f of fields) {
    delete out[f.name];
    const v = validated[f.name];
    if (v !== undefined && v !== f.defaultValue) out[f.name] = v;
  }
  return out;
}

/** Validate edited values: no NaN, integers are whole, within min/max. */
export function validate(
  fields: readonly ParamField[],
  values: FormValues,
): ValidationResult {
  const errors: Record<string, string> = {};
  const params: Record<string, unknown> = {};
  for (const f of fields) {
    const raw = values[f.name];
    if (f.kind === "boolean") {
      params[f.name] = raw === true;
      continue;
    }
    const text = typeof raw === "string" ? raw.trim() : "";
    if (text === "") {
      if (f.defaultValue === null) continue; // optional, left unset
      if (f.kind === "string") params[f.name] = "";
      else errors[f.name] = "Required";
      continue;
    }
    if (f.kind === "string") {
      params[f.name] = text;
      continue;
    }
    const n = Number(text);
    if (!Number.isFinite(n)) {
      errors[f.name] = "Must be a number";
    } else if (f.kind === "integer" && !Number.isInteger(n)) {
      errors[f.name] = "Must be a whole number";
    } else if (f.min !== undefined && n < f.min) {
      errors[f.name] = `Must be at least ${f.min}`;
    } else if (f.max !== undefined && n > f.max) {
      errors[f.name] = `Must be at most ${f.max}`;
    } else {
      params[f.name] = n;
    }
  }
  return { errors, params };
}
