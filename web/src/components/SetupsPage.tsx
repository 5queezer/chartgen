// The Setups page at /setups (ADR-0091): the owner's setups as a record to read. Figures
// on top, the open and the closed setups as sortable tables, the results per day, the
// track record grouped, and one setup in a drawer with a link to its chart. Read only:
// setups are made and changed by the agent's tools. Every count and sum is the server's.

import {
  For,
  Show,
  createMemo,
  createResource,
  createSignal,
  onCleanup,
  onMount,
  type JSX,
} from "solid-js";

import { WON_AT_A_LOSS, entryText, formatR, isOpen, wonAtALoss } from "../lib/setups";
import {
  NO_FILTERS,
  age,
  featureNames,
  filterRows,
  nextSort,
  recordArgs,
  resultsByDay,
  sortRows,
  tagGroupings,
  type Column,
  type Filters,
  type Sort,
} from "../lib/setupsPage";
import { formatPrice } from "../lib/quote";
import { formatDateTime, resolveTimeZone } from "../lib/timezone";
import type { McpSession } from "../mcp";
import { enterSetupsPage, setupChartHref } from "../services/setupsPage";
import { timeZone } from "../stores/layout";
import type { GetTrackRecordInput } from "../types/generated-input";
import type {
  SetupOpenRisk,
  SetupRow,
  SetupRowsPayload,
  SetupStatus,
  TrackRecordPayload,
} from "../types/responses";
import { RChart } from "./RChart";

const REFRESH_MS = 60_000;

/** The most setups the page lists; `get_track_record` reads more for its counts. */
export const PAGE_LIMIT = 500;

export type PageLoad =
  | { kind: "off" }
  | { kind: "error"; message: string }
  | {
      kind: "ready";
      list: SetupRowsPayload;
      takes: TrackRecordPayload | null;
      skips: TrackRecordPayload | null;
    };

const reason = (err: unknown) =>
  (err instanceof Error ? err.message : String(err)).replace(/^MCP tool error: /, "");

/** The list, and the record's totals for takes and for skips under the filters. */
export async function loadSetupsPage(session: McpSession, filters: Filters = NO_FILTERS): Promise<PageLoad> {
  let list: SetupRowsPayload;
  try {
    list = await session.callListSetupRows({ limit: PAGE_LIMIT });
  } catch (err) {
    return { kind: "error", message: reason(err) };
  }
  if (!list.enabled) return { kind: "off" };
  const record = (decision: "take" | "skip") =>
    session.callGetTrackRecord({ ...recordArgs(filters), decision }).catch(() => null);
  const [takes, skips] = await Promise.all([record("take"), record("skip")]);
  return { kind: "ready", list, takes, skips };
}

const tone = (r: number | null | undefined) =>
  r === null || r === undefined || r === 0
    ? "text-[color:var(--color-fg-muted)]"
    : r > 0
      ? "text-[color:var(--color-bull)]"
      : "text-[color:var(--color-bear)]";
const muted = "text-[color:var(--color-fg-muted)]";
const panel = "rounded-md border border-[color:var(--color-border)] bg-[color:var(--color-surface)] p-3 min-w-0";
const control =
  "rounded border border-[color:var(--color-border)] bg-[color:var(--color-bg)] px-1.5 py-0.5 text-xs";
const STATUSES: SetupStatus[] = ["watching", "triggered", "won", "invalidated", "expired", "cancelled"];
const when = (ms: number, symbol: string) => formatDateTime(ms, resolveTimeZone(timeZone(), symbol));
const pct = (n: number, of: number) => (of === 0 ? "–" : `${Math.round((100 * n) / of)}%`);

