// Right-side watchlist panel (TradingView style): Symbol / Last / Chg /
// Chg% per row, polled via `get_quotes` while the tab is visible. Chg and
// Chg% are colored bull/bear and the Last cell flashes on each tick.
// Clicking a row switches the chart symbol; "+" reuses the symbol search
// dialog, × on hover removes a row. Collapses to a thin edge strip, and
// polling stops while collapsed. The header switches between named lists
// and the read-only Flagged view; lists hold collapsible sections, rows
// reorder by pointer drag or Alt+Arrow, and a right-click sets a color flag
// (ADR-0031).

import { Dialog, DropdownMenu } from "@kobalte/core";
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
import { subscribeSignInNotice } from "../oauth";
import { createQuotesQuery } from "../queries";
import { showToast } from "../services/alertEngine";
import { copyText } from "../services/shareLink";
import { setSymbol, symbol } from "../stores/layout";
import {
  FLAGGED_ID,
  FLAG_COLORS,
  MAX_LISTS,
  MAX_NAME_LENGTH,
  MAX_SECTIONS,
  activeItems,
  activeList,
  activeListId,
  addSection,
  addToWatchlist,
  createList,
  deleteList,
  duplicateList,
  flags,
  isFlaggedView,
  moveItem,
  moveItemBy,
  removeFromWatchlist,
  removeSection,
  renameList,
  renameSection,
  setActiveList,
  setFlag,
  toggleSection,
  watchlist,
  watchlists,
  type FlagColor,
  type WatchItem,
} from "../stores/watchlist";
import type { Quote, QuoteError } from "../types/responses";
import { openAlertDialog } from "./AlertDialog";
import { openContextMenu } from "./ContextMenu";
import { SymbolSearch } from "./SymbolSearch";

/** How long a Last cell stays highlighted after a tick. */
const FLASH_MS = 700;

export interface WatchlistProps {
  session: McpSession;
  /** Gates polling — true once auth is ready. */
  enabled: boolean;
}

const iconButtonClass =
  "inline-flex items-center justify-center w-7 h-7 rounded-md cursor-pointer text-[color:var(--color-fg-muted)] hover:text-[color:var(--color-fg)] hover:bg-[color:var(--color-bg-elev)]";
const menuClass =
  "bg-[color:var(--color-bg-elev)] border border-[color:var(--color-border)] rounded-md shadow-lg z-50 py-1 min-w-48";
const itemClass =
  "px-3 py-1.5 text-sm cursor-pointer flex items-center justify-between gap-6 outline-none data-[highlighted]:bg-[color:var(--color-border)] data-[disabled]:opacity-40 data-[disabled]:cursor-not-allowed";
const dialogClass =
  "pointer-events-auto w-80 bg-[color:var(--color-bg)] border border-[color:var(--color-border)] rounded-lg shadow-xl p-4";
const inputClass =
  "w-full bg-[color:var(--color-bg-elev)] border border-[color:var(--color-border)] rounded-md px-3 py-2 text-sm text-[color:var(--color-fg)] focus:outline-none focus:border-[color:var(--color-accent)]";
const actionClass =
  "px-3 py-1.5 rounded-md text-sm cursor-pointer border border-[color:var(--color-border)] hover:bg-[color:var(--color-bg-elev)]";
const primaryClass =
  "px-3 py-1.5 rounded-md text-sm cursor-pointer bg-[color:var(--color-accent)] text-black font-semibold";

function SectionIcon(): JSX.Element {
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
      <path d="M3 4.5h12M3 13.5h12M6 9h6" />
    </svg>
  );
}

function FlagIcon(): JSX.Element {
  return (
    <svg
      width="12"
      height="12"
      viewBox="0 0 12 12"
      fill="currentColor"
      aria-hidden="true"
    >
      <path d="M2 1v10h1V7.5h6.5L8 4.2 9.5 1z" />
    </svg>
  );
}

type Dialogue =
  | {
      kind: "name";
      title: string;
      label: string;
      initial: string;
      submit: (name: string) => void;
    }
  | { kind: "delete"; id: string; name: string };

