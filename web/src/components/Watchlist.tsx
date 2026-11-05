// Right-side watchlist panel (TradingView style): Symbol / Last / Chg /
// Chg% per row, polled via `get_quotes` while the tab is visible. Chg and
// Chg% are colored bull/bear and the Last cell flashes on each tick.
// Clicking a row switches the chart symbol; "+" reuses the symbol search
// dialog, × on hover removes a row. Collapses to a thin edge strip, and
// polling stops while collapsed.

import {
  For,
  Show,
  createEffect,
  createMemo,
  createSignal,
  on,
  onCleanup,
  type JSX,
} from "solid-js";

import {
  formatChange,
  formatChangePct,
  formatPrice,
  trend,
} from "../lib/quote";
import type { McpSession } from "../mcp";
import { createQuotesQuery } from "../queries";
import { setSymbol, symbol } from "../stores/layout";
import {
  addToWatchlist,
  removeFromWatchlist,
  setWatchlistCollapsed,
  watchlist,
  watchlistCollapsed,
} from "../stores/watchlist";
import type { Quote, QuoteError } from "../types/responses";
import { SymbolSearch } from "./SymbolSearch";

/** How long a Last cell stays highlighted after a tick. */
const FLASH_MS = 700;

export interface WatchlistProps {
  session: McpSession;
  /** Gates polling — true once auth is ready. */
  enabled: boolean;
  /** Inside the sidebar: it supplies the width and left border. */
  embedded?: boolean;
}

const iconButtonClass =
  "inline-flex items-center justify-center w-7 h-7 rounded-md cursor-pointer text-[color:var(--color-fg-muted)] hover:text-[color:var(--color-fg)] hover:bg-[color:var(--color-bg-elev)]";

function ListIcon(): JSX.Element {
  return (
    <svg
      width="18"
      height="18"
      viewBox="0 0 18 18"
      fill="none"
      stroke="currentColor"
      stroke-width="1.5"
      aria-hidden="true"
    >
      <path d="M3 4.5h12M3 9h12M3 13.5h12" />
    </svg>
  );
}

export function Watchlist(props: WatchlistProps): JSX.Element {
  const [searchOpen, setSearchOpen] = createSignal(false);

  const query = createQuotesQuery(props.session, () => ({
    symbols: watchlist(),
    enabled: props.enabled && !watchlistCollapsed(),
  }));

  const quotes = createMemo(
    () => new Map((query.data?.quotes ?? []).map((q) => [q.symbol, q])),
  );

  return (
    <Show
      when={!watchlistCollapsed()}
      fallback={
        <aside
          class="w-10 shrink-0 flex flex-col items-center py-2 border-l border-[color:var(--color-border)]"
          aria-label="Watchlist (collapsed)"
        >
          <button
            type="button"
            class={iconButtonClass}
            aria-label="Expand watchlist"
            title="Watchlist"
            onClick={() => setWatchlistCollapsed(false)}
          >
            <ListIcon />
          </button>
        </aside>
      }
    >
      <aside
        class="watchlist flex flex-col min-h-0"
        classList={{
          "flex-1": props.embedded,
          "w-80 shrink-0 border-l border-[color:var(--color-border)]":
            !props.embedded,
        }}
        aria-label="Watchlist"
      >
        <div class="flex items-center gap-1 px-3 py-2 border-b border-[color:var(--color-border)]">
          <h2 class="text-sm font-semibold flex-1">Watchlist</h2>
          <button
            type="button"
            class={`${iconButtonClass} text-lg leading-none`}
            aria-label="Add symbol"
            title="Add symbol"
            onClick={() => setSearchOpen(true)}
          >
            +
          </button>
          <button
            type="button"
            class={iconButtonClass}
            aria-label="Collapse watchlist"
            title="Collapse watchlist"
            onClick={() => setWatchlistCollapsed(true)}
          >
            <span aria-hidden="true">»</span>
          </button>
        </div>
        <div role="table" aria-label="Watchlist" class="wl-table flex-1 min-h-0 overflow-y-auto">
          <div
            role="row"
            class="wl-grid px-3 py-1 text-xs text-[color:var(--color-fg-muted)]"
          >
            <span role="columnheader">Symbol</span>
            <span role="columnheader" class="text-right">Last</span>
            <span role="columnheader" class="text-right">Chg</span>
            <span role="columnheader" class="text-right">Chg%</span>
          </div>
          <For each={watchlist()}>
            {(sym) => (
              <WatchlistRow
                symbol={sym}
                quote={quotes().get(sym)}
                active={symbol() === sym}
              />
            )}
          </For>
          <Show when={query.isError}>
            <p class="col-span-full px-3 py-2 text-xs text-[color:var(--color-danger)]">
              Quotes unavailable: {query.error?.message}
            </p>
          </Show>
        </div>
        <SymbolSearch
          open={searchOpen()}
          onOpenChange={setSearchOpen}
          initialQuery=""
          title="Add symbol"
          onSelect={addToWatchlist}
          session={props.session}
        />
      </aside>
    </Show>
  );
}

