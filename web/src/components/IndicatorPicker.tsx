// "Add Indicator" modal. Kobalte Dialog hosts a search input and a
// per-category Accordion listing every indicator returned by
// `list_indicators`. Clicking a row adds the indicator with its default
// parameters (from `configurable_params`) and leaves the modal open for
// multi-add; removal happens via the chip row, not here.
//
// The accordion is normally all-collapsed. When the search field is
// non-empty we force every category that contains a match open, so the
// user doesn't have to click through empty sections to see their hits.

import { Accordion, Dialog } from "@kobalte/core";
import {
  For,
  Show,
  createMemo,
  createSignal,
  type JSX,
} from "solid-js";

import {
  addIndicator,
  indicatorName,
  type ActiveIndicator,
} from "../stores/indicators";
import type {
  IndicatorInfoSchema,
  ListIndicatorsPayload,
} from "../types/responses";
import type { z } from "zod";

type IndicatorInfo = z.infer<typeof IndicatorInfoSchema>;

export interface IndicatorPickerProps {
  /** Controlled open state. */
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Response payload from `list_indicators`; null while loading. */
  data: ListIndicatorsPayload | null;
  /** True while the list is being fetched. */
  loading: boolean;
  /** Non-null when the list failed to load. */
  error: Error | null;
  /** Current active set — used to mark rows as "added". */
  active: ActiveIndicator[];
}

// Human-readable category labels for the `type` field returned by the
// registry. Keep keys in sync with src/indicators/mod.rs.
const CATEGORY_ORDER: readonly string[] = ["overlay", "panel", "external"];
const CATEGORY_LABEL: Record<string, string> = {
  overlay: "Overlays",
  panel: "Panels",
  external: "External",
};

function categoryLabel(category: string): string {
  return CATEGORY_LABEL[category] ?? category;
}

/**
 * Build the default-parameter entry for an indicator. If the registry
 * publishes `configurable_params` as an object, merge its defaults into
 * `{ name, ... }`. Otherwise fall back to the bare name.
 */
function toActiveEntry(info: IndicatorInfo): ActiveIndicator {
  const params = info.configurable_params;
  if (params && typeof params === "object" && !Array.isArray(params)) {
    const defaults: Record<string, unknown> = {};
    // Expect `{ param: { default: value, ... } }` — the registry's shape
    // varies, so we defensively accept either a nested `default` or a
    // bare value.
    for (const [k, v] of Object.entries(params as Record<string, unknown>)) {
      if (v && typeof v === "object" && "default" in (v as object)) {
        defaults[k] = (v as { default: unknown }).default;
      }
    }
    if (Object.keys(defaults).length > 0) {
      return { name: info.name, ...defaults };
    }
  }
  return info.name;
}

function matches(info: IndicatorInfo, query: string): boolean {
  if (!query) return true;
  const q = query.toLowerCase();
  if (info.name.toLowerCase().includes(q)) return true;
  // Zod marks description as required, but an empty string survives the
  // parse — guard defensively so a future relaxation does not crash the
  // filter.
  if (info.description && info.description.toLowerCase().includes(q)) {
    return true;
  }
  if (info.aliases?.some((a) => a.toLowerCase().includes(q))) return true;
  return false;
}

const inputClass =
  "bg-[color:var(--color-bg-elev)] border border-[color:var(--color-border)] rounded-md px-3 py-1.5 text-sm text-[color:var(--color-fg)] focus:outline-none focus:border-[color:var(--color-accent)]";

/** Tiny inline SVG icons for the overlay/panel/external hint. */
function CategoryIcon(props: { category: string }): JSX.Element {
  // Minimal visual cue — overlay draws on price, panel below, external is
  // a cloud-ish hint. Keep these stroke-only to inherit text color.
  return (
    <Show when={props.category === "overlay"}>
      <svg
        aria-hidden="true"
        viewBox="0 0 16 16"
        class="w-3.5 h-3.5 text-[color:var(--color-fg-muted)]"
        fill="none"
        stroke="currentColor"
        stroke-width="1.5"
      >
        <path d="M1 11 L5 7 L8 9 L15 3" stroke-linecap="round" />
      </svg>
    </Show>
  );
}