export function Watchlist(props: WatchlistProps): JSX.Element {
  const [signInNotice, setSignInNotice] = createSignal<string | null>(null);
  onCleanup(subscribeSignInNotice(setSignInNotice));
  const [searchOpen, setSearchOpen] = createSignal(false);
  const [dialogue, setDialogue] = createSignal<Dialogue | null>(null);
  const [flagMenu, setFlagMenu] = createSignal<{
    symbol: string;
    x: number;
    y: number;
  } | null>(null);
  // Drop target while dragging: insert before the item at this index.
  const [dropBefore, setDropBefore] = createSignal<number | null>(null);

  const query = createQuotesQuery(props.session, () => ({
    symbols: watchlist(),
    enabled: props.enabled,
  }));

  const quotes = createMemo(
    () => new Map((query.data?.quotes ?? []).map((q) => [q.symbol, q])),
  );

  const items = createMemo(() => activeItems());
  /** Items on screen: symbols inside a collapsed section are hidden. */
  const visible = createMemo(() => {
    let hidden = false;
    return items().filter((i) => {
      if (i.kind === "section") {
        hidden = i.collapsed;
        return true;
      }
      return !hidden;
    });
  });
  const indexOf = (item: WatchItem) => items().indexOf(item);
  /** A drop at the very end is drawn under the last row on screen. */
  const isLastDrop = (item: WatchItem) =>
    dropBefore() === items().length && visible().at(-1) === item;
  const readOnly = () => isFlaggedView();
  const listName = () =>
    readOnly() ? "Flagged" : activeList().name;

  const askName = (
    title: string,
    label: string,
    initial: string,
    submit: (name: string) => void,
  ) => setDialogue({ kind: "name", title, label, initial, submit });

  function startDrag(e: PointerEvent, from: number) {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    const grip = e.currentTarget as HTMLElement;
    grip.setPointerCapture(e.pointerId);
    const target = (ev: PointerEvent): number | null => {
      const row = document
        .elementFromPoint(ev.clientX, ev.clientY)
        ?.closest<HTMLElement>("[data-wl-index]");
      if (!row) return null;
      const i = Number(row.dataset.wlIndex);
      const r = row.getBoundingClientRect();
      if (ev.clientY < r.top + r.height / 2) return i;
      // Below a collapsed header, skip its hidden symbols.
      const it = items()[i];
      if (it?.kind === "section" && it.collapsed) {
        let end = i + 1;
        while (items()[end]?.kind === "symbol") end++;
        return end;
      }
      return i + 1;
    };
    const move = (ev: PointerEvent) => setDropBefore(target(ev));
    // One teardown for every way a drag ends; only a pointerup commits.
    const finish = (ev: Event) => {
      grip.removeEventListener("pointermove", move);
      grip.removeEventListener("pointerup", finish);
      grip.removeEventListener("pointercancel", finish);
      grip.removeEventListener("lostpointercapture", finish);
      window.removeEventListener("keydown", onKey, true);
      const before = ev.type === "pointerup" ? target(ev as PointerEvent) : null;
      setDropBefore(null);
      if (before !== null) moveItem(from, before);
    };
    const onKey = (ev: KeyboardEvent) => {
      if (ev.key !== "Escape") return;
      ev.stopPropagation();
      finish(ev);
    };
    grip.addEventListener("pointermove", move);
    grip.addEventListener("pointerup", finish);
    grip.addEventListener("pointercancel", finish);
    grip.addEventListener("lostpointercapture", finish);
    window.addEventListener("keydown", onKey, true);
  }

  function onRowKeyDown(e: KeyboardEvent, item: WatchItem) {
    if (!e.altKey || (e.key !== "ArrowUp" && e.key !== "ArrowDown")) return;
    e.preventDefault();
    if (readOnly()) return;
    const row = e.currentTarget as HTMLElement;
    const key =
      item.kind === "symbol"
        ? `[data-symbol="${CSS.escape(item.symbol)}"]`
        : `[data-section="${CSS.escape(item.id)}"]`;
    moveItemBy(indexOf(item), e.key === "ArrowUp" ? -1 : 1);
    // Moving the node in the DOM drops focus; put it back.
    queueMicrotask(() =>
      row.parentElement?.querySelector<HTMLElement>(key)?.focus(),
    );
  }

  function openFlagMenu(symbol: string, x: number, y: number) {
    setFlagMenu({ symbol, x, y });
  }

  return (
    <aside class="watchlist flex flex-col min-h-0 flex-1" aria-label="Watchlist">
      <div class="flex items-center gap-1 px-3 py-2 border-b border-[color:var(--color-border)]">
        <DropdownMenu.Root>
          <DropdownMenu.Trigger
            class="flex-1 min-w-0 flex items-center gap-1.5 px-1.5 py-1 -ml-1.5 rounded-md text-sm font-semibold cursor-pointer hover:bg-[color:var(--color-bg-elev)]"
            aria-label={`Watchlist: ${listName()}`}
            title="Switch watchlist"
          >
            <span class="truncate" data-testid="watchlist-name">
              {listName()}
            </span>
            <DropdownMenu.Icon aria-hidden="true">▾</DropdownMenu.Icon>
          </DropdownMenu.Trigger>
          <DropdownMenu.Portal>
            <DropdownMenu.Content class={menuClass}>
              <For each={watchlists()}>
                {(l) => (
                  <DropdownMenu.Item
                    class={itemClass}
                    data-list-id={l.id}
                    onSelect={() => setActiveList(l.id)}
                  >
                    <span class="truncate">{l.name}</span>
                    <Show when={l.id === activeListId()}>
                      <span aria-label="current">✓</span>
                    </Show>
                  </DropdownMenu.Item>
                )}
              </For>
              <DropdownMenu.Separator class="my-1 border-t border-[color:var(--color-border)]" />
              <DropdownMenu.Item
                class={itemClass}
                onSelect={() => setActiveList(FLAGGED_ID)}
              >
                Flagged
                <Show when={readOnly()}>
                  <span aria-label="current">✓</span>
                </Show>
              </DropdownMenu.Item>
              <DropdownMenu.Separator class="my-1 border-t border-[color:var(--color-border)]" />
              <DropdownMenu.Item
                class={itemClass}
                disabled={watchlists().length >= MAX_LISTS}
                onSelect={() =>
                  askName("Create new list", "List name", "", (name) => {
                    createList(name);
                  })
                }
              >
                Create new list…
              </DropdownMenu.Item>
              <DropdownMenu.Item
                class={itemClass}
                disabled={readOnly()}
                onSelect={() =>
                  askName(
                    "Rename list",
                    "List name",
                    activeList().name,
                    (name) => renameList(activeList().id, name),
                  )
                }
              >
                Rename…
              </DropdownMenu.Item>
              <DropdownMenu.Item
                class={itemClass}
                disabled={readOnly() || watchlists().length >= MAX_LISTS}
                onSelect={() => duplicateList(activeList().id)}
              >
                Duplicate
              </DropdownMenu.Item>
              <DropdownMenu.Item
                class={itemClass}
                disabled={readOnly() || watchlists().length <= 1}
                onSelect={() =>
                  setDialogue({
                    kind: "delete",
                    id: activeList().id,
                    name: activeList().name,
                  })
                }
              >
                Delete…
              </DropdownMenu.Item>
            </DropdownMenu.Content>
          </DropdownMenu.Portal>
        </DropdownMenu.Root>
        <button
          type="button"
          class={`${iconButtonClass} text-lg leading-none`}
          aria-label="Add symbol"
          title="Add symbol"
          disabled={readOnly()}
          onClick={() => setSearchOpen(true)}
        >
          +
        </button>
        <button
          type="button"
          class={iconButtonClass}
          aria-label="Add section"
          title="Add section"
          disabled={
            readOnly() ||
            items().filter((i) => i.kind === "section").length >=
              MAX_SECTIONS
          }
          onClick={() =>
            askName("Add section", "Section name", "", (name) => {
              addSection(name);
            })
          }
        >
          <SectionIcon />
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
        <For each={visible()}>
          {(item) =>
            item.kind === "section" ? (
              <SectionRow
                section={item}
                index={indexOf(item)}
                readOnly={readOnly()}
                dropBefore={dropBefore() === indexOf(item)}
                dropAfter={isLastDrop(item)}
                onGrip={startDrag}
                onKeyDown={(e) => onRowKeyDown(e, item)}
                onRename={() =>
                  askName("Rename section", "Section name", item.name, (n) =>
                    renameSection(item.id, n),
                  )
                }
              />
            ) : (
              <WatchlistRow
                symbol={item.symbol}
                index={indexOf(item)}
                quote={quotes().get(item.symbol)}
                active={symbol() === item.symbol}
                flag={flags()[item.symbol]}
                readOnly={readOnly()}
                dropBefore={dropBefore() === indexOf(item)}
                dropAfter={isLastDrop(item)}
                onGrip={startDrag}
                onKeyDown={(e) => onRowKeyDown(e, item)}
                onFlagMenu={openFlagMenu}
              />
            )
          }
        </For>
        <Show when={items().length === 0}>
          <p class="col-span-full px-3 py-3 text-xs text-[color:var(--color-fg-muted)]">
            {readOnly()
              ? "No flagged symbols. Right-click a row to flag it."
              : "This list is empty. Use + to add a symbol."}
          </p>
        </Show>
        <Show when={signInNotice()}>
          <p
            class="col-span-full px-3 py-2 text-xs text-[color:var(--color-fg-muted)]"
            data-testid="signin-notice"
          >
            {signInNotice()}
          </p>
        </Show>
        <Show when={query.isError && !signInNotice()}>
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
      <Show when={flagMenu()}>
        {(m) => <FlagMenu menu={m()} close={() => setFlagMenu(null)} />}
      </Show>
      <Dialog.Root
        open={dialogue() !== null}
        onOpenChange={(open) => !open && setDialogue(null)}
        modal
      >
        <Dialog.Portal>
          <Dialog.Overlay class="fixed inset-0 z-40 bg-black/40" />
          <div class="fixed inset-0 z-50 flex items-start justify-center pt-[20vh] px-4 pointer-events-none">
            <Dialog.Content class={dialogClass}>
              {(() => {
                const d = dialogue();
                if (d === null) return null;
                if (d.kind === "name") {
                  return <NameForm d={d} close={() => setDialogue(null)} />;
                }
                return (
                  <>
                    <Dialog.Title class="text-sm font-semibold mb-1">
                      Delete list
                    </Dialog.Title>
                    <Dialog.Description class="text-sm text-[color:var(--color-fg-muted)] mb-4">
                      Delete “{d.name}”? This cannot be undone.
                    </Dialog.Description>
                    <div class="flex justify-end gap-2">
                      <button
                        type="button"
                        class={actionClass}
                        onClick={() => setDialogue(null)}
                      >
                        Cancel
                      </button>
                      <button
                        type="button"
                        class="px-3 py-1.5 rounded-md text-sm cursor-pointer bg-[color:var(--color-danger)] text-white font-semibold"
                        onClick={() => {
                          deleteList(d.id);
                          setDialogue(null);
                        }}
                      >
                        Delete
                      </button>
                    </div>
                  </>
                );
              })()}
            </Dialog.Content>
          </div>
        </Dialog.Portal>
      </Dialog.Root>
    </aside>
  );
}

