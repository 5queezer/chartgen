// TanStack Solid Query definitions. The chart query is the only
// server-state query for v0.1; alerts, positions, and orders will grow
// their own hooks here as they land.

import { createQuery } from "@tanstack/solid-query";
import type { Accessor } from "solid-js";

import type { McpSession } from "./mcp";
import { clearAccessToken } from "./oauth";
import {
  activeIndicatorsKey,
  type ActiveIndicator,
} from "./stores/indicators";
import type {
  ListIndicatorsPayload,
  SeriesPayload,
} from "./types/responses";

export interface ChartQueryInput {
  ticker: string;
  timeframe: string;
  indicators: ActiveIndicator[];
  /** Gates the query — flip to true when the user clicks Load. */
  enabled: boolean;
}

/**
 * Create a TanStack Solid Query for `generate_chart`. The accessor
 * contract means every reactive read from `input()` becomes a
 * dependency — Solid Query re-evaluates on each tick and refetches
 * when the key changes.
 *
 * The active indicator set is included in the key via a stable JSON
 * encoding so that toggling indicators invalidates the cache while
 * semantically-equal sets (same entries, same param values) hit the
 * same cache entry.
 */
export function createChartQuery(
  session: McpSession,
  input: Accessor<ChartQueryInput>,
) {
  return createQuery<SeriesPayload, Error>(() => {
    const { ticker, timeframe, indicators, enabled } = input();
    const indicatorsKey = activeIndicatorsKey(indicators);
    return {
      queryKey: [
        "generate_chart",
        ticker,
        timeframe,
        indicatorsKey,
      ] as const,
      queryFn: () =>
        session.callGenerateChart({ ticker, timeframe, indicators }),
      staleTime: 60_000,
      enabled,
      retry: (failureCount: number, error: Error) => {
        // The MCP wrapper re-auths once on 401, so any 401 that bubbles
        // out has already survived a retry. Drop the token defensively
        // and let the user re-trigger manually.
        if (/401|unauthoriz/i.test(error.message)) {
          clearAccessToken();
          return false;
        }
        // No automatic retries for v0.1.
        return failureCount < 0;
      },
    };
  });
}

/**
 * Create a TanStack Solid Query for `list_indicators`. The registry does
 * not change at runtime, so we cache aggressively — `Infinity` for both
 * stale and gc time. `enabled` gates the fetch until auth is ready.
 */
export function createListIndicatorsQuery(
  session: McpSession,
  enabled: Accessor<boolean>,
) {
  return createQuery<ListIndicatorsPayload, Error>(() => ({
    queryKey: ["list_indicators"] as const,
    queryFn: () => session.callListIndicators(),
    staleTime: Infinity,
    gcTime: Infinity,
    enabled: enabled(),
    retry: (failureCount: number, error: Error) => {
      if (/401|unauthoriz/i.test(error.message)) {
        clearAccessToken();
        return false;
      }
      return failureCount < 0;
    },
  }));
}
