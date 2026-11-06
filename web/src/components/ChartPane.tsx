// One chart of the grid (ADR-0043): its query, live feed, bar replay and compare
// lines, and what renders them. `createChartPane` is the state, one per visible
// pane, so the header can read the active pane's; `ChartPane` is the cell. All
// of it reads this pane's stores by number, never the active chart's exports.

import {
  createEffect,
  createMemo,
  createSignal,
  on,
  onCleanup,
  Show,
  untrack,
  type Accessor,
  type JSX,
} from "solid-js";
import { Portal } from "solid-js/web";
import { useQueryClient } from "@tanstack/solid-query";
import type { Chart as KLineChart } from "klinecharts";

import { rememberLabels } from "../lib/indicatorNames";
import { alignAsOf, COMPARE_PALETTE } from "../lib/compare";
import { createCompareFeed, type CompareData } from "../lib/compareFeed";
import { barCloseMs, isContinuousMarket } from "../lib/countdown";
import {
  createLiveFeed,
  FAST_POLL_MS,
  SLOW_POLL_MS,
  type HistoryStatus,
  type LiveRequest,
  type LiveSnapshot,
  type PollStatus,
} from "../lib/liveFeed";
import { resolveTimeZone } from "../lib/timezone";
import type { McpSession } from "../mcp";
import { chartQueryKey, createChartQuery } from "../queries";
import { copyChartLink, importPending } from "../services/shareLink";
import { feedChartSnapshot } from "../services/alertEngine";
import { createReplay } from "../services/replay";
import { forgetReadings } from "../stores/alerts";
import { compareStoreOf } from "../stores/compare";
import { activePane, PaneProvider, setActivePane } from "../stores/grid";
import {
  indicatorStoreOf,
  indicatorStyle,
  instanceKey,
  toWire,
} from "../stores/indicators";
import { addRecent, chartStoreOf, chartStyleOf, timeZone } from "../stores/layout";
import type { SeriesPayload } from "../types/responses";
import { AlertLines } from "./AlertLines";
import { Chart } from "./Chart";
import { ChartContextMenu } from "./ChartContextMenu";
import { ChartLoadError } from "./ChartLoadError";
import { DrawingToolbar, type DrawingsApi } from "./DrawingToolbar";
import { ReplayBar } from "./ReplayBar";

export interface PaneDeps {
  session: McpSession;
  authReady: Accessor<boolean>;
}

export type ChartPaneState = ReturnType<typeof createChartPane>;

