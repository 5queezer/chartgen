// Price-axis extras anchored over the chart: the bar-close countdown
// under the last-price badge, and the %/log/auto scale buttons in the
// bottom-right corner of the axis (as in TradingView). Also applies the
// persisted scale mode to klinecharts.
//
// The countdown is a DOM badge rather than a klinecharts style because
// the library's last-price mark only supports a single text line. It is
// aligned to the last close via `convertToPixel` and re-aligned on
// scroll/zoom, resize and every 1 s tick.

import type { Chart as KLineChart } from "klinecharts";
import {
  Show,
  createEffect,
  createMemo,
  createSignal,
  onCleanup,
  untrack,
  type JSX,
} from "solid-js";

import { DEFAULT_GAP, lockRatio as lockRatioOf, lockedGap } from "../lib/priceRatio";
import { barCountdown, isContinuousMarket } from "../lib/countdown";
import {
  autoScale,
  invertScale,
  lockRatio,
  scaleMode,
  setAutoScale,
  setInvertScale,
  setScaleMode,
  type ScaleMode,
} from "../stores/layout";
import { chartSettings } from "../stores/chartSettings";
import type { SeriesPayload } from "../types/responses";

const CANDLE_PANE_ID = "candle_pane";
const FALLBACK_AXIS_WIDTH = 64;

export interface PriceAxisControlsProps {
  chart: KLineChart | null;
  series: SeriesPayload | null;
}

interface BadgePos {
  top: number;
  left: number;
  width: number;
  color: string;
}