/** Name prompt shared by Create, Rename and Add section. */
function NameForm(props: {
  d: Extract<Dialogue, { kind: "name" }>;
  close: () => void;
}): JSX.Element {
  const [value, setValue] = createSignal(props.d.initial);
  const valid = () => value().trim().length > 0;
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        if (!valid()) return;
        props.d.submit(value());
        props.close();
      }}
    >
      <Dialog.Title class="text-sm font-semibold mb-2">
        {props.d.title}
      </Dialog.Title>
      <input
        type="text"
        autofocus
        aria-label={props.d.label}
        maxlength={MAX_NAME_LENGTH}
        autocomplete="off"
        spellcheck={false}
        class={inputClass}
        value={value()}
        onInput={(e) => setValue(e.currentTarget.value)}
        onFocus={(e) => e.currentTarget.select()}
      />
      <div class="flex justify-end gap-2 mt-4">
        <button type="button" class={actionClass} onClick={props.close}>
          Cancel
        </button>
        <button
          type="submit"
          class={`${primaryClass} disabled:opacity-50`}
          disabled={!valid()}
        >
          OK
        </button>
      </div>
    </form>
  );
}

/** Color picker popover for a row's flag, closed by Escape or an outside press. */
function FlagMenu(props: {
  menu: { symbol: string; x: number; y: number };
  close: () => void;
}): JSX.Element {
  let el!: HTMLDivElement;
  const onDown = (e: PointerEvent) => {
    if (!el.contains(e.target as Node)) props.close();
  };
  // Closing by keyboard or pick returns focus to the row; an outside press
  // keeps the focus where the user clicked.
  const closeAndRefocus = () => {
    const symbol = props.menu.symbol;
    props.close();
    // After the app's own "focus fell to <body>" recovery (keys.ts).
    setTimeout(() => {
      document
        .querySelector<HTMLElement>(
          `.wl-row[data-symbol="${CSS.escape(symbol)}"]`,
        )
        ?.focus();
    }, 0);
  };
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape") closeAndRefocus();
  };
  const onMenuKey = (e: KeyboardEvent) => {
    const keys = ["ArrowRight", "ArrowDown", "ArrowLeft", "ArrowUp", "Home", "End"];
    if (!keys.includes(e.key)) return;
    e.preventDefault();
    const items = [...el.querySelectorAll<HTMLElement>("button")];
    const at = items.indexOf(document.activeElement as HTMLElement);
    const next =
      e.key === "Home"
        ? 0
        : e.key === "End"
          ? items.length - 1
          : (at + (e.key === "ArrowRight" || e.key === "ArrowDown" ? 1 : -1) +
              items.length) %
            items.length;
    items[next]?.focus();
  };
  window.addEventListener("pointerdown", onDown, true);
  window.addEventListener("keydown", onKey);
  onCleanup(() => {
    window.removeEventListener("pointerdown", onDown, true);
    window.removeEventListener("keydown", onKey);
  });
  const pick = (color: FlagColor | null) => {
    setFlag(props.menu.symbol, color);
    closeAndRefocus();
  };
  return (
    <div
      ref={(n) => {
        el = n;
        // On the next task: opened by Enter on a menu item, focused any sooner its
        // first swatch would get that key's click.
        setTimeout(() => n.querySelector<HTMLElement>("button")?.focus());
      }}
      role="menu"
      aria-label={`Flag ${props.menu.symbol}`}
      onKeyDown={onMenuKey}
      class="fixed z-50 flex items-center gap-1.5 p-2 rounded-md shadow-lg bg-[color:var(--color-bg-elev)] border border-[color:var(--color-border)]"
      style={{
        left: `${Math.min(props.menu.x, window.innerWidth - 240)}px`,
        top: `${Math.min(props.menu.y, window.innerHeight - 48)}px`,
      }}
    >
      <For each={FLAG_COLORS}>
        {(c) => (
          <button
            type="button"
            role="menuitem"
            class="wl-swatch"
            data-flag={c}
            aria-label={`Flag ${c}`}
            title={c}
            onClick={() => pick(c)}
          />
        )}
      </For>
      <button
        type="button"
        role="menuitem"
        class="wl-swatch wl-swatch-none"
        aria-label="No flag"
        title="No flag"
        onClick={() => pick(null)}
      >
        ×
      </button>
    </div>
  );
}

