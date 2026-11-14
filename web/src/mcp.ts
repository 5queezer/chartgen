// Thin wrapper around `@modelcontextprotocol/sdk`'s Streamable HTTP
// client. One long-lived Client per app session. The bearer token is
// attached via `requestInit.headers` so it flows through both the POST
// tool calls and the GET SSE stream; when the server answers 401 we
// clear the token and re-run OAuth before the next attempt.

import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";
import type { Notification } from "@modelcontextprotocol/sdk/types.js";
import { z } from "zod";

import type {
  CreateServerAlertInput,
  DeleteServerAlertInput,
  GenerateChartInput,
  GetFootprintInput,
  GetQuotesInput,
  GetSetupInput,
  GetTrackRecordInput,
  GetTpoInput,
  ListSetupsInput,
  SearchSymbolsInput,
  SubscribeChartInput,
  SubscribePushInput,
  UnsubscribeChartInput,
  UnsubscribePushInput,
  UpdateServerAlertInput,
} from "./types/generated-input";
import {
  CHART_TICK_METHOD,
  ChartTickParamsSchema,
  DeleteIndicatorPayloadSchema,
  DeleteServerAlertPayloadSchema,
  ListIndicatorsPayloadSchema,
  MyIndicatorSourcePayloadSchema,
  MyIndicatorsPayloadSchema,
  FootprintPayloadSchema,
  QuotesPayloadSchema,
  TpoPayloadSchema,
  RegisteredIndicatorPayloadSchema,
  SearchSymbolsPayloadSchema,
  SeriesPayloadSchema,
  SetupPayloadSchema,
  SetupsPayloadSchema,
  TrackRecordPayloadSchema,
  ServerAlertChangePayloadSchema,
  ServerAlertsPayloadSchema,
  SubscribeChartPayloadSchema,
  SubscribePushPayloadSchema,
  UnsubscribeChartPayloadSchema,
  UnsubscribePushPayloadSchema,
  type DeleteIndicatorPayload,
  type DeleteServerAlertPayload,
  type ListIndicatorsPayload,
  type MyIndicatorSourcePayload,
  type MyIndicatorsPayload,
  type FootprintPayload,
  type QuotesPayload,
  type TpoPayload,
  type RegisteredIndicatorPayload,
  type ScriptRejection,
  type SearchSymbolsPayload,
  type SeriesPayload,
  type SetupPayload,
  type SetupsPayload,
  type TrackRecordPayload,
  type ServerAlertChangePayload,
  type ServerAlertsPayload,
  type SubscribeChartPayload,
  type SubscribePushPayload,
  type UnsubscribeChartPayload,
  type UnsubscribePushPayload,
} from "./types/responses";
import { isNotFound, scriptRejection } from "./lib/scripts";
import { clearAccessToken, getAccessToken } from "./oauth";
import { fetchComingBack } from "./lib/retryAfter";

// Validate the MCP `CallToolResult` shape at the boundary (ADR-0004)
// instead of casting the SDK's `unknown`-typed return value.
const CallToolResultSchema = z.object({
  content: z.array(
    z.object({
      type: z.string(),
      text: z.string().optional(),
    }),
  ),
  isError: z.boolean().optional(),
});

const BASE_URL =
  (import.meta.env.VITE_CHARTGEN_BASE_URL as string | undefined) ?? "";

/** MCP endpoint URL — honor `VITE_CHARTGEN_BASE_URL` or go same-origin. */
function mcpUrl(): URL {
  const origin = BASE_URL || window.location.origin;
  return new URL("/mcp", origin);
}

/**
 * The tool answered with `isError`. The call itself went through, so its text
 * says nothing about auth, even when it contains "401" (a script's line 401).
 */
export class McpToolError extends Error {
  constructor(readonly text: string) {
    super(`MCP tool error: ${text || "(no message)"}`);
  }
}

export type AlertNotificationHandler = (
  notification: Notification,
) => void | Promise<void>;

