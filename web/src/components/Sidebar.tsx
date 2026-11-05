// Right sidebar with TradingView-style tabs: Watchlist | Alerts. The
// collapsed state is the watchlist store's, so the edge strip is the
// Watchlist's own and expanding it returns to the last tab. The tab is kept
// in localStorage. Switching tabs unmounts the other panel, so the
// watchlist only polls quotes while its tab is showing.

import { For, Show, createSignal, type JSX } from "solid-js";

import type { McpSession } from "../mcp";
import { alerts } from "../stores/alerts";
import { watchlistCollapsed } from "../stores/watchlist";
import { AlertsPanel } from "./AlertsPanel";
import { Watchlist } from "./Watchlist";

const TABS = ["watchlist", "alerts"] as const;
type Tab = (typeof TABS)[number];
const LABELS: Record<Tab, string> = { watchlist: "Watchlist", alerts: "Alerts" };
const KEY = "chartgen.sidebar.tab";

function loadTab(): Tab {
  try {
    const v = typeof localStorage === "undefined" ? null : localStorage.getItem(KEY);
    return v === "alerts" ? "alerts" : "watchlist";
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
                class="flex-1 px-3 py-2 text-sm cursor-pointer border-b-2 border-transparent text-[color:var(--color-fg-muted)] aria-selected:text-[color:var(--color-fg)] aria-selected:border-[color:var(--color-accent)]"
                onClick={() => setSidebarTab(t)}
              >
                {LABELS[t]}
                <Show when={t === "alerts" && activeCount() > 0}>
                  <span class="ml-1.5 text-xs text-[color:var(--color-fg-muted)]">
                    {activeCount()}
                  </span>
                </Show>
              </button>
            )}
          </For>
        </div>
        <Show when={tab() === "alerts"} fallback={<Watchlist embedded session={props.session} enabled={props.enabled} />}>
          <AlertsPanel />
        </Show>
      </aside>
    </Show>
  );
}
