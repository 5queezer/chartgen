// The operator page (ADR-0063): who signed in, which clients registered, how the
// server is. Read-only: nothing here changes anything. Names are text other people
// chose and are written as text.

import { createSignal, For, Show, type JSX } from "solid-js";

import { fetchOverview, forgetOperatorPage } from "../services/operatorPage";
import type { OperatorOverview } from "../types/responses";

const PROVIDERS: Record<string, string> = { github: "GitHub", google: "Google" };

const when = (seconds: number | null): string =>
  seconds === null ? "never" : new Date(seconds * 1000).toLocaleString();

const size = (bytes: number | null): string => {
  if (bytes === null) return "in memory";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MiB`;
};

const th = "text-left font-medium px-2 py-1 border-b border-[color:var(--color-border)] whitespace-nowrap";
const td = "px-2 py-1 border-b border-[color:var(--color-border)] align-top";
const num = `${td} text-right tabular-nums`;

export function OperatorPage(props: { overview: OperatorOverview }): JSX.Element {
  const [overview, setOverview] = createSignal(props.overview);
  const [stale, setStale] = createSignal(false);
  const refresh = async () => {
    const next = await fetchOverview();
    if (next === null) setStale(true);
    else {
      setOverview(next);
      setStale(false);
    }
  };
  const leave = () => {
    forgetOperatorPage();
    window.location.assign("/");
  };
  const server = () => overview().server;
  const on = () =>
    Object.entries(server().features)
      .filter(([, value]) => value)
      .map(([name]) => name.replace("_", " "))
      .join(", ");

  return (
    <main
      class="min-h-screen p-4 text-sm bg-[color:var(--color-bg)] text-[color:var(--color-fg)]"
      data-testid="operator-page"
    >
      <div class="flex items-center gap-3 mb-4">
        <h1 class="text-lg font-semibold">Operator</h1>
        <button
          type="button"
          class="px-2 py-1 rounded-md border border-[color:var(--color-border)] cursor-pointer"
          onClick={() => void refresh()}
        >
          Refresh
        </button>
        <button
          type="button"
          class="px-2 py-1 rounded-md border border-[color:var(--color-border)] cursor-pointer"
          onClick={leave}
        >
          Back to the chart
        </button>
        <Show when={stale()}>
          <span role="status">Could not refresh: sign in again, or the server is busy.</span>
        </Show>
      </div>

      <section aria-labelledby="operator-server" class="mb-6">
        <h2 id="operator-server" class="font-semibold mb-1">
          Server
        </h2>
        <table data-testid="operator-server">
          <tbody>
            <tr>
              <th class={th}>Version</th>
              <td class={td}>{server().version}</td>
            </tr>
            <tr>
              <th class={th}>Started</th>
              <td class={td}>{when(server().started_at)}</td>
            </tr>
            <tr>
              <th class={th}>Store</th>
              <td class={td}>
                schema {server().schema_version}, {size(server().store_bytes)}
              </td>
            </tr>
            <tr>
              <th class={th}>On</th>
              <td class={td}>{on()}</td>
            </tr>
            <tr>
              <th class={th}>Watched pairs</th>
              <td class={td}>
                {server().feeds.watched_pairs} of {server().feeds.watched_pairs_limit}
              </td>
            </tr>
            <tr>
              <th class={th}>Chart push</th>
              <td class={td}>
                {server().feeds.push_subscriptions} subscriptions on {server().feeds.push_feeds} feeds
              </td>
            </tr>
            <tr>
              <th class={th}>Setups</th>
              <td class={td}>{server().setups_total}</td>
            </tr>
            <tr>
              <th class={th}>Alert events kept</th>
              <td class={td}>{server().pending_deliveries}</td>
            </tr>
          </tbody>
        </table>
      </section>

      <section aria-labelledby="operator-users" class="mb-6">
        <h2 id="operator-users" class="font-semibold mb-1">
          Users: {overview().users.total}
          <Show when={overview().users.total > overview().users.shown.length}>
            {" "}
            (the {overview().users.shown.length} who signed in last)
          </Show>
        </h2>
        <div class="overflow-x-auto">
          <table data-testid="operator-users">
            <thead>
              <tr>
                <th class={th}>Name</th>
                <th class={th}>Id</th>
                <th class={th}>Through</th>
                <th class={th}>Created</th>
                <th class={th}>Last sign-in</th>
                <th class={th}>Last token issued</th>
                <th class={th}>Documents</th>
                <th class={th}>Setups open / closed</th>
                <th class={th}>Server alerts</th>
                <th class={th}>Authorisations</th>
                <th class={th}>Push devices</th>
              </tr>
            </thead>
            <tbody>
              <For each={overview().users.shown}>
                {(user) => (
                  <tr data-user={user.id}>
                    <td class={td}>{user.name}</td>
                    <td class={`${td} font-mono`}>{user.id}</td>
                    <td class={td}>{PROVIDERS[user.provider] ?? user.provider}</td>
                    <td class={td}>{when(user.created_at)}</td>
                    <td class={td}>{when(user.last_login_at)}</td>
                    <td class={td}>{when(user.last_token_at)}</td>
                    <td class={num}>
                      {user.workspace_docs} ({size(user.workspace_bytes)})
                    </td>
                    <td class={num}>
                      {user.setups_open} / {user.setups_closed}
                    </td>
                    <td class={num}>{user.server_alerts}</td>
                    <td class={num}>{user.authorisations}</td>
                    <td class={num}>{user.push_devices}</td>
                  </tr>
                )}
              </For>
            </tbody>
          </table>
        </div>
      </section>

      <section aria-labelledby="operator-clients">
        <h2 id="operator-clients" class="font-semibold mb-1">
          Clients: {overview().clients.total} of {overview().clients.limit}
          <Show when={overview().clients.total > overview().clients.shown.length}>
            {" "}
            (the {overview().clients.shown.length} registered last)
          </Show>
        </h2>
        <div class="overflow-x-auto">
          <table data-testid="operator-clients">
            <thead>
              <tr>
                <th class={th}>Name</th>
                <th class={th}>Id starts with</th>
                <th class={th}>Registered</th>
                <th class={th}>Last token issued</th>
                <th class={th}>Keeps an authorisation</th>
                <th class={th}>Owns setups</th>
              </tr>
            </thead>
            <tbody>
              <For each={overview().clients.shown}>
                {(client) => (
                  <tr>
                    <td class={td}>{client.name ?? "(no name)"}</td>
                    <td class={`${td} font-mono`}>{client.id}</td>
                    <td class={td}>{when(client.created_at)}</td>
                    <td class={td}>{when(client.last_token_at)}</td>
                    <td class={td}>{client.has_authorisation ? "yes" : "no"}</td>
                    <td class={td}>{client.owns_setups ? "yes" : "no"}</td>
                  </tr>
                )}
              </For>
            </tbody>
          </table>
        </div>
      </section>
    </main>
  );
}
