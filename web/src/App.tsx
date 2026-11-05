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
import { useQueryClient } from "@tanstack/solid-query";

import type { Chart as KLineChart } from "klinecharts";

import { BottomBar } from "./components/BottomBar";
import { AlertDialog, AlertButton } from "./components/AlertDialog";
import { AlertLines } from "./components/AlertLines";
import { Chart, paletteColor } from "./components/Chart";
import { DrawingToolbar, type DrawingsApi } from "./components/DrawingToolbar";
import { IndicatorChips } from "./components/IndicatorChips";
import { IndicatorPicker } from "./components/IndicatorPicker";
import { IndicatorSettings } from "./components/IndicatorSettings";
import { LayoutMenu } from "./components/LayoutMenu";
import { ReplayBar } from "./components/ReplayBar";
import { mergeParams } from "./lib/paramForm";
import { PALETTES } from "./lib/theme";
import { resolveTimeZone } from "./lib/timezone";
import { plotsOf } from "./lib/plotStyle";
import { StatusLine, type Status } from "./components/StatusLine";
import { SymbolSearch } from "./components/SymbolSearch";
import { Toasts } from "./components/Toasts";
import { Toolbar } from "./components/Toolbar";
import { Sidebar } from "./components/Sidebar";
import {
  applyChartAction,
  chartKeyAction,
  downloadChartImage,
  resetChart,
  screenshotFilename,
} from "./lib/chartActions";
import { isTypingContext, keepKeyboardFocus } from "./lib/keys";
import { alignAsOf, COMPARE_PALETTE } from "./lib/compare";
import { createCompareFeed, type CompareData } from "./lib/compareFeed";
import { barCloseMs, isContinuousMarket } from "./lib/countdown";
import { createMcpSession } from "./mcp";
import { startSignIn, tryHandlePendingRedirect } from "./oauth";
import {
  chartQueryKey,
  createChartQuery,
  createListIndicatorsQuery,
} from "./queries";
import {
  createLiveFeed,
  FAST_POLL_MS,
  SLOW_POLL_MS,
  type HistoryStatus,
  type LiveRequest,
  type LiveSnapshot,
  type PollStatus,
} from "./lib/liveFeed";
import type { SeriesPayload } from "./types/responses";
import { createAlertLogger } from "./services/notifications";
import { createReplay } from "./services/replay";
import {
  createAlertQuotes,
  feedChartSnapshot,
  feedPrice,
} from "./services/alertEngine";
import {
  activeIndicators,
  indicatorName,
  indicatorParams,
  indicatorStyle,
  isHidden,
  removeIndicator,
  toggleIndicatorHidden,
  toWire,
  updateIndicatorSettings,
} from "./stores/indicators";
import {
  addCompare,
  compareEntries,
  compareSymbols,
  removeCompare,
} from "./stores/compare";
import { setParamDefaults, type ParamDefaults } from "./stores/layouts";
import {
  addRecent,
  chartStyleOf,
  chartType,
  theme,
  symbol,
  timeframe,
  timeZone,
} from "./stores/layout";

