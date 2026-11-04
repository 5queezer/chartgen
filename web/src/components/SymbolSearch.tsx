// Symbol search dialog. Filters the curated catalogue by ticker/name,
// lists recent symbols when the query is empty, and accepts any free-text
// ticker (uppercased) since there is no search backend. Fully keyboard
// driven: ArrowUp/Down move, Enter picks, Esc closes (Kobalte Dialog).

import { Dialog } from "@kobalte/core";
import {
  For,
  Show,
  createEffect,
  createMemo,
  createSignal,
  type JSX,
} from "solid-js";

import { SYMBOLS, searchSymbols, type SymbolEntry } from "../lib/symbols";
import { recentSymbols } from "../stores/layout";

export interface SymbolSearchProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Text the input starts with (e.g. the letter that opened the dialog). */
  initialQuery: string;
  onSelect: (ticker: string) => void;
}

interface Row {
  section: string;
  ticker: string;
  name: string;
  badge: string;
}

function toRow(section: string, s: SymbolEntry): Row {
  return {
    section,
    ticker: s.ticker,
    name: s.name,
    badge: `${s.kind} · ${s.exchange}`,
  };
}

export function SymbolSearch(props: SymbolSearchProps): JSX.Element {
  let input!: HTMLInputElement;
  const [query, setQuery] = createSignal("");
  const [active, setActive] = createSignal(0);

  createEffect(() => {
    if (props.open) {
      setQuery(props.initialQuery);
      setActive(0);
    }
  });

  const rows = createMemo<Row[]>(() => {
    const q = query().trim();
    if (q === "") {
      const recent = recentSymbols().map((t): Row => {
        const known = SYMBOLS.find((s) => s.ticker === t);
        return known
          ? toRow("Recent", known)
          : { section: "Recent", ticker: t, name: "", badge: "" };
      });
      return [...recent, ...SYMBOLS.map((s) => toRow("Popular", s))];
    }
    const matches = searchSymbols(q).map((s) => toRow("Symbols", s));
    const upper = q.toUpperCase();
    if (matches.some((r) => r.ticker === upper)) return matches;
    return [
      ...matches,
      {
        section: "Other",
        ticker: upper,
        name: `Use "${upper}" as typed`,
        badge: "custom",
      },
    ];
  });

  // Keep the highlighted row inside the list after the filter shrinks.
  createEffect(() => {
    if (active() >= rows().length) setActive(0);
  });

  const pick = (ticker: string) => {
    props.onSelect(ticker);
    props.onOpenChange(false);
  };

  const onKeyDown = (e: KeyboardEvent) => {
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
              input.focus();
              input.setSelectionRange(input.value.length, input.value.length);
            }}
          >
            <div class="flex items-center justify-between px-4 pt-3">
              <Dialog.Title class="text-sm font-semibold">
                Symbol search
              </Dialog.Title>
              <Dialog.CloseButton
                class="text-[color:var(--color-fg-muted)] hover:text-[color:var(--color-fg)] cursor-pointer text-lg leading-none"
                aria-label="Close"
              >
                ×
              </Dialog.CloseButton>
            </div>
            <div class="px-4 py-3 border-b border-[color:var(--color-border)]">
              <input
                ref={input}
                type="text"
                role="combobox"
                aria-expanded="true"
                aria-controls="symbol-rows"
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
                      <span class="flex-1 truncate text-[color:var(--color-fg-muted)]">
                        {row.name}
                      </span>
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