export function SetupsPage(props: { session: McpSession; first: Exclude<PageLoad, { kind: "off" }> }): JSX.Element {
  const [load, setLoad] = createSignal(props.first);
  const [filters, setFilters] = createSignal<Filters>(NO_FILTERS);
  const [selected, setSelected] = createSignal<string | null>(null);
  const [now, setNow] = createSignal(Date.now());
  if (props.first.kind === "ready") enterSetupsPage();

  const refresh = async () => {
    const next = await loadSetupsPage(props.session, filters());
    setNow(Date.now());
    if (next.kind === "ready") enterSetupsPage();
    // A failed refresh keeps what is shown; a server that stopped keeping setups shows nothing.
    if (next.kind !== "error" || load().kind !== "ready") {
      setLoad(next.kind === "off" ? { kind: "error", message: "This server keeps no setups." } : next);
    }
  };
  const setFilter = <K extends keyof Filters>(key: K, value: Filters[K]) => {
    setFilters({ ...filters(), [key]: value });
    void refresh();
  };
  onMount(() => {
    const timer = window.setInterval(() => void refresh(), REFRESH_MS);
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setSelected(null);
    window.addEventListener("keydown", onKey);
    onCleanup(() => {
      window.clearInterval(timer);
      window.removeEventListener("keydown", onKey);
    });
  });

  const ready = () => {
    const l = load();
    return l.kind === "ready" ? l : null;
  };
  const rows = createMemo(() => ready()?.list.setups ?? []);
  const shown = createMemo(() => filterRows(rows(), filters()));
  const open = createMemo(() => shown().filter(isOpen));
  const closed = createMemo(() => shown().filter((s) => !isOpen(s)));
  const options = (pick: (s: SetupRow) => string[]) => [...new Set(rows().flatMap(pick))].sort();

  return (
    <main
      class="min-h-screen p-3 sm:p-4 text-sm bg-[color:var(--color-bg)] text-[color:var(--color-fg)] flex flex-col gap-3"
      data-testid="setups-page"
    >
      <header class="flex flex-wrap items-center gap-2">
        <h1 class="text-lg font-semibold mr-auto">Setups</h1>
        <span class={`hidden sm:inline text-xs ${muted}`}>read only: your agent makes and changes setups</span>
        <button type="button" class={`${control} cursor-pointer`} onClick={() => void refresh()}>
          Refresh
        </button>
        <a href="/" class={`${control} no-underline text-[color:var(--color-fg)]`}>
          Back to the chart
        </a>
      </header>
      <Show when={(() => { const l = load(); return l.kind === "error" ? l : null; })()}>
        {(l) => (
          <p role="status" data-role="setups-error" class="text-[color:var(--color-danger)]">
            Setups could not be read: {l().message}
          </p>
        )}
      </Show>
      <Show when={ready()}>
        {(r) => (
          <>
            <Figures takes={r().takes} skips={r().skips} risk={r().list.open_risk} />
            <div class="flex flex-wrap items-center gap-2 text-xs" role="group" aria-label="Filters">
              <Select label="Status" value={filters().status} options={["all", ...STATUSES]} onChange={(v) => setFilter("status", v as Filters["status"])} />
              <Select label="Decision" value={filters().decision} options={["all", "take", "skip"]} onChange={(v) => setFilter("decision", v as Filters["decision"])} />
              <Select label="Side" value={filters().direction} options={["all", "long", "short"]} onChange={(v) => setFilter("direction", v as Filters["direction"])} />
              <Select label="Symbol" value={filters().symbol} options={["", ...options((s) => [s.symbol])]} onChange={(v) => setFilter("symbol", v)} />
              <Select label="Tag" value={filters().tag} options={["", ...options((s) => s.tags)]} onChange={(v) => setFilter("tag", v)} />
              <label class="flex items-center gap-1">
                From
                <input type="date" aria-label="Made from" class={control} value={filters().from} onChange={(e) => setFilter("from", e.currentTarget.value)} />
              </label>
              <label class="flex items-center gap-1">
                to
                <input type="date" aria-label="Made until" class={control} value={filters().to} onChange={(e) => setFilter("to", e.currentTarget.value)} />
              </label>
              <button type="button" class={`${control} cursor-pointer`} onClick={() => { setFilters(NO_FILTERS); void refresh(); }}>
                Clear
              </button>
              <Show when={(r().list.total ?? 0) > r().list.setups.length}>
                <span class={muted}>
                  The newest {r().list.setups.length} of {r().list.total} setups are listed.
                </span>
              </Show>
            </div>
            <div class="grid gap-3 lg:grid-cols-3">
              <section class={`${panel} lg:col-span-2`} aria-labelledby="setups-results">
                <h2 id="setups-results" class="font-semibold mb-1">Results per day</h2>
                <p class={`text-xs ${muted} mb-1`}>
                  Each closed setup listed here at its own result, by the day it ended; the line is their running sum.
                </p>
                <RChart days={resultsByDay(closed())} />
              </section>
              <section class={panel} aria-labelledby="setups-risk">
                <h2 id="setups-risk" class="font-semibold mb-1">Open risk by symbol</h2>
                <RiskTable risk={r().list.open_risk} />
              </section>
            </div>
            <section class={panel} aria-labelledby="setups-open">
              <h2 id="setups-open" class="font-semibold mb-1">Open: {open().length}</h2>
              <SetupTable
                name="open"
                rows={open()}
                columns={["symbol", "side", "status", "entry", "stop", "target", "mfe", "mae", "made", "expires"]}
                initial={{ column: "made", descending: true }}
                now={now()}
                selected={selected()}
                onSelect={setSelected}
              />
            </section>
            <section class={panel} aria-labelledby="setups-closed">
              <h2 id="setups-closed" class="font-semibold mb-1">Closed: {closed().length}</h2>
              <SetupTable
                name="closed"
                rows={closed()}
                columns={["symbol", "side", "status", "entry", "stop", "target", "result", "mfe", "mae", "made", "ended"]}
                initial={{ column: "ended", descending: true }}
                now={now()}
                selected={selected()}
                onSelect={setSelected}
              />
            </section>
            <Breakdown session={props.session} filters={filters()} rows={rows()} stamp={now()} />
          </>
        )}
      </Show>
      <Show when={selected()}>
        {(id) => <Detail session={props.session} id={id()} onClose={() => setSelected(null)} />}
      </Show>
    </main>
  );
}