/** Receives each `notifications/chart_tick` (ADR-0021), already validated. */
export type ChartTickHandler = (subscriptionId: string, series: SeriesPayload) => void;

export interface McpSession {
  /**
   * Invoke `generate_chart` with `format: 'series'` and parse the response.
   * `bars`, `warmup` and `end_time` page and poll the series (ADR-0009);
   * omitted ones take the server defaults. `signal` aborts the call.
   */
  callGenerateChart: (
    args: {
      ticker: string;
      timeframe: string;
      indicators: GenerateChartInput["indicators"];
    } & Pick<
      GenerateChartInput,
      "bars" | "warmup" | "end_time" | "chart_style" | "chart_params" | "profile_intrabars"
    >,
    signal?: AbortSignal,
  ) => Promise<SeriesPayload>;
  /** Invoke `get_footprint` (ADR-0048). */
  callGetFootprint: (args: GetFootprintInput, signal?: AbortSignal) => Promise<FootprintPayload>;
  /** Invoke `get_tpo` (ADR-0048). */
  callGetTpo: (args: GetTpoInput, signal?: AbortSignal) => Promise<TpoPayload>;
  /** Invoke `list_indicators` and parse the response. */
  callListIndicators: () => Promise<ListIndicatorsPayload>;
  /** Invoke `get_quotes` for the watchlist and parse the response. */
  callGetQuotes: (symbols: string[]) => Promise<QuotesPayload>;
  /**
   * Invoke `search_symbols` for the symbol search dialog. Aborting `signal`
   * cancels the in-flight request.
   */
  callSearchSymbols: (
    args: SearchSymbolsInput,
    signal?: AbortSignal,
  ) => Promise<SearchSymbolsPayload>;
  /**
   * Invoke `subscribe_chart`: pushed tails of this chart arrive at the
   * `onChartTick` handler. Rejects for charts that cannot be pushed.
   */
  callSubscribeChart: (args: SubscribeChartInput) => Promise<SubscribeChartPayload>;
  /** Invoke `unsubscribe_chart`. */
  callUnsubscribeChart: (
    args: UnsubscribeChartInput,
  ) => Promise<UnsubscribeChartPayload>;
  /** Does `tools/list` have the server alert tools? One request, and no tool call. */
  serverAlertsListed: () => Promise<boolean>;
  /** Invoke `list_server_alerts`; says so when the server does not run them. */
  callListServerAlerts: () => Promise<ServerAlertsPayload>;
  /** Invoke `create_server_alert`; a signing secret is only in this response. */
  callCreateServerAlert: (args: CreateServerAlertInput) => Promise<ServerAlertChangePayload>;
  /** Invoke `update_server_alert`. */
  callUpdateServerAlert: (args: UpdateServerAlertInput) => Promise<ServerAlertChangePayload>;
  /** Invoke `delete_server_alert`. */
  callDeleteServerAlert: (args: DeleteServerAlertInput) => Promise<DeleteServerAlertPayload>;
  /** Invoke `subscribe_push` (ADR-0045): this browser's push subscription. */
  callSubscribePush: (args: SubscribePushInput) => Promise<SubscribePushPayload>;
  /** Invoke `unsubscribe_push`. */
  callUnsubscribePush: (args: UnsubscribePushInput) => Promise<UnsubscribePushPayload>;
  /** Invoke `list_setups` (ADR-0059): the caller's setups; says so when the server keeps none. */
  callListSetups: (args: ListSetupsInput) => Promise<SetupsPayload>;
  /** Invoke `get_setup`: one of the caller's setups with its log. */
  callGetSetup: (args: GetSetupInput) => Promise<SetupPayload>;
  /** Invoke `get_track_record` (ADR-0064): counts over the caller's closed setups. */
  callGetTrackRecord: (args: GetTrackRecordInput) => Promise<TrackRecordPayload>;
  /** The names `tools/list` reports; the scripting tools are there only when enabled. */
  callListToolNames: () => Promise<string[]>;
  /** Invoke `list_my_indicators` (ADR-0025): the caller's scripts. */
  callListMyIndicators: () => Promise<MyIndicatorsPayload>;
  /** Invoke `get_my_indicator` (ADR-0041): one script's stored source; null if you have none. */
  callGetMyIndicator: (name: string) => Promise<MyIndicatorSourcePayload | null>;
  /**
   * Invoke `register_indicator`. A script the server refuses is a result,
   * with the errors to show next to the code, not a rejection.
   */
  callRegisterIndicator: (args: {
    name: string;
    source: string;
    /** Refuse instead of replacing a script of that name. */
    create_only: boolean;
  }) => Promise<RegisteredIndicatorPayload | ScriptRejection>;
  /** Invoke `delete_indicator`. */
  callDeleteIndicator: (name: string) => Promise<DeleteIndicatorPayload>;
  /** Install the handler for `notifications/chart_tick`. */
  onChartTick: (handler: ChartTickHandler) => void;
  /** Install a handler invoked for every other `notifications/*` frame. */
  onNotification: (handler: AlertNotificationHandler) => void;
  /** Close the underlying transport. */
  close: () => Promise<void>;
}

