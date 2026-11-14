// Right sidebar with TradingView-style icon tabs: Watchlist, Alerts, Setups, Data
// Window and Object Tree. The collapsed state is the watchlist store's, so
// the rail of tab icons is shown instead of the panel. Clicking the selected
// tab collapses; clicking a rail icon opens that tab. The tab is kept in localStorage. Switching tabs unmounts the other
// panel, so the watchlist only polls quotes while its tab is showing.

import { For, Show, createEffect, createSignal, on, onCleanup, onMount, type JSX } from "solid-js";

import type { McpSession } from "../mcp";
import { compact } from "../lib/viewport";
import { alerts } from "../stores/alerts";
import { serverAlerts } from "../stores/serverAlerts";
import { symbol } from "../stores/layout";
import { setWatchlistCollapsed, watchlistCollapsed } from "../stores/watchlist";
import { AlertsPanel } from "./AlertsPanel";
import { DataWindow, type DataWindowProps } from "./DataWindow";
import { ObjectTree, type ObjectTreeProps } from "./ObjectTree";
import { SetupsPanel } from "./SetupsPanel";
import { Watchlist } from "./Watchlist";

const TABS = ["watchlist", "alerts", "setups", "data", "tree"] as const;
type Tab = (typeof TABS)[number];
const LABELS: Record<Tab, string> = {
  watchlist: "Watchlist",
  alerts: "Alerts",
  setups: "Setups",
  data: "Data Window",
  tree: "Object Tree",
};
// 16x16 stroke paths.
const ICONS: Record<Tab, string> = {
  watchlist: "M2 4h12M2 8h12M2 12h8",
  alerts: "M8 2a4 4 0 0 0-4 4v3l-1.5 2.5h11L12 9V6a4 4 0 0 0-4-4zM6.5 13.5a1.5 1.5 0 0 0 3 0",
  // A crosshair on a target: the levels a setup is scored against.
  setups: "M8 1.5v3M8 11.5v3M1.5 8h3M11.5 8h3M8 4.5a3.5 3.5 0 1 0 0 7a3.5 3.5 0 1 0 0-7",
  data: "M2 3h12v10H2zM2 6.5h12M6 6.5V13",
  tree: "M3 3h4M6 8h4M6 13h4M4.5 4v9M4.5 8H6M4.5 13H6",
};
const KEY = "chartgen.sidebar.tab";

const SidebarIcon = (props: { tab: Tab }) => (
  <svg
    aria-hidden="true"
    viewBox="0 0 16 16"
    width="16"
    height="16"
    fill="none"
    stroke="currentColor"
    stroke-width="1.5"
    stroke-linecap="round"
    stroke-linejoin="round"
  >
    <path d={ICONS[props.tab]} />
  </svg>
);

function loadTab(): Tab {
  try {
    const v = typeof localStorage === "undefined" ? null : localStorage.getItem(KEY);
    return TABS.find((t) => t === v) ?? "watchlist";
  } catch {
    return "watchlist";
  }
}

const [tab, setTabSignal] = createSignal<Tab>(loadTab());

/** Switch tab and remember it. */
export function setSidebarTab(next: Tab): void {
  setTabSignal(next);
  try {
    localStorage.setItem(KEY, next);
  } catch {
    // Best-effort only.
  }
}

/** Open the sidebar on a tab. */
function openTab(t: Tab): void {
  setSidebarTab(t);
  setWatchlistCollapsed(false);
}

export interface SidebarProps {
  session: McpSession;
  /** Gates polling — true once auth is ready. */
  enabled: boolean;
  /** What the Data Window and Object Tree read from the chart. */
  panels: DataWindowProps & ObjectTreeProps;
}

