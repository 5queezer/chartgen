// Transient notices (alert triggers, confirmations), bottom-left, newest
// last. Dismissed on click or after a few seconds (services/alertEngine).

import { For, type JSX } from "solid-js";

import { dismissToast, toasts } from "../services/alertEngine";

export function Toasts(): JSX.Element {
  return (
    <div
      role="status"
      aria-live="polite"
      class="fixed bottom-4 left-16 z-[60] flex flex-col gap-2 pointer-events-none"
    >
      <For each={toasts()}>
        {(t) => (
          <button
            type="button"
            data-toast
            onClick={() => dismissToast(t.id)}
            class="pointer-events-auto text-left max-w-sm px-3 py-2 rounded-md text-sm cursor-pointer bg-[color:var(--color-bg-elev)] border border-[color:var(--color-border)] border-l-4 border-l-[color:var(--color-accent)] shadow-lg text-[color:var(--color-fg)]"
          >
            {t.text}
          </button>
        )}
      </For>
    </div>
  );
}
