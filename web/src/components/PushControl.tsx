// Notifications on this device for server alerts (ADR-0045): the Alerts tab and the
// alert dialog show this. It says what is possible here, and asks for permission only
// when its button is clicked.

import { Match, Show, Switch, type JSX } from "solid-js";

import {
  disablePush,
  enablePush,
  pushBusy,
  pushDeviceState,
  pushError,
} from "../stores/pushDevice";

const buttonClass =
  "shrink-0 px-2 py-1 text-xs rounded border border-[color:var(--color-border)] cursor-pointer hover:bg-[color:var(--color-bg-elev)] disabled:opacity-50";
const textClass = "text-xs text-[color:var(--color-fg-muted)] min-w-0";

export function PushControl(props: { onEnabled?: () => void }): JSX.Element {
  return (
    <div data-role="push-control" data-state={pushDeviceState()} class="flex flex-col gap-1">
      <Switch>
        <Match when={pushDeviceState() === "unsupported"}>
          <p class={textClass}>
            This browser cannot notify with chartgen closed. Alerts still show here while a tab is
            open.
          </p>
        </Match>
        <Match when={pushDeviceState() === "install"}>
          <p class={textClass}>
            On iPhone and iPad, notifications need chartgen on the Home Screen: tap Share, then Add
            to Home Screen, and open it from there.
          </p>
        </Match>
        <Match when={pushDeviceState() === "denied"}>
          <p class={textClass}>
            Notifications are blocked for this site. Allow them in the browser's site settings, then
            reload.
          </p>
        </Match>
        <Match when={pushDeviceState() === "off"}>
          <div class="flex items-center justify-between gap-2">
            <p class={textClass}>Get alerts on this device with chartgen closed.</p>
            <button
              type="button"
              class={buttonClass}
              disabled={pushBusy()}
              onClick={() => {
                void enablePush().then((enabled) => {
                  if (enabled) props.onEnabled?.();
                });
              }}
            >
              Enable notifications
            </button>
          </div>
        </Match>
        <Match when={pushDeviceState() === "on"}>
          <div class="flex items-center justify-between gap-2">
            <p class={textClass}>Notifications are on for this device.</p>
            <button
              type="button"
              class={buttonClass}
              disabled={pushBusy()}
              onClick={() => void disablePush()}
            >
              Turn off
            </button>
          </div>
        </Match>
      </Switch>
      <Show when={pushError()}>
        <p role="alert" class="text-xs text-[color:var(--color-danger)]">
          {pushError()}
        </p>
      </Show>
    </div>
  );
}
