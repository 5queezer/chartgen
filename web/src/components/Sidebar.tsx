// Right sidebar with TradingView-style icon tabs: Watchlist | Alerts | Data
// Window | Object Tree. The
// collapsed state is the watchlist store's, so the edge strip is the
// Watchlist's own and expanding it returns to the last tab. The tab is kept
// in localStorage. Switching tabs unmounts the other panel, so the
// watchlist only polls quotes while its tab is showing.

import { For, Show, createSignal, type JSX } from "solid-js";

import type { McpSession } from "../mcp";
import { alerts } from "../stores/alerts";
import { watchlistCollapsed } from "../stores/watchlist";
import { AlertsPanel } from "./AlertsPanel";
import { DataWindow, type DataWindowProps } from "./DataWindow";
import { ObjectTree, type ObjectTreeProps } from "./ObjectTree";
import { Watchlist } from "./Watchlist";

const TABS = ["watchlist", "alerts", "data", "tree"] as const;
type Tab = (typeof TABS)[number];
const LABELS: Record<Tab, string> = {
  watchlist: "Watchlist",
  alerts: "Alerts",
  data: "Data Window",
  tree: "Object Tree",
};
// 16x16 stroke paths.
const ICONS: Record<Tab, string> = {
  watchlist: "M2 4h12M2 8h12M2 12h8",
  alerts: "M8 2a4 4 0 0 0-4 4v3l-1.5 2.5h11L12 9V6a4 4 0 0 0-4-4zM6.5 13.5a1.5 1.5 0 0 0 3 0",
  data: "M2 3h12v10H2zM2 6.5h12M6 6.5V13",
  tree: "M3 3h4M6 8h4M6 13h4M4.5 4v9M4.5 8H6M4.5 13H6",
};
const KEY = "chartgen.sidebar.tab";

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

export interface SidebarProps {
  session: McpSession;
  /** Gates polling — true once auth is ready. */
  enabled: boolean;
  /** What the Data Window and Object Tree read from the chart. */
  panels: DataWindowProps & ObjectTreeProps;
}

export function Sidebar(props: SidebarProps): JSX.Element {
  const activeCount = () => alerts().filter((a) => a.active).length;

  return (
    <Show
      when={!watchlistCollapsed()}
      fallback={<Watchlist session={props.session} enabled={props.enabled} />}
    >
      <aside
        class="w-72 shrink-0 flex flex-col min-h-0 border-l border-[color:var(--color-border)]"
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
                onClick={() => setSidebarTab(t)}
              >
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
                  <path d={ICONS[t]} />
                </svg>
                <span class="sr-only">{LABELS[t]}</span>
                <Show when={t === "alerts" && activeCount() > 0}>
                  <span class="ml-1.5 text-xs text-[color:var(--color-fg-muted)]">
                    {activeCount()}
                  </span>
                </Show>
              </button>
            )}
          </For>
        </div>
        <Show when={tab() === "watchlist"}>
          <Watchlist embedded session={props.session} enabled={props.enabled} />
        </Show>
        <Show when={tab() === "alerts"}>
          <AlertsPanel />
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
