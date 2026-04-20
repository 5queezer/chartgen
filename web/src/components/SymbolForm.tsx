// Ticker + timeframe form, built from Kobalte primitives. Kobalte
// gives accessible keyboard/screen-reader behavior by default; we
// layer Tailwind utility classes on top, matching the dark theme.

import { Select, TextField } from "@kobalte/core";
import { type JSX } from "solid-js";

export const TIMEFRAME_OPTIONS = [
  "1m",
  "5m",
  "15m",
  "1h",
  "4h",
  "1d",
  "1wk",
] as const;
export type Timeframe = (typeof TIMEFRAME_OPTIONS)[number];

export interface SymbolFormProps {
  ticker: string;
  timeframe: Timeframe;
  onTickerChange: (next: string) => void;
  onTimeframeChange: (next: Timeframe) => void;
  onSubmit: () => void;
  disabled: boolean;
}

const inputClass =
  "bg-[color:var(--color-bg-elev)] border border-[color:var(--color-border)] rounded-md px-3 py-1.5 text-sm text-[color:var(--color-fg)] focus:outline-none focus:border-[color:var(--color-accent)]";

const buttonClass =
  "bg-[color:var(--color-accent)] text-black px-4 py-1.5 rounded-md text-sm font-medium hover:opacity-90 disabled:opacity-50 disabled:cursor-not-allowed";

export function SymbolForm(props: SymbolFormProps): JSX.Element {
  const onSubmit = (e: Event) => {
    e.preventDefault();
    if (!props.disabled) props.onSubmit();
  };

  return (
    <form class="flex items-center gap-3" onSubmit={onSubmit}>
      <TextField.Root
        value={props.ticker}
        onChange={(v) => props.onTickerChange(v)}
        class="flex flex-col"
      >
        <TextField.Label class="text-xs text-[color:var(--color-fg-muted)] mb-0.5">
          Ticker
        </TextField.Label>
        <TextField.Input class={`${inputClass} w-32`} autocomplete="off" />
      </TextField.Root>

      <Select.Root<Timeframe>
        value={props.timeframe}
        onChange={(v) => {
          if (v !== null) props.onTimeframeChange(v);
        }}
        options={TIMEFRAME_OPTIONS as unknown as Timeframe[]}
        placeholder="timeframe"
        itemComponent={(itemProps) => (
          <Select.Item
            item={itemProps.item}
            class="px-3 py-1.5 text-sm cursor-pointer hover:bg-[color:var(--color-bg-elev)]"
          >
            <Select.ItemLabel>{itemProps.item.rawValue}</Select.ItemLabel>
          </Select.Item>
        )}
      >
        <div class="flex flex-col">
          <Select.Label class="text-xs text-[color:var(--color-fg-muted)] mb-0.5">
            Timeframe
          </Select.Label>
          <Select.Trigger class={`${inputClass} w-24 text-left`}>
            <Select.Value<Timeframe>>
              {(state) => state.selectedOption()}
            </Select.Value>
          </Select.Trigger>
        </div>
        <Select.Portal>
          <Select.Content class="bg-[color:var(--color-bg-elev)] border border-[color:var(--color-border)] rounded-md shadow-lg z-50">
            <Select.Listbox />
          </Select.Content>
        </Select.Portal>
      </Select.Root>

      <button
        type="submit"
        class={buttonClass}
        disabled={props.disabled}
        // Use a plain <button> here; Kobalte's Button primitive adds
        // polymorphic overhead we don't need for a simple submit.
      >
        Load
      </button>
    </form>
  );
}