function PanelIcon(props: { category: string }): JSX.Element {
  return (
    <Show when={props.category === "panel"}>
      <svg
        aria-hidden="true"
        viewBox="0 0 16 16"
        class="w-3.5 h-3.5 text-[color:var(--color-fg-muted)]"
        fill="none"
        stroke="currentColor"
        stroke-width="1.5"
      >
        <rect x="1.5" y="9" width="13" height="5" rx="1" />
        <path d="M1.5 11 H14.5" stroke-dasharray="1.5 1.5" />
      </svg>
    </Show>
  );
}

function ExternalIcon(props: { category: string }): JSX.Element {
  return (
    <Show when={props.category === "external"}>
      <svg
        aria-hidden="true"
        viewBox="0 0 16 16"
        class="w-3.5 h-3.5 text-[color:var(--color-fg-muted)]"
        fill="none"
        stroke="currentColor"
        stroke-width="1.5"
      >
        <path
          d="M4 11 A3 3 0 1 1 5 5 A4 4 0 0 1 13 6 A3 3 0 0 1 12 12 Z"
          stroke-linejoin="round"
        />
      </svg>
    </Show>
  );
}

export function IndicatorPicker(props: IndicatorPickerProps): JSX.Element {
  const [query, setQuery] = createSignal("");

  const activeNames = createMemo(
    () => new Set(props.active.map(indicatorName)),
  );

  // Group indicators by category. `group.items` preserves registry order.
  const groups = createMemo<
    Array<{ category: string; items: IndicatorInfo[] }>
  >(() => {
    const byCategory = new Map<string, IndicatorInfo[]>();
    for (const info of props.data?.indicators ?? []) {
      const bucket = byCategory.get(info.type) ?? [];
      bucket.push(info);
      byCategory.set(info.type, bucket);
    }
    const ordered = [...CATEGORY_ORDER, ...byCategory.keys()];
    const seen = new Set<string>();
    const out: Array<{ category: string; items: IndicatorInfo[] }> = [];
    for (const category of ordered) {
      if (seen.has(category)) continue;
      seen.add(category);
      const items = byCategory.get(category);
      if (items && items.length > 0) out.push({ category, items });
    }
    return out;
  });

  // Filtered groups for the current search query; categories with zero
  // matches are hidden entirely.
  const filteredGroups = createMemo(() => {
    const q = query();
    return groups()
      .map((g) => ({
        category: g.category,
        items: g.items.filter((i) => matches(i, q)),
      }))
      .filter((g) => g.items.length > 0);
  });

  // Which accordion sections the user has expanded manually. When the
  // search input is non-empty we override this with "all matching
  // sections" so the user sees every hit without extra clicks, but we
  // restore the user's previous choices as soon as the query clears.
  const [userExpanded, setUserExpanded] = createSignal<string[]>([]);
  const expanded = createMemo<string[]>(() => {
    if (query().length === 0) return userExpanded();
    return filteredGroups().map((g) => g.category);
  });

  const handleAdd = (info: IndicatorInfo) => {
    addIndicator(toActiveEntry(info));
  };

  return (
    <Dialog.Root open={props.open} onOpenChange={props.onOpenChange} modal>
      <Dialog.Portal>
        <Dialog.Overlay class="fixed inset-0 z-40 bg-black/60" />
        <div class="fixed inset-0 z-50 flex items-start justify-center pt-[10vh] px-4">
          <Dialog.Content class="w-full max-w-lg max-h-[80vh] flex flex-col bg-[color:var(--color-bg)] border border-[color:var(--color-border)] rounded-lg shadow-xl">
            <div class="flex items-center justify-between px-4 py-3 border-b border-[color:var(--color-border)]">
              <Dialog.Title class="text-sm font-semibold">
                Add indicator
              </Dialog.Title>
              <Dialog.CloseButton
                aria-label="Close"
                class="text-[color:var(--color-fg-muted)] hover:text-[color:var(--color-fg)] cursor-pointer px-2 py-1 text-sm"
              >
                ×
              </Dialog.CloseButton>
            </div>
            <div class="p-3 border-b border-[color:var(--color-border)]">
              <input
                type="text"
                value={query()}
                onInput={(e) => setQuery(e.currentTarget.value)}
                placeholder="Search indicators..."
                autocomplete="off"
                class={`${inputClass} w-full`}
              />
            </div>
            <div class="flex-1 min-h-0 overflow-y-auto">
              <Show when={props.loading}>
                <div class="p-4 text-sm text-[color:var(--color-fg-muted)]">
                  Loading...
                </div>
              </Show>
              <Show when={props.error}>
                <div class="p-4 text-sm text-[color:var(--color-danger)]">
                  {props.error?.message ?? "Failed to load indicators."}
                </div>
              </Show>
              <Show when={!props.loading && !props.error}>
                <Show
                  when={filteredGroups().length > 0}
                  fallback={
                    <div class="p-4 text-sm text-[color:var(--color-fg-muted)]">
                      No matches.
                    </div>
                  }
                >
                  <Accordion.Root
                    multiple
                    collapsible
                    value={expanded()}
                    onChange={(v) => {
                      // Only store the user's choice when no search is
                      // active — otherwise we'd overwrite it with the
                      // "all matching" override.
                      if (query().length === 0) setUserExpanded(v);
                    }}
                    class="flex flex-col"
                  >
                    <For each={filteredGroups()}>
                      {(group) => (
                        <Accordion.Item
                          value={group.category}
                          class="border-b border-[color:var(--color-border)] last:border-b-0"
                        >
                          <Accordion.Header>
                            <Accordion.Trigger class="flex w-full items-center justify-between px-4 py-2 text-sm font-medium text-left hover:bg-[color:var(--color-bg-elev)] cursor-pointer">
                              <span>
                                {categoryLabel(group.category)}
                                <span class="ml-2 text-xs text-[color:var(--color-fg-muted)]">
                                  {group.items.length}
                                </span>
                              </span>
                              <span
                                aria-hidden="true"
                                class="text-xs text-[color:var(--color-fg-muted)]"
                              >
                                ▾
                              </span>
                            </Accordion.Trigger>
                          </Accordion.Header>
                          <Accordion.Content>
                            <ul class="flex flex-col">
                              <For each={group.items}>
                                {(info) => {
                                  const isActive = () =>
                                    activeNames().has(info.name);
                                  return (
                                    <li>
                                      <button
                                        type="button"
                                        disabled={isActive()}
                                        onClick={() => handleAdd(info)}
                                        class="w-full flex items-start gap-2 px-6 py-2 text-left text-sm hover:bg-[color:var(--color-bg-elev)] disabled:opacity-50 disabled:cursor-default cursor-pointer"
                                      >
                                        <span class="pt-0.5 flex-shrink-0">
                                          <CategoryIcon category={info.type} />
                                          <PanelIcon category={info.type} />
                                          <ExternalIcon category={info.type} />
                                        </span>
                                        <span class="flex flex-col gap-0.5 min-w-0">
                                          <span class="font-mono text-[color:var(--color-fg)]">
                                            {info.name}
                                            <Show when={isActive()}>
                                              <span class="ml-2 text-xs text-[color:var(--color-accent)]">
                                                added
                                              </span>
                                            </Show>
                                          </span>
                                          <span class="text-xs text-[color:var(--color-fg-muted)] line-clamp-2">
                                            {info.description}
                                          </span>
                                        </span>
                                      </button>
                                    </li>
                                  );
                                }}
                              </For>
                            </ul>
                          </Accordion.Content>
                        </Accordion.Item>
                      )}
                    </For>
                  </Accordion.Root>
                </Show>
              </Show>
            </div>
          </Dialog.Content>
        </div>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
