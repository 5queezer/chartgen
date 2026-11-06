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
  GetQuotesInput,
  SearchSymbolsInput,
  SubscribeChartInput,
  UnsubscribeChartInput,
  UpdateServerAlertInput,
} from "./types/generated-input";
import {
  CHART_TICK_METHOD,
  ChartTickParamsSchema,
  DeleteServerAlertPayloadSchema,
  ListIndicatorsPayloadSchema,
  QuotesPayloadSchema,
  SearchSymbolsPayloadSchema,
  SeriesPayloadSchema,
  ServerAlertChangePayloadSchema,
  ServerAlertsPayloadSchema,
  SubscribeChartPayloadSchema,
  UnsubscribeChartPayloadSchema,
  type DeleteServerAlertPayload,
  type ListIndicatorsPayload,
  type QuotesPayload,
  type SearchSymbolsPayload,
  type SeriesPayload,
  type ServerAlertChangePayload,
  type ServerAlertsPayload,
  type SubscribeChartPayload,
  type UnsubscribeChartPayload,
} from "./types/responses";
import { clearAccessToken, getAccessToken } from "./oauth";

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
    } & Pick<GenerateChartInput, "bars" | "warmup" | "end_time" | "chart_style">,
    signal?: AbortSignal,
  ) => Promise<SeriesPayload>;
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
  /** Invoke `list_server_alerts`; says so when the server does not run them. */
  callListServerAlerts: () => Promise<ServerAlertsPayload>;
  /** Invoke `create_server_alert`; a signing secret is only in this response. */
  callCreateServerAlert: (args: CreateServerAlertInput) => Promise<ServerAlertChangePayload>;
  /** Invoke `update_server_alert`. */
  callUpdateServerAlert: (args: UpdateServerAlertInput) => Promise<ServerAlertChangePayload>;
  /** Invoke `delete_server_alert`. */
  callDeleteServerAlert: (args: DeleteServerAlertInput) => Promise<DeleteServerAlertPayload>;
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
    throw new Error(`MCP tool error: ${msg || "(no message)"}`);
  }
  const first = result.content[0];
  if (!first || first.type !== "text" || typeof first.text !== "string") {
    throw new Error("MCP response missing text content");
  }
  return JSON.parse(first.text);
}

function isUnauthorized(err: unknown): boolean {
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

  async function callToolWithAuthRetry(
    name: string,
    args: Record<string, unknown>,
    signal?: AbortSignal,
  ): Promise<unknown> {
    let conn = await ensureConnected();
    try {
      const result = await conn.client.callTool(
        { name, arguments: args },
        undefined,
        { signal },
      );
      return extractJsonFromResult(CallToolResultSchema.parse(result));
    } catch (err) {
      if (!isUnauthorized(err)) throw err;
      // Re-auth once, then retry.
      clearAccessToken();
      await dropConnection(conn);
      conn = await ensureConnected();
      const result = await conn.client.callTool(
        { name, arguments: args },
        undefined,
        { signal },
      );
      return extractJsonFromResult(CallToolResultSchema.parse(result));
    }
  }

  return {
    async callGenerateChart(
      { ticker, timeframe, indicators, bars, warmup, end_time, chart_style },
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
