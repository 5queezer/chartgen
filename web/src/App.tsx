// Top-level component. One screen: title, toolbar, status, chart.

import {
  createEffect,
  createMemo,
  createSignal,
  onCleanup,
  untrack,
  onMount,
  type JSX,
} from "solid-js";

import type { Chart as KLineChart } from "klinecharts";

import { Chart } from "./components/Chart";
import { DrawingToolbar } from "./components/DrawingToolbar";
import { IndicatorChips } from "./components/IndicatorChips";
import { IndicatorPicker } from "./components/IndicatorPicker";
import { IndicatorSettings } from "./components/IndicatorSettings";
import { mergeParams } from "./lib/paramForm";
import { StatusLine, type Status } from "./components/StatusLine";
import { Toolbar } from "./components/Toolbar";
import { Watchlist } from "./components/Watchlist";
import {
  applyChartAction,
  chartKeyAction,
  downloadChartImage,
  resetChart,
  screenshotFilename,
} from "./lib/chartActions";
import { isTypingContext } from "./lib/keys";
import { createMcpSession } from "./mcp";
import { tryHandlePendingRedirect } from "./oauth";
import { createChartQuery, createListIndicatorsQuery } from "./queries";
import { createAlertLogger } from "./services/notifications";
import {
  activeIndicators,
  indicatorName,
  indicatorParams,
  removeIndicator,
  toggleIndicatorHidden,
  toWire,
  updateIndicatorParams,
} from "./stores/indicators";
import {
  addRecent,
  chartType,
  symbol,
  timeframe,
} from "./stores/layout";

// Matches `--color-bg` / the chart's PALETTE.bg so exported PNGs are not transparent.
const SCREENSHOT_BACKGROUND = "#131722";

export function App(): JSX.Element {
  // Long-lived session for the app's lifetime.
  const session = createMcpSession();

  const [authReady, setAuthReady] = createSignal(false);
  const [pickerOpen, setPickerOpen] = createSignal(false);
  // Index (into the active set) of the indicator whose settings dialog is open.
  const [settingsIndex, setSettingsIndex] = createSignal<number | null>(null);
  const [chart, setChart] = createSignal<KLineChart | null>(null);

  // Chart keyboard navigation: never while typing or a dialog/menu is open.
  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      const c = chart();
      const action = chartKeyAction(e);
      if (!c || !action || isTypingContext(e)) return;
      e.preventDefault();
      applyChartAction(c, action);
    };
    window.addEventListener("keydown", onKey);
    onCleanup(() => window.removeEventListener("keydown", onKey));
  });

  const screenshot = () => {
    const c = chart();
    if (!c) return;
    downloadChartImage(
      c,
      screenshotFilename(symbol(), timeframe(), new Date()),
      SCREENSHOT_BACKGROUND,
    );
  };

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

  // Hidden indicators are client-only and never requested from the server.
  const wireIndicators = createMemo(() => toWire(activeIndicators()));

  const indexOf = (name: string) =>
    activeIndicators().findIndex((e) => indicatorName(e) === name);

  // Only re-evaluates when the dialog opens/closes, so edits elsewhere do
  // not reset a form in progress.
  const settingsTarget = createMemo(() => {
    const i = settingsIndex();
    const entry = i === null ? undefined : untrack(activeIndicators)[i];
    return entry === undefined
      ? null
      : { name: indicatorName(entry), params: indicatorParams(entry) };
  });

  const query = createChartQuery(session, () => ({
    ticker: symbol(),
    timeframe: timeframe(),
    indicators: wireIndicators(),
    enabled: authReady(),
  }));

  // A symbol only becomes a "recent" once its data actually loaded, so
  // typos never end up in the list.
  createEffect(() => {
    const loaded = query.isSuccess && !query.isPlaceholderData;
    const current = symbol();
    if (loaded) untrack(() => addRecent(current));
  });

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
        <Toolbar
          onReselect={() => void query.refetch()}
          onScreenshot={screenshot}
          onResetChart={() => {
            const c = chart();
            if (c) resetChart(c);
          }}
        />
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
        <StatusLine
          status={status()}
          onRetry={() => void query.refetch()}
        />
      </header>
      <IndicatorChips
        entries={activeIndicators()}
        onRemove={removeIndicator}
        onToggleHidden={toggleIndicatorHidden}
        onSettings={setSettingsIndex}
      />
      <IndicatorSettings
        target={settingsTarget()}
        specs={
          listIndicators.data?.indicators.find((i) => {
            const n = settingsTarget()?.name;
            return i.name === n || i.aliases?.includes(n ?? "");
          })?.configurable_params
        }
        onClose={() => setSettingsIndex(null)}
        onSubmit={(params, fields) => {
          const i = settingsIndex();
          const target = settingsTarget();
          if (i !== null && target) {
            updateIndicatorParams(
              i,
              mergeParams(fields, target.params, params),
            );
          }
          setSettingsIndex(null);
        }}
      />
      <IndicatorPicker
        open={pickerOpen()}
        onOpenChange={setPickerOpen}
        data={listIndicators.data ?? null}
        loading={listIndicators.isFetching}
        error={listIndicators.isError ? listIndicators.error : null}
        active={activeIndicators()}
      />
      <main class="flex flex-1 min-h-0">
        <DrawingToolbar chart={chart()} symbol={symbol()} />
        <div class="flex-1 min-w-0">
          <Chart
            series={query.data ?? null}
            activeIndicators={wireIndicators()}
            chartType={chartType()}
            onToggleHidden={(n) => toggleIndicatorHidden(indexOf(n))}
            onSettings={(n) => setSettingsIndex(indexOf(n))}
            onRemove={removeIndicator}
            onChartReady={setChart}
          />
        </div>
        <Watchlist session={session} enabled={authReady()} />
      </main>
    </div>
  );
}
