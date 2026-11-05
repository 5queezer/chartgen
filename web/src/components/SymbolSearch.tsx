// Symbol search dialog. Queries the `search_symbols` tool (all Binance spot
// pairs and Yahoo instruments) after a short debounce, with category tabs
// that filter by instrument type. While results load or the search fails
// the curated catalogue answers instantly, recent symbols and the curated
// list show when the query is empty, and any free-text ticker (uppercased)
// stays available as the last row. Fully keyboard driven: ArrowUp/Down
// move, Enter picks, Esc closes (Kobalte Dialog).

import { Dialog } from "@kobalte/core";
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

import { CATEGORIES, buildRows, type Category } from "../lib/symbolSearch";
import type { McpSession } from "../mcp";
import { recentSymbols } from "../stores/layout";
import type { SearchResult } from "../types/responses";

/** Pause after the last keystroke before the server is queried. */
const SEARCH_DEBOUNCE_MS = 200;
/** Longest query `search_symbols` accepts. */
const MAX_QUERY_LENGTH = 40;

interface Remote {
  /** Query and category the results belong to. */
  key: string;
  results: SearchResult[];
  warnings: string[];
}

export interface SymbolSearchProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Text the input starts with (e.g. the letter that opened the dialog). */
  initialQuery: string;
  /** Dialog title; defaults to "Symbol search". */
  title?: string;
  onSelect: (ticker: string) => void;
  session: Pick<McpSession, "callSearchSymbols">;
}

