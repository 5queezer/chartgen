// Row of chips, one per active indicator, with hide/show, settings and
// remove buttons. Hidden indicators are dimmed. Rendered beneath the header.
// The empty state is a muted hint pointing at the picker button. A repeated
// indicator (ADR-0028) shows its params so its chips read apart.

import { createMemo, Index, Show, type JSX } from "solid-js";

import { chipName } from "../lib/indicatorNames";
import {
  indicatorName,
  instanceKey,
  isHidden,
  paramLabels,
  type ActiveIndicator,
} from "../stores/indicators";
import { paramDefaults } from "../stores/layouts";

export interface IndicatorChipsProps {
  entries: ActiveIndicator[];
  /** Remove the instance with this key. */
  onRemove: (key: string) => void;
  /** Toggle visibility of the entry at `index`. */
  onToggleHidden: (index: number) => void;
  /** Open settings for the entry at `index`. */
  onSettings: (index: number) => void;
}

const chipButton =
  "chip-btn inline-flex items-center justify-center w-4 h-4 rounded-full text-[color:var(--color-fg-muted)] hover:text-[color:var(--color-fg)] hover:bg-[color:var(--color-bg)] cursor-pointer";

export function IndicatorChips(props: IndicatorChipsProps): JSX.Element {
  const labels = createMemo(() => paramLabels(props.entries, paramDefaults(), true));
  return (
    <div class="flex flex-wrap items-center gap-2 px-4 py-2 border-b border-[color:var(--color-border)] min-h-[2.5rem] compact:hidden">
      <Show
        when={props.entries.length > 0}
        fallback={
          <span class="text-xs text-[color:var(--color-fg-muted)]">
            No indicators — click Indicators to add.
          </span>
        }
      >
        <Index each={props.entries}>
          {(entry, index) => {
            const params = () => labels().get(instanceKey(entry()));
            const shown = () => chipName(indicatorName(entry()), params());
            const name = () =>
              [indicatorName(entry()), params()].filter(Boolean).join(" ");
            return (
              <span
                class="inline-flex items-center gap-1 bg-[color:var(--color-bg-elev)] border border-[color:var(--color-border)] rounded-full pl-1.5 pr-1 py-0.5 text-xs font-mono text-[color:var(--color-fg)]"
                classList={{ "opacity-50": isHidden(entry()) }}
              >
                <button
                  type="button"
                  aria-label={`${isHidden(entry()) ? "Show" : "Hide"} ${name()}`}
                  aria-pressed={isHidden(entry())}
                  onClick={() => props.onToggleHidden(index)}
                  class={chipButton}
                >
                  <svg aria-hidden="true" viewBox="0 0 16 16" width="12" height="12" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M1 8 C3 4.5 5.5 3 8 3 S13 4.5 15 8 C13 11.5 10.5 13 8 13 S3 11.5 1 8 Z" />
                    <circle cx="8" cy="8" r="2" />
                    <Show when={isHidden(entry())}>
                      <path d="M2 14 L14 2" />
                    </Show>
                  </svg>
                </button>
                <span
                  title={name()}
                  classList={{ "line-through": isHidden(entry()) }}
                >
                  {shown()}
                </span>
                <button
                  type="button"
                  aria-label={`Settings for ${name()}`}
                  onClick={() => props.onSettings(index)}
                  class={chipButton}
                >
                  <svg aria-hidden="true" viewBox="0 0 16 16" width="12" height="12" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
                    <circle cx="8" cy="8" r="2" />
                    <path d="M8 1.5 V3.5 M8 12.5 V14.5 M1.5 8 H3.5 M12.5 8 H14.5 M3.4 3.4 L4.8 4.8 M11.2 11.2 L12.6 12.6 M3.4 12.6 L4.8 11.2 M11.2 4.8 L12.6 3.4" />
                  </svg>
                </button>
                <button
                  type="button"
                  aria-label={`Remove ${name()}`}
                  onClick={() => props.onRemove(instanceKey(entry()))}
                  class={`${chipButton} hover:!text-[color:var(--color-danger)]`}
                >
                  ×
                </button>
              </span>
            );
          }}
        </Index>
      </Show>
    </div>
  );
}
