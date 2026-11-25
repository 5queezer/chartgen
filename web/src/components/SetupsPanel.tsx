// Setups tab of the right sidebar (ADR-0071, ADR-0091): the signed-in owner's setups on
// the chart's symbol, open ones first and the last few closed, each opening its chart with
// its drawings and a skip's bracket. The whole list and the track record are on the
// Setups page at /setups. Read only: setups are made and changed by the agent's tools.

import { For, Show, createEffect, createMemo, on, onCleanup, onMount, type JSX } from "solid-js";

import { WON_AT_A_LOSS, entryText, formatR, isOpen, sortSetups, wonAtALoss } from "../lib/setups";
import { formatPrice } from "../lib/quote";
import { formatDateTime, resolveTimeZone } from "../lib/timezone";
import type { McpSession } from "../mcp";
import { symbol, timeZone } from "../stores/layout";
import { openSetup, refreshSetups, setupsState } from "../stores/setups";
import type { Setup } from "../types/responses";

/** How often the open tab asks again: setups move on closed 1m bars. */
const REFRESH_MS = 60_000;
/** Closed setups the tab shows, the most recently ended; the rest are on the page. */
export const CLOSED_SHOWN = 5;

const STATUS_CLASS: Record<Setup["status"], string> = {
  watching: "text-[color:var(--color-fg-muted)]",
  triggered: "text-[color:var(--color-accent)]",
  won: "text-[color:var(--color-bull)]",
  invalidated: "text-[color:var(--color-bear)]",
  expired: "text-[color:var(--color-fg-muted)]",
  cancelled: "text-[color:var(--color-fg-muted)]",
};

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
  const refresh = () => {
    if (props.enabled) void refreshSetups(props.session, symbol());
  };
  // As soon as the session is ready (the tab may be open, remembered, before it is), and
  // for each symbol the chart switches to.
  createEffect(on([() => props.enabled, symbol], ([enabled]) => enabled && refresh()));
  onMount(() => {
    const timer = window.setInterval(refresh, REFRESH_MS);
    onCleanup(() => window.clearInterval(timer));
  });

  // Only an answer about the chart's symbol: one about the last is not shown.
  const ready = createMemo(() => {
    const s = setupsState();
    return s.kind === "ready" && s.symbol === symbol() ? s : null;
  });
  const sorted = createMemo(() => sortSetups(ready()?.setups ?? []));
  const shown = createMemo(() => [
    ...sorted().filter(isOpen),
    ...sorted().filter((s) => !isOpen(s)).slice(0, CLOSED_SHOWN),
  ]);
  const more = () => (ready()?.total ?? 0) - shown().length;

  return (
    <div class="flex flex-col flex-1 min-h-0" aria-label="Setups" role="region">
      <div class="flex items-center gap-2 px-3 py-2 border-b border-[color:var(--color-border)]">
        <h2 class="text-sm font-semibold flex-1 truncate">Setups on {symbol()}</h2>
        <a
          href="/setups"
          data-role="setups-page-link"
          class="text-xs text-[color:var(--color-accent)] no-underline hover:underline whitespace-nowrap"
          title="Every setup, sortable and filtered, with the track record and open risk"
        >
          All setups
        </a>
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
      <div class="flex-1 min-h-0 overflow-y-auto">
        <Show
          when={shown().length > 0}
          fallback={
            <Show when={ready()}>
              <p data-role="setups-empty" class="px-3 py-3 text-xs text-[color:var(--color-fg-muted)]">
                No setups on {symbol()}.
              </p>
            </Show>
          }
        >
          <For each={shown()}>
            {(s) => <SetupRow setup={s} onOpen={() => void openSetup(props.session, s.id)} />}
          </For>
        </Show>
        <Show when={more() > 0}>
          <a
            href="/setups"
            data-role="setups-more"
            class="block px-3 py-2 text-xs text-[color:var(--color-fg-muted)] no-underline hover:underline"
          >
            {more()} more on {symbol()} on the Setups page
          </a>
        </Show>
      </div>
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
