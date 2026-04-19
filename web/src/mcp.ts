/**
 * MCP client wrapper.
 *
 * Uses the official `@modelcontextprotocol/sdk` Client with its deprecated
 * `SSEClientTransport` (chartgen implements the 2024-11-05 SSE+POST transport
 * as documented in `website/src/content/docs/reference/mcp.md`, not the newer
 * Streamable HTTP transport).
 *
 * - `tools/call` goes over the SDK Client (which POSTs to the `/message`
 *   endpoint the server announces in its `endpoint` SSE event).
 * - SSE notifications are received via `onNotification`.
 * - Bearer token is injected into both the initial SSE GET and the POSTs via
 *   `requestInit.headers`. No query-param leak.
 */

import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { SSEClientTransport } from '@modelcontextprotocol/sdk/client/sse.js';
import type {
  AlertTriggeredNotification,
  SeriesPayload,
} from './types.js';

/** Opaque handle exposing the bits the UI needs. */
export interface McpSession {
  callGenerateChart(args: {
    ticker: string;
    timeframe: string;
  }): Promise<SeriesPayload>;
  close(): Promise<void>;
}

export interface McpSessionOptions {
  baseUrl: string;
  accessToken: string;
  onNotification?: (n: AlertTriggeredNotification) => void;
}

/**
 * Open an MCP session over SSE. Throws if the server rejects the token.
 */
export async function openMcpSession(
  opts: McpSessionOptions,
): Promise<McpSession> {
  const sseUrl = new URL('/sse', opts.baseUrl);

  const authHeaders = { Authorization: `Bearer ${opts.accessToken}` };

  const transport = new SSEClientTransport(sseUrl, {
    // Applied to both the initial SSE GET and the POST /message round-trips.
    requestInit: { headers: authHeaders },
    // The eventsource polyfill uses this fetch to receive the SSE stream —
    // we pin the same headers there so the initial GET /sse carries the
    // Bearer. The SDK normally sets this, but we provide it explicitly so
    // a missing token fails fast with a 401 at connect time.
    eventSourceInit: {
      fetch: (url, init) =>
        fetch(url, {
          ...init,
          headers: { ...(init?.headers ?? {}), ...authHeaders },
        }),
    },
  });

  const client = new Client(
    { name: 'chartgen-web', version: '0.0.0' },
    { capabilities: {} },
  );

  // Forward alert_triggered JSON-RPC notifications to the caller. The SDK
  // surfaces unknown server notifications through `fallbackNotificationHandler`.
  client.fallbackNotificationHandler = async (notification) => {
    if (notification.method === 'notifications/alert_triggered') {
      opts.onNotification?.(notification as unknown as AlertTriggeredNotification);
    }
  };

  await client.connect(transport);

  return {
    async callGenerateChart({ ticker, timeframe }) {
      const result = await client.callTool({
        name: 'generate_chart',
        arguments: {
          ticker,
          timeframe,
          indicators: ['ema_stack', { name: 'rsi', length: 14 }],
          format: 'series',
        },
      });

      // chartgen returns series as a single text-type content item whose
      // `text` is a stringified JSON matching SeriesPayload.
      const content = result.content;
      if (!Array.isArray(content) || content.length === 0) {
        throw new Error('generate_chart returned no content');
      }
      const first = content[0];
      if (!first || first.type !== 'text' || typeof first.text !== 'string') {
        throw new Error('generate_chart: expected text content item');
      }
      return JSON.parse(first.text) as SeriesPayload;
    },

    async close() {
      await client.close().catch(() => {});
    },
  };
}