interface WatchlistRowProps {
  symbol: string;
  quote: Quote | QuoteError | undefined;
  active: boolean;
}

function WatchlistRow(props: WatchlistRowProps): JSX.Element {
  const quote = () =>
    props.quote && !("error" in props.quote) ? props.quote : null;
  const error = () =>
    props.quote && "error" in props.quote ? props.quote.error : null;
  const last = () => quote()?.last ?? null;
  const change = () => quote()?.change ?? null;
  const dir = () => trend(change());

  // Flash the Last cell when the price ticks; the first value never flashes.
  const [flash, setFlash] = createSignal<"up" | "down" | null>(null);
  let timer: ReturnType<typeof setTimeout> | undefined;
  createEffect(
    on(last, (next, prev) => {
      if (next === null || prev === null || prev === undefined || next === prev) {
        return;
      }
      setFlash(next > prev ? "up" : "down");
      clearTimeout(timer);
      timer = setTimeout(() => setFlash(null), FLASH_MS);
    }),
  );
  onCleanup(() => clearTimeout(timer));

  const tone = () =>
    dir() === "up"
      ? "text-[color:var(--color-bull)]"
      : dir() === "down"
        ? "text-[color:var(--color-bear)]"
        : "";

  return (
    <div
      role="row"
      data-symbol={props.symbol}
      aria-current={props.active ? "true" : undefined}
      class="wl-row wl-grid px-3 py-1.5 text-sm cursor-pointer"
      onClick={() => setSymbol(props.symbol)}
      title={error() ?? undefined}
    >
      <span role="cell" class="min-w-0 truncate" title={props.symbol}>
        <button
          type="button"
          class="font-semibold cursor-pointer text-left truncate max-w-full"
          aria-label={`Show ${props.symbol} on the chart`}
        >
          {props.symbol}
        </button>
        {/* Absolutely positioned over the row's right edge (CSS). */}
        <button
          type="button"
          class="wl-remove"
          aria-label={`Remove ${props.symbol} from watchlist`}
          title="Remove"
          onClick={(e) => {
            e.stopPropagation();
            removeFromWatchlist(props.symbol);
          }}
        >
          ×
        </button>
      </span>
      <span
        role="cell"
        class="wl-last text-right"
        classList={{
          "wl-flash-up": flash() === "up",
          "wl-flash-down": flash() === "down",
          "text-[color:var(--color-fg-muted)]": error() !== null,
        }}
        data-flash={flash() ?? undefined}
      >
        {formatPrice(last())}
      </span>
      <span role="cell" class={`text-right ${tone()}`}>
        {formatChange(change(), last())}
      </span>
      <span role="cell" class={`text-right ${tone()}`}>
        {formatChangePct(quote()?.change_pct ?? null)}
      </span>
    </div>
  );
}
