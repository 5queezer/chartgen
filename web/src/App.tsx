// Top-level component. One screen: title, toolbar, status, chart.

import {
  createEffect,
  createMemo,
  createSignal,
  For,
  mapArray,
  onCleanup,
  untrack,
  onMount,
  Show,
  type JSX,
} from "solid-js";

import { BottomBar } from "./components/BottomBar";
import {
  AlertDialog,
  AlertButton,
  lastPlotValue,
  openAlertDialog,
  plotRef,
} from "./components/AlertDialog";
import { paletteColor } from "./components/Chart";
import { ChartPane, createChartPane } from "./components/ChartPane";
import { ChartSettings } from "./components/ChartSettings";
import { DRAWING_LABELS } from "./components/DrawingToolbar";
import { GridMenu } from "./components/GridMenu";
import { isDrawingAlertTool, isHorizontalTool } from "./lib/alerts";
import { chartContext, setIndicatorSignatures } from "./stores/alerts";
import { IndicatorChips } from "./components/IndicatorChips";
import { IndicatorPicker } from "./components/IndicatorPicker";
import { IndicatorSettings } from "./components/IndicatorSettings";
import { LayoutMenu } from "./components/LayoutMenu";
import { chipName } from "./lib/indicatorNames";
import { mergeParams } from "./lib/paramForm";
import { resolveColors } from "./lib/chartStyle";
import { PALETTES } from "./lib/theme";
import { geometryOf } from "./lib/geometryStyle";
import { plotsOf } from "./lib/plotStyle";
import { StatusLine, type Status } from "./components/StatusLine";
import { SymbolSearch } from "./components/SymbolSearch";
import { Toasts } from "./components/Toasts";
import { Toolbar } from "./components/Toolbar";
import { Sidebar, SidebarDock } from "./components/Sidebar";
import {
  applyChartAction,
  chartKeyAction,
  downloadChartImage,
  resetChart,
  screenshotFilename,
} from "./lib/chartActions";
import { isTypingContext, keepKeyboardFocus, swallowMenuGhostClicks } from "./lib/keys";
import { compact } from "./lib/viewport";
import { createMcpSession } from "./mcp";
import { startSignIn, tryHandlePendingRedirect } from "./oauth";
import { createListIndicatorsQuery } from "./queries";
import { createAlertLogger } from "./services/notifications";
import { copyChartLink, reconcileImported } from "./services/shareLink";
import {
  createAlertQuotes,
  chartTick,
  feedPrice,
} from "./services/alertEngine";
import {
  activeIndicators,
  activeIndicatorsKey,
  indicatorName,
  indicatorParams,
  indicatorStyle,
  instanceKey,
  isHidden,
  paramLabels,
  removeIndicator,
  toggleIndicatorHidden,
  updateIndicatorSettings,
} from "./stores/indicators";
import { addCompare } from "./stores/compare";
import {
  activePane,
  grid,
  GRID_SHAPES,
  paneCount,
} from "./stores/grid";
import { chartSettings } from "./stores/chartSettings";
import {
  paramDefaults,
  setParamDefaults,
  type ParamDefaults,
} from "./stores/layouts";
import {
  theme,
  symbol,
  timeframe,
} from "./stores/layout";

