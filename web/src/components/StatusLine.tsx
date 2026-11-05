// Status indicator next to the form. Pure presentational — the state
// machine lives in App.tsx. v0.1 statuses: ready, authenticating...,
// loading..., loaded, error (message in the tooltip), sign-in
// failed: <message> (with a Sign in again action). After it, a
// small live-feed badge: live, paused (hidden tab), retrying, resyncing.

import { Show, type JSX } from "solid-js";

import type { PollStatus } from "../lib/liveFeed";

export type Status =
  | { kind: "ready" }
  | { kind: "authenticating" }
  | { kind: "loading" }
  | { kind: "loaded" }
  | { kind: "error"; message: string }
  | { kind: "auth-error"; message: string };

export interface StatusLineProps {
  status: Status;
  /** Live tail polling state; nothing is shown while it is off. */
  live?: PollStatus;
  /** Called when the user clicks Sign in again on the auth-error status. */
  onSignIn: () => void;
}

function formatStatus(s: Status): string {
  switch (s.kind) {
    case "ready":
      return "ready";
    case "authenticating":
      return "authenticating...";
    case "loading":
      return "loading...";
    case "loaded":
      return "loaded";
    case "error":
      return "error";
    case "auth-error":
      return `sign-in failed: ${s.message}`;
  }
}

function colorFor(s: Status): string {
  switch (s.kind) {
    case "error":
    case "auth-error":
      return "text-[color:var(--color-danger)]";
    case "loaded":
      return "text-[color:var(--color-accent)]";
    default:
      return "text-[color:var(--color-fg-muted)]";
  }
}

/** Badge text and tooltip; null while polling is off. */
function liveBadge(s: PollStatus): { text: string; title: string } | null {
  switch (s.kind) {
    case "off":
      return null;
    case "push":
      return { text: "live", title: "Streaming updates from the server" };
    case "live":
      return { text: "polling", title: `Updating every ${s.everyMs / 1000}s` };
    case "paused":
      return { text: "paused", title: "Live updates pause while the tab is hidden" };
    case "retrying":
      return {
        text: `live: retry in ${Math.round(s.inMs / 1000)}s`,
        title: `Live update failed: ${s.message}`,
      };
    case "reloading":
      return { text: "resyncing...", title: "Reloading the chart to catch up" };
  }
}

export function StatusLine(props: StatusLineProps): JSX.Element {
  const badge = () => (props.live ? liveBadge(props.live) : null);
  return (
    // One line whatever the text, so the header never grows. An error is a
    // compact badge: the full message is its tooltip (the chart area shows
    // the explanation and the Retry).
    <span
      class={`text-sm font-mono whitespace-nowrap shrink-0 inline-flex items-center ${colorFor(props.status)}`}
    >
      <span
        data-testid="status-text"
        title={
          props.status.kind === "error" || props.status.kind === "auth-error"
            ? props.status.message
            : undefined
        }
        class="max-w-[28ch] truncate max-[899px]:hidden"
      >
        {formatStatus(props.status)}
      </span>
      <Show when={props.status.kind === "auth-error"}>
        <button
          type="button"
          onClick={props.onSignIn}
          class="ml-2 underline cursor-pointer hover:opacity-80"
        >
          Sign in again
        </button>
      </Show>
      <Show when={badge()}>
        {(b) => (
          <span
            data-testid="live-status"
            data-state={props.live?.kind}
            class={`live-status is-${props.live?.kind}`}
            title={b().title}
          >
            {b().text}
          </span>
        )}
      </Show>
    </span>
  );
}