export function PriceAxisControls(props: PriceAxisControlsProps): JSX.Element {
  const [now, setNow] = createSignal(Date.now());
  const [pos, setPos] = createSignal<BadgePos | null>(null);
  const [axisWidth, setAxisWidth] = createSignal(FALLBACK_AXIS_WIDTH);

  const lastBar = createMemo(() => props.series?.bars.at(-1) ?? null);
  const text = createMemo(() => {
    const bar = lastBar();
    const interval = props.series?.interval;
    // Only 24/7 symbols: session-based markets (Yahoo) close bars at
    // times the interval alone cannot give.
    if (!bar || bar.t === null || !interval) return null;
    if (!isContinuousMarket(props.series?.symbol)) return null;
    return barCountdown(bar.t, interval, now());
  });

  const reposition = () => {
    const c = props.chart;
    const bar = lastBar();
    if (!c || !bar || bar.c === null) {
      setPos(null);
      return;
    }
    const axis = c.getSize(CANDLE_PANE_ID, "yAxis" as never);
    const main = c.getSize(CANDLE_PANE_ID, "main" as never);
    const y = (
      c.convertToPixel({ value: bar.c }, {
        paneId: CANDLE_PANE_ID,
        absolute: true,
      }) as { y?: number }
    ).y;
    if (!axis || !main || y === undefined || !Number.isFinite(y)) {
      setPos(null);
      return;
    }
    setAxisWidth(axis.width);
    // Same color rule as klinecharts's last-price mark (close vs open).
    const color =
      bar.o !== null && bar.c > bar.o
        ? "var(--color-bull)"
        : bar.o !== null && bar.c < bar.o
          ? "var(--color-bear)"
          : "var(--color-accent)";
    // The price badge is ~18px tall and centered on y; sit just below it.
    const top = y + 10;
    setPos(
      top < main.top || top > main.top + main.height - 18
        ? null
        : { top, left: axis.left, width: axis.width, color },
    );
  };

  // Scale mode + auto-fit. Re-applying the style type also makes
  // klinecharts drop any manual price-axis zoom, which is what "auto"
  // means, so it also runs whenever auto is switched back on.
  createEffect(() => {
    const c = props.chart;
    if (!c) return;
    const mode = scaleMode();
    c.setStyles({ yAxis: { type: mode } } as never);
    requestAnimationFrame(reposition);
  });

  // Inverted scale: klinecharts maps every price<->pixel conversion
  // (crosshair, drawings, alert lines) through `yAxis.reverse`.
  createEffect(() => {
    const c = props.chart;
    if (!c) return;
    c.setStyles({ yAxis: { reverse: invertScale() } } as never);
    requestAnimationFrame(reposition);
  });

  // Lock price to bar ratio: see lib/priceRatio.ts. `ratio` is captured the
  // first time the lock applies and dropped when it is released.
  let ratio: number | null = null;
  let gap = DEFAULT_GAP;
  const applyLock = () => {
    const c = props.chart;
    if (!c) return;
    if (!untrack(lockRatio)) return;
    const height = c.getSize(CANDLE_PANE_ID, "yAxis" as never)?.height;
    if (!height) return;
    const [top, bottom] = c.convertFromPixel([{ y: 0 }, { y: height }], {
      paneId: CANDLE_PANE_ID,
    }) as { value?: number }[];
    if (top?.value === undefined || bottom?.value === undefined) return;
    const tf = untrack(scaleMode) === "log" ? Math.log10 : (v: number) => v;
    const range = Math.abs(tf(top.value) - tf(bottom.value));
    const barSpace = c.getBarSpace();
    if (!(range > 0) || !(barSpace > 0)) return;
    ratio ??= lockRatioOf(range, height, barSpace);
    const g = lockedGap({ range, gap, height, barSpace, ratio });
    if (Math.abs(g - gap.top) < 1e-4) return;
    gap = { top: g, bottom: g };
    c.setPaneOptions({ id: CANDLE_PANE_ID, gap });
  };

  // Lock or mode change: back to the default gap, re-capture the ratio.
  createEffect(() => {
    const c = props.chart;
    if (!c) return;
    lockRatio();
    scaleMode();
    ratio = null;
    gap = DEFAULT_GAP;
    c.setPaneOptions({ id: CANDLE_PANE_ID, gap: DEFAULT_GAP });
    requestAnimationFrame(() => {
      applyLock();
      reposition();
    });
  });

  createEffect(() => {
    props.series;
    requestAnimationFrame(applyLock);
  });

  createEffect(() => {
    const c = props.chart;
    if (!c) return;
    const actions = ["onScroll", "onZoom", "onVisibleRangeChange"] as const;
    for (const a of actions) c.subscribeAction(a as never, applyLock);
    onCleanup(() => {
      for (const a of actions) c.unsubscribeAction(a as never, applyLock);
    });
  });

  createEffect(() => {
    const c = props.chart;
    if (!c || !autoScale()) return;
    c.setStyles({ yAxis: { type: untrack(scaleMode) } } as never);
    requestAnimationFrame(reposition);
  });

  // Dragging the axis scales it by hand, which turns auto off (as in
  // TradingView); double-click resets to auto (klinecharts does the reset
  // itself; this keeps the persisted flag and the button in sync).
  createEffect(() => {
    const axisDom = props.chart?.getDom(CANDLE_PANE_ID, "yAxis" as never);
    if (!axisDom) return;
    let pressed = false;
    const onDown = () => (pressed = true);
    const onMove = () => {
      if (pressed && autoScale()) setAutoScale(false);
    };
    const onUp = () => (pressed = false);
    const onDblClick = () => setAutoScale(true);
    axisDom.addEventListener("pointerdown", onDown);
    axisDom.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    axisDom.addEventListener("dblclick", onDblClick);
    onCleanup(() => {
      axisDom.removeEventListener("pointerdown", onDown);
      axisDom.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      axisDom.removeEventListener("dblclick", onDblClick);
    });
  });

  createEffect(() => {
    const c = props.chart;
    if (!c) return;
    const actions = ["onScroll", "onZoom", "onVisibleRangeChange"] as const;
    for (const a of actions) c.subscribeAction(a as never, reposition);
    onCleanup(() => {
      for (const a of actions) c.unsubscribeAction(a as never, reposition);
    });
  });

  createEffect(() => {
    props.series;
    requestAnimationFrame(reposition);
  });

  // Tick on wall-clock second boundaries so the badge changes as the
  // bar's remaining seconds do.
  let timer: ReturnType<typeof setTimeout>;
  const tick = () => {
    setNow(Date.now());
    reposition();
    timer = setTimeout(tick, 1000 - (Date.now() % 1000));
  };
  timer = setTimeout(tick, 1000 - (Date.now() % 1000));
  onCleanup(() => clearTimeout(timer));

  const toggle = (mode: ScaleMode) =>
    setScaleMode(scaleMode() === mode ? "normal" : mode);

  const btn = (active: boolean) =>
    `price-scale-btn${active ? " is-active" : ""}`;

  return (
    <>
      <Show when={text() !== null && pos()}>
        <div
          class="price-countdown"
          data-testid="bar-countdown"
          title="Time until the current bar closes"
          style={{
            top: `${pos()!.top}px`,
            left: `${pos()!.left}px`,
            width: `${pos()!.width}px`,
            "background-color": pos()!.color,
          }}
        >
          {text()}
        </div>
      </Show>
      <div
        class="price-scale-controls"
        role="group"
        aria-label="Price scale"
        style={{
          width: `${axisWidth()}px`,
          ...(chartSettings().options.scalePlacement === "left"
            ? { left: "0", right: "auto" }
            : {}),
        }}
      >
        <button
          type="button"
          class={btn(scaleMode() === "percentage")}
          aria-pressed={scaleMode() === "percentage"}
          title="Percent scale"
          onClick={() => toggle("percentage")}
        >
          %
        </button>
        <button
          type="button"
          class={btn(scaleMode() === "log")}
          aria-pressed={scaleMode() === "log"}
          title="Logarithmic scale"
          onClick={() => toggle("log")}
        >
          log
        </button>
        <button
          type="button"
          class={btn(invertScale())}
          aria-pressed={invertScale()}
          title="Invert scale (Alt+I)"
          onClick={() => setInvertScale(!invertScale())}
        >
          inv
        </button>
        <button
          type="button"
          class={btn(autoScale())}
          aria-pressed={autoScale()}
          title="Auto-fit price scale (double-click the axis to reset)"
          onClick={() => setAutoScale(!autoScale())}
        >
          auto
        </button>
      </div>
    </>
  );
}