interface ConnectedTransport {
  client: Client;
  transport: StreamableHTTPClientTransport;
  token: string;
}

async function connectWithToken(
  token: string,
  onNotification: AlertNotificationHandler,
): Promise<ConnectedTransport> {
  const transport = new StreamableHTTPClientTransport(mcpUrl(), {
    requestInit: {
      headers: { Authorization: `Bearer ${token}` },
    },
    fetch: fetchComingBack((input, init) => fetch(input, init)),
  });

  const client = new Client(
    { name: "chartgen-web", version: "0.1.0" },
    { capabilities: {} },
  );

  client.fallbackNotificationHandler = async (n: Notification) => {
    await onNotification(n);
  };

  try {
    await client.connect(transport);
  } catch (err) {
    // Don't leak a half-open client when the handshake fails.
    await client.close().catch(() => {});
    throw err;
  }
  return { client, transport, token };
}

interface McpSessionInternalState {
  connected: ConnectedTransport | null;
  /** In-flight connect shared by concurrent callers; cleared on settle. */
  connecting: Promise<ConnectedTransport> | null;
  notificationHandler: AlertNotificationHandler | null;
  chartTickHandler: ChartTickHandler | null;
}

/**
 * Extract the JSON payload from a CallToolResult. chartgen returns a
 * single `text` content block whose body is JSON per the wire contract
 * documented in `website/src/content/docs/reference/mcp.md`.
 */
function extractJsonFromResult(result: {
  content: Array<{ type: string; text?: string }>;
  isError?: boolean;
}): unknown {
  if (result.isError) {
    const msg = result.content
      .map((c) => (c.type === "text" ? c.text : ""))
      .filter(Boolean)
      .join("\n");
    throw new McpToolError(msg);
  }
  const first = result.content[0];
  if (!first || first.type !== "text" || typeof first.text !== "string") {
    throw new Error("MCP response missing text content");
  }
  return JSON.parse(first.text);
}

function isUnauthorized(err: unknown): boolean {
  if (err instanceof McpToolError) return false;
  if (err instanceof Error) {
    const m = err.message || "";
    if (m.includes("401") || /unauthoriz/i.test(m)) return true;
  }
  // The SDK throws UnauthorizedError with a .code of 401 for Streamable
  // HTTP transport auth failures. Duck-type instead of importing, to
  // avoid coupling this wrapper to the SDK's error class layout.
  const maybe = err as { code?: number } | null;
  return maybe !== null && maybe.code === 401;
}

