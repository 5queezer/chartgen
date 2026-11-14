// Setups tab of the right sidebar (ADR-0071): the signed-in owner's setups the server
// scores on paper (ADR-0059), open ones first, with an open/closed filter and the track
// record's totals for takes and skips. Clicking a row opens its chart with its drawings,
// and a skip's bracket. Read only: setups are made and changed by the agent's tools.

import { For, Show, createMemo, createSignal, onCleanup, onMount, type JSX } from "solid-js";

import {
  SETUP_FILTERS,
  WON_AT_A_LOSS,
  entryText,
  filterSetups,
  formatR,
  isOpen,
  sortSetups,
  wonAtALoss,
  type SetupFilter,
} from "../lib/setups";
import { formatPrice } from "../lib/quote";
import { formatDateTime, resolveTimeZone } from "../lib/timezone";
import type { McpSession } from "../mcp";
import { timeZone } from "../stores/layout";
import { LIST_LIMIT, openSetup, refreshSetups, setupsState } from "../stores/setups";
import type { Setup, TrackRecordPayload } from "../types/responses";

/** How often the open tab asks again: setups move on closed 1m bars. */
const REFRESH_MS = 60_000;
const FILTER_KEY = "chartgen.setups.filter";

const STATUS_CLASS: Record<Setup["status"], string> = {
  watching: "text-[color:var(--color-fg-muted)]",
  triggered: "text-[color:var(--color-accent)]",
  won: "text-[color:var(--color-bull)]",
  invalidated: "text-[color:var(--color-bear)]",
  expired: "text-[color:var(--color-fg-muted)]",
  cancelled: "text-[color:var(--color-fg-muted)]",
};

const FILTER_LABELS: Record<SetupFilter, string> = { open: "Open", closed: "Closed", all: "All" };

function loadFilter(): SetupFilter {
  try {
    const v = localStorage.getItem(FILTER_KEY);
    return SETUP_FILTERS.find((f) => f === v) ?? "open";
  } catch {
    return "open";
  }
}

/** "MM-DD HH:mm", with the year only when it is not this one's. */
function shortTime(ms: number, tz: string): string {
  const full = formatDateTime(ms, tz);
  return full.slice(0, 4) === formatDateTime(Date.now(), tz).slice(0, 4) ? full.slice(5) : full;
}

const rClass = (r: number | null | undefined): string =>
  r === null || r === undefined || r === 0
    ? "text-[color:var(--color-fg-muted)]"
    : r > 0
      ? "text-[color:var(--color-bull)]"
      : "text-[color:var(--color-bear)]";

export function SetupsPanel(props: { session: McpSession; enabled: boolean }): JSX.Element {
  const [filter, setFilterSignal] = createSignal<SetupFilter>(loadFilter());
  const setFilter = (next: SetupFilter) => {
    setFilterSignal(next);
    try {
      localStorage.setItem(FILTER_KEY, next);
    } catch {
      // Best-effort only.
    }
  };
  const refresh = () => {
    if (props.enabled) void refreshSetups(props.session);
  };
  onMount(() => {
    refresh();
    const timer = window.setInterval(refresh, REFRESH_MS);
    onCleanup(() => window.clearInterval(timer));
  });

  const ready = createMemo(() => {
    const s = setupsState();
    return s.kind === "ready" ? s : null;
  });
  const shown = createMemo(() => sortSetups(filterSetups(ready()?.setups ?? [], filter())));
  const count = (f: SetupFilter) => filterSetups(ready()?.setups ?? [], f).length;

  return (
    <div class="flex flex-col flex-1 min-h-0" aria-label="Setups" role="region">
      <div class="flex items-center gap-2 px-3 py-2 border-b border-[color:var(--color-border)]">
        <h2 class="text-sm font-semibold flex-1">Setups</h2>
        <div role="group" aria-label="Show setups" class="flex text-xs rounded-md border border-[color:var(--color-border)] overflow-hidden">
          <For each={SETUP_FILTERS}>
            {(f) => (
              <button
                type="button"
                aria-pressed={filter() === f}
                class="px-2 py-0.5 cursor-pointer text-[color:var(--color-fg-muted)] aria-pressed:text-[color:var(--color-fg)] aria-pressed:bg-[color:var(--color-bg-elev)]"
                onClick={() => setFilter(f)}
              >
                {FILTER_LABELS[f]}
                <Show when={ready()}>
                  <span class="ml-1 tabular-nums">{count(f)}</span>
                </Show>
              </button>
            )}
          </For>
        </div>
      </div>
      <Show when={setupsState().kind === "off"}>
        <p data-role="setups-off" class="px-3 py-3 text-xs text-[color:var(--color-fg-muted)]">
          This server keeps no setups. An operator turns them on with CHARTGEN_SETUPS=1.
        </p>
      </Show>
      <Show when={(() => { const s = setupsState(); return s.kind === "error" ? s : null; })()}>
        {(s) => (
          <p data-role="setups-error" class="px-3 py-3 text-xs text-[color:var(--color-danger)]">
            Setups could not be read: {s().message}
          </p>
        )}
      </Show>
      <Show when={ready()}>
        {(r) => (
          <>
            <div
              data-role="setups-totals"
              class="grid grid-cols-[auto_repeat(5,minmax(0,1fr))] gap-x-2 gap-y-0.5 px-3 py-1.5 text-xs tabular-nums border-b border-[color:var(--color-border)]"
              title="Closed setups, as the server's track record counts them; open ones are not in it"
            >
              <span />
              <For each={["closed", "won", "total", "mean", "open"]}>
                {(h) => <span class="text-right text-[color:var(--color-fg-muted)]">{h}</span>}
              </For>
              <Totals label="Taken" decision="take" record={r().takes} />
              <Totals label="Skipped" decision="skip" record={r().skips} />
            </div>
            <Show when={r().total > r().setups.length}>
              <p class="px-3 py-1 text-xs text-[color:var(--color-fg-muted)]">
                The newest {LIST_LIMIT} of {r().total} setups.
              </p>
            </Show>
          </>
        )}
      </Show>
      <div class="flex-1 min-h-0 overflow-y-auto">
        <Show
          when={shown().length > 0}
          fallback={
            <Show when={ready()}>
              <p data-role="setups-empty" class="px-3 py-3 text-xs text-[color:var(--color-fg-muted)]">
                {filter() === "closed" ? "No closed setups yet." : filter() === "open" ? "No open setups." : "No setups yet. Your agent makes them with create_setup."}
              </p>
            </Show>
          }
        >
          <For each={shown()}>
            {(s) => <SetupRow setup={s} onOpen={() => void openSetup(props.session, s.id)} />}
          </For>
        </Show>
      </div>
    </div>
  );
}

