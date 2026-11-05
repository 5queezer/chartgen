// Bar replay UI (ADR-0023): TradingView's floating control bar over the
// bottom of the chart, the cut line that picks the start bar in "select"
// mode, the "Select date" dialog and the replay shortcuts (Shift+→ step,
// Shift+↓ play/pause, Esc exit). The state lives in services/replay.ts.
// While picking, a layer over the chart takes the mouse: the pick click
// never reaches klinecharts (so no drawing tool gets a point), and drags
// and the wheel are passed on as scroll and zoom.
// Mounted inside the chart's (relatively positioned) wrapper.

import { Dialog, DropdownMenu } from "@kobalte/core";
import {
  For,
  Show,
  createSignal,
  onCleanup,
  onMount,
  type JSX,
} from "solid-js";

import type { Chart as KLineChart, Coordinate, Point } from "klinecharts";

import { isTypingContext } from "../lib/keys";
import { REPLAY_SPEEDS, type ReplaySpeed } from "../lib/replay";
import type { Replay } from "../services/replay";
import { GoToDateForm } from "./BottomBar";

const CANDLE_PANE_ID = "candle_pane";
/** A press that moves further than this is a drag (pan), not a pick. */
const CLICK_SLOP_PX = 4;
/** Zoom per wheel notch while picking, as klinecharts' own wheel zoom. */
const WHEEL_ZOOM = 1.1;

const buttonClass =
  "px-2 py-1.5 rounded text-sm cursor-pointer text-[color:var(--color-fg)] hover:bg-[color:var(--color-border)] inline-flex items-center gap-1.5 disabled:opacity-40 disabled:cursor-default";
const menuClass =
  "bg-[color:var(--color-bg-elev)] border border-[color:var(--color-border)] rounded-md shadow-lg z-50 py-1 min-w-24";
const itemClass =
  "px-3 py-1.5 text-sm cursor-pointer flex items-center gap-2 outline-none data-[highlighted]:bg-[color:var(--color-border)]";
const iconProps = {
  width: 16,
  height: 16,
  viewBox: "0 0 16 16",
  fill: "none",
  stroke: "currentColor",
  "stroke-width": 1.5,
  "aria-hidden": true,
} as const;

export interface ReplayBarProps {
  replay: Replay;
  chart: KLineChart | null;
  /** A drawing tool is armed: Esc cancels it rather than ending replay. */
  drawingArmed: boolean;
  /** Time zone of the "Select date" form. */
  timeZone: string;
}