export function createMcpSession(): McpSession {
  const state: McpSessionInternalState = {
    connected: null,
    connecting: null,
    notificationHandler: null,
    chartTickHandler: null,
  };

  // Every connection dispatches through the current handlers, so they can
  // be installed or swapped at any time. Chart ticks are validated at the
  // boundary (ADR-0004); a malformed one is dropped.
  const dispatch = async (n: Notification) => {
    if (n.method === CHART_TICK_METHOD) {
      const p = ChartTickParamsSchema.safeParse(n.params);
      if (p.success) state.chartTickHandler?.(p.data.subscription_id, p.data.series);
      return;
    }
    await state.notificationHandler?.(n);
  };

  function ensureConnected(): Promise<ConnectedTransport> {
    if (state.connected) return Promise.resolve(state.connected);
    // Concurrent callers share one in-flight connect. The memo is cleared
    // on settle, so a failed attempt is retried by the next call.
    state.connecting ??= (async () => {
      try {
        const token = await getAccessToken();
        const conn = await connectWithToken(token, dispatch);
        state.connected = conn;
        return conn;
      } finally {
        state.connecting = null;
      }
    })();
    return state.connecting;
  }

  /** Close `stale` (default: the current connection) and reset state. */
  async function dropConnection(stale?: ConnectedTransport): Promise<void> {
    const target = stale ?? state.connected;
    if (!target) return;
    // A concurrent caller may already have replaced it; only reset state
    // if it is still the current one.
    if (state.connected === target) state.connected = null;
    try {
      await target.client.close();
    } catch {
      // best-effort; close errors after a 401 are not actionable.
    }
  }

  async function withAuthRetry<T>(
    request: (conn: ConnectedTransport) => Promise<T>,
  ): Promise<T> {
    const conn = await ensureConnected();
    try {
      return await request(conn);
    } catch (err) {
      if (!isUnauthorized(err)) throw err;
      // Re-auth once, then retry.
      clearAccessToken();
      await dropConnection(conn);
      return request(await ensureConnected());
    }
  }

  function callToolWithAuthRetry(
    name: string,
    args: Record<string, unknown>,
    signal?: AbortSignal,
  ): Promise<unknown> {
    return withAuthRetry(async (conn) => {
      const result = await conn.client.callTool(
        { name, arguments: args },
        undefined,
        { signal },
      );
      return extractJsonFromResult(CallToolResultSchema.parse(result));
    });
  }

  return {
    async callGenerateChart(
      { ticker, timeframe, indicators, bars, warmup, end_time, chart_style, chart_params, profile_intrabars },
      signal,
    ) {
      const args: GenerateChartInput = {
        ticker,
        timeframe,
        format: "series",
        indicators,
        ...(bars !== undefined && { bars }),
        ...(warmup !== undefined && { warmup }),
        ...(end_time !== undefined && { end_time }),
        ...(chart_style !== undefined && { chart_style }),
        ...(chart_params !== undefined && { chart_params }),
        ...(profile_intrabars !== undefined && { profile_intrabars }),
      };
      const json = await callToolWithAuthRetry(
        "generate_chart",
        args as unknown as Record<string, unknown>,
        signal,
      );
      return SeriesPayloadSchema.parse(json);
    },
    async callListIndicators() {
      const json = await callToolWithAuthRetry("list_indicators", {});
      return ListIndicatorsPayloadSchema.parse(json);
    },
    async callGetFootprint(args, signal) {
      const json = await callToolWithAuthRetry(
        "get_footprint",
        args as unknown as Record<string, unknown>,
        signal,
      );
      return FootprintPayloadSchema.parse(json);
    },
    async callGetTpo(args, signal) {
      const json = await callToolWithAuthRetry(
        "get_tpo",
        args as unknown as Record<string, unknown>,
        signal,
      );
      return TpoPayloadSchema.parse(json);
    },
    async callGetQuotes(symbols) {
      const args: GetQuotesInput = { symbols };
      const json = await callToolWithAuthRetry(
        "get_quotes",
        args as unknown as Record<string, unknown>,
      );
      return QuotesPayloadSchema.parse(json);
    },
    async callSearchSymbols(args, signal) {
      const json = await callToolWithAuthRetry(
        "search_symbols",
        args as unknown as Record<string, unknown>,
        signal,
      );
      return SearchSymbolsPayloadSchema.parse(json);
    },
    async callSubscribeChart(args) {
      const json = await callToolWithAuthRetry(
        "subscribe_chart",
        args as unknown as Record<string, unknown>,
      );
      return SubscribeChartPayloadSchema.parse(json);
    },
    async callUnsubscribeChart(args) {
      const json = await callToolWithAuthRetry(
        "unsubscribe_chart",
        args as unknown as Record<string, unknown>,
      );
      return UnsubscribeChartPayloadSchema.parse(json);
    },
    async serverAlertsListed() {
      const conn = await ensureConnected();
      const { tools } = await conn.client.listTools();
      return tools.some((t) => t.name === "list_server_alerts");
    },
    async callListServerAlerts() {
      const json = await callToolWithAuthRetry("list_server_alerts", {});
      return ServerAlertsPayloadSchema.parse(json);
    },
    async callCreateServerAlert(args) {
      const json = await callToolWithAuthRetry(
        "create_server_alert",
        args as unknown as Record<string, unknown>,
      );
      return ServerAlertChangePayloadSchema.parse(json);
    },
    async callUpdateServerAlert(args) {
      const json = await callToolWithAuthRetry(
        "update_server_alert",
        args as unknown as Record<string, unknown>,
      );
      return ServerAlertChangePayloadSchema.parse(json);
    },
    async callDeleteServerAlert(args) {
      const json = await callToolWithAuthRetry(
        "delete_server_alert",
        args as unknown as Record<string, unknown>,
      );
      return DeleteServerAlertPayloadSchema.parse(json);
    },
    async callSubscribePush(args) {
      const json = await callToolWithAuthRetry(
        "subscribe_push",
        args as unknown as Record<string, unknown>,
      );
      return SubscribePushPayloadSchema.parse(json);
    },
    async callUnsubscribePush(args) {
      const json = await callToolWithAuthRetry(
        "unsubscribe_push",
        args as unknown as Record<string, unknown>,
      );
      return UnsubscribePushPayloadSchema.parse(json);
    },
    async callListSetups(args) {
      const json = await callToolWithAuthRetry(
        "list_setups",
        args as unknown as Record<string, unknown>,
      );
      return SetupsPayloadSchema.parse(json);
    },
    async callGetSetup(args) {
      const json = await callToolWithAuthRetry(
        "get_setup",
        args as unknown as Record<string, unknown>,
      );
      return SetupPayloadSchema.parse(json);
    },
    async callGetTrackRecord(args) {
      const json = await callToolWithAuthRetry(
        "get_track_record",
        args as unknown as Record<string, unknown>,
      );
      return TrackRecordPayloadSchema.parse(json);
    },
    callListToolNames() {
      return withAuthRetry(async (conn) => {
        const { tools } = await conn.client.listTools();
        return tools.map((t) => t.name);
      });
    },
    async callListMyIndicators() {
      const json = await callToolWithAuthRetry("list_my_indicators", {});
      return MyIndicatorsPayloadSchema.parse(json);
    },
    async callGetMyIndicator(name) {
      try {
        const json = await callToolWithAuthRetry("get_my_indicator", { name });
        return MyIndicatorSourcePayloadSchema.parse(json);
      } catch (err) {
        if (err instanceof McpToolError && isNotFound(err.text)) return null;
        throw err;
      }
    },
    async callRegisterIndicator(args) {
      try {
        const json = await callToolWithAuthRetry("register_indicator", args);
        return RegisteredIndicatorPayloadSchema.parse(json);
      } catch (err) {
        if (err instanceof McpToolError) return scriptRejection(err.text);
        throw err;
      }
    },
    async callDeleteIndicator(name) {
      const json = await callToolWithAuthRetry("delete_indicator", { name });
      return DeleteIndicatorPayloadSchema.parse(json);
    },
    onChartTick(handler) {
      state.chartTickHandler = handler;
    },
    onNotification(handler) {
      state.notificationHandler = handler;
    },
    async close() {
      // Let an in-flight connect settle first so its client is not leaked.
      await state.connecting?.catch(() => {});
      await dropConnection();
    },
  };
}
