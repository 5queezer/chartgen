// Create / edit dialog for price alerts, plus the entry points that open it:
// the header bell button, Alt+A (TradingView's shortcut: an alert at the
// last price) and `openAlertDialog`, which the Alerts tab "+", the chart
// context menu or any other caller can use.

import { Dialog } from "@kobalte/core";
import { For, Show, createSignal, onCleanup, onMount, type JSX } from "solid-js";

import {
  CONDITIONS,
  CONDITION_LABELS,
  TRIGGERS,
  TRIGGER_LABELS,
  type AlertCondition,
  type AlertTrigger,
} from "../lib/alerts";
import { isTypingContext } from "../lib/keys";
import { priceDecimals } from "../lib/quote";
import {
  MAX_ALERTS,
  addAlert,
  alerts,
  lastPrice,
  updateAlert,
} from "../stores/alerts";
import { symbol as chartSymbol } from "../stores/layout";
import {
  requestNotificationPermission,
  showToast,
} from "../services/alertEngine";

interface Draft {
  /** Set when editing an existing alert. */
  id?: string;
  symbol: string;
  condition: AlertCondition;
  price: string;
  trigger: AlertTrigger;
  message: string;
}

const [draft, setDraft] = createSignal<Draft | null>(null);

/** Plain decimal text of a price, at the precision the watchlist shows. */
const priceText = (p: number) => String(+p.toFixed(priceDecimals(p)));

export interface OpenAlertOptions {
  /** Edit this alert; the other fields are ignored. */
  id?: string;
  /** Defaults to the chart symbol. */
  symbol?: string;
  /** Defaults to the symbol's last live price, if known. */
  price?: number;
}

/** Open the dialog to create an alert (or edit one with `id`). */
export function openAlertDialog(opts: OpenAlertOptions = {}): void {
  const existing =
    opts.id === undefined ? undefined : alerts().find((a) => a.id === opts.id);
  if (existing) {
    setDraft({
      id: existing.id,
      symbol: existing.symbol,
      condition: existing.condition,
      price: priceText(existing.price),
      trigger: existing.trigger,
      message: existing.message ?? "",
    });
    return;
  }
  const sym = opts.symbol ?? chartSymbol();
  const price = opts.price ?? lastPrice(sym);
  setDraft({
    symbol: sym,
    condition: "crossing",
    price: price === null ? "" : priceText(price),
    trigger: "once",
    message: "",
  });
}

const inputClass =
  "bg-[color:var(--color-bg-elev)] border border-[color:var(--color-border)] rounded-md px-3 py-1.5 text-sm text-[color:var(--color-fg)] focus:outline-none focus:border-[color:var(--color-accent)] w-44 text-right tabular-nums";
const selectClass =
  "bg-[color:var(--color-bg-elev)] border border-[color:var(--color-border)] rounded-md px-2 py-1.5 text-sm text-[color:var(--color-fg)] w-44";
const buttonClass =
  "border border-[color:var(--color-border)] rounded-md px-3 py-1.5 text-sm hover:border-[color:var(--color-accent)] cursor-pointer";

export function BellIcon(): JSX.Element {
  return (
    <svg
      width="18"
      height="18"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="1.6"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <path d="M6 16V11a6 6 0 0 1 12 0v5l2 2H4z" />
      <path d="M10 21a2 2 0 0 0 4 0" />
    </svg>
  );
}

/** Header button: an alert at the chart symbol's last price. */
export function AlertButton(): JSX.Element {
  return (
    <button
      type="button"
      aria-label="Add alert"
      title="Add alert (Alt+A)"
      onClick={() => openAlertDialog()}
      class="inline-flex items-center justify-center w-8 h-8 rounded-md cursor-pointer text-[color:var(--color-fg)] hover:bg-[color:var(--color-bg-elev)]"
    >
      <BellIcon />
    </button>
  );
}