interface SectionRowProps {
  section: Extract<WatchItem, { kind: "section" }>;
  index: number;
  readOnly: boolean;
  dropBefore: boolean;
  dropAfter: boolean;
  onGrip: (e: PointerEvent, index: number) => void;
  onKeyDown: (e: KeyboardEvent) => void;
  onRename: () => void;
}

function SectionRow(props: SectionRowProps): JSX.Element {
  return (
    <div
      role="row"
      data-section={props.section.id}
      data-wl-index={props.index}
      tabindex="0"
      class="wl-section"
      classList={{
        "wl-drop-before": props.dropBefore,
        "wl-drop-after": props.dropAfter,
      }}
      onKeyDown={props.onKeyDown}
      onContextMenu={(e) =>
        openContextMenu(e, {
          label: `Section ${props.section.name} actions`,
          items: [
            {
              label: props.section.collapsed ? "Expand" : "Collapse",
              onSelect: () => toggleSection(props.section.id),
            },
            { label: "Rename…", onSelect: props.onRename },
            null,
            {
              label: "Remove section",
              danger: true,
              onSelect: () => removeSection(props.section.id),
            },
          ],
        })
      }
    >
      <span role="cell" aria-colspan="4" class="wl-section-cell">
      <button
        type="button"
        class="wl-section-toggle"
        aria-expanded={!props.section.collapsed}
        aria-label={`Section ${props.section.name}`}
        onClick={() => toggleSection(props.section.id)}
      >
        <span aria-hidden="true">{props.section.collapsed ? "▸" : "▾"}</span>
        <span class="truncate">{props.section.name}</span>
      </button>
      <button
        type="button"
        class="wl-row-action"
        aria-label={`Rename section ${props.section.name}`}
        title="Rename section"
        onClick={props.onRename}
      >
        ✎
      </button>
      <button
        type="button"
        class="wl-row-action"
        aria-label={`Remove section ${props.section.name}`}
        title="Remove section (keeps its symbols)"
        onClick={() => removeSection(props.section.id)}
      >
        ×
      </button>
      </span>
      <Show when={!props.readOnly}>
        <button
          type="button"
          class="wl-row-action wl-grip"
          aria-label={`Drag section ${props.section.name} to reorder`}
          title="Drag to reorder (Alt+Arrow with the row focused)"
          onPointerDown={(e) => props.onGrip(e, props.index)}
          onClick={(e) => e.stopPropagation()}
        >
          ⋮⋮
        </button>
      </Show>
    </div>
  );
}