export function Sidebar(props: SidebarProps): JSX.Element {
    const activeCount = () =>
    alerts().filter((a) => a.active).length + serverAlerts().filter((a) => a.active).length;
  const sheetShown = () => compact() && !watchlistCollapsed();
  const closeSheet = () => setWatchlistCollapsed(true);

  // Picking a symbol in the sheet returns to the chart.
  createEffect(on(symbol, () => sheetShown() && closeSheet(), { defer: true }));
  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !e.defaultPrevented && sheetShown()) closeSheet();
    };
    window.addEventListener("keydown", onKey);
    onCleanup(() => window.removeEventListener("keydown", onKey));
  });

  return (
    <Show
      when={!watchlistCollapsed()}
      fallback={
        <Show when={!compact()}>
          <nav
            class="w-10 shrink-0 flex flex-col items-center py-2 gap-1 border-l border-[color:var(--color-border)]"
            aria-label="Sidebar (collapsed)"
          >
            <For each={TABS}>
              {(t) => (
                <button
                  type="button"
                  class="relative inline-flex items-center justify-center w-7 h-7 rounded-md cursor-pointer text-[color:var(--color-fg-muted)] hover:text-[color:var(--color-fg)] hover:bg-[color:var(--color-bg-elev)]"
                  aria-label={LABELS[t]}
                  title={LABELS[t]}
                  onClick={() => openTab(t)}
                >
                  <SidebarIcon tab={t} />
                  <Show when={t === "alerts" && activeCount() > 0}>
                    <span class="absolute -top-0.5 -right-0.5 text-[10px] leading-none text-[color:var(--color-fg)]">
                      {activeCount()}
                    </span>
                  </Show>
                </button>
              )}
            </For>
          </nav>
        </Show>
      }
    >
      <Show when={compact()}>
        <div class="sheet-backdrop" data-testid="sheet-backdrop" onClick={closeSheet} />
      </Show>
      <aside
        class="sidebar w-80 shrink-0 flex flex-col min-h-0 border-l border-[color:var(--color-border)]"
        aria-label="Sidebar"
      >
        <div
          role="tablist"
          aria-label="Sidebar panels"
          class="flex border-b border-[color:var(--color-border)]"
        >
          <For each={TABS}>
            {(t) => (
              <button
                type="button"
                role="tab"
                aria-selected={tab() === t}
                class="flex-1 px-3 py-2 text-sm cursor-pointer inline-flex items-center justify-center border-b-2 border-transparent text-[color:var(--color-fg-muted)] aria-selected:text-[color:var(--color-fg)] aria-selected:border-[color:var(--color-accent)]"
                title={LABELS[t]}
                onClick={() => (tab() === t && !compact() ? setWatchlistCollapsed(true) : setSidebarTab(t))}
              >
                <SidebarIcon tab={t} />
                <span class="sr-only">{LABELS[t]}</span>
                <Show when={t === "alerts" && activeCount() > 0}>
                  <span class="ml-1.5 text-xs text-[color:var(--color-fg-muted)]">
                    {activeCount()}
                  </span>
                </Show>
              </button>
            )}
          </For>
          <Show when={compact()}>
            <button
              type="button"
              class="px-3 cursor-pointer text-lg leading-none text-[color:var(--color-fg-muted)]"
              aria-label="Close panel"
              onClick={closeSheet}
            >
              ×
            </button>
          </Show>
        </div>
        <Show when={tab() === "watchlist"}>
          <Watchlist session={props.session} enabled={props.enabled} />
        </Show>
        <Show when={tab() === "alerts"}>
          <AlertsPanel />
        </Show>
        <Show when={tab() === "setups"}>
          <SetupsPanel session={props.session} enabled={props.enabled} />
        </Show>
        <Show when={tab() === "data"}>
          <DataWindow {...props.panels} />
        </Show>
        <Show when={tab() === "tree"}>
          <ObjectTree {...props.panels} />
        </Show>
      </aside>
    </Show>
  );
}

/** Compact shell: the row of buttons that opens the sheet's panels and the drawing tools. */
export function SidebarDock(props: { drawActive: boolean; onDraw: () => void }): JSX.Element {
  return (
    <nav class="sidebar-dock flex items-center justify-around" aria-label="Panels">
      <button
        type="button"
        aria-label="Draw"
        aria-pressed={props.drawActive}
        class="aria-pressed:text-[color:var(--color-accent)]"
        onClick={props.onDraw}
      >
        <svg aria-hidden="true" viewBox="0 0 16 16" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
          <path d="M3 13l1-3.5L11 2.5 13.5 5l-7 7zM9.5 4l2.5 2.5" />
        </svg>
      </button>
      <For each={TABS}>
        {(t) => (
          <button
            type="button"
            aria-label={LABELS[t]}
            onClick={() => openTab(t)}
          >
            <SidebarIcon tab={t} />
          </button>
        )}
      </For>
    </nav>
  );
}