function Select(props: { label: string; name?: string; value: string; options: string[]; onChange: (v: string) => void }): JSX.Element {
  return (
    <label class="flex items-center gap-1">
      {props.label}
      <select class={control} aria-label={props.name ?? props.label} value={props.value} onChange={(e) => props.onChange(e.currentTarget.value)}>
        <For each={props.options}>{(o) => <option value={o}>{o === "" ? "all" : o}</option>}</For>
      </select>
    </label>
  );
}

function Figure(props: { label: string; role: string; children: JSX.Element; sub?: JSX.Element }): JSX.Element {
  return (
    <div class={panel} data-role={props.role}>
      <div class={`text-xs ${muted}`}>{props.label}</div>
      <div class="text-lg font-semibold tabular-nums">{props.children}</div>
      <div class={`text-xs ${muted} tabular-nums`}>{props.sub}</div>
    </div>
  );
}

/** The figures on top: what the server counts, as it counts them. */
function Figures(props: {
  takes: TrackRecordPayload | null;
  skips: TrackRecordPayload | null;
  risk: SetupOpenRisk | undefined;
}): JSX.Element {
  const side = (d: "long" | "short") => (
    <Figure
      label={`Open risk ${d}`}
      role={`risk-${d}`}
      sub={<>{props.risk?.[d].filled ?? 0} filled, {formatR(props.risk?.[d].filled_at_stop_r ?? 0)} at their stops</>}
    >
      {props.risk?.[d].risk_r ?? 0}R
    </Figure>
  );
  const record = (label: string, role: string, rec: TrackRecordPayload | null) => (
    <Figure
      label={label}
      role={role}
      sub={
        rec && (
          <>
            {rec.total.net_gain} of {rec.total.count} above 0 R after costs ({pct(rec.total.net_gain, rec.total.count)}), mean{" "}
            {formatR(rec.total.mean_r)}
          </>
        )
      }
    >
      <span data-role="total-r" class={tone(rec?.total.total_r)}>
        {rec ? formatR(rec.total.total_r) : "–"}
      </span>
    </Figure>
  );
  return (
    <div class="grid gap-3 grid-cols-2 md:grid-cols-3 xl:grid-cols-6" data-testid="setups-figures">
      <Figure label="Open" role="open" sub={<>{props.takes?.open ?? "–"} taken, {props.skips?.open ?? "–"} skipped</>}>
        {(props.takes?.open ?? 0) + (props.skips?.open ?? 0)}
      </Figure>
      {side("long")}
      {side("short")}
      <Figure label="Closed" role="closed" sub={<>{props.takes?.closed ?? "–"} taken, {props.skips?.closed ?? "–"} skipped</>}>
        {(props.takes?.closed ?? 0) + (props.skips?.closed ?? 0)}
      </Figure>
      {record("Taken, R after costs", "taken", props.takes)}
      {record("Skipped, R after costs", "skipped", props.skips)}
    </div>
  );
}