export function ReplayBar(props: ReplayBarProps): JSX.Element {
  const [dateOpen, setDateOpen] = createSignal(false);
  const r = () => props.replay;

  // Capture phase: runs before the chart's own arrow-key navigation, which
  // then sees the key handled (isTypingContext checks defaultPrevented).
  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (r().mode() === "off" || e.altKey || e.ctrlKey || e.metaKey || isTypingContext(e)) {
        return;
      }
      if (e.shiftKey && e.key === "ArrowRight") {
        e.preventDefault();
        r().pause();
        r().step();
      } else if (e.shiftKey && e.key === "ArrowDown") {
        e.preventDefault();
        r().togglePlay();
      } else if (e.key === "Escape" && !e.shiftKey && !props.drawingArmed) {
        r().exit();
      }
    };
    window.addEventListener("keydown", onKey, true);
    onCleanup(() => window.removeEventListener("keydown", onKey, true));
  });

  const active = () => r().mode() === "active";

  return (
    <Show when={r().mode() !== "off"}>
      <Show when={r().mode() === "select" && props.chart}>
        {(chart) => <CutLine chart={chart()} onPick={(i) => r().startAt(i)} />}
      </Show>
      <div
        class="absolute bottom-10 left-1/2 -translate-x-1/2 z-20 flex items-center gap-0.5 whitespace-nowrap px-1.5 py-1 rounded-md border border-[color:var(--color-border)] bg-[color:var(--color-bg-elev)] shadow-lg text-[color:var(--color-fg)]"
        role="toolbar"
        aria-label="Bar replay"
        data-testid="replay-bar"
        data-mode={r().mode()}
      >
        <button
          type="button"
          class={buttonClass}
          classList={{ "text-[color:var(--color-accent)]": r().mode() === "select" }}
          aria-pressed={r().mode() === "select"}
          data-testid="replay-select-bar"
          title="Select bar: click the bar to start from"
          onClick={() => r().selectStart()}
        >
          <svg {...iconProps}>
            <path d="M8 1.5v13" stroke-dasharray="2 1.5" />
            <path d="M3 5.5 5.5 8 3 10.5M13 5.5 10.5 8 13 10.5" />
          </svg>
          Select bar
        </button>
        <button
          type="button"
          class={buttonClass}
          data-testid="replay-select-date"
          title="Select date"
          aria-label="Select date"
          onClick={() => setDateOpen(true)}
        >
          <svg {...iconProps}>
            <rect x="1.5" y="2.5" width="13" height="12" rx="1.5" />
            <path d="M1.5 6h13M5 1v3M11 1v3" />
          </svg>
        </button>
        <span class="w-px h-5 bg-[color:var(--color-border)] mx-1" />
        <button
          type="button"
          class={buttonClass}
          data-testid="replay-play"
          disabled={!active()}
          aria-label={r().playing() ? "Pause" : "Play"}
          title={r().playing() ? "Pause (Shift+↓)" : "Play (Shift+↓)"}
          onClick={() => r().togglePlay()}
        >
          <svg {...iconProps}>
            <Show when={r().playing()} fallback={<path d="M4 2.5v11l9-5.5z" fill="currentColor" />}>
              <path d="M5 2.5v11M11 2.5v11" stroke-width="2.5" />
            </Show>
          </svg>
        </button>
        <button
          type="button"
          class={buttonClass}
          data-testid="replay-step"
          disabled={!active()}
          aria-label="Forward"
          title="Forward one bar (Shift+→)"
          onClick={() => {
            r().pause();
            r().step();
          }}
        >
          <svg {...iconProps}>
            <path d="M3 2.5v11l7.5-5.5z" fill="currentColor" />
            <path d="M13 2.5v11" stroke-width="2" />
          </svg>
        </button>
        <DropdownMenu.Root>
          <DropdownMenu.Trigger
            class={buttonClass}
            data-testid="replay-speed"
            aria-label={`Replay speed: ${r().speed()} bars per second`}
            title="Replay speed"
          >
            {r().speed()}x
          </DropdownMenu.Trigger>
          <DropdownMenu.Portal>
            <DropdownMenu.Content class={menuClass}>
              <DropdownMenu.RadioGroup
                value={String(r().speed())}
                onChange={(v) => r().setSpeed(Number(v) as ReplaySpeed)}
              >
                <For each={REPLAY_SPEEDS}>
                  {(s) => (
                    <DropdownMenu.RadioItem value={String(s)} class={itemClass} closeOnSelect>
                      {s}x · {s} bar{s === 1 ? "" : "s"}/s
                    </DropdownMenu.RadioItem>
                  )}
                </For>
              </DropdownMenu.RadioGroup>
            </DropdownMenu.Content>
          </DropdownMenu.Portal>
        </DropdownMenu.Root>
        <span class="w-px h-5 bg-[color:var(--color-border)] mx-1" />
        <button
          type="button"
          class={buttonClass}
          data-testid="replay-realtime"
          title="Jump to real time (Esc)"
          onClick={() => r().exit()}
        >
          <svg {...iconProps}>
            <path d="M2 2.5v11l6-5.5zM8 2.5v11l6-5.5z" fill="currentColor" />
          </svg>
          Jump to real time
        </button>
        <button
          type="button"
          class={buttonClass}
          data-testid="replay-close"
          aria-label="Close replay"
          title="Close replay (Esc)"
          onClick={() => r().exit()}
        >
          <svg {...iconProps}>
            <path d="M4 4l8 8M12 4l-8 8" />
          </svg>
        </button>
        <Show when={r().mode() === "select"}>
          <span class="px-2 text-xs text-[color:var(--color-fg-muted)]" data-testid="replay-hint">
            Click a bar to start
          </span>
        </Show>
        <Show when={r().notice()}>
          {(text) => (
            <span class="px-2 text-xs text-[color:var(--color-danger)]" role="status" data-testid="replay-notice">
              {text()}
            </span>
          )}
        </Show>
      </div>

      <Dialog.Root open={dateOpen()} onOpenChange={setDateOpen} modal>
        <Dialog.Portal>
          <Dialog.Overlay class="fixed inset-0 z-40 bg-black/40" />
          <div class="fixed inset-0 z-50 flex items-start justify-center pt-[20vh] px-4 pointer-events-none">
            <Dialog.Content class="pointer-events-auto w-80 bg-[color:var(--color-bg)] border border-[color:var(--color-border)] rounded-lg shadow-xl p-4">
              <GoToDateForm
                tz={props.timeZone}
                title="Start replay at"
                onSubmit={(ms) => {
                  setDateOpen(false);
                  void r().startAtDate(ms);
                }}
                onCancel={() => setDateOpen(false)}
              />
            </Dialog.Content>
          </div>
        </Dialog.Portal>
      </Dialog.Root>
    </Show>
  );
}

