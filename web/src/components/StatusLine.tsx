// Status indicator next to the form. Pure presentational — the state
// machine lives in App.tsx. v0.1 statuses: ready, authenticating...,
// loading..., loaded, error: <message>.

import type { JSX } from "solid-js";

export type Status =
  | { kind: "ready" }
  | { kind: "authenticating" }
  | { kind: "loading" }
  | { kind: "loaded" }
  | { kind: "error"; message: string };

export interface StatusLineProps {
  status: Status;
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
      return `error: ${s.message}`;
  }
}

function colorFor(s: Status): string {
  switch (s.kind) {
    case "error":
      return "text-[color:var(--color-danger)]";
    case "loaded":
      return "text-[color:var(--color-accent)]";
    default:
      return "text-[color:var(--color-fg-muted)]";
  }
}

export function StatusLine(props: StatusLineProps): JSX.Element {
  return (
    <span class={`text-sm font-mono ${colorFor(props.status)}`}>
      {formatStatus(props.status)}
    </span>
  );
}
