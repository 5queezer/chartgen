// Color swatch that opens a small TradingView-like palette popover: a grid of
// preset colors, an opacity slider and a native color input for custom
// colors. Emits `#rrggbb` when opaque and `rgba(r, g, b, a)` otherwise, the
// forms the store accepts (ADR-0017).

import { Popover } from "@kobalte/core";
import { createSignal, For, type JSX } from "solid-js";

import { formatColor, parseColor } from "../lib/color";

const PRESETS = [
  "#ffffff", "#d1d4dc", "#b2b5be", "#787b86", "#5d606b",
  "#434651", "#2a2e39", "#1e222d", "#131722", "#000000",
  "#f23645", "#ff9800", "#ffeb3b", "#4caf50", "#089981",
  "#00bcd4", "#2962ff", "#673ab7", "#9c27b0", "#e91e63",
  "#fccbcd", "#ffe0b2", "#fff9c4", "#c8e6c9", "#b2dfdb",
  "#b2ebf2", "#bbd9fb", "#d1c4e9", "#e1bee7", "#f8bbd0",
] as const;

let closeOpenPopover: (() => void) | null = null;

/**
 * Close the open palette popover; false when none is open. For callers whose
 * Escape handling runs ahead of Kobalte's (a focused tab swallows the key).
 */
export function closeColorPopover(): boolean {
  if (closeOpenPopover === null) return false;
  closeOpenPopover();
  return true;
}

export interface ColorPickerProps {
  /** Current color (`#rgb`, `#rrggbb`, `#rrggbbaa` or `rgba()`). */
  value: string;
  /** Accessible name of the swatch button, e.g. "RSI color". */
  label: string;
  onChange: (color: string) => void;
}

export function ColorPicker(props: ColorPickerProps): JSX.Element {
  const parsed = () => parseColor(props.value);
  const [open, setOpen] = createSignal(false);
  const close = () => {
    setOpen(false);
    closeOpenPopover = null;
  };

  return (
    <Popover.Root
      placement="bottom-start"
      gutter={6}
      open={open()}
      onOpenChange={(o) => {
        if (o) {
          setOpen(true);
          closeOpenPopover = close;
        } else close();
      }}
    >
      <Popover.Trigger
        aria-label={props.label}
        class="w-6 h-6 rounded border border-[color:var(--color-border)] cursor-pointer p-0 overflow-hidden"
        style={{
          background: `linear-gradient(${props.value}, ${props.value}), repeating-conic-gradient(#808080 0% 25%, #fff 0% 50%) 0 0 / 8px 8px`,
        }}
        data-testid="color-swatch"
      />
      <Popover.Portal>
        <Popover.Content class="z-[60] w-56 p-3 flex flex-col gap-3 bg-[color:var(--color-bg)] border border-[color:var(--color-border)] rounded-lg shadow-xl">
          <div class="grid grid-cols-10 gap-1">
            <For each={PRESETS}>
              {(c) => (
                <button
                  type="button"
                  aria-label={`Color ${c}`}
                  class="w-4 h-4 rounded-sm border border-[color:var(--color-border)] cursor-pointer p-0"
                  classList={{
                    "ring-2 ring-[color:var(--color-accent)]": parsed().hex === c,
                  }}
                  style={{ background: c }}
                  onClick={() =>
                    props.onChange(
                      formatColor(c, parsed().alpha === 0 ? 1 : parsed().alpha),
                    )
                  }
                />
              )}
            </For>
          </div>
          <label class="flex items-center gap-2 min-w-0 text-xs text-[color:var(--color-fg-muted)]">
            Opacity
            <input
              type="range"
              min="0"
              max="100"
              aria-label="Opacity"
              value={Math.round(parsed().alpha * 100)}
              onInput={(e) =>
                props.onChange(
                  formatColor(parsed().hex, Number(e.currentTarget.value) / 100),
                )
              }
              class="flex-1 min-w-0"
            />
            <span class="w-10 shrink-0 text-right tabular-nums">
              {Math.round(parsed().alpha * 100)}%
            </span>
          </label>
          <label class="flex items-center justify-between text-xs text-[color:var(--color-fg-muted)]">
            Custom
            <input
              type="color"
              aria-label="Custom color"
              value={parsed().hex}
              onInput={(e) =>
                props.onChange(formatColor(e.currentTarget.value, parsed().alpha))
              }
              class="w-8 h-6 bg-transparent cursor-pointer"
            />
          </label>
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  );
}
