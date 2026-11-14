// The signed-in owner's setups for the Setups panel (ADR-0071), read through the same
// MCP session and token as every other tool: the server answers with the caller's own
// setups only. Nothing is kept in storage; the panel asks again when it opens.

import { createSignal } from "solid-js";

import { setupDrawingPrefix, setupDrawings } from "../lib/setups";
import { parseTimeframe } from "../lib/timeframe";
import type { McpSession } from "../mcp";
import { showToast } from "../services/alertEngine";
import type { Setup, TrackRecordPayload } from "../types/responses";
import { announceDrawings, replaceDrawings } from "./drawings";
import { setSymbol, setTimeframe, symbol } from "./layout";

/** The most `list_setups` returns at once. */
export const LIST_LIMIT = 500;

export type SetupsState =
  | { kind: "loading" }
  | { kind: "off" }
  | { kind: "error"; message: string }
  | {
      kind: "ready";
      setups: Setup[];
      /** How many the server has; more than `setups` holds past `LIST_LIMIT`. */
      total: number;
      /** Counts over the closed setups taken and skipped, as `get_track_record` gives them. */
      takes: TrackRecordPayload | null;
      skips: TrackRecordPayload | null;
    };

const [state, setState] = createSignal<SetupsState>({ kind: "loading" });
export const setupsState = state;

/** The server's reason, without the transport's prefix. */
function reason(err: unknown): string {
  const text = err instanceof Error ? err.message : String(err);
  return text.replace(/^MCP tool error: /, "");
}

/**
 * Ask the server for the caller's setups and their track record. What was shown stays
 * while a refresh fails; the totals are left out alone when only they could not be read.
 */
export async function refreshSetups(session: McpSession): Promise<void> {
  try {
    const list = await session.callListSetups({ limit: LIST_LIMIT });
    if (!list.enabled) {
      setState({ kind: "off" });
      return;
    }
    const record = (decision: "take" | "skip") =>
      session.callGetTrackRecord({ decision }).catch(() => null);
    const [takes, skips] = await Promise.all([record("take"), record("skip")]);
    setState({
      kind: "ready",
      setups: list.setups,
      total: list.total ?? list.setups.length,
      takes,
      skips,
    });
  } catch (err) {
    if (state().kind !== "ready") setState({ kind: "error", message: reason(err) });
  }
}

/**
 * Open a setup on the chart: its symbol at its interval, with its drawings and, for a skip,
 * the server's bracket. They join the symbol's drawings as the user's own, as a link's do
 * (ADR-0054), under ids of the setup's own, and replace what an earlier click drew: a setup
 * that moved on is shown once, as it is now. The setup is read again first, so the chart
 * shows it as the server has it now.
 */
export async function openSetup(session: McpSession, id: string): Promise<void> {
  let setup: Setup;
  try {
    setup = await session.callGetSetup({ id });
  } catch (err) {
    showToast(`Setup could not be read: ${reason(err)}`);
    return;
  }
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
