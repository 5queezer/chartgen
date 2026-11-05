// Alerts tab of the right sidebar: alerts grouped by symbol with what they
// watch, their status (active, triggered, paused, or inactive with the
// reason: a deleted drawing, an indicator alert whose chart is not open)
// and edit / pause-resume / delete actions. Clicking a row switches the
// chart to that symbol; "+" opens the create dialog.

import { For, Show, createMemo, type JSX } from "solid-js";

import {
  CONDITION_LABELS,
  TRIGGER_LABELS,
  alertStatus,
  describeAlert,
  type Alert,
} from "../lib/alerts";
import { formatPrice } from "../lib/quote";
import {
  alerts,
  inactiveReason,
  removeAlert,
  setAlertActive,
} from "../stores/alerts";
import { setSymbol, symbol } from "../stores/layout";
import { setWatchlistCollapsed } from "../stores/watchlist";
import { leader } from "../services/alertEngine";
import { openAlertDialog } from "./AlertDialog";

const iconButtonClass =
  "inline-flex items-center justify-center w-7 h-7 rounded-md cursor-pointer text-[color:var(--color-fg-muted)] hover:text-[color:var(--color-fg)] hover:bg-[color:var(--color-bg-elev)]";

const STATUS_CLASS = {
  active: "text-[color:var(--color-bull)]",
  triggered: "text-[color:var(--color-accent)]",
  paused: "text-[color:var(--color-fg-muted)]",
} as const;

/** Alerts grouped by symbol; the chart symbol first, then alphabetical. */
function groupBySymbol(list: Alert[], current: string) {
  const groups = new Map<string, Alert[]>();
  for (const a of list) groups.set(a.symbol, [...(groups.get(a.symbol) ?? []), a]);
  return [...groups.entries()]
    .sort(([a], [b]) =>
      a === current ? -1 : b === current ? 1 : a.localeCompare(b),
    )
    .map(([sym, items]) => ({ symbol: sym, items }));
}

export function AlertsPanel(): JSX.Element {
  const groups = createMemo(() => groupBySymbol(alerts(), symbol()));

  return (
    <div class="flex flex-col flex-1 min-h-0" aria-label="Alerts" role="region">
      <div class="flex items-center gap-1 px-3 py-2 border-b border-[color:var(--color-border)]">
        <h2 class="text-sm font-semibold flex-1">Alerts</h2>
        <button
          type="button"
          class={`${iconButtonClass} text-lg leading-none`}
          aria-label="Create alert"
          title="Create alert"
          onClick={() => openAlertDialog()}
        >
          +
        </button>
        <button
          type="button"
          class={iconButtonClass}
          aria-label="Collapse sidebar"
          title="Collapse sidebar"
          onClick={() => setWatchlistCollapsed(true)}
        >
          <span aria-hidden="true">»</span>
        </button>
      </div>
      <Show when={!leader()}>
        <p
          data-role="alerts-elsewhere"
          class="px-3 py-1.5 text-xs text-[color:var(--color-fg-muted)] border-b border-[color:var(--color-border)]"
        >
          Alerts are evaluated in another open chartgen tab.
        </p>
      </Show>
      <div class="flex-1 min-h-0 overflow-y-auto">
        <Show
          when={groups().length > 0}
          fallback={
            <p class="px-3 py-3 text-xs text-[color:var(--color-fg-muted)]">
              No alerts yet. Press Alt+A or use + to create one.
            </p>
          }
        >
          <For each={groups()}>
            {(g) => (
              <section aria-label={`${g.symbol} alerts`}>
                <h3 class="px-3 pt-2 pb-1 text-xs font-semibold text-[color:var(--color-fg-muted)]">
                  {g.symbol}
                </h3>
                <For each={g.items}>{(a) => <AlertRow alert={a} />}</For>
              </section>
            )}
          </For>
        </Show>
      </div>
    </div>
  );
}

function AlertRow(props: { alert: Alert }): JSX.Element {
  const a = () => props.alert;
  const status = () => alertStatus(a());
  const reason = () => inactiveReason(a());
  const stop = (fn: () => void) => (e: MouseEvent) => {
    e.stopPropagation();
    fn();
  };

  return (
    <div
      data-alert-id={a().id}
      data-status={reason() === null ? status() : "inactive"}
      class="wl-row group flex items-center gap-2 px-3 py-1.5 text-sm cursor-pointer"
      onClick={() => setSymbol(a().symbol)}
    >
      <div class="flex-1 min-w-0">
        <div class="truncate" data-role="description">
          <Show
            when={a().source}
            fallback={
              <>
                {CONDITION_LABELS[a().condition]}{" "}
                <span class="font-semibold tabular-nums">{formatPrice(a().price)}</span>
              </>
            }
          >
            {describeAlert(a())}
          </Show>
        </div>
        <div class="text-xs text-[color:var(--color-fg-muted)] truncate">
          <span
            data-role="status"
            class={reason() === null ? STATUS_CLASS[status()] : STATUS_CLASS.paused}
          >
            {reason() === null ? status() : `inactive: ${reason()}`}
          </span>
          {" · "}
          {TRIGGER_LABELS[a().trigger]}
          <Show when={a().message}>{(m) => <> · {m()}</>}</Show>
        </div>
      </div>
      <div class="flex shrink-0 opacity-0 group-hover:opacity-100 focus-within:opacity-100">
        <button
          type="button"
          class={iconButtonClass}
          aria-label="Edit alert"
          title="Edit"
          onClick={stop(() => openAlertDialog({ id: a().id }))}
        >
          <span aria-hidden="true">✎</span>
        </button>
        <button
          type="button"
          class={iconButtonClass}
          aria-label={a().active ? "Pause alert" : "Resume alert"}
          title={a().active ? "Pause" : "Resume"}
          onClick={stop(() => setAlertActive(a().id, !a().active))}
        >
          <span aria-hidden="true">{a().active ? "❚❚" : "▶"}</span>
        </button>
        <button
          type="button"
          class={iconButtonClass}
          aria-label="Delete alert"
          title="Delete"
          onClick={stop(() => removeAlert(a().id))}
        >
          <span aria-hidden="true">×</span>
        </button>
      </div>
    </div>
  );
}
