// Shown over the chart area when the chart request failed (a mistyped
// symbol, a provider outage). The header only carries a compact error badge;
// the explanation, Retry and a way back to the last chart that loaded live
// here. The failed symbol stays active, as in TradingView.

import { Show, type JSX } from "solid-js";

export interface ChartLoadErrorProps {
  symbol: string;
  /** Raw error message of the failed request. */
  message: string;
  /** Symbol of the last chart that loaded, if different from `symbol`. */
  previousSymbol: string | null;
  onRetry: () => void;
  onBack: (symbol: string) => void;
}

/** Short reason for the headline; the raw message is the tooltip. */
function reason(message: string): string {
  return /\b404\b|not found|no data|unknown symbol|invalid symbol/i.test(message)
    ? "symbol not found"
    : "request failed";
}

const buttonClass =
  "px-3 py-1.5 rounded-md text-sm cursor-pointer border border-[color:var(--color-border)] hover:bg-[color:var(--color-bg-elev)]";

export function ChartLoadError(props: ChartLoadErrorProps): JSX.Element {
  return (
    <div
      role="alert"
      data-testid="chart-load-error"
      class="absolute inset-0 z-10 flex flex-col items-center justify-center gap-3 px-4 text-center bg-[color:var(--color-bg)]"
    >
      <p class="text-sm text-[color:var(--color-fg)]" title={props.message}>
        Couldn't load {props.symbol} — {reason(props.message)}
      </p>
      <div class="flex items-center gap-2">
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