export function App(): JSX.Element {
  // Long-lived session for the app's lifetime.
  const session = createMcpSession();

  const [authReady, setAuthReady] = createSignal(false);
  const [authError, setAuthError] = createSignal<string | null>(null);
  const [pickerOpen, setPickerOpen] = createSignal(false);
  // Index (into the active set) of the indicator whose settings dialog is open.
  const [settingsIndex, setSettingsIndex] = createSignal<number | null>(null);
  const [settingsOpen, setSettingsOpen] = createSignal(false);
  const [compareOpen, setCompareOpen] = createSignal(false);

  onMount(() => onCleanup(keepKeyboardFocus()));
  onMount(() => onCleanup(swallowMenuGhostClicks()));

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
      resolveColors(PALETTES[theme()], chartSettings().themes[theme()]).background,
    );
  };

  const resetView = () => {
    const c = chart();
    if (c) resetChart(c);
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

  // Chart legends and the object tree address an instance by its key (ADR-0028).
  const indexOf = (key: string) =>
    activeIndicators().findIndex((e) => instanceKey(e) === key);

  // Only re-evaluates when the dialog opens/closes, so edits elsewhere do
  // not reset a form in progress.
  const settingsTarget = createMemo(() => {
    const i = settingsIndex();
    const entries = untrack(activeIndicators);
    const entry = i === null ? undefined : entries[i];
    if (entry === undefined) return null;
    const key = instanceKey(entry);
    // A repeated indicator's title says which instance it is (ADR-0028).
    const params = paramLabels(entries, untrack(paramDefaults), true).get(key);
    return {
      name: indicatorName(entry),
      title: untrack(() => chipName(indicatorName(entry), params)),
      key,
      params: indicatorParams(entry),
      style: indicatorStyle(entry),
      hidden: isHidden(entry),
    };
  });

  // --- Charts of the grid (ADR-0043) ------------------------------------
  // The compact shell shows only the active chart; the grid and the other
  // charts stay as they are, to come back when there is room.
  const shape = () => (compact() ? "1" : grid());
  const paneIds = createMemo(
    () =>
      compact()
        ? [activePane()]
        : Array.from({ length: paneCount(grid()) }, (_, i) => i),
    [],
    { equals: (a, b) => a.join() === b.join() },
  );
  const panes = createMemo(
    mapArray(paneIds, (pane) => createChartPane(pane, { session, authReady })),
  );
  // The chart the header, panels and shortcuts act on.
  const view = createMemo(
    () => panes().find((p) => p.pane === activePane()) ?? panes()[0]!,
  );
  const [rail, setRail] = createSignal<HTMLElement>();
  // Compact shell: the drawing toolbar behind the dock's Draw button.
  const [drawOpen, setDrawOpen] = createSignal(false);
  const chart = () => view().chart();
  const live = () => view().live();
  const shown = () => view().shown();
  const drawings = () => view().drawings();
  const wireIndicators = () => view().wireIndicators();
  const plotStyles = () => view().plotStyles();
  // Pushed tails carry their subscription id; each feed takes only its own.
  session.onChartTick((id, tail) => {
    for (const p of panes()) p.feed.applyPush(id, tail);
  });
  // ---------------------------------------------------------------------

  // --- Drawing and indicator alerts (ADR-0034) ---------------------------
  const alertDrawings = createMemo(() =>
    (drawings()?.list() ?? []).flatMap((d) =>
      isDrawingAlertTool(d.name) && !d.vertical
        ? [{ id: d.id, label: DRAWING_LABELS[d.name], sloped: !isHorizontalTool(d.name) }]
        : [],
    ),
  );
  const alertParams = createMemo(() => paramLabels(wireIndicators(), paramDefaults()));
  // Name and params per instance key: an indicator alert only runs on the
  // instance it was created on.
  createEffect(() =>
    setIndicatorSignatures(
      Object.fromEntries(wireIndicators().map((e) => [instanceKey(e), activeIndicatorsKey([e])])),
    ),
  );
  // From a legend row: an overlay (EMA) is the level the price crosses, an
  // oscillator (RSI) the source, prefilled with its last value.
  const addIndicatorAlert = (key: string) => {
    const series = live()?.series ?? null;
    const ind = series?.indicators[key];
    if (!ind?.lines?.length) return;
    if (ind.is_overlay) {
      openAlertDialog({ level: plotRef(key, 0) });
      return;
    }
    openAlertDialog({ source: plotRef(key, 0), price: lastPlotValue(series, key, 0) ?? undefined });
  };
  // ---------------------------------------------------------------------

  // Other symbols with active alerts are evaluated from their own quotes poll.
  // While replaying the chart symbol is polled there too (ADR-0023): the
  // feed is suspended, and its alerts still need real prices.
  createAlertQuotes(session, authReady, () => (view().replay.mode() === "off" ? symbol() : ""));

  if (import.meta.env.DEV) {
    // E2E hook (tests/e2e/live-paging.spec.ts): what the chart holds.
    (window as unknown as Record<string, unknown>).__chartgenTest = {
      chart,
      series: () => live()?.series ?? null,
      // Multi-chart e2e (tests/e2e/multi-chart.spec.ts): what alerts read, and each chart of the grid.
      alertContext: chartContext,
      panes: () =>
        panes().map((p) => ({
          pane: p.pane,
          symbol: p.layout.symbol(),
          timeframe: p.layout.timeframe(),
          indicators: p.wireIndicators(),
          invert: p.layout.invertScale(),
          reverse: p.chart()?.getStyles().yAxis.reverse ?? null,
          candleIndicators: [
            ...((p.chart()?.getIndicatorByPaneId("candle_pane") as Map<string, unknown> | null)?.keys() ?? []),
          ],
          visibleFrom: p.chart()?.getDataList()[p.chart()!.getVisibleRange().from]?.timestamp ?? null,
          candleHeight: p.chart()?.getSize("candle_pane")?.height ?? null,
          stackHeight: p.chart()?.getSize()?.height ?? null,
          // Canvases whose backing store does not match their size draw blank or stretched.
          staleCanvases: [...(p.chart()?.getDom()?.querySelectorAll("canvas") ?? [])].filter(
            (el) => el.height !== Math.round(el.clientHeight * devicePixelRatio),
          ).length,
          bars: p.live()?.series.bars.length ?? 0,
          version: p.live()?.update.version ?? null,
          updateKind: p.live()?.update.kind ?? null,
          poll: p.pollStatus().kind,
          drawings: p.drawings()?.list().length ?? 0,
        })),
      // Alert e2e (tests/e2e/alerts.spec.ts): inject a live price. For the
      // chart symbol it comes with the chart's bars and indicators, as a
      // live tail would (sloped drawing alerts need them).
      feedPrice: (sym: string, price: number) => {
        const s = live()?.series;
        feedPrice(sym, price, s && sym === symbol() ? chartTick(s) : undefined);
      },
      // Bar replay e2e (tests/e2e/bar-replay.spec.ts): the frame on screen
      // and the replay state.
      shownSeries: () => shown()?.series ?? null,
      replay: () => ({
        mode: view().replay.mode(),
        playing: view().replay.playing(),
        cursorTime: view().replay.cursorTime(),
      }),
    };
  }

  // The indicator registry only changes between deploys, so this query
  // is cached forever. It fires as soon as auth is ready.
  const listIndicators = createListIndicatorsQuery(session, authReady);

  // Indicators from a share link are checked against the registry before
  // the first chart request; if the registry cannot be loaded they are kept.
  createEffect(() => {
    if (listIndicators.isSuccess) {
      const list = listIndicators.data.indicators;
      untrack(() => reconcileImported(list));
    } else if (listIndicators.isError) {
      untrack(() => reconcileImported(undefined));
    }
  });

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
    const key = settingsTarget()?.key;
    const ind = key === undefined ? undefined : view().query.data?.indicators[key];
    return key === undefined || !ind
      ? null
      : plotsOf(ind, (n) => paletteColor(key, n), key);
  });

  // Undefined only while the registry cannot say. The server leaves out
  // `configurable_params` for an indicator without params (EMA Stack), which
  // is an empty form with its Style tab, not a failed load.
  const settingsSpecs = createMemo(() => {
    const n = settingsTarget()?.name;
    const found = listIndicators.data?.indicators.find(
      (i) => i.name === n || i.aliases?.includes(n ?? ""),
    );
    return found && (found.configurable_params ?? []);
  });

  const settingsGeometry = createMemo(() => {
    const key = settingsTarget()?.key;
    const ind = key === undefined ? undefined : view().query.data?.indicators[key];
    return key === undefined || !ind ? null : geometryOf(key, ind);
  });

  const status = createMemo<Status>(() => {
    const authFailed = authError();
    if (authFailed !== null) return { kind: "auth-error", message: authFailed };
    if (!authReady()) return { kind: "authenticating" };
    const query = view().query;
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
    <div class="app-shell flex flex-col h-dvh w-screen">
      <header class="flex flex-nowrap items-center gap-2 min-[1280px]:gap-3 px-4 h-[52px] shrink-0 overflow-hidden border-b border-[color:var(--color-border)]">
        <h1 class="text-lg font-semibold mr-1 min-[1280px]:mr-2 leading-none max-[639px]:hidden">
          chartgen
        </h1>
        <Toolbar
          session={session}
          onReselect={() => void view().refetch()}
          onScreenshot={screenshot}
          onCopyLink={() => void copyChartLink()}
          onResetChart={resetView}
          onSettings={() => setSettingsOpen(true)}
          onCompare={() => setCompareOpen(true)}
          replay={view().replay.mode() !== "off"}
          onReplay={() => (view().replay.mode() === "off" ? view().replay.enter() : view().replay.exit())}
        />
        <SymbolSearch
          open={compareOpen()}
          onOpenChange={setCompareOpen}
          initialQuery=""
          title="Compare symbol"
          onSelect={(ticker) => addCompare(ticker, symbol())}
          session={session}
        />
        <button
          type="button"
          onClick={() => setPickerOpen(true)}
          class="bg-[color:var(--color-bg-elev)] border border-[color:var(--color-border)] text-[color:var(--color-fg)] px-3 py-1.5 rounded-md text-sm hover:border-[color:var(--color-accent)] cursor-pointer inline-flex items-center gap-2 shrink-0"
          title="Indicators"
        >
          <span class="max-[1399px]:sr-only">Indicators</span>
          <svg aria-hidden="true" viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" class="min-[1400px]:hidden">
            <path d="M1 11 L5 7 L8 9 L15 3" />
          </svg>
          <span class="inline-flex items-center justify-center min-w-[1.25rem] h-5 px-1 rounded-full bg-[color:var(--color-accent)] text-black text-xs font-semibold">
            {activeIndicators().length}
          </span>
        </button>
        {/* Below 480px the header has no room: alerts stay reachable from the
         * sidebar panel, the status text drops (the sign-in recovery stays) and the layout name collapses. */}
        <div class="max-[479px]:hidden compact:hidden">
          <AlertButton />
        </div>
        <div>
          <StatusLine
            status={status()}
            live={view().replay.mode() === "off" ? view().pollStatus() : { kind: "off" }}
            onSignIn={() =>
              void startSignIn().catch((e: unknown) =>
                setAuthError(e instanceof Error ? e.message : String(e)),
              )
            }
          />
        </div>
        <div class="ml-auto compact:hidden">
          <GridMenu />
        </div>
        <div class="min-w-0 compact:ml-auto max-[479px]:[&_[data-testid=layout-name]]:hidden">
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
        geometry={settingsGeometry()}
        specs={settingsSpecs()}
        onClose={() => setSettingsIndex(null)}
        onSubmit={(params, fields, style) => {
          const target = settingsTarget();
          // By key: the entry may have moved while the dialog was open.
          const i = target ? indexOf(target.key) : -1;
          if (i >= 0 && target) {
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
      <ChartSettings
        open={settingsOpen()}
        onClose={() => setSettingsOpen(false)}
      />
      <main class="flex flex-1 min-h-0" data-draw-open={drawOpen() ? "" : undefined}>
        <div ref={setRail} class="contents" />
        <div
          class="chart-grid"
          data-testid="chart-grid"
          data-multi={paneIds().length > 1 ? "" : undefined}
          style={{
            "grid-template-columns": `repeat(${GRID_SHAPES[shape()].columns}, minmax(0, 1fr))`,
            "grid-template-rows": `repeat(${GRID_SHAPES[shape()].rows}, minmax(0, 1fr))`,
          }}
        >
          <For each={panes()}>
            {(state) => (
              <ChartPane
                state={state}
                rail={rail()}
                multi={paneIds().length > 1}
                onToolArmed={() => setDrawOpen(false)}
                onIndicatorSettings={(key) => setSettingsIndex(indexOf(key))}
                onAddAlert={addIndicatorAlert}
                onResetView={resetView}
                onSettings={() => setSettingsOpen(true)}
              />
            )}
          </For>
        </div>
        <Sidebar
          session={session}
          enabled={authReady()}
          panels={{
            // The frame on screen: in replay, never a bar after the cursor.
            series: shown()?.series ?? null,
            crosshair: view().crosshair(),
            styles: plotStyles(),
            compare: view().shownCompare(),
            compareFrom: view().compareFrom(),
            timeZone: view().timeZone(),
            drawings: drawings(),
            onSettings: (n) => setSettingsIndex(indexOf(n)),
          }}
        />
      </main>
      <div class="bottom-rows flex flex-col shrink-0">
        <BottomBar
          chart={chart()}
          series={shown()?.series ?? null}
          loadUntil={(ts, isCurrent) => view().feed.loadUntil(ts, isCurrent)}
        />
        <Show when={compact()}>
          <SidebarDock
            drawActive={drawOpen() || view().drawingArmed()}
            onDraw={() => setDrawOpen(!drawOpen())}
          />
        </Show>
      </div>
      <AlertDialog
        series={live()?.series ?? null}
        params={alertParams()}
        drawings={alertDrawings()}
      />
      <Toasts />
    </div>
  );
}
