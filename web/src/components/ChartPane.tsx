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

import { isOrderFlowType } from "../lib/chartTypes";
import { resolveColors } from "../lib/chartStyle";
import { rememberLabels } from "../lib/indicatorNames";
import {
  CELL_MIN_BAR,
  FOOTPRINT_MAX_TAIL,
  footprintMethod,
  footprintTail,
  mergeFootprintMeta,
  mergeTpoMeta,
  TPO_MAX_TAIL,
  tpoMethod,
  tpoTail,
  type CellMode,
} from "../lib/orderFlow";
import type { FootprintExtend, TpoExtend } from "../lib/orderFlowDraw";
import { createFlowFeed, type FlowState } from "../lib/orderFlowFeed";
import { PALETTES } from "../lib/theme";
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
import { isDateBar, resolveTimeZone } from "../lib/timezone";
import type { McpSession } from "../mcp";
import { chartQueryKey, createChartQuery } from "../queries";
import { copyChartLink, importPending } from "../services/shareLink";
import { feedChartSnapshot } from "../services/alertEngine";
import { createReplay } from "../services/replay";
import { forgetReadings, setChartContext } from "../stores/alerts";
import { compareStoreOf } from "../stores/compare";
import { activePane, PaneProvider, setActivePane } from "../stores/grid";
import {
  indicatorName,
  indicatorStoreOf,
  indicatorStyle,
  instanceKey,
  toWire,
} from "../stores/indicators";
import { addRecent, chartStoreOf, chartStyleOf, theme, timeZone } from "../stores/layout";
import { chartSettings } from "../stores/chartSettings";
import { orderFlowSettings, tpoSessions } from "../stores/orderFlow";
import type { GetTpoInput } from "../types/generated-input";
import type {
  FootprintBar,
  FootprintMeta,
  SeriesPayload,
  TpoMeta,
  TpoProfile,
} from "../types/responses";
import { AlertLines } from "./AlertLines";
import { Chart } from "./Chart";
import { ChartContextMenu } from "./ChartContextMenu";
import { ChartLoadError } from "./ChartLoadError";
import { DrawingToolbar, type DrawingsApi } from "./DrawingToolbar";
import { OrderFlowSettingsDialog } from "./OrderFlowSettings";
import { ReplayBar } from "./ReplayBar";

/** Slow on purpose: each poll reads finer bars on the server (ADR-0048). */
const FOOTPRINT_POLL_MS = 10_000;
const TPO_POLL_MS = 30_000;

const EMPTY_FLOW = { items: new Map(), meta: null, status: "loading", message: null } as const;