export function AlertDialog(): JSX.Element {
  const [error, setError] = createSignal<string | null>(null);

  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.code !== "KeyA" || !e.altKey || e.ctrlKey || e.metaKey) return;
      if (isTypingContext(e)) return;
      e.preventDefault();
      openAlertDialog();
    };
    window.addEventListener("keydown", onKey);
    onCleanup(() => window.removeEventListener("keydown", onKey));
  });

  const close = () => {
    setDraft(null);
    setError(null);
  };

  const patch = (p: Partial<Draft>) => {
    setDraft((d) => (d ? { ...d, ...p } : d));
    setError(null);
  };

  const submit = (e: SubmitEvent) => {
    e.preventDefault();
    const d = draft();
    if (!d) return;
    const price = Number(d.price);
    if (d.price.trim() === "" || !Number.isFinite(price) || price <= 0) {
      setError("Enter a price greater than zero.");
      return;
    }
    if (d.id !== undefined) {
      updateAlert(d.id, {
        condition: d.condition,
        price,
        trigger: d.trigger,
        message: d.message,
      });
    } else if (
      addAlert({
        symbol: d.symbol,
        condition: d.condition,
        price,
        trigger: d.trigger,
        message: d.message,
      }) === null
    ) {
      setError(`Alert limit reached (${MAX_ALERTS}).`);
      return;
    } else {
      requestNotificationPermission();
      showToast(`Alert created for ${d.symbol}`);
    }
    close();
  };

  return (
    <Dialog.Root
      open={draft() !== null}
      onOpenChange={(open) => {
        if (!open) close();
      }}
      modal
    >
      <Dialog.Portal>
        <Dialog.Overlay class="fixed inset-0 z-40 bg-black/60" />
        <div class="fixed inset-0 z-50 flex items-start justify-center pt-[10vh] px-4">
          <Dialog.Content class="w-full max-w-md bg-[color:var(--color-bg)] border border-[color:var(--color-border)] rounded-lg shadow-xl">
            <Show when={draft()}>
              {(d) => (
                <form class="flex flex-col gap-3 p-4" onSubmit={submit}>
                  <Dialog.Title class="text-sm font-semibold">
                    {d().id === undefined ? "Create alert" : "Edit alert"} on{" "}
                    {d().symbol}
                  </Dialog.Title>
                  <label class="flex items-center justify-between gap-3 text-sm">
                    Condition
                    <select
                      class={selectClass}
                      value={d().condition}
                      onChange={(e) =>
                        patch({ condition: e.currentTarget.value as AlertCondition })
                      }
                    >
                      <For each={CONDITIONS}>
                        {(c) => <option value={c}>{CONDITION_LABELS[c]}</option>}
                      </For>
                    </select>
                  </label>
                  <label class="flex items-center justify-between gap-3 text-sm">
                    Price
                    <input
                      class={inputClass}
                      inputmode="decimal"
                      autofocus
                      value={d().price}
                      onInput={(e) => patch({ price: e.currentTarget.value })}
                    />
                  </label>
                  <label class="flex items-center justify-between gap-3 text-sm">
                    Trigger
                    <select
                      class={selectClass}
                      value={d().trigger}
                      onChange={(e) =>
                        patch({ trigger: e.currentTarget.value as AlertTrigger })
                      }
                    >
                      <For each={TRIGGERS}>
                        {(t) => <option value={t}>{TRIGGER_LABELS[t]}</option>}
                      </For>
                    </select>
                  </label>
                  <label class="flex items-center justify-between gap-3 text-sm">
                    Message
                    <input
                      class={`${inputClass} text-left`}
                      maxlength="200"
                      placeholder="Optional"
                      value={d().message}
                      onInput={(e) => patch({ message: e.currentTarget.value })}
                    />
                  </label>
                  <Show when={error()}>
                    <p role="alert" class="text-xs text-[color:var(--color-danger)]">
                      {error()}
                    </p>
                  </Show>
                  <div class="flex justify-end gap-2">
                    <button type="button" class={buttonClass} onClick={close}>
                      Cancel
                    </button>
                    <button type="submit" class={buttonClass}>
                      {d().id === undefined ? "Create" : "Save"}
                    </button>
                  </div>
                </form>
              )}
            </Show>
          </Dialog.Content>
        </div>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
