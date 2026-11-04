// Top-level component. One screen: title, toolbar, status, chart.

import {
  createMemo,
  createSignal,
  onCleanup,
  onMount,
  type JSX,
} from "solid-js";

import { Chart } from "./components/Chart";
import { IndicatorChips } from "./components/IndicatorChips";
import { IndicatorPicker } from "./components/IndicatorPicker";
import { StatusLine, type Status } from "./components/StatusLine";
import { Toolbar } from "./components/Toolbar";
import { createMcpSession } from "./mcp";
import { tryHandlePendingRedirect } from "./oauth";
import { createChartQuery, createListIndicatorsQuery } from "./queries";
import { createAlertLogger } from "./services/notifications";
import {
  activeIndicators,
  removeIndicator,
} from "./stores/indicators";
import { chartType, symbol, timeframe } from "./stores/layout";

export function App(): JSX.Element {
  // Long-lived session for the app's lifetime.
  const session = createMcpSession();

  const [authReady, setAuthReady] = createSignal(false);
  const [pickerOpen, setPickerOpen] = createSignal(false);

  // Consume the OAuth redirect (if any) before the first render gets
  // far enough to trigger a query. This is best-effort — if it fails,
  // the first query will surface the error.
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
    ticker: symbol(),
    timeframe: timeframe(),
    indicators: activeIndicators(),
    enabled: authReady(),
  }));

  // The indicator registry only changes between deploys, so this query
  // is cached forever. It fires as soon as auth is ready.
  const listIndicators = createListIndicatorsQuery(session, authReady);

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

  return (
    <div class="flex flex-col h-screen w-screen">
      <header class="flex items-center gap-4 px-4 py-3 border-b border-[color:var(--color-border)]">
        <h1 class="text-lg font-semibold mr-4 leading-none">chartgen</h1>
        <Toolbar />
        <button
          type="button"
          onClick={() => setPickerOpen(true)}
          class="bg-[color:var(--color-bg-elev)] border border-[color:var(--color-border)] text-[color:var(--color-fg)] px-3 py-1.5 rounded-md text-sm hover:border-[color:var(--color-accent)] cursor-pointer inline-flex items-center gap-2"
        >
          Indicators
          <span class="inline-flex items-center justify-center min-w-[1.25rem] h-5 px-1 rounded-full bg-[color:var(--color-accent)] text-black text-xs font-semibold">
            {activeIndicators().length}
          </span>
        </button>
        <StatusLine status={status()} />
      </header>
      <IndicatorChips
        entries={activeIndicators()}
        onRemove={removeIndicator}
      />
      <IndicatorPicker
        open={pickerOpen()}
        onOpenChange={setPickerOpen}
        data={listIndicators.data ?? null}
        loading={listIndicators.isFetching}
        error={listIndicators.isError ? listIndicators.error : null}
        active={activeIndicators()}
      />
      <main class="flex-1 min-h-0">
        <Chart
          series={query.data ?? null}
          activeIndicators={activeIndicators()}
          chartType={chartType()}
        />
      </main>
    </div>
  );
}