const decimalsOf = (tick: number | undefined) =>
  tick ? Math.min(8, Math.max(0, Math.ceil(-Math.log10(tick)))) : 2;

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

  // The cache entry is shared with every chart on the same request (ADR-0043),
  // so a new result only replaces this chart's series if this chart asked for it.
  // Otherwise a refetch elsewhere would drop its paged history and its scroll.
  let refetchAsked = false;
  const refetch = (options?: { cancelRefetch?: boolean }) => {
    refetchAsked = true;
    return query.refetch(options);
  };

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
    // Joins a fetch in flight: charts on one request reloading together, after a
    // sleep, would otherwise cancel each other's.
    reload: () => refetch({ cancelRefetch: false }),
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
        if (!active) return;
        const s = untrack(live)?.series;
        if (s && untrack(replay.mode) === "off") {
          feedChartSnapshot(feedTicker, { series: s, update: { version: 0, kind: "reset" } });
        } else {
          // Nothing evaluates here yet: not the previous chart's alerts either.
          setChartContext(null);
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
  // Each successful fetch stamps the entry with a new dataUpdatedAt, so this
  // chart's own refetch of the same key (Retry, re-pick, the feed's own reload)
  // is adopted even when the payload is equal, and switching back to a cached
  // key adopts that key's own data.
  const queryClient = useQueryClient();
  let feedKey = "";
  // dataUpdatedAt of the adopted entry; null: none yet, -1: cleared.
  let adoptedAt: number | null = null;
  // A script changed (`refreshChartsUsing`): the entry is out of date for every chart
  // on it, not just one, so each takes the result. In the event, not when the result
  // lands, as the entry is no longer marked invalid by then.
  onCleanup(
    queryClient.getQueryCache().subscribe((event) => {
      if (
        event.type === "updated" &&
        event.action.type === "invalidate" &&
        JSON.stringify(event.query.queryKey) === feedKey
      ) {
        refetchAsked = true;
      }
    }),
  );
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
        refetchAsked = false;
        feed.suspend();
      }
      const state = queryClient.getQueryState<SeriesPayload, Error>(key);
      if (state?.status === "error") refetchAsked = false;
      if (state?.data !== undefined) {
        if (state.dataUpdatedAt === adoptedAt) return;
        if (adoptedAt !== null && adoptedAt !== -1 && !refetchAsked) return;
        // Back on a key invalidated while away: this data is old, its refetch is ours.
        refetchAsked = state.isInvalidated;
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

  // --- Order-flow chart types (ADR-0048) --------------------------------
  const flowKind = createMemo(() => {
    const t = chartType();
    return isOrderFlowType(t) ? t : null;
  });
  const [flowMode, setFlowMode] = createSignal<CellMode>("cells");
  const [footprints, setFootprints] = createSignal<FlowState<FootprintBar, FootprintMeta>>(EMPTY_FLOW);
  const [profiles, setProfiles] = createSignal<FlowState<TpoProfile, TpoMeta>>(EMPTY_FLOW);
  const footprintFeed = createFlowFeed<FootprintBar, FootprintMeta>({
    pollMs: FOOTPRINT_POLL_MS,
    doc: replay.liveDoc,
    onChange: setFootprints,
    mergeMeta: mergeFootprintMeta,
    tailSize: (gap) => footprintTail(gap, untrack(timeframe)),
    maxTail: FOOTPRINT_MAX_TAIL,
    fetch: async ({ tail, endTime, meta: load }, signal) => {
      const f = untrack(orderFlowSettings).footprint;
      const { footprints: items, has_more, ...meta } = await session.callGetFootprint(
        {
          ticker: untrack(symbol),
          timeframe: untrack(timeframe),
          bars: tail ?? 60,
          ...(endTime !== undefined && { end_time: endTime }),
          ...(load !== undefined
            ? { row_size: load.row_size }
            : f.rowTicks > 0 && { row_ticks: f.rowTicks }),
          // A poll reads what its load read: its few bars could afford finer ones.
          ...(tail !== undefined &&
            load && { intrabar_interval: load.intrabar_interval ?? "none" }),
          atr_length: f.atrLength,
          value_area: f.valueArea,
          imbalance: f.imbalance,
          stacked: f.stacked,
        },
        signal,
      );
      return { items, meta, hasMore: has_more };
    },
  });
  const tpoFeed = createFlowFeed<TpoProfile, TpoMeta>({
    pollMs: TPO_POLL_MS,
    doc: replay.liveDoc,
    onChange: setProfiles,
    mergeMeta: mergeTpoMeta,
    tailSize: (gap) => tpoTail(gap, untrack(orderFlowSettings).tpo.period),
    maxTail: TPO_MAX_TAIL,
    fetch: async ({ tail, endTime, meta: load }, signal) => {
      const t = untrack(orderFlowSettings).tpo;
      const sessions = tpoSessions(t.sessions);
      const { profiles: items, has_more, ...meta } = await session.callGetTpo(
        {
          ticker: untrack(symbol),
          period: t.period,
          block: t.block,
          periods: tail ?? 10,
          ...(endTime !== undefined && { end_time: endTime }),
          ...(load !== undefined
            ? { row_size: load.row_size }
            : t.rowTicks > 0 && { row_ticks: t.rowTicks }),
          value_area: t.valueArea,
          ib_blocks: t.ibBlocks,
          ...(sessions.length > 0 && { sessions: sessions as GetTpoInput["sessions"] }),
        },
        signal,
      );
      return { items, meta, hasMore: has_more };
    },
  });
  onCleanup(footprintFeed.dispose);
  onCleanup(tpoFeed.dispose);
  // What the server computes restarts a feed; what only changes the drawing does not.
  const footprintKey = createMemo(() => {
    const f = orderFlowSettings().footprint;
    return [symbol(), timeframe(), f.rowTicks, f.atrLength, f.valueArea, f.imbalance, f.stacked].join();
  });
  const tpoKey = createMemo(() => {
    const t = orderFlowSettings().tpo;
    return [symbol(), t.period, t.block, t.rowTicks, t.valueArea, t.ibBlocks, t.sessions].join();
  });
  createEffect(() => {
    footprintKey();
    const on = authReady() && flowKind() === "footprint";
    if (on) footprintFeed.restart();
    else footprintFeed.stop();
  });
  // A profile holds the whole period, so in replay it would show the future.
  createEffect(() => {
    tpoKey();
    const on = authReady() && flowKind() === "tpo" && replay.mode() === "off";
    if (on) tpoFeed.restart();
    else tpoFeed.stop();
  });
  const flowView = createMemo<FootprintExtend | TpoExtend | null>(() => {
    const kind = flowKind();
    if (!kind) return null;
    const palette = PALETTES[theme()];
    const colors = resolveColors(palette, chartSettings().themes[theme()]);
    const base = {
      up: colors.bodyUp,
      down: colors.bodyDown,
      text: palette.fg,
      muted: palette.fgMuted,
      surface: colors.background ?? palette.bg,
    };
    if (kind === "footprint") {
      const state = footprints();
      return {
        ...base,
        kind,
        bars: state.items,
        settings: orderFlowSettings().footprint,
        rowSize: state.meta?.row_size ?? 1,
        decimals: decimalsOf(state.meta?.tick),
        intraday: !isDateBar(timeframe()),
        report: setFlowMode,
      };
    }
    const state = profiles();
    return {
      ...base,
      kind,
      profiles: state.items,
      settings: orderFlowSettings().tpo,
      decimals: decimalsOf(state.meta?.tick),
    };
  });
  const flowStatus = createMemo(() => {
    const kind = flowKind();
    if (!kind) return null;
    if (kind === "tpo" && replay.mode() !== "off") return "TPO is off in replay: showing candles";
    const state = kind === "footprint" ? footprints() : profiles();
    if (state.status === "error") return `Order flow failed: ${state.message}`;
    if (!state.meta) return "Loading order flow…";
    if (kind === "footprint") {
      const method = footprintMethod(footprints().meta!);
      return flowMode() === "candles" ? `Zoom in for footprint cells · ${method}` : method;
    }
    return tpoMethod(profiles().meta!);
  });
  // No paging for bars drawn as candles: those need no cells, so no data. Read from
  // the chart, as the mode only changes on the next paint, after this.
  const onVisibleFrom = (t: number) => {
    if (flowKind() === "tpo") tpoFeed.ensure(t);
    else if (flowKind() === "footprint" && (chart()?.getBarSpace() ?? 0) >= CELL_MIN_BAR) {
      footprintFeed.ensure(t);
    }
  };
  // ---------------------------------------------------------------------

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
    refetch,
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
    flowKind,
    flowView,
    flowStatus,
    flowMode,
    flowTable: () => {
      const view = flowView();
      return view?.kind === "footprint" ? view.settings.summary === "table" : !!view?.settings.summary;
    },
    onVisibleFrom,
  };
}

