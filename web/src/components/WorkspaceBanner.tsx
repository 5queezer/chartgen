// What the workspace sync has to say (ADR-0050): that this browser's data from
// before sign-in can be brought in, that another device changed something, or that
// something did not sync or gave way. Never replaced without saying so.

import { createSignal, For, Show, type JSX } from "solid-js";

import type { SignedInUser } from "../oauth";
import {
  acceptLegacy,
  dismissNotice,
  legacyOffer,
  notices,
  rejectLegacy,
  reloadEverywhere,
  remoteChanged,
} from "../services/workspace";

const rowClass =
  "flex flex-wrap items-center gap-x-3 gap-y-1 px-4 py-1.5 text-sm border-b border-[color:var(--color-border)] bg-[color:var(--color-bg-elev)]";
const buttonClass =
  "px-2 py-0.5 rounded-md cursor-pointer border border-[color:var(--color-border)] hover:bg-[color:var(--color-border)]";

export function WorkspaceBanner(props: { user: SignedInUser | null }): JSX.Element {
  const [offerHidden, setOfferHidden] = createSignal(false);
  const discard = () => {
    if (window.confirm("Delete the data this browser kept from before you signed in?")) {
      rejectLegacy();
    }
  };
  return (
    <>
      <Show when={legacyOffer() && !offerHidden()}>
        <div class={rowClass} data-testid="workspace-offer">
          <span class="min-w-0">
            This browser has charts, layouts and drawings from before you signed in. Add them to your
            account?
          </span>
          <button type="button" class={buttonClass} data-testid="workspace-import" onClick={() => props.user && acceptLegacy(props.user)}>
            Add to my account
          </button>
          <button type="button" class={buttonClass} data-testid="workspace-discard" onClick={discard}>
            Delete them
          </button>
          <button
            type="button"
            class={buttonClass}
            data-testid="workspace-later"
            onClick={() => setOfferHidden(true)}
          >
            Not now
          </button>
        </div>
      </Show>
      <Show when={remoteChanged()}>
        <div class={rowClass} data-testid="workspace-remote">
          <span class="min-w-0">Your workspace changed on another device.</span>
          <button type="button" class={buttonClass} data-testid="workspace-reload" onClick={reloadEverywhere}>
            Reload
          </button>
        </div>
      </Show>
      <For each={notices()}>
        {(text) => (
          <div class={rowClass} data-testid="workspace-notice">
            <span class="min-w-0">{text}</span>
            <button type="button" class={buttonClass} onClick={() => dismissNotice(text)}>
              Dismiss
            </button>
          </div>
        )}
      </For>
    </>
  );
}