function RiskTable(props: { risk: SetupOpenRisk | undefined }): JSX.Element {
  return (
    <Show when={(props.risk?.by_symbol.length ?? 0) > 0} fallback={<p class={`text-xs ${muted}`}>No open takes.</p>}>
      <table class="w-full text-xs tabular-nums" data-testid="setups-risk">
        <thead>
          <tr class={muted}>
            <th class="text-left font-normal">Symbol</th>
            <th class="text-left font-normal">Side</th>
            <th class="text-right font-normal">Takes</th>
            <th class="text-right font-normal">Filled</th>
            <th class="text-right font-normal">At stops</th>
          </tr>
        </thead>
        <tbody>
          <For each={props.risk!.by_symbol}>
            {(row) => (
              <tr>
                <td>{row.symbol}</td>
                <td class={row.direction === "long" ? "text-[color:var(--color-bull)]" : "text-[color:var(--color-bear)]"}>{row.direction}</td>
                <td class="text-right">{row.takes}</td>
                <td class="text-right">{row.filled}</td>
                <td class={`text-right ${tone(row.filled_at_stop_r)}`}>{formatR(row.filled_at_stop_r)}</td>
              </tr>
            )}
          </For>
        </tbody>
      </table>
    </Show>
  );
}

const HEADS: Record<Column, string> = {
  symbol: "Symbol",
  side: "Side",
  status: "Status",
  entry: "Entry",
  stop: "Stop",
  target: "T1",
  mfe: "MFE",
  mae: "MAE",
  result: "Result",
  made: "Made",
  ended: "Ended",
  expires: "Expires",
};

function cell(s: SetupRow, column: Column, now: number): JSX.Element {
  switch (column) {
    case "symbol":
      return <><span class="font-semibold">{s.symbol}</span> <span class={`text-xs ${muted}`}>{s.interval}</span></>;
    case "side":
      return (
        <>
          <span class={s.direction === "long" ? "text-[color:var(--color-bull)]" : "text-[color:var(--color-bear)]"}>{s.direction}</span>
          <Show when={s.decision === "skip"}> <span class={`text-xs ${muted}`}>skip</span></Show>
        </>
      );
    case "status":
      return <span data-role="status">{s.status}</span>;
    case "entry":
      return entryText(s, formatPrice);
    case "stop":
      return formatPrice(s.invalidation);
    case "target":
      return formatPrice(s.targets[0] ?? null);
    case "mfe":
      return <span class={tone(s.mfe_r)}>{formatR(s.mfe_r)}</span>;
    case "mae":
      return <span class={tone(s.mae_r)}>{formatR(s.mae_r)}</span>;
    case "result":
      return (
        <span data-role="result" class={`font-semibold ${tone(s.result_r)}`} title={wonAtALoss(s) ? WON_AT_A_LOSS : undefined}>
          {formatR(s.result_r)}
          <Show when={wonAtALoss(s)}> ⚠</Show>
          <Show when={s.gap}> <span class={muted} title="Bars were missed and not scored">gap</span></Show>
        </span>
      );
    case "made":
      return <span title={when(s.created_at, s.symbol)}>{age(s.created_at, now)}</span>;
    case "ended":
      return when(s.exit?.bar_time ?? s.scored_through ?? s.created_at, s.symbol);
    case "expires":
      return when(s.expires_at, s.symbol);
  }
}