export function SymbolSearch(props: SymbolSearchProps): JSX.Element {
  let input!: HTMLInputElement;
  let previouslyFocused: HTMLElement | null = null;
  const [query, setQuery] = createSignal("");
  const [active, setActive] = createSignal(0);
  const [category, setCategory] = createSignal<Category>(CATEGORIES[0]!);
  const [remote, setRemote] = createSignal<Remote | null>(null);
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal("");

  createEffect(() => {
    if (props.open) {
      setQuery(props.initialQuery);
      setCategory(CATEGORIES[0]!);
      setActive(0);
    }
  });

  const searchKey = () => `${category().id}|${query().trim()}`;

  // Debounced server search; changing the query or tab cancels the pending
  // timer and the in-flight request, so outdated answers never land.
  createEffect(() => {
    const q = query().trim();
    const cat = category();
    const key = searchKey();
    setError("");
    if (!props.open || q === "" || q.length > MAX_QUERY_LENGTH) {
      setRemote(null);
      setLoading(false);
      return;
    }
    setLoading(true);
    const ctrl = new AbortController();
    const timer = setTimeout(() => {
      props.session
        .callSearchSymbols(
          { query: q, types: cat.types.length ? [...cat.types] : undefined },
          ctrl.signal,
        )
        .then((r) => {
          if (ctrl.signal.aborted) return;
          setRemote({ key, results: r.results, warnings: r.warnings ?? [] });
          setLoading(false);
        })
        .catch((e: unknown) => {
          if (ctrl.signal.aborted) return;
          setError(e instanceof Error ? e.message : String(e));
          setLoading(false);
        });
    }, SEARCH_DEBOUNCE_MS);
    onCleanup(() => {
      clearTimeout(timer);
      ctrl.abort();
    });
  });

  const current = () => {
    const r = remote();
    return r && r.key === searchKey() ? r : null;
  };

  // New results replace the rows under the highlight, so Enter must never
  // pick a row other than the one shown: restart at the top.
  createEffect(
    on(
      () => current()?.key ?? null,
      () => setActive(0),
      { defer: true },
    ),
  );

  const rows = createMemo(() =>
    buildRows({
      query: query(),
      category: category(),
      recents: recentSymbols(),
      remote: current()?.results ?? null,
    }),
  );

  // Keep the highlighted row inside the list after the filter shrinks.
  createEffect(() => {
    if (active() >= rows().length) setActive(0);
  });

  const pick = (ticker: string) => {
    props.onSelect(ticker);
    props.onOpenChange(false);
  };

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.isComposing) return;
    const n = rows().length;
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setActive((i) => (i + 1) % n);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive((i) => (i - 1 + n) % n);
    } else if (e.key === "Enter") {
      e.preventDefault();
      const row = rows()[active()];
      if (row) pick(row.ticker);
    }
  };

  // Scroll the highlighted row into view after each move.
  createEffect(() => {
    active();
    document
      .querySelector("[data-symbol-row][aria-selected='true']")
      ?.scrollIntoView({ block: "nearest" });
  });

  return (
    <Dialog.Root open={props.open} onOpenChange={props.onOpenChange} modal>
      <Dialog.Portal>
        <Dialog.Overlay class="fixed inset-0 z-40 bg-black/60" />
        <div class="fixed inset-0 z-50 flex items-start justify-center pt-[12vh] px-4 pointer-events-none">
          <Dialog.Content
            class="pointer-events-auto w-full max-w-lg max-h-[70vh] flex flex-col bg-[color:var(--color-bg)] border border-[color:var(--color-border)] rounded-lg shadow-xl"
            onOpenAutoFocus={(e) => {
              e.preventDefault();
              previouslyFocused =
                document.activeElement instanceof HTMLElement
                  ? document.activeElement
                  : null;
              input.focus();
              input.setSelectionRange(input.value.length, input.value.length);
            }}
            onCloseAutoFocus={(e) => {
              e.preventDefault();
              previouslyFocused?.focus();
            }}
          >
            <div class="flex items-center justify-between px-4 pt-3">
              <Dialog.Title class="text-sm font-semibold">
                {props.title ?? "Symbol search"}
              </Dialog.Title>
              <Dialog.CloseButton
                class="text-[color:var(--color-fg-muted)] hover:text-[color:var(--color-fg)] cursor-pointer text-lg leading-none"
                aria-label="Close"
              >
                ×
              </Dialog.CloseButton>
            </div>
            <div class="px-4 pt-3 pb-2 border-b border-[color:var(--color-border)]">
              <input
                ref={input}
                type="text"
                role="combobox"
                aria-expanded="true"
                aria-controls="symbol-rows"
                aria-activedescendant={`symbol-row-${active()}`}
                aria-label="Search symbol or type any ticker"
                autocomplete="off"
                spellcheck={false}
                placeholder="Search symbol or type any ticker"
                value={query()}
                onInput={(e) => {
                  setQuery(e.currentTarget.value);
                  setActive(0);
                }}
                onKeyDown={onKeyDown}
                class="w-full bg-[color:var(--color-bg-elev)] border border-[color:var(--color-border)] rounded-md px-3 py-2 text-sm text-[color:var(--color-fg)] focus:outline-none focus:border-[color:var(--color-accent)]"
              />
              <div
                role="tablist"
                aria-label="Symbol type"
                class="flex flex-wrap gap-1 mt-2"
              >
                <For each={CATEGORIES}>
                  {(c) => (
                    <button
                      type="button"
                      role="tab"
                      aria-selected={c.id === category().id}
                      // Keep focus in the input so typing and arrows keep working.
                      onMouseDown={(e) => e.preventDefault()}
                      onClick={() => {
                        setCategory(c);
                        setActive(0);
                        input.focus();
                      }}
                      class="px-2.5 py-1 rounded-full text-xs cursor-pointer border border-[color:var(--color-border)] text-[color:var(--color-fg-muted)] hover:text-[color:var(--color-fg)] aria-selected:bg-[color:var(--color-bg-elev)] aria-selected:text-[color:var(--color-accent)] aria-selected:border-[color:var(--color-accent)]"
                    >
                      {c.label}
                    </button>
                  )}
                </For>
              </div>
              <p
                role="status"
                class="mt-2 min-h-4 text-xs text-[color:var(--color-fg-muted)]"
              >
                <Show when={loading()}>Searching…</Show>
                <Show when={error()}>
                  <span class="text-[color:var(--color-danger)]">
                    Search unavailable ({error()}); showing built-in symbols.
                  </span>
                </Show>
                <Show when={!loading() && !error() && current()?.warnings.length}>
                  Some results may be missing: {current()?.warnings.join("; ")}
                </Show>
              </p>
            </div>
            <ul
              id="symbol-rows"
              role="listbox"
              class="overflow-y-auto py-1 flex-1 min-h-0"
            >
              <For each={rows()}>
                {(row, i) => (
                  <>
                    <Show when={i() === 0 || rows()[i() - 1]?.section !== row.section}>
                      <li
                        role="presentation"
                        class="px-4 pt-2 pb-1 text-xs uppercase tracking-wide text-[color:var(--color-fg-muted)]"
                      >
                        {row.section}
                      </li>
                    </Show>
                    <li
                      id={`symbol-row-${i()}`}
                      role="option"
                      data-symbol-row
                      aria-selected={i() === active()}
                      onMouseMove={() => setActive(i())}
                      onClick={() => pick(row.ticker)}
                      class="flex items-center gap-3 px-4 py-1.5 text-sm cursor-pointer aria-selected:bg-[color:var(--color-bg-elev)]"
                    >
                      <span class="font-semibold w-28 shrink-0 truncate">
                        {row.ticker}
                      </span>
                      <span
                        class="flex-1 truncate text-[color:var(--color-fg-muted)]"
                        title={row.name}
                      >
                        {row.name}
                      </span>
                      <Show when={row.exchange}>
                        <span class="text-xs text-[color:var(--color-fg-muted)] shrink-0">
                          {row.exchange}
                        </span>
                      </Show>
                      <Show when={row.badge}>
                        <span class="text-xs text-[color:var(--color-fg-muted)] border border-[color:var(--color-border)] rounded px-1.5 py-0.5 shrink-0">
                          {row.badge}
                        </span>
                      </Show>
                    </li>
                  </>
                )}
              </For>
            </ul>
          </Dialog.Content>
        </div>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