export function createChartPane(pane: number, deps: PaneDeps) {
  const { session, authReady } = deps;
  const layout = chartStoreOf(pane);
  const indicators = indicatorStoreOf(pane);
  const compare = compareStoreOf(pane);
  const { symbol, timeframe, chartType } = layout;
  const { activeIndicators } = indicators;
  const isActive = () => activePane() === pane;

  const [chart, setChart] = createSignal<KLineChart | null>(null);

  // Hidden indicators are client-only and never requested from the server.
  const wireIndicators = createMemo(() => toWire(activeIndicators()));

  // Plot styles are client-only (ADR-0017): they reach the chart separately
  // from the wire indicators, so a style change never refetches.
  const plotStyles = createMemo(() =>
    Object.fromEntries(
      activeIndicators().map((e) => [instanceKey(e), indicatorStyle(e)]),
    ),
  );

  const query = createChartQuery(session, () => ({
    ticker: symbol(),
    timeframe: timeframe(),
    indicators: wireIndicators(),
    chart_style: chartStyleOf(chartType()),
    // Indicators from a share link wait for the registry check below.
    enabled: authReady() && !importPending(),
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
  // True while exiting replay restarts the feed from its series: that reset
  // holds the close of the replay start, older than the prices the alerts
  // saw meanwhile (quotes poll), so it must not become their baseline.
  let resuming = false;
  // Whether a drawing tool is armed (Esc then cancels only the tool).
  const [drawingArmed, setDrawingArmed] = createSignal(false);
  const replay = createReplay({
    live,
    request: () => feedRequest,
    suspendLive: () => feed.suspend(),
    // Only when the feed still holds the current request's series: if a
    // new load is in flight, adopting it restarts the feed.
    resumeLive: () => {
      const s = untrack(live)?.series;
      if (s && feedRequest && JSON.stringify(chartQueryKey(feedRequest)) === feedKey) {
        resuming = true;
        feed.reset(s, feedRequest);
        resuming = false;
        // Indicator readings from before replay are no baseline for the
        // live values that follow (ADR-0034).
        forgetReadings(feedTicker);
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
      // frames never pass here: while replaying the feed only pages. Only the
      // active chart feeds them: the others' symbols are on the quotes poll.
      if (snapshot && !resuming && untrack(isActive)) feedChartSnapshot(feedTicker, snapshot);
    },
    onPollStatus: setPollStatus,
    onHistoryStatus: setHistoryStatus,
    // Replay (ADR-0023) reads as a hidden tab: no polls, no pushes.
    doc: replay.liveDoc,
  });
  onCleanup(feed.dispose);

  // Becoming the active chart makes this the chart the alerts read (ADR-0043).
  createEffect(
    on(
      isActive,
      (active) => {
        const s = untrack(live)?.series;
        if (active && s && untrack(replay.mode) === "off") {
          feedChartSnapshot(feedTicker, { series: s, update: { version: 0, kind: "reset" } });
        }
      },
      { defer: true },
    ),
  );

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
    if (untrack(compare.compareSymbols).includes(main)) compare.removeCompare(main);
  });
  const comparing = createMemo(() =>
    compare.compareEntries().filter((e) => e.symbol !== symbol()),
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

  // A symbol only becomes a "recent" once its data actually loaded, so
  // typos never end up in the list. The last one that loaded is also what
  // a failed load offers to go back to.
  const [lastLoaded, setLastLoaded] = createSignal<string | null>(null);
  createEffect(() => {
    const loaded = query.isSuccess && !query.isPlaceholderData;
    const current = symbol();
    if (loaded) {
      setLastLoaded(current);
      untrack(() => addRecent(current));
    }
  });

  // Chips and picker show the legend's names (EMA, BB), not registry ids.
  createEffect(() => rememberLabels(live()?.series.indicators));

  return {
    pane,
    layout,
    indicators,
    compare,
    isActive,
    chart,
    setChart,
    wireIndicators,
    plotStyles,
    query,
    live,
    shown,
    pollStatus,
    historyStatus,
    replay,
    feed,
    drawingArmed,
    setDrawingArmed,
    shownCompare,
    crosshair,
    setCrosshair,
    compareFrom,
    setCompareFrom,
    drawings,
    setDrawings,
    lastLoaded,
    timeZone: () => resolveTimeZone(timeZone(), symbol()),
  };
}

export interface ChartPaneProps {
  state: ChartPaneState;
  /** Where the drawing toolbars go: one rail, showing the active chart's. */
  rail: HTMLElement | undefined;
  /** More than one chart shows, so each says what it is. */
  multi: boolean;
  /** Open the settings of an indicator of the active chart. */
  onIndicatorSettings: (key: string) => void;
  onAddAlert: (key: string) => void;
  onResetView: () => void;
  onSettings: () => void;
}

export function ChartPane(props: ChartPaneProps): JSX.Element {
  const s = props.state;
  const { symbol, timeframe, chartType, setSymbol } = s.layout;
  const { activeIndicators, removeIndicator, setActiveIndicators, toggleIndicatorHidden } = s.indicators;
  const indexOf = (key: string) =>
    activeIndicators().findIndex((e) => instanceKey(e) === key);

  return (
    <PaneProvider value={s.pane}>
      <div
        class="chart-pane"
        data-testid="chart-pane"
        data-pane={s.pane}
        data-active={s.isActive() ? "" : undefined}
        // Capture: before the chart acts on the press, so the shortcuts, menus
        // and dialogs that press leads to already read this chart.
        ref={(el) => el.addEventListener("pointerdown", () => setActivePane(s.pane), true)}
      >
        <Portal mount={props.rail} ref={(el: HTMLDivElement) => (el.style.display = "contents")}>
          <div style={{ display: s.isActive() ? "contents" : "none" }}>
            <DrawingToolbar
              chart={s.chart()}
              symbol={symbol()}
              onApi={s.setDrawings}
              onArmedChange={s.setDrawingArmed}
              disarm={s.replay.mode() === "select" || !s.isActive()}
              active={s.isActive()}
            />
          </div>
        </Portal>
        <AlertLines chart={s.chart()} symbol={symbol()} />
        <ChartContextMenu
          chart={s.chart()}
          symbol={symbol()}
          drawings={s.drawings()}
          hasIndicators={activeIndicators().length > 0}
          onResetView={props.onResetView}
          onCopyLink={() => void copyChartLink()}
          onRemoveAllIndicators={() => setActiveIndicators([])}
          onSettings={props.onSettings}
        >
          <Show when={s.query.isError && !s.query.isFetching}>
            <ChartLoadError
              symbol={symbol()}
              message={
                s.query.error instanceof Error
                  ? s.query.error.message
                  : String(s.query.error)
              }
              previousSymbol={s.lastLoaded() === symbol() ? null : s.lastLoaded()}
              onRetry={() => void s.query.refetch()}
              onBack={setSymbol}
            />
          </Show>
          <Chart
            series={s.shown()?.series ?? null}
            update={s.shown()?.update ?? null}
            history={s.historyStatus()}
            onLoadOlder={s.feed.loadOlder}
            activeIndicators={s.wireIndicators()}
            styles={s.plotStyles()}
            chartType={chartType()}
            timeZone={s.timeZone()}
            onToggleHidden={(n) => toggleIndicatorHidden(indexOf(n))}
            onSettings={props.onIndicatorSettings}
            onRemove={removeIndicator}
            onAddAlert={props.onAddAlert}
            onChartReady={s.setChart}
            onCrosshair={s.setCrosshair}
            onCompareFrom={s.setCompareFrom}
            compare={s.shownCompare()}
            onRemoveCompare={s.compare.removeCompare}
          />
          {/* Bar replay controls and start picker (ADR-0023). */}
          <ReplayBar
            replay={s.replay}
            chart={s.chart()}
            drawingArmed={s.drawingArmed()}
            timeZone={s.timeZone()}
            active={s.isActive()}
          />
        </ChartContextMenu>
        <Show when={props.multi}>
          <div class="pane-tag" data-testid="pane-tag">
            {symbol()} · {timeframe()}
          </div>
        </Show>
      </div>
    </PaneProvider>
  );
}