function SetupTable(props: {
  name: string;
  rows: SetupRow[];
  columns: Column[];
  initial: Sort;
  now: number;
  selected: string | null;
  onSelect: (id: string) => void;
}): JSX.Element {
  const [sort, setSort] = createSignal<Sort>(props.initial);
  const sorted = createMemo(() => sortRows(props.rows, sort()));
  return (
    <Show when={props.rows.length > 0} fallback={<p class={`text-xs ${muted}`}>None.</p>}>
      <div class="overflow-x-auto">
        <table class="w-full text-xs tabular-nums whitespace-nowrap" data-testid={`setups-${props.name}`}>
          <thead>
            <tr>
              <For each={props.columns}>
                {(c) => (
                  <th
                    class="text-left font-medium px-1.5 py-1 border-b border-[color:var(--color-border)]"
                    aria-sort={sort().column === c ? (sort().descending ? "descending" : "ascending") : "none"}
                  >
                    <button type="button" class="cursor-pointer" onClick={() => setSort(nextSort(sort(), c))}>
                      {HEADS[c]}
                      {sort().column === c ? (sort().descending ? " ▾" : " ▴") : ""}
                    </button>
                  </th>
                )}
              </For>
            </tr>
          </thead>
          <tbody>
            <For each={sorted()}>
              {(s) => (
                <tr
                  data-setup-id={s.id}
                  tabIndex={0}
                  aria-selected={props.selected === s.id}
                  class="wl-row cursor-pointer aria-selected:bg-[color:var(--color-bg-elev)]"
                  onClick={() => props.onSelect(s.id)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === " ") {
                      e.preventDefault();
                      props.onSelect(s.id);
                    }
                  }}
                >
                  <For each={props.columns}>
                    {(c) => <td class="px-1.5 py-1 border-b border-[color:var(--color-border)]" data-col={c}>{cell(s, c, props.now)}</td>}
                  </For>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>
    </Show>
  );
}

const GROUPINGS = ["symbol", "direction", "playbook_version", "skip_reason", "tag", "feature", "probability", "bracket_anchor"] as const;
type Grouping = (typeof GROUPINGS)[number];

