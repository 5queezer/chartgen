// The setups of the chart's symbol for the Setups tab (ADR-0071, ADR-0091), read through
// the same MCP session and token as every other tool: the server answers with the
// caller's own setups only. The whole record is the Setups page's. Nothing is kept in
// storage; the tab asks again when it opens and when the symbol changes.

import { createSignal } from "solid-js";

import { setupDrawingPrefix, setupDrawings } from "../lib/setups";
import { parseTimeframe } from "../lib/timeframe";
import type { McpSession } from "../mcp";
import { showToast } from "../services/alertEngine";
import type { Setup } from "../types/responses";
import { announceDrawings, replaceDrawings } from "./drawings";
import { setSymbol, setTimeframe, symbol } from "./layout";

/** The most of one symbol's setups the tab asks for: its newest. */
export const LIST_LIMIT = 50;

export type SetupsState =
  | { kind: "loading" }
  | { kind: "off" }
  | { kind: "error"; message: string }
  | {
      kind: "ready";
      symbol: string;
      setups: Setup[];
      /** How many the symbol has; more than `setups` holds past `LIST_LIMIT`. */
      total: number;
    };

const [state, setState] = createSignal<SetupsState>({ kind: "loading" });
export const setupsState = state;

/** The server's reason, without the transport's prefix. */
function reason(err: unknown): string {
  const text = err instanceof Error ? err.message : String(err);
  return text.replace(/^MCP tool error: /, "");
}

/** Refreshes started, and setups opened: only the latest one's answer is taken. */
let refreshes = 0;
let opens = 0;

/**
 * Ask the server for the caller's setups on `on`. What was shown of `on` stays while a
 * refresh fails; another symbol's gives way to the error. A refresh that answers after a later one is dropped: its statuses would be
 * older, or its symbol not the chart's any more.
 */
export async function refreshSetups(session: McpSession, on: string): Promise<void> {
  const mine = ++refreshes;
  const latest = () => mine === refreshes;
  try {
    const list = await session.callListSetups({ symbol: on, limit: LIST_LIMIT });
    if (!latest()) return;
    if (!list.enabled) {
      setState({ kind: "off" });
      return;
    }
    setState({ kind: "ready", symbol: on, setups: list.setups, total: list.total ?? list.setups.length });
  } catch (err) {
    // What was shown stays only when it is this symbol's.
    const s = state();
    if (latest() && !(s.kind === "ready" && s.symbol === on)) {
      setState({ kind: "error", message: reason(err) });
    }
  }
}

/**
 * Open a setup on the chart: its symbol at its interval, with its drawings and, for a skip,
 * the server's bracket. They join the symbol's drawings as the user's own, as a link's do
 * (ADR-0054), under ids of the setup's own, and replace what an earlier click drew: a setup
 * that moved on is shown once, as it is now. The setup is read again first, so the chart
 * shows it as the server has it now. Only the setup clicked last is opened: one whose read
 * answers after a later click's is dropped.
 */
export async function openSetup(session: McpSession, id: string): Promise<void> {
  const mine = ++opens;
  let setup: Setup;
  try {
    setup = await session.callGetSetup({ id });
  } catch (err) {
    if (mine === opens) showToast(`Setup could not be read: ${reason(err)}`);
    return;
  }
  if (mine !== opens) return;
  const records = setupDrawings(setup);
  const put = replaceDrawings(setup.symbol, setupDrawingPrefix(setup.id), records);
  if (put === null) showToast("The setup's drawings could not be saved");
  const switching = setup.symbol !== symbol();
  setSymbol(setup.symbol);
  const interval = parseTimeframe(setup.interval);
  if (interval !== null) setTimeframe(interval);
  // A chart already on the symbol loads the drawings again (ADR-0043); one switching to it
  // reads them as it loads.
  if (!switching && put !== null) announceDrawings(setup.symbol, -1);
}
