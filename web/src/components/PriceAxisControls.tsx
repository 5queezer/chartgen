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
  type JSX,
} from "solid-js";

import { barCountdown } from "../lib/countdown";
import {
  autoScale,
  scaleMode,
  setAutoScale,
  setScaleMode,
  type ScaleMode,
} from "../stores/layout";
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
    if (!bar || bar.t === null || !interval) return null;
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
  // means; with auto-fit on, dragging the axis is disabled.
  createEffect(() => {
    const c = props.chart;
    if (!c) return;
    const mode = scaleMode();
    const auto = autoScale();
    c.setStyles({ yAxis: { type: mode } } as never);
    c.setPaneOptions({
      id: CANDLE_PANE_ID,
      axisOptions: { scrollZoomEnabled: !auto },
    });
    requestAnimationFrame(reposition);
  });

  // Double-click on the axis resets to auto (klinecharts does the reset
  // itself; this keeps the persisted flag and the button in sync).
  createEffect(() => {
    const axisDom = props.chart?.getDom(CANDLE_PANE_ID, "yAxis" as never);
    if (!axisDom) return;
    const onDblClick = () => setAutoScale(true);
    axisDom.addEventListener("dblclick", onDblClick);
    onCleanup(() => axisDom.removeEventListener("dblclick", onDblClick));
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

  const timer = setInterval(() => {
    setNow(Date.now());
    reposition();
  }, 1000);
  onCleanup(() => clearInterval(timer));

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
        style={{ width: `${axisWidth()}px` }}
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