/** The track record under the filters, grouped by one thing, as `get_track_record` answers. */
function Breakdown(props: { session: McpSession; filters: Filters; rows: SetupRow[]; stamp: number }): JSX.Element {
  const [by, setBy] = createSignal<Grouping>("symbol");
  const [detail, setDetail] = createSignal("");
  const tags = createMemo(() => tagGroupings(props.rows));
  const features = createMemo(() => featureNames(props.rows));
  const args = createMemo((): GetTrackRecordInput | null => {
    const base = { ...recordArgs(props.filters), group_by: by() };
    void props.stamp;
    if (by() === "tag") {
      const g = tags().find((t) => t.label === detail()) ?? tags()[0];
      return g ? { ...base, tag_prefixes: g.prefixes as NonNullable<GetTrackRecordInput["tag_prefixes"]> } : null;
    }
    if (by() === "feature") {
      const f = features().includes(detail()) ? detail() : features()[0];
      return f ? { ...base, feature: f } : null;
    }
    return base;
  });
  const [record] = createResource(args, (a) => props.session.callGetTrackRecord(a));
  const maxR = () => Math.max(1e-9, ...(record()?.groups ?? []).map((g) => Math.abs(g.total_r)));

  return (
    <section class={panel} aria-labelledby="setups-record">
      <div class="flex flex-wrap items-center gap-2 mb-2">
        <h2 id="setups-record" class="font-semibold mr-auto">Track record by</h2>
        <Select label="" name="Group by" value={by()} options={[...GROUPINGS]} onChange={(v) => { setBy(v as Grouping); setDetail(""); }} />
        <Show when={by() === "tag"}>
          <Select label="" name="Tags to group by" value={detail()} options={tags().map((t) => t.label)} onChange={setDetail} />
        </Show>
        <Show when={by() === "feature"}>
          <Select label="" name="Feature to group by" value={detail()} options={features()} onChange={setDetail} />
        </Show>
      </div>
      <Show when={args() === null}>
        <p class={`text-xs ${muted}`}>No {by() === "tag" ? "tags" : "features"} to group by.</p>
      </Show>
      <Show when={record.error}>
        <p class="text-xs text-[color:var(--color-danger)]">The track record could not be read: {reason(record.error)}</p>
      </Show>
      <Show when={args() !== null && record()}>
        {(rec) => (
          <div class="overflow-x-auto">
            <table class="w-full text-xs tabular-nums whitespace-nowrap" data-testid="setups-record">
              <thead>
                <tr class={muted}>
                  <For each={["", "Closed", "Won", "Above 0 R", "Below 0 R", "Total R", "", "Mean R", "Entered", "Mean R entered", "Gaps"]}>
                    {(h, i) => <th class={`font-normal px-1.5 ${i() === 0 || i() === 6 ? "text-left" : "text-right"}`}>{h}</th>}
                  </For>
                </tr>
              </thead>
              <tbody>
                <For each={[...rec().groups.map((g) => ({ ...g, label: g.key === null ? "(none)" : String(g.key) })), { ...rec().total, label: "All" }]}>
                  {(g) => (
                    <tr data-group={g.label} class={g.label === "All" ? "font-semibold" : ""}>
                      <td class="px-1.5 py-0.5">{g.label}</td>
                      <td class="px-1.5 text-right">{g.count}</td>
                      <td class="px-1.5 text-right">{g.won}</td>
                      <td class="px-1.5 text-right">{g.net_gain}</td>
                      <td class="px-1.5 text-right">{g.net_loss}</td>
                      <td class={`px-1.5 text-right ${tone(g.total_r)}`} data-role="total-r">{formatR(g.total_r)}</td>
                      <td class="px-1.5 w-24">
                        <Show when={g.label !== "All"}>
                          <div
                            class="h-2 rounded-sm"
                            style={{
                              width: `${(100 * Math.abs(g.total_r)) / maxR()}%`,
                              background: g.total_r >= 0 ? "var(--color-bull)" : "var(--color-bear)",
                            }}
                          />
                        </Show>
                      </td>
                      <td class={`px-1.5 text-right ${tone(g.mean_r)}`}>{formatR(g.mean_r)}</td>
                      <td class="px-1.5 text-right">{g.entered}</td>
                      <td class={`px-1.5 text-right ${tone(g.mean_r_entered)}`}>{formatR(g.mean_r_entered)}</td>
                      <td class="px-1.5 text-right">{g.gaps}</td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
            <p class={`text-xs ${muted} mt-1`}>
              {by() === "tag" ? "A setup is in the group of each prefix one of its tags starts with: groups can overlap. " : ""}
              Counts and sums from the server, over {rec().closed} closed setups{rec().truncated ? " (the newest; older ones were left out)" : ""}. A group of a few setups says little.
            </p>
          </div>
        )}
      </Show>
    </section>
  );
}

const VARIANTS = { v1_half_t1_be: "half at T1, rest at T2", v2_be_at_1r: "stop to break-even at +1R", v3_time_stop: "time stop" } as const;

/** One setup, read whole with `get_setup`, with the link that opens it on its chart. */
function Detail(props: { session: McpSession; id: string; onClose: () => void }): JSX.Element {
  const [setup] = createResource(() => props.id, (id) => props.session.callGetSetup({ id }));
  const dl = "grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 text-xs tabular-nums";
  return (
    <aside
      class="fixed inset-y-0 right-0 z-10 w-full sm:w-[26rem] overflow-y-auto border-l border-[color:var(--color-border)] bg-[color:var(--color-surface)] p-4 flex flex-col gap-3 shadow-xl"
      aria-label="Setup"
      data-testid="setup-detail"
    >
      <div class="flex items-center gap-2">
        <a href={setupChartHref(props.id)} class={`${control} no-underline text-[color:var(--color-fg)]`}>
          Open chart
        </a>
        <span class="flex-1" />
        <button type="button" class={`${control} cursor-pointer`} onClick={() => props.onClose()}>
          Close
        </button>
      </div>
      <Show when={setup.error}>
        <p class="text-xs text-[color:var(--color-danger)]">The setup could not be read: {reason(setup.error)}</p>
      </Show>
      <Show when={setup()}>
        {(s) => (
          <>
            <h2 class="text-base font-semibold">
              {s().symbol} {s().interval} {s().direction}
              {s().decision === "skip" ? " (skip)" : ""} · {s().status}{" "}
              <span class={tone(s().result_r)}>{isOpen(s()) ? "" : formatR(s().result_r)}</span>
            </h2>
            <Show when={wonAtALoss(s())}>
              <p class="text-xs text-[color:var(--color-danger)]" title={WON_AT_A_LOSS}>⚠ Won below 0 R: the costs were more than the move to T1.</p>
            </Show>
            <p class="whitespace-pre-wrap" data-role="thesis">{s().thesis}</p>
            <dl class={dl}>
              <dt class={muted}>Entry</dt><dd>{entryText({ ...s(), fill: null }, formatPrice)}</dd>
              <dt class={muted}>Fill</dt><dd>{s().fill ? `${formatPrice(s().fill!.price)} at ${when(s().fill!.bar_time, s().symbol)}` : "–"}</dd>
              <dt class={muted}>Stop</dt><dd>{formatPrice(s().invalidation)}</dd>
              <dt class={muted}>Targets</dt><dd>{s().targets.map(formatPrice).join(", ")}</dd>
              <dt class={muted}>Made</dt><dd>{when(s().created_at, s().symbol)}</dd>
              <dt class={muted}>Expires</dt><dd>{when(s().expires_at, s().symbol)}</dd>
              <dt class={muted}>Exit</dt><dd>{s().exit ? `${formatPrice(s().exit!.price)} at ${when(s().exit!.bar_time, s().symbol)}` : "–"}</dd>
              <dt class={muted}>Net</dt><dd>{s().net ?? "–"}{s().gap ? ", bars were missed and not scored" : ""}</dd>
              <dt class={muted}>MFE / MAE</dt><dd>{formatR(s().mfe_r)} / {formatR(s().mae_r)} (before costs)</dd>
              <dt class={muted}>Bars to fill / exit</dt><dd>{s().bars_to_fill ?? "–"} / {s().bars_to_exit ?? "–"}</dd>
              <dt class={muted}>Levels by</dt><dd>{s().bracket}{s().bracket_anchor ? `, from the ${s().bracket_anchor === "fill" ? "fill" : "signal's close"}` : ""}</dd>
              <dt class={muted}>Costs</dt><dd>{s().costs.fee_bps_per_side} + {s().costs.slippage_bps_per_side} bps a side</dd>
              <Show when={s().playbook_version}><dt class={muted}>Playbook</dt><dd>{s().playbook_version}</dd></Show>
              <Show when={s().probability !== undefined}><dt class={muted}>Probability</dt><dd>{s().probability}</dd></Show>
              <Show when={s().skip_reason}><dt class={muted}>Skip reason</dt><dd>{s().skip_reason}</dd></Show>
              <Show when={s().tags.length > 0}><dt class={muted}>Tags</dt><dd>{s().tags.join(", ")}</dd></Show>
              <For each={Object.entries(s().features ?? {})}>
                {([k, v]) => <><dt class={muted}>{k}</dt><dd>{String(v)}</dd></>}
              </For>
            </dl>
            <Show when={s().shadow_results}>
              {(shadows) => (
                <section>
                  <h3 class="text-xs font-semibold mb-1">Shadow exits, for comparison only</h3>
                  <dl class={dl} data-role="shadows">
                    <For each={Object.entries(VARIANTS) as [keyof typeof VARIANTS, string][]}>
                      {([k, label]) => (
                        <>
                          <dt class={muted}>{label}</dt>
                          <dd>{shadows()[k].status} <span class={tone(shadows()[k].result_r)}>{formatR(shadows()[k].result_r)}</span></dd>
                        </>
                      )}
                    </For>
                  </dl>
                </section>
              )}
            </Show>
          </>
        )}
      </Show>
    </aside>
  );
}
