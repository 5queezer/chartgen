// Thin wrapper around `@modelcontextprotocol/sdk`'s Streamable HTTP
// client. One long-lived Client per app session. The bearer token is
// attached via `requestInit.headers` so it flows through both the POST
// tool calls and the GET SSE stream; when the server answers 401 we
// clear the token and re-run OAuth before the next attempt.

import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";
import type { Notification } from "@modelcontextprotocol/sdk/types.js";
import { z } from "zod";

import type { GenerateChartInput } from "./types/generated-input";
import { SeriesPayloadSchema, type SeriesPayload } from "./types/responses";
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

export interface McpSession {
  /** Invoke `generate_chart` with `format: 'series'` and parse the response. */
  callGenerateChart: (args: {
    ticker: string;
    timeframe: string;
  }) => Promise<SeriesPayload>;
  /** Install a handler invoked for every `notifications/*` frame. */
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
  onNotification: AlertNotificationHandler | null,
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

  if (onNotification) {
    client.fallbackNotificationHandler = async (n: Notification) => {
      await onNotification(n);
    };
  }

  await client.connect(transport);
  return { client, transport, token };
}

interface McpSessionInternalState {
  connected: ConnectedTransport | null;
  notificationHandler: AlertNotificationHandler | null;
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
    notificationHandler: null,
  };

  async function ensureConnected(): Promise<ConnectedTransport> {
    if (state.connected) return state.connected;
    const token = await getAccessToken();
    state.connected = await connectWithToken(token, state.notificationHandler);
    return state.connected;
  }

  async function dropConnection(): Promise<void> {
    if (!state.connected) return;
    try {
      await state.connected.client.close();
    } catch {
      // best-effort; close errors after a 401 are not actionable.
    }
    state.connected = null;
  }

  async function callToolWithAuthRetry(
    name: string,
    args: Record<string, unknown>,
  ): Promise<unknown> {
    let conn = await ensureConnected();
    try {
      const result = await conn.client.callTool({ name, arguments: args });
      return extractJsonFromResult(CallToolResultSchema.parse(result));
    } catch (err) {
      if (!isUnauthorized(err)) throw err;
      // Re-auth once, then retry.
      clearAccessToken();
      await dropConnection();
      conn = await ensureConnected();
      const result = await conn.client.callTool({ name, arguments: args });
      return extractJsonFromResult(CallToolResultSchema.parse(result));
    }
  }

  return {
    async callGenerateChart({ ticker, timeframe }) {
      const args: GenerateChartInput = {
        ticker,
        timeframe,
        format: "series",
        indicators: ["rsi"],
      };
      const json = await callToolWithAuthRetry(
        "generate_chart",
        args as unknown as Record<string, unknown>,
      );
      return SeriesPayloadSchema.parse(json);
    },
    onNotification(handler) {
      state.notificationHandler = handler;
      if (state.connected) {
        state.connected.client.fallbackNotificationHandler = async (
          n: Notification,
        ) => {
          await handler(n);
        };
      }
    },
    async close() {
      await dropConnection();
    },
  };
}
