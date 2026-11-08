// Indicators are managed from their legend rows (PaneHeader). An instance the
// chart reports as failed has no payload, so no row: it is listed here, beneath
// the header, with the reason as text (touch has no tooltips) and a way to
// remove it.

import { createMemo, For, Show, type JSX } from "solid-js";

import { chipName } from "../lib/indicatorNames";
import { indicatorName, instanceKey, type ActiveIndicator } from "../stores/indicators";
import type { IndicatorError } from "../types/responses";

export interface IndicatorErrorsProps {
  entries: ActiveIndicator[];
  /** Remove the instance with this key. */
  onRemove: (key: string) => void;
  /** Why an instance is missing from the chart, by instance key (`indicator_errors`). */
  errors?: Record<string, IndicatorError> | undefined;
}

export function IndicatorErrors(props: IndicatorErrorsProps): JSX.Element {
  const failed = createMemo(() =>
    props.entries.flatMap((entry) => {
      const key = instanceKey(entry);
      const error = props.errors?.[key];
      return error ? [{ key, name: indicatorName(entry), message: error.message }] : [];
    }),
  );
  return (
    <Show when={failed().length > 0}>
      <ul
        data-testid="indicator-errors"
        class="shrink-0 max-h-24 overflow-y-auto border-b border-[color:var(--color-danger)] text-xs"
      >
        <For each={failed()}>
          {(item) => (
            <li class="flex items-center gap-2 pl-3 pr-1">
              <span class="min-w-0 flex-1 py-1 break-words">
                <span class="font-mono text-[color:var(--color-fg)]">{chipName(item.name)}</span>
                <span class="text-[color:var(--color-danger)]"> {item.message}</span>
              </span>
              <button
                type="button"
                aria-label={`Remove ${item.name}`}
                onClick={() => props.onRemove(item.key)}
                class="shrink-0 min-w-10 min-h-10 cursor-pointer text-[color:var(--color-fg-muted)]"
              >
                ×
              </button>
            </li>
          )}
        </For>
      </ul>
    </Show>
  );
}
