// Row of removable chips, one per active indicator. Rendered beneath the
// header. The empty state is a muted hint pointing at the picker button.

import { For, Show, type JSX } from "solid-js";

import {
  indicatorName,
  type ActiveIndicator,
} from "../stores/indicators";

export interface IndicatorChipsProps {
  entries: ActiveIndicator[];
  onRemove: (name: string) => void;
}

export function IndicatorChips(props: IndicatorChipsProps): JSX.Element {
  return (
    <div class="flex flex-wrap items-center gap-2 px-4 py-2 border-b border-[color:var(--color-border)] min-h-[2.5rem]">
      <Show
        when={props.entries.length > 0}
        fallback={
          <span class="text-xs text-[color:var(--color-fg-muted)]">
            No indicators — click Indicators to add.
          </span>
        }
      >
        <For each={props.entries}>
          {(entry) => {
            const name = indicatorName(entry);
            return (
              <span class="inline-flex items-center gap-1.5 bg-[color:var(--color-bg-elev)] border border-[color:var(--color-border)] rounded-full pl-2.5 pr-1 py-0.5 text-xs font-mono text-[color:var(--color-fg)]">
                {name}
                <button
                  type="button"
                  aria-label={`Remove ${name}`}
                  onClick={() => props.onRemove(name)}
                  class="inline-flex items-center justify-center w-4 h-4 rounded-full text-[color:var(--color-fg-muted)] hover:text-[color:var(--color-danger)] hover:bg-[color:var(--color-bg)] cursor-pointer"
                >
                  ×
                </button>
              </span>
            );
          }}
        </For>
      </Show>
    </div>
  );
}
