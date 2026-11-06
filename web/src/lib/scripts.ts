// Pure parts of the script editor (ADR-0041): detecting the scripting tools,
// reading a refused script's errors, and the unsaved drafts in localStorage.

import { z } from "zod";

import {
  IndicatorNotFoundSchema,
  ScriptRejectionSchema,
  type ScriptRejection,
} from "../types/responses";

/** Everything the editor calls; it appears only when the server lists them all. */
const EDITOR_TOOLS = [
  "list_my_indicators",
  "get_my_indicator",
  "register_indicator",
  "delete_indicator",
] as const;

export function scriptingEnabled(toolNames: readonly string[] | undefined): boolean {
  return EDITOR_TOOLS.every((t) => toolNames?.includes(t));
}

export const SCRIPTING_DOCS_URL =
  "https://5queezer.github.io/chartgen/guides/scripted-indicators/";

/** What a new script starts from: the smallest one that registers and draws. */
export const STARTER_SCRIPT = `PARAMS = [
    {"name": "length", "type": "int", "default": 20, "min": 1, "max": 500, "desc": "EMA length"},
]
META = {"title": "My EMA", "overlay": True, "description": "EMA of the close"}

def compute(ohlcv, params, chart):
    chart.line(ta.ema(ohlcv.close, params["length"]), title = "EMA", color = "#2962ff")
`;

/**
 * The errors in a refused `register_indicator`: a validation failure is JSON
 * with a line per error, any other refusal (name, limit) is plain text.
 */
export function scriptRejection(text: string): ScriptRejection {
  try {
    const parsed = ScriptRejectionSchema.safeParse(JSON.parse(text));
    if (parsed.success) return parsed.data;
  } catch {
    // Not JSON: a plain refusal.
  }
  return { ok: false, errors: [{ step: "register", line: null, message: text }] };
}

/** Whether an error text is the server's own "you have no such script". */
export function isNotFound(text: string): boolean {
  try {
    return IndicatorNotFoundSchema.safeParse(JSON.parse(text)).success;
  } catch {
    return false;
  }
}

/** Whether a chart query's indicators key (`activeIndicatorsKey`) includes the indicator. */
export function usesIndicator(indicatorsKey: string, name: string): boolean {
  try {
    const entries: unknown = JSON.parse(indicatorsKey);
    return (
      Array.isArray(entries) &&
      entries.some((e) => (typeof e === "string" ? e : (e as { name?: unknown })?.name) === name)
    );
  } catch {
    return false;
  }
}

const DraftSchema = z.object({
  name: z.string(),
  source: z.string(),
  /** The saved text this draft started from; none for a script never saved. */
  base: z.string().optional(),
});
export type ScriptDraft = z.infer<typeof DraftSchema>;

const DraftsSchema = z.object({
  /** The script that was open; null for a new, unsaved one. */
  selected: z.string().nullable(),
  /** Unsaved text by script name; NEW_DRAFT holds the new script's. */
  drafts: z.record(z.string(), DraftSchema),
});
export type ScriptDrafts = z.infer<typeof DraftsSchema>;

export const NEW_DRAFT = "";

const DRAFTS_KEY = "chartgen.scripts.drafts";

type DraftStorage = Pick<Storage, "getItem" | "setItem">;

export function loadDrafts(storage: DraftStorage = localStorage): ScriptDrafts {
  try {
    const parsed = DraftsSchema.safeParse(JSON.parse(storage.getItem(DRAFTS_KEY) ?? "null"));
    if (parsed.success) return parsed.data;
  } catch {
    // Unreadable: start empty.
  }
  return { selected: null, drafts: {} };
}

/** The draft stored under `key`, never something inherited (a script may be called `constructor`). */
export function draftAt(drafts: ScriptDrafts["drafts"], key: string): ScriptDraft | undefined {
  return Object.hasOwn(drafts, key) ? drafts[key] : undefined;
}

/**
 * Change one key, on top of what is stored now: another tab's drafts stay.
 * `selected` is last-writer-wins. False when the browser refused it (quota,
 * private mode): the draft is not kept.
 */
export function writeDraft(
  key: string,
  draft: ScriptDraft | null,
  selected: string | null,
  storage: DraftStorage = localStorage,
): boolean {
  const { [key]: _old, ...others } = loadDrafts(storage).drafts;
  const drafts = draft ? { ...others, [key]: draft } : others;
  try {
    storage.setItem(DRAFTS_KEY, JSON.stringify({ selected, drafts }));
    return true;
  } catch {
    return false;
  }
}
