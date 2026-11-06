// TanStack Solid Query definitions. The chart query is the only
// server-state query for v0.1; alerts, positions, and orders will grow
// their own hooks here as they land.

import { createQuery, keepPreviousData, type QueryClient } from "@tanstack/solid-query";
import type { Accessor } from "solid-js";

import { INITIAL_BARS, WARMUP } from "./lib/liveFeed";
import { usesIndicator } from "./lib/scripts";
import type { ChartStyle } from "./stores/layout";
import { McpToolError, type McpSession } from "./mcp";
import { clearAccessToken } from "./oauth";
import {
  activeIndicatorsKey,
  type ActiveIndicator,
} from "./stores/indicators";
import type {
  ListIndicatorsPayload,
  QuotesPayload,
  SeriesPayload,
} from "./types/responses";

/** A failed sign-in, not a tool's own error text that happens to say "401". */
function isAuthError(error: Error): boolean {
  return !(error instanceof McpToolError) && /401|unauthoriz/i.test(error.message);
}

/** Watchlist poll period while the tab is visible. */
export const QUOTES_POLL_MS = 5_000;

export interface ChartQueryInput {
  ticker: string;
  timeframe: string;
  indicators: ActiveIndicator[];
  chart_style: ChartStyle;
  /** Gates the query — true once auth is ready. */
  enabled: boolean;
}

/** Query key of the chart query for a request. */
export function chartQueryKey(
  input: Pick<ChartQueryInput, "ticker" | "timeframe" | "indicators" | "chart_style">,
) {
  return [
    "generate_chart",
    input.ticker,
    input.timeframe,
    activeIndicatorsKey(input.indicators),
    input.chart_style,
  ] as const;
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
    const { ticker, timeframe, indicators, chart_style, enabled } = input();
    const queryKey = chartQueryKey({
      ticker,
      timeframe,
      indicators,
      chart_style,
    });
    return {
      queryKey,
      // Warmed up like every page and tail (ADR-0009), so the first page
      // seam has no null-indicator gap.
      queryFn: () =>
        session.callGenerateChart({
          ticker,
          timeframe,
          indicators,
          chart_style,
          bars: INITIAL_BARS,
          warmup: WARMUP,
        }),
      // Every fetch result is stored as it came in, never merged into the
      // previous one (replaceEqualDeep). App adopts a load by the cache
      // entry's dataUpdatedAt (App.tsx), not by object identity; this just
      // keeps each load a distinct object.
      structuralSharing: false,
      // This query is only the full load. The live feed (lib/liveFeed.ts)
      // keeps the data fresh from there and pages history into its own
      // merged copy, so a background refetch would only throw that away
      // and snap the view back to the newest bars. Hence: never stale, no
      // refetch on focus or reconnect. Data is refetched on a key change
      // (symbol, timeframe, indicator set or params), an explicit
      // refetch() (Retry, re-selecting the symbol), or when the feed asks
      // for a full reload after a gap it cannot bridge.
      staleTime: Infinity,
      refetchOnWindowFocus: false,
      refetchOnReconnect: false,
      // Keep the previous chart on screen while a new symbol/timeframe
      // loads. The feed only adopts the current key's cache entry (App.tsx).
      placeholderData: keepPreviousData,
      enabled,
      retry: (failureCount: number, error: Error) => {
        // The MCP wrapper re-auths once on 401, so any 401 that bubbles
        // out has already survived a retry. Drop the token defensively
        // and let the user retry from the status line.
        if (isAuthError(error)) {
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
 * stale and gc time. `enabled` gates the fetch until auth is ready. A user
 * script changing it invalidates the query (`refreshIndicatorList`).
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
      if (isAuthError(error)) {
        clearAccessToken();
        return false;
      }
      return failureCount < 0;
    },
  }));
}

/**
 * The tool names of `tools/list`, once per session: what the server lists does
 * not change while it runs. A failure reads as "not listed" and is not retried.
 */
export function createToolNamesQuery(
  session: McpSession,
  enabled: Accessor<boolean>,
) {
  return createQuery<string[], Error>(() => ({
    queryKey: ["tools_list"] as const,
    queryFn: () => session.callListToolNames(),
    staleTime: Infinity,
    gcTime: Infinity,
    enabled: enabled(),
    retry: false,
  }));
}

/** A user script was saved or deleted: reload the picker's list. */
export function refreshIndicatorList(queryClient: QueryClient): Promise<void> {
  return queryClient.invalidateQueries({ queryKey: ["list_indicators"] });
}

/**
 * The script `name` changed: the chart reloads if it shows it, and other cached
 * charts using it refetch when next shown. A chart without it keeps its view.
 */
export function refreshChartsUsing(queryClient: QueryClient, name: string): Promise<void> {
  return queryClient.invalidateQueries({
    queryKey: ["generate_chart"],
    predicate: (q) => usesIndicator(String(q.queryKey[3]), name),
  });
}

/**
 * Create a TanStack Solid Query for `get_quotes` (the watchlist). The
 * interval timer keeps running at `QUOTES_POLL_MS`; with the default
 * `refetchIntervalInBackground: false` each tick is skipped while the tab
 * is hidden, so polling resumes on the first tick after the tab is
 * visible again. (`refetchOnWindowFocus` is off globally, so no refetch
 * fires on the return itself.) The caller stops polling through
 * `enabled`, e.g. while the panel is collapsed. The previous quotes stay
 * on screen while a changed list loads.
 */
export function createQuotesQuery(
  session: McpSession,
  input: Accessor<{ symbols: string[]; enabled: boolean }>,
) {
  return createQuery<QuotesPayload, Error>(() => {
    const { symbols, enabled } = input();
    return {
      queryKey: ["get_quotes", ...[...symbols].sort()] as const,
      queryFn: () => session.callGetQuotes(symbols),
      enabled: enabled && symbols.length > 0,
      staleTime: QUOTES_POLL_MS - 1_000,
      refetchInterval: QUOTES_POLL_MS,
      placeholderData: keepPreviousData,
      retry: (_failureCount: number, error: Error) => {
        if (isAuthError(error)) clearAccessToken();
        // The next poll is the retry.
        return false;
      },
    };
  });
}
