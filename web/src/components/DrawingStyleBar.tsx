// Floating style bar for the selected drawing: color, width, line style, and
// per tool the rectangle's fill or the text size. Anchored to the top-right
// of the candle pane's plot area, below the legend when that would collide
// with it. Locked drawings show the controls disabled.

import { createEffect, createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import type { Chart as KChart } from "klinecharts";

import { PALETTES } from "../lib/theme";
import { theme } from "../stores/layout";
import {
  DASH_STYLES,
  FONT_SIZES,
  MAX_WIDTH,
  type DrawingStyle,
} from "../stores/drawings";
import { ColorPicker } from "./ColorPicker";
import type { DrawingItem } from "./DrawingToolbar";

/** klinecharts' default overlay blue, what an unstyled line is drawn in. */
const DEFAULT_LINE = "#1677ff";
const DEFAULT_FILL = "rgba(41, 98, 255, 0.15)";
const GAP = 8;

const seg =
  "min-w-7 h-7 px-1 inline-flex items-center justify-center rounded cursor-pointer text-xs text-[color:var(--color-fg-muted)] hover:bg-[color:var(--color-border)] aria-pressed:bg-[color:var(--color-border)] aria-pressed:text-[color:var(--color-accent)] disabled:opacity-40 disabled:cursor-not-allowed";

export interface DrawingStyleBarProps {
  chart: KChart | null;
  drawing: DrawingItem;
  /** `key` names the changed field so a slider drag is one undo step. */
  onChange: (patch: Partial<DrawingStyle>, key: string) => void;
}

export function DrawingStyleBar(props: DrawingStyleBarProps): JSX.Element {
  let el: HTMLDivElement | undefined;
  const [pos, setPos] = createSignal<{ x: number; y: number } | null>(null);

  /** Right edge at the plot's top-right; below the legend rows if they would collide. */
  const place = () => {
    const dom = props.chart?.getDom();
    if (!dom || !el) return;
    const host = dom.getBoundingClientRect();
    const pane = props.chart?.getSize("candle_pane", "main" as never);
    const right = host.left + (pane ? pane.left + pane.width : host.width);
    const paneTop = host.top + (pane?.top ?? 0);
    const paneBottom = paneTop + (pane?.height ?? host.height);
    const w = el.offsetWidth;
    const h = el.offsetHeight;
    let y = paneTop + GAP;
    const rows = [
      ...(dom.parentElement ?? document).querySelectorAll(".pane-header"),
    ]
      .map((n) => n.getBoundingClientRect())
      .filter((r) => r.top >= paneTop - 1 && r.top < paneBottom);
    const left = right - GAP - w;
    const hit = rows.filter((r) => r.top < y + h && r.right + GAP > left);
    if (hit.length > 0) y = Math.max(...rows.map((r) => r.bottom)) + 4;
    setPos({ x: right - GAP, y });
  };

  onMount(() => {
    place();
    const dom = props.chart?.getDom();
    if (dom) {
      const ro = new ResizeObserver(place);
      ro.observe(dom);
      onCleanup(() => ro.disconnect());
    }
  });
  // The bar's width depends on the drawing kind.
  createEffect(() => {
    void props.drawing.name;
    queueMicrotask(place);
  });

  const d = () => props.drawing;
  const style = () => d().style ?? {};
  const isText = () => d().name === "cgText";
  const isRect = () => d().name === "cgRect";
  const disabled = () => d().lock || d().offRange;
  const stored = (k: "borderColor" | "color") =>
    (d().styles?.polygon as Record<string, string> | undefined)?.[k];
  const color = () =>
    style().color ??
    (isText() ? PALETTES[theme()].fg : (isRect() ? stored("borderColor") : undefined) ?? DEFAULT_LINE);
  const fill = () => style().fill ?? stored("color") ?? DEFAULT_FILL;

  const divider = () => <span class="w-px h-5 bg-[color:var(--color-border)] mx-1" />;

  return (
    <div
      ref={el}
      role="toolbar"
      aria-label="Drawing style"
      data-testid="drawing-style-bar"
      class="fixed z-40 -translate-x-full flex items-center gap-1 px-2 py-1 rounded-md border border-[color:var(--color-border)] bg-[color:var(--color-bg-elev)] shadow-lg"
      style={{
        left: `${pos()?.x ?? 0}px`,
        top: `${pos()?.y ?? 0}px`,
        visibility: pos() ? "visible" : "hidden",
      }}
      title={disabled() ? "Unlock the drawing to style it" : undefined}
    >
      <fieldset disabled={disabled()} class="contents">
        <ColorPicker
          value={color()}
          label={isText() ? "Text color" : "Line color"}
          onChange={(c) => props.onChange({ color: c }, "color")}
        />
        <Show when={isRect()}>
          <ColorPicker
            value={fill()}
            label="Fill color"
            onChange={(c) => props.onChange({ fill: c }, "fill")}
          />
        </Show>
        <Show when={!isText()}>
          {divider()}
          <For each={Array.from({ length: MAX_WIDTH }, (_, i) => i + 1)}>
            {(w) => (
              <button
                type="button"
                class={seg}
                aria-label={`Width ${w}`}
                aria-pressed={(style().width ?? 1) === w}
                onClick={() => props.onChange({ width: w }, "width")}
              >
                {w}
              </button>
            )}
          </For>
          {divider()}
          <For each={DASH_STYLES}>
            {(k) => (
              <button
                type="button"
                class={seg}
                aria-label={`Line style ${k}`}
                title={k}
                aria-pressed={(style().dash ?? "solid") === k}
                onClick={() => props.onChange({ dash: k }, "dash")}
              >
                <svg width="18" height="8" viewBox="0 0 18 8" aria-hidden="true">
                  <path
                    d="M1 4h16"
                    stroke="currentColor"
                    stroke-width="2"
                    stroke-dasharray={k === "solid" ? undefined : k === "dashed" ? "5 3" : "1 3"}
                  />
                </svg>
              </button>
            )}
          </For>
        </Show>
        <Show when={isText()}>
          {divider()}
          <For each={FONT_SIZES}>
            {(n) => (
              <button
                type="button"
                class={seg}
                aria-label={`Text size ${n}`}
                aria-pressed={(style().fontSize ?? 12) === n}
                onClick={() => props.onChange({ fontSize: n }, "size")}
              >
                {n}
              </button>
            )}
          </For>
        </Show>
      </fieldset>
    </div>
  );
}
