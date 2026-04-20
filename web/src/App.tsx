// Top-level component. One screen: title, form, status, chart.

import {
  createMemo,
  createSignal,
  onCleanup,
  onMount,
  type JSX,
} from "solid-js";

import { Chart } from "./components/Chart";
import { StatusLine, type Status } from "./components/StatusLine";
import { SymbolForm, type Timeframe } from "./components/SymbolForm";
import { createMcpSession } from "./mcp";
import { tryHandlePendingRedirect } from "./oauth";
import { createChartQuery } from "./queries";
import { createAlertLogger } from "./services/notifications";

export function App(): JSX.Element {
  // Long-lived session for the app's lifetime.
  const session = createMcpSession();

  const [ticker, setTicker] = createSignal("BTCUSDT");
  const [timeframe, setTimeframe] = createSignal<Timeframe>("1h");
  // The query only runs once the user clicks Load — a trigger signal
  // gates `enabled`. Clicking Load bumps a counter so the query also
  // refetches if the user re-submits with identical inputs.
  const [triggered, setTriggered] = createSignal(false);
  const [authReady, setAuthReady] = createSignal(false);

  // Consume the OAuth redirect (if any) before the first render gets
  // far enough to trigger a query. This is best-effort — if it fails,
  // the first Load click will surface the error via the query path.
  onMount(async () => {
    await tryHandlePendingRedirect();
    setAuthReady(true);

    const logger = createAlertLogger(session);
    logger.install();
    onCleanup(() => {
      logger.uninstall();
      void session.close();
    });
  });

  const query = createChartQuery(session, () => ({
    ticker: ticker(),
    timeframe: timeframe(),
    enabled: triggered() && authReady(),
  }));

  const status = createMemo<Status>(() => {
    if (!authReady()) return { kind: "authenticating" };
    if (query.isFetching) return { kind: "loading" };
    if (query.isError) {
      const err = query.error;
      return {
        kind: "error",
        message: err instanceof Error ? err.message : String(err),
      };
    }
    if (query.data) return { kind: "loaded" };
    return { kind: "ready" };
  });

  const onSubmit = () => {
    setTriggered(true);
    // Force a refetch if the key hasn't changed.
    void query.refetch();
  };

  return (
    <div class="flex flex-col h-screen w-screen">
      <header class="flex items-center gap-4 px-4 py-3 border-b border-[color:var(--color-border)]">
        <h1 class="text-lg font-semibold mr-4">chartgen</h1>
        <SymbolForm
          ticker={ticker()}
          timeframe={timeframe()}
          onTickerChange={setTicker}
          onTimeframeChange={setTimeframe}
          onSubmit={onSubmit}
          disabled={!authReady() || query.isFetching}
        />
        <StatusLine status={status()} />
      </header>
      <main class="flex-1 min-h-0">
        <Chart series={query.data ?? null} />
      </main>
    </div>
  );
}