interface WatchlistRowProps {
  symbol: string;
  index: number;
  quote: Quote | QuoteError | undefined;
  active: boolean;
  flag: FlagColor | undefined;
  readOnly: boolean;
  dropBefore: boolean;
  dropAfter: boolean;
  onGrip: (e: PointerEvent, index: number) => void;
  onKeyDown: (e: KeyboardEvent) => void;
  onFlagMenu: (symbol: string, x: number, y: number) => void;
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
      data-wl-index={props.index}
      data-flag={props.flag}
      tabindex="0"
      aria-current={props.active ? "true" : undefined}
      class="wl-row wl-grid px-3 py-1.5 text-sm cursor-pointer"
      classList={{
        "wl-drop-before": props.dropBefore,
        "wl-drop-after": props.dropAfter,
      }}
      onClick={() => setSymbol(props.symbol)}
      onKeyDown={props.onKeyDown}
      onContextMenu={(e) => {
        const { clientX: x, clientY: y } = e;
        openContextMenu(e, {
          label: `${props.symbol} actions`,
          items: [
            { label: `Open ${props.symbol} in chart`, onSelect: () => setSymbol(props.symbol) },
            { label: `Add alert on ${props.symbol}…`, onSelect: () => openAlertDialog({ symbol: props.symbol }) },
            { label: "Flag…", onSelect: () => props.onFlagMenu(props.symbol, x, y) },
            {
              label: "Copy symbol",
              onSelect: () =>
                void copyText(props.symbol).then((ok) => showToast(ok ? "Symbol copied" : "Copy failed")),
            },
            ...(props.readOnly
              ? []
              : [
                  null,
                  {
                    label: "Remove from watchlist",
                    danger: true,
                    onSelect: () => removeFromWatchlist(props.symbol),
                  },
                ]),
          ],
        });
      }}
      title={error() ?? undefined}
    >
      <span role="cell" class="wl-symbol min-w-0" title={props.symbol}>
        <button
          type="button"
          class="font-semibold cursor-pointer text-left truncate max-w-full"
          aria-label={`Show ${props.symbol} on the chart`}
        >
          {props.symbol}
        </button>
        {/* Absolutely positioned at the symbol cell's edges, clear of the numbers (CSS). */}
        <Show when={!props.readOnly}>
          <button
            type="button"
            class="wl-row-action wl-grip"
            aria-label={`Drag ${props.symbol} to reorder`}
            title="Drag to reorder (Alt+Arrow with the row focused)"
            onPointerDown={(e) => props.onGrip(e, props.index)}
            onClick={(e) => e.stopPropagation()}
          >
            ⋮⋮
          </button>
        </Show>
        <button
          type="button"
          class="wl-row-action wl-flag"
          aria-label={`Flag ${props.symbol}`}
          title="Set flag"
          onClick={(e) => {
            e.stopPropagation();
            const r = e.currentTarget.getBoundingClientRect();
            props.onFlagMenu(props.symbol, r.left, r.bottom + 4);
          }}
        >
          <FlagIcon />
        </button>
        <Show when={!props.readOnly}>
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
        </Show>
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