export interface ChartPaneProps {
  state: ChartPaneState;
  /** Where the drawing toolbars go: one rail, showing the active chart's. */
  rail: HTMLElement | undefined;
  /** More than one chart shows: its sub-panes share a smaller height. */
  multi: boolean;
  /** The script editor takes part of the window (ADR-0041). */
  fitPanes: boolean;
  /** A drawing tool was armed here. */
  onToolArmed: () => void;
  /** Open the settings of an indicator of the active chart. */
  onIndicatorSettings: (key: string) => void;
  onAddAlert: (key: string) => void;
  onResetView: () => void;
  onSettings: () => void;
}

export function ChartPane(props: ChartPaneProps): JSX.Element {
  const s = props.state;
  const { symbol, chartType, setSymbol } = s.layout;
  const { activeIndicators, removeIndicator, setActiveIndicators, toggleIndicatorHidden } = s.indicators;
  const indexOf = (key: string) =>
    activeIndicators().findIndex((e) => instanceKey(e) === key);
  const [flowSettingsOpen, setFlowSettingsOpen] = createSignal(false);

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
              onArmedChange={(armed) => {
                s.setDrawingArmed(armed);
                // The chart needs the room to place the tool's points.
                if (armed) props.onToolArmed();
              }}
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
              onRetry={() => void s.refetch()}
              onBack={setSymbol}
              onRemoveIndicators={(names) =>
                setActiveIndicators(
                  activeIndicators().filter((e) => !names.includes(indicatorName(e))),
                )
              }
            />
          </Show>
          <Chart
            series={s.shown()?.series ?? null}
            update={s.shown()?.update ?? null}
            history={s.historyStatus()}
            onLoadOlder={s.feed.loadOlder}
            activeIndicators={s.wireIndicators()}
            fitPanes={props.fitPanes}
            orderFlow={s.flowView()}
            onVisibleFrom={s.onVisibleFrom}
            styles={s.plotStyles()}
            chartType={chartType()}
            timeZone={s.timeZone()}
            onToggleHidden={(n) => toggleIndicatorHidden(indexOf(n))}
            // Both read the active chart. A press has activated this one, a key has not.
            onSettings={(key) => {
              setActivePane(s.pane);
              props.onIndicatorSettings(key);
            }}
            onRemove={removeIndicator}
            onAddAlert={(key) => {
              setActivePane(s.pane);
              props.onAddAlert(key);
            }}
            onChartReady={s.setChart}
            multi={props.multi}
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
        <Show when={s.flowStatus()}>
          {(text) => (
            <div
              class="orderflow-status"
              classList={{ "above-table": s.flowTable() }}
              data-testid="orderflow-status"
              data-mode={s.flowMode()}
            >
              <span class="truncate" title={text()}>{text()}</span>
              <button
                type="button"
                aria-label="Order flow settings"
                data-testid="orderflow-settings-open"
                onClick={() => setFlowSettingsOpen(true)}
              >
                ⚙
              </button>
            </div>
          )}
        </Show>
        <OrderFlowSettingsDialog
          kind={s.flowKind()}
          open={flowSettingsOpen()}
          onClose={() => setFlowSettingsOpen(false)}
        />
      </div>
    </PaneProvider>
  );
}