/** One row of the totals grid: what `get_track_record` counts, as it counts it. */
function Totals(props: {
  label: string;
  decision: "take" | "skip";
  record: TrackRecordPayload | null;
}): JSX.Element {
  const cell = "text-right";
  return (
    <div data-role={`totals-${props.decision}`} class="contents">
      <span class="text-[color:var(--color-fg-muted)]">{props.label}</span>
      <Show
        when={props.record}
        fallback={
          <For each={[0, 1, 2, 3, 4]}>
            {() => <span class={`${cell} text-[color:var(--color-fg-muted)]`}>–</span>}
          </For>
        }
      >
        {(rec) => (
          <>
            <span data-role="count" class={cell}>
              {rec().total.count}
            </span>
            <span data-role="won" class={cell}>
              {rec().total.won}
            </span>
            <span data-role="total-r" class={`${cell} ${rClass(rec().total.total_r)}`}>
              {formatR(rec().total.total_r)}
            </span>
            <span data-role="mean-r" class={`${cell} ${rClass(rec().total.mean_r)}`}>
              {formatR(rec().total.mean_r)}
            </span>
            <span data-role="open" class={cell}>
              {rec().open}
            </span>
          </>
        )}
      </Show>
    </div>
  );
}

function SetupRow(props: { setup: Setup; onOpen: () => void }): JSX.Element {
  const s = () => props.setup;
  const tz = () => resolveTimeZone(timeZone(), s().symbol);
  return (
    <div
      data-setup-id={s().id}
      data-status={s().status}
      data-decision={s().decision}
      role="button"
      tabIndex={0}
      title={s().thesis}
      class="wl-row flex flex-col gap-0.5 px-3 py-1.5 text-sm cursor-pointer border-b border-[color:var(--color-border)]"
      onClick={() => props.onOpen()}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          props.onOpen();
        }
      }}
    >
      <div class="flex items-baseline gap-1.5 min-w-0">
        <span class="font-semibold truncate">{s().symbol}</span>
        <span class="text-xs text-[color:var(--color-fg-muted)]">{s().interval}</span>
        <span
          data-role="direction"
          class="text-xs uppercase"
          classList={{
            "text-[color:var(--color-bull)]": s().direction === "long",
            "text-[color:var(--color-bear)]": s().direction === "short",
          }}
        >
          {s().direction}
        </span>
        <Show when={s().decision === "skip"}>
          <span data-role="skip" class="text-xs text-[color:var(--color-fg-muted)] border border-[color:var(--color-border)] rounded px-1">
            skip
          </span>
        </Show>
        <span class="flex-1" />
        <span data-role="status" class={`text-xs ${STATUS_CLASS[s().status]}`}>
          {s().status}
        </span>
        <span data-role="result" class={`text-xs font-semibold tabular-nums ${rClass(s().result_r)}`}>
          {isOpen(s()) ? "" : formatR(s().result_r)}
        </span>
      </div>
      {/* Wraps between its parts, never inside one: all four levels stay readable. */}
      <div
        class="flex flex-wrap gap-x-1.5 text-xs text-[color:var(--color-fg-muted)] tabular-nums [&>span]:whitespace-nowrap"
        data-role="levels"
      >
        <span data-role="entry">{entryText(s(), formatPrice)}</span>
        <span data-role="stop">stop {formatPrice(s().invalidation)}</span>
        <Show when={s().targets[0]}>{(t1) => <span data-role="t1">T1 {formatPrice(t1())}</span>}</Show>
        <span data-role="expiry" title={`Expires ${formatDateTime(s().expires_at, tz())}`}>
          exp {shortTime(s().expires_at, tz())}
        </span>
      </div>
      <Show when={wonAtALoss(s())}>
        <div data-role="won-at-a-loss" class="text-xs text-[color:var(--color-danger)]" title={WON_AT_A_LOSS}>
          ⚠ Won below 0 R: the costs were more than the move to T1.
        </div>
      </Show>
      <Show when={s().gap}>
        <div data-role="gap" class="text-xs text-[color:var(--color-fg-muted)]" title="Bars were missed and not scored: the result does not cover them.">
          incomplete: bars were missed
        </div>
      </Show>
      <Show when={s().tags.length > 0}>
        <div class="flex flex-wrap gap-1" data-role="tags">
          <For each={s().tags}>
            {(t) => (
              <span class="text-[10px] px-1 rounded bg-[color:var(--color-bg-elev)] text-[color:var(--color-fg-muted)]">
                {t}
              </span>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}