export function App(): JSX.Element {
  // Long-lived session for the app's lifetime.
  const session = createMcpSession();

  const [authReady, setAuthReady] = createSignal(false);
  const [authError, setAuthError] = createSignal<string | null>(null);
  const [pickerOpen, setPickerOpen] = createSignal(false);
  // Index (into the active set) of the indicator whose settings dialog is open.
  const [settingsIndex, setSettingsIndex] = createSignal<number | null>(null);
  const [chart, setChart] = createSignal<KLineChart | null>(null);

  onMount(() => onCleanup(keepKeyboardFocus()));

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
      PALETTES[theme()].bg,
    );
  };

  // Consume the OAuth redirect (if any) before the first render gets
  // far enough to trigger a query. If sign-in failed even after its one
  // automatic retry, queries stay off (a query would only restart the
  // flow, in a loop) and the status line offers "Sign in again".
  onMount(async () => {
    const err = await tryHandlePendingRedirect();
    if (err) {
      setAuthError(err.message);
      return;
    }
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
      : {
          name: indicatorName(entry),
          params: indicatorParams(entry),
          style: indicatorStyle(entry),
          hidden: isHidden(entry),
        };
  });

  // Plot styles are client-only (ADR-0017): they reach the chart separately
  // from the wire indicators, so a style change never refetches.
  const plotStyles = createMemo(() =>
    Object.fromEntries(
      activeIndicators().map((e) => [indicatorName(e), indicatorStyle(e)]),
    ),
  );

  const query = createChartQuery(session, () => ({
    ticker: symbol(),
    timeframe: timeframe(),
    indicators: wireIndicators(),
    chart_style: chartStyleOf(chartType()),
    enabled: authReady(),
  }));

  // The live feed owns the chart's data from each full load on: it polls
  // the tail and pages in history, and the chart renders its snapshots.
  const [live, setLive] = createSignal<LiveSnapshot | null>(null);
  const [pollStatus, setPollStatus] = createSignal<PollStatus>({ kind: "off" });
  const [historyStatus, setHistoryStatus] = createSignal<HistoryStatus>({
    kind: "idle",
  });
  let feedTicker = "";

  // --- Bar replay (ADR-0023) ------------------------------------------
  // The request of the series the feed holds (set where it is adopted).
  let feedRequest: LiveRequest | null = null;
  const replay = createReplay({
    live,
    request: () => feedRequest,
    suspendLive: () => feed.suspend(),
    // Only when the feed still holds the current request's series: if a
    // new load is in flight, adopting it restarts the feed.
    resumeLive: () => {
      const s = untrack(live)?.series;
      if (s && feedRequest && JSON.stringify(chartQueryKey(feedRequest)) === feedKey) {
        feed.reset(s, feedRequest);
      }
    },
    loadUntil: (ts, isCurrent) => feed.loadUntil(ts, isCurrent),
    fetch: (request, { signal, ...args }) =>
      session.callGenerateChart({ ...request, ...args }, signal),
  });
  // What the chart shows: the live snapshot, or the replay frame.
  const shown = replay.snapshot;
  // ---------------------------------------------------------------------

  const feed = createLiveFeed({
    fetch: (request, { signal, ...args }) =>
      session.callGenerateChart({ ...request, ...args }, signal),
    reload: () => query.refetch(),
    // Server push over the SSE stream (ADR-0021); polling is the fallback.
    push: {
      subscribe: async (request, warmup) =>
        (await session.callSubscribeChart({ ...request, warmup })).subscription_id,
      unsubscribe: (subscription_id) => session.callUnsubscribeChart({ subscription_id }),
    },
    onSnapshot: (snapshot) => {
      setLive(snapshot);
      // Price alerts (ADR-0020) on the chart symbol's last close. Replay
      // frames never pass here: while replaying the feed only pages.
      if (snapshot) feedChartSnapshot(feedTicker, snapshot);
    },
    onPollStatus: setPollStatus,
    onHistoryStatus: setHistoryStatus,
    // Replay (ADR-0023) reads as a hidden tab: no polls, no pushes.
    doc: replay.liveDoc,
  });
  session.onChartTick(feed.applyPush);
  onCleanup(feed.dispose);

  // Hand each full load to the feed. A request change (symbol, timeframe,
  // visible indicators, params) suspends the feed at once, which drops
  // every in-flight poll and page of the old chart; the old chart stays on
  // screen until the new load lands (keepPreviousData), and is cleared if
  // that load fails.
  //
  // What gets adopted is read from the query cache entry of exactly the
  // current key, not from `query.data`: that can still be the previous
  // key's result (placeholder, or published a tick after the key changed).
  // Each successful fetch stamps the entry with a new dataUpdatedAt, so a
  // refetch of the same key (Retry, re-pick, the feed's own reload) is
  // adopted even when the payload is equal, and switching back to a cached
  // key adopts that key's own data.
  const queryClient = useQueryClient();
  let feedKey = "";
  // dataUpdatedAt of the adopted entry; null: none yet, -1: cleared.
  let adoptedAt: number | null = null;
  createEffect(() => {
    const request: LiveRequest = {
      ticker: symbol(),
      timeframe: timeframe(),
      indicators: wireIndicators(),
      chart_style: chartStyleOf(chartType()),
    };
    // Re-run whenever the query publishes a result.
    void query.data;
    void query.dataUpdatedAt;
    void query.status;
    untrack(() => {
      const key = chartQueryKey(request);
      const json = JSON.stringify(key);
      if (json !== feedKey) {
        feedKey = json;
        adoptedAt = null;
        feed.suspend();
      }
      const state = queryClient.getQueryState<SeriesPayload, Error>(key);
      if (state?.data !== undefined) {
        if (state.dataUpdatedAt === adoptedAt) return;
        adoptedAt = state.dataUpdatedAt;
        // A copy: the feed updates its series in place, the cache stays as loaded.
        feedTicker = request.ticker;
        feedRequest = request;
        feed.reset(structuredClone(state.data), request);
      } else if (state?.status === "error" && adoptedAt !== -1) {
        adoptedAt = -1;
        feed.reset(null, request);
      }
    });
  });

  // --- Compare symbols (ADR-0022) -------------------------------------
  const [compareOpen, setCompareOpen] = createSignal(false);
  const [compareData, setCompareData] = createSignal(
    new Map<string, CompareData>(),
  );
  const compareFeed = createCompareFeed({
    fetch: (request, { signal, ...args }) =>
      session.callGenerateChart(
        { ...request, indicators: [], chart_style: "candles", ...args },
        signal,
      ),
    onChange: setCompareData,
    pollMs: (s) => (isContinuousMarket(s) ? FAST_POLL_MS : SLOW_POLL_MS),
    // No compare tail polls while replaying (ADR-0023).
    doc: replay.liveDoc,
  });
  onCleanup(compareFeed.dispose);
  // The main symbol is never one of its own compare symbols: pick it as the
  // main symbol and it leaves the list.
  createEffect(() => {
    const main = symbol();
    if (untrack(compareSymbols).includes(main)) removeCompare(main);
  });
  const comparing = createMemo(() =>
    compareEntries().filter((e) => e.symbol !== symbol()),
  );
  // Sized to the main series once it matches the timeframe (a kept previous
  // chart of another timeframe must not size the loads).
  createEffect(() => {
    const s = live()?.series;
    const ready = authReady() && s?.interval === timeframe();
    compareFeed.sync({
      symbols: authReady() ? comparing().map((e) => e.symbol) : [],
      timeframe: timeframe(),
      mainCount: ready ? s.bars.length : 0,
      mainFirst: ready ? (s.bars[0]?.t ?? null) : null,
    });
  });
  const compareLines = createMemo(() => {
    const series = live()?.series;
    const times = series?.bars.map((b) => b.t) ?? [];
    // Join on the main bar's end (see alignAsOf); 1 s when the interval is unknown.
    const last = times.at(-1);
    const close =
      last != null && series?.interval
        ? barCloseMs(last, series.interval)
        : null;
    const step = last != null && close !== null ? close / 1000 - last : 1;
    return comparing().map(({ symbol: sym, slot }) => {
      const data = compareData().get(sym);
      return {
        symbol: sym,
        color: COMPARE_PALETTE[slot]!,
        values: data ? alignAsOf(times, data.bars, step) : times.map(() => null),
      };
    });
  });
  // ---------------------------------------------------------------------
  // Sidebar tabs: Data Window (crosshair bar) and Object Tree (drawings).
  const [crosshair, setCrosshair] = createSignal<number | null>(null);
  const [compareFrom, setCompareFrom] = createSignal(0);
  const [drawings, setDrawings] = createSignal<DrawingsApi | null>(null);
  // ---------------------------------------------------------------------

  // --- Bar replay (ADR-0023) ------------------------------------------
  // Compare lines as of the cursor: their values align with the full series,
  // whose prefix the frame is.
  const shownCompare = createMemo(() => {
    const lines = compareLines();
    if (replay.mode() !== "active") return lines;
    const n = shown()?.series.bars.length ?? 0;
    return lines.map((l) => ({ ...l, values: l.values.slice(0, n) }));
  });
  // ---------------------------------------------------------------------

  // Other symbols with active alerts are evaluated from their own quotes poll.
  // While replaying the chart symbol is polled there too (ADR-0023): the
  // feed is suspended, and its alerts still need real prices.
  createAlertQuotes(session, authReady, () => (replay.mode() === "off" ? symbol() : ""));

  if (import.meta.env.DEV) {
    // E2E hook (tests/e2e/live-paging.spec.ts): what the chart holds.
    (window as unknown as Record<string, unknown>).__chartgenTest = {
      chart,
      series: () => live()?.series ?? null,
      // Alert e2e (tests/e2e/alerts.spec.ts): inject a live price.
      feedPrice,
      // Bar replay e2e (tests/e2e/bar-replay.spec.ts): the frame on screen
      // and the replay state.
      shownSeries: () => shown()?.series ?? null,
      replay: () => ({
        mode: replay.mode(),
        playing: replay.playing(),
        cursorTime: replay.cursorTime(),
      }),
    };
  }

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

  // Layout dirty check ignores params that equal the registry default.
  createEffect(() => {
    const defaults: ParamDefaults = {};
    for (const i of listIndicators.data?.indicators ?? []) {
      const params = Object.fromEntries(
        (i.configurable_params ?? []).map((p) => [p.name, p.default]),
      );
      for (const n of [i.name, ...(i.aliases ?? [])]) defaults[n] = params;
    }
    setParamDefaults(defaults);
  });

  // Plots of the open dialog's indicator, from the last series payload.
  const settingsPlots = createMemo(() => {
    const name = settingsTarget()?.name;
    const ind = name === undefined ? undefined : query.data?.indicators[name];
    return name === undefined || !ind
      ? null
      : plotsOf(ind, (n) => paletteColor(name, n));
  });

  const status = createMemo<Status>(() => {
    const authFailed = authError();
    if (authFailed !== null) return { kind: "auth-error", message: authFailed };
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
          session={session}
          onReselect={() => void query.refetch()}
          onScreenshot={screenshot}
          onResetChart={() => {
            const c = chart();
            if (c) resetChart(c);
          }}
          onCompare={() => setCompareOpen(true)}
          replay={replay.mode() !== "off"}
          onReplay={() => (replay.mode() === "off" ? replay.enter() : replay.exit())}
        />
        <SymbolSearch
          open={compareOpen()}
          onOpenChange={setCompareOpen}
          initialQuery=""
          onSelect={(ticker) => addCompare(ticker, symbol())}
          session={session}
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
        <AlertButton />
        <StatusLine
          status={status()}
          live={replay.mode() === "off" ? pollStatus() : { kind: "off" }}
          onRetry={() => void query.refetch()}
          onSignIn={() =>
            void startSignIn().catch((e: unknown) =>
              setAuthError(e instanceof Error ? e.message : String(e)),
            )
          }
        />
        <div class="ml-auto">
          <LayoutMenu />
        </div>
      </header>
      <IndicatorChips
        entries={activeIndicators()}
        onRemove={removeIndicator}
        onToggleHidden={toggleIndicatorHidden}
        onSettings={setSettingsIndex}
      />
      <IndicatorSettings
        target={settingsTarget()}
        plots={settingsPlots()}
        specs={
          listIndicators.data?.indicators.find((i) => {
            const n = settingsTarget()?.name;
            return i.name === n || i.aliases?.includes(n ?? "");
          })?.configurable_params
        }
        onClose={() => setSettingsIndex(null)}
        onSubmit={(params, fields, style) => {
          const i = settingsIndex();
          const target = settingsTarget();
          if (i !== null && target) {
            // A style-only OK must not rewrite the params: dropping a
            // value equal to its default would change the wire entry and
            // refetch.
            const paramsChanged = fields.some(
              (f) =>
                (target.params[f.name] ?? f.defaultValue ?? undefined) !==
                (params[f.name] ?? undefined),
            );
            updateIndicatorSettings(
              i,
              paramsChanged
                ? mergeParams(fields, target.params, params)
                : undefined,
              style,
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
        <DrawingToolbar chart={chart()} symbol={symbol()} onApi={setDrawings} />
        <AlertLines chart={chart()} symbol={symbol()} />
        <div class="flex-1 min-w-0 relative">
          <Chart
            series={shown()?.series ?? null}
            update={shown()?.update ?? null}
            history={historyStatus()}
            onLoadOlder={feed.loadOlder}
            activeIndicators={wireIndicators()}
            styles={plotStyles()}
            chartType={chartType()}
            timeZone={resolveTimeZone(timeZone(), symbol())}
            onToggleHidden={(n) => toggleIndicatorHidden(indexOf(n))}
            onSettings={(n) => setSettingsIndex(indexOf(n))}
            onRemove={removeIndicator}
            onChartReady={setChart}
            onCrosshair={setCrosshair}
            onCompareFrom={setCompareFrom}
            compare={shownCompare()}
            onRemoveCompare={removeCompare}
          />
          {/* Bar replay controls and start picker (ADR-0023). */}
          <ReplayBar
            replay={replay}
            chart={chart()}
            timeZone={resolveTimeZone(timeZone(), symbol())}
          />
        </div>
        <Sidebar
          session={session}
          enabled={authReady()}
          panels={{
            // The frame on screen: in replay, never a bar after the cursor.
            series: shown()?.series ?? null,
            crosshair: crosshair(),
            styles: plotStyles(),
            compare: shownCompare(),
            compareFrom: compareFrom(),
            timeZone: resolveTimeZone(timeZone(), symbol()),
            drawings: drawings(),
            onSettings: (n) => setSettingsIndex(indexOf(n)),
          }}
        />
      </main>
      <BottomBar chart={chart()} series={shown()?.series ?? null} loadUntil={feed.loadUntil} />
      <AlertDialog />
      <Toasts />
    </div>
  );
}
