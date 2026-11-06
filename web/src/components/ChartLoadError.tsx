// Shown over the chart area when the chart request failed (a mistyped
// symbol, a provider outage). The header only carries a compact error badge;
// the explanation, Retry and a way back to the last chart that loaded live
// here. The failed symbol stays active, as in TradingView.

import { Show, type JSX } from "solid-js";

import { loadErrorReason } from "../lib/chartLoadError";

export interface ChartLoadErrorProps {
  symbol: string;
  /** Raw error message of the failed request. */
  message: string;
  /** Symbol of the last chart that loaded, if different from `symbol`. */
  previousSymbol: string | null;
  onRetry: () => void;
  onBack: (symbol: string) => void;
  /** Take these indicators off the chart. */
  onRemoveIndicators: (names: string[]) => void;
}

const buttonClass =
  "px-3 py-1.5 rounded-md text-sm cursor-pointer border border-[color:var(--color-border)] hover:bg-[color:var(--color-bg-elev)]";

export function ChartLoadError(props: ChartLoadErrorProps): JSX.Element {
  // The raw message stays as the tooltip.
  const reason = () => loadErrorReason(props.message);
  return (
    <div
      role="alert"
      data-testid="chart-load-error"
      class="absolute inset-0 z-10 flex flex-col items-center justify-center gap-3 px-4 text-center bg-[color:var(--color-bg)]"
    >
      <p class="text-sm text-[color:var(--color-fg)]" title={props.message}>
        Couldn't load {props.symbol} — {reason().text}
      </p>
      <div class="flex flex-wrap items-center justify-center gap-2">
        <Show when={reason().indicators.length > 0}>
          <button
            type="button"
            data-testid="chart-error-remove"
            class={`${buttonClass} bg-[color:var(--color-accent)] text-black font-semibold border-transparent hover:bg-[color:var(--color-accent)]`}
            onClick={() => props.onRemoveIndicators(reason().indicators)}
          >
            Remove {reason().indicators.join(", ")}
          </button>
        </Show>
        <Show when={props.previousSymbol}>
          {(prev) => (
            <button
              type="button"
              class={`${buttonClass} bg-[color:var(--color-accent)] text-black font-semibold border-transparent hover:bg-[color:var(--color-accent)]`}
              onClick={() => props.onBack(prev())}
            >
              Back to {prev()}
            </button>
          )}
        </Show>
        <button type="button" class={buttonClass} onClick={props.onRetry}>
          Retry
        </button>
      </div>
    </div>
  );
}