interface CutPosition {
  /** Line x, pane right edge, chart top and height, relative to the wrapper. */
  x: number;
  right: number;
  top: number;
  height: number;
}

/**
 * The vertical cut line under the mouse in "select" mode, with the bars
 * right of it dimmed, as TradingView draws it. A click starts the replay at
 * that bar. The layer takes the mouse from the chart: a drag scrolls it and
 * the wheel zooms it instead.
 */
function CutLine(props: { chart: KLineChart; onPick: (index: number) => void }): JSX.Element {
  let layer!: HTMLDivElement;
  const [pos, setPos] = createSignal<CutPosition | null>(null);

  onMount(() => {
    const host = layer.parentElement!;
    const paneWidth = () => props.chart.getSize(CANDLE_PANE_ID, "main" as never)?.width ?? 0;

    /** Bar under clientX, or null outside the chart's bars area. */
    const indexAt = (e: MouseEvent): number | null => {
      const dom = props.chart.getDom();
      const n = props.chart.getDataList().length;
      if (!dom || n === 0) return null;
      const rect = dom.getBoundingClientRect();
      if (e.clientY < rect.top || e.clientY > rect.bottom) return null;
      const x = e.clientX - rect.left;
      if (x < 0 || x > paneWidth()) return null;
      const [p] = props.chart.convertFromPixel([{ x }], { paneId: CANDLE_PANE_ID }) as Array<
        Partial<Point>
      >;
      return typeof p?.dataIndex === "number" ? Math.max(0, Math.min(n - 1, p.dataIndex)) : null;
    };

    // Press position, and the last x a drag scrolled the chart to.
    let down: { x: number; y: number; lastX: number } | null = null;
    const onMove = (e: MouseEvent) => {
      if (down && e.buttons & 1) {
        props.chart.scrollByDistance(e.clientX - down.lastX);
        down.lastX = e.clientX;
      }
      const i = indexAt(e);
      const dom = props.chart.getDom();
      if (i === null || !dom) return setPos(null);
      const c = props.chart.convertToPixel({ dataIndex: i }, { paneId: CANDLE_PANE_ID }) as Partial<Coordinate>;
      if (c.x === undefined) return setPos(null);
      const rect = dom.getBoundingClientRect();
      const origin = host.getBoundingClientRect();
      const left = rect.left - origin.left;
      setPos({ x: left + c.x, right: left + paneWidth(), top: rect.top - origin.top, height: rect.height });
    };
    const onDown = (e: MouseEvent) => {
      if (e.button !== 0) return;
      e.preventDefault();
      down = { x: e.clientX, y: e.clientY, lastX: e.clientX };
    };
    const onUp = (e: MouseEvent) => {
      const start = down;
      down = null;
      if (!start || Math.hypot(e.clientX - start.x, e.clientY - start.y) > CLICK_SLOP_PX) return;
      const i = indexAt(e);
      if (i !== null) props.onPick(i);
    };
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const dom = props.chart.getDom();
      if (!dom || e.deltaY === 0) return;
      const rect = dom.getBoundingClientRect();
      props.chart.zoomAtCoordinate(e.deltaY < 0 ? WHEEL_ZOOM : 1 / WHEEL_ZOOM, {
        x: e.clientX - rect.left,
        y: e.clientY - rect.top,
      });
    };
    const onLeave = () => {
      down = null;
      setPos(null);
    };
    layer.addEventListener("mousemove", onMove);
    layer.addEventListener("mousedown", onDown);
    layer.addEventListener("mouseup", onUp);
    layer.addEventListener("wheel", onWheel, { passive: false });
    layer.addEventListener("mouseleave", onLeave);
    onCleanup(() => {
      layer.removeEventListener("mousemove", onMove);
      layer.removeEventListener("mousedown", onDown);
      layer.removeEventListener("mouseup", onUp);
      layer.removeEventListener("wheel", onWheel);
      layer.removeEventListener("mouseleave", onLeave);
    });
  });

  return (
    <div ref={layer} class="absolute inset-0 z-10 cursor-crosshair" data-testid="replay-cutline">
      <Show when={pos()}>
        {(p) => (
          <>
            <div
              class="absolute bg-[color:var(--color-bg)] opacity-50"
              style={{
                left: `${p().x}px`,
                width: `${Math.max(0, p().right - p().x)}px`,
                top: `${p().top}px`,
                height: `${p().height}px`,
              }}
            />
            <div
              class="absolute w-px bg-[color:var(--color-accent)]"
              style={{ left: `${p().x}px`, top: `${p().top}px`, height: `${p().height}px` }}
            />
          </>
        )}
      </Show>
    </div>
  );
}
