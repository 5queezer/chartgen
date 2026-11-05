// Floating style bar for the selected drawing: color, width, line style, and
// per tool the rectangle's fill or the text size. Anchored top-center over
// the chart. Locked drawings show the controls disabled.

import { createSignal, For, onCleanup, Show, type JSX } from "solid-js";
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

const seg =
  "w-7 h-7 inline-flex items-center justify-center rounded cursor-pointer text-xs text-[color:var(--color-fg-muted)] hover:bg-[color:var(--color-border)] aria-pressed:bg-[color:var(--color-border)] aria-pressed:text-[color:var(--color-accent)] disabled:opacity-40 disabled:cursor-not-allowed";

export interface DrawingStyleBarProps {
  chart: KChart | null;
  drawing: DrawingItem;
  /** `key` names the changed field so a slider drag is one undo step. */
  onChange: (patch: Partial<DrawingStyle>, key: string) => void;
}

export function DrawingStyleBar(props: DrawingStyleBarProps): JSX.Element {
  const [box, setBox] = createSignal(
    props.chart?.getDom()?.getBoundingClientRect() ?? null,
  );
  const onResize = () => setBox(props.chart?.getDom()?.getBoundingClientRect() ?? null);
  window.addEventListener("resize", onResize);
  onCleanup(() => window.removeEventListener("resize", onResize));

  const d = () => props.drawing;
  const style = () => d().style ?? {};
  const isText = () => d().name === "cgText";
  const isRect = () => d().name === "cgRect";
  const disabled = () => d().lock || d().offRange;
  const color = () =>
    style().color ?? (isText() ? PALETTES[theme()].fg : DEFAULT_LINE);

  return (
    <Show when={box()}>
      {(b) => (
        <div
          role="toolbar"
          aria-label="Drawing style"
          data-testid="drawing-style-bar"
          class="fixed z-40 -translate-x-1/2 flex items-center gap-1 px-2 py-1 rounded-md border border-[color:var(--color-border)] bg-[color:var(--color-bg-elev)] shadow-lg"
          style={{ left: `${b().left + b().width / 2}px`, top: `${b().top + 8}px` }}
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
                value={style().fill ?? DEFAULT_FILL}
                label="Fill color"
                onChange={(c) => props.onChange({ fill: c }, "fill")}
              />
            </Show>
            <Show when={!isText()}>
              <span class="w-px h-5 bg-[color:var(--color-border)] mx-1" />
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
              <span class="w-px h-5 bg-[color:var(--color-border)] mx-1" />
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
                        stroke-dasharray={
                          k === "solid" ? undefined : k === "dashed" ? "5 3" : "1 3"
                        }
                      />
                    </svg>
                  </button>
                )}
              </For>
            </Show>
            <Show when={isText()}>
              <span class="w-px h-5 bg-[color:var(--color-border)] mx-1" />
              <select
                aria-label="Text size"
                class="bg-transparent text-xs border border-[color:var(--color-border)] rounded px-1 py-1 text-[color:var(--color-fg)] disabled:opacity-40"
                value={String(style().fontSize ?? 12)}
                onChange={(e) =>
                  props.onChange({ fontSize: Number(e.currentTarget.value) }, "size")
                }
              >
                <For each={FONT_SIZES}>
                  {(n) => (
                    <option value={String(n)} selected={(style().fontSize ?? 12) === n}>
                      {n}
                    </option>
                  )}
                </For>
              </select>
            </Show>
          </fieldset>
        </div>
      )}
    </Show>
  );
}
