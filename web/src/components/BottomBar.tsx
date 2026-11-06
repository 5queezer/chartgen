// Bottom bar under the chart (TradingView's): date-range buttons that pick
// an interval and fit the range, a go-to-date dialog (Alt+G), the clock
// and the time zone selector. The zone itself lives in stores/layout.

import { Dialog } from "@kobalte/core";
import {
  For,
  Show,
  createEffect,
  createMemo,
  createSignal,
  onCleanup,
  onMount,
  type JSX,
} from "solid-js";

import type { Chart as KLineChart } from "klinecharts";

import { centerOnTime, fitRange } from "../lib/chartNav";
import { isTypingContext } from "../lib/keys";
import type { LoadUntilResult } from "../lib/liveFeed";
import {
  RANGES,
  RANGE_TIMEFRAME,
  barAlignedStart,
  rangeStart,
  type RangeId,
} from "../lib/range";
import type { Timeframe } from "../lib/timeframe";
import {
  COMMON_TIME_ZONES,
  isDateBar,
  formatClock,
  localTimeZone,
  offsetLabel,
  resolveTimeZone,
  wallClock,
  zonedToUtc,
} from "../lib/timezone";
import { setTimeframe, setTimeZone, symbol, timeframe, timeZone } from "../stores/layout";
import type { SeriesPayload } from "../types/responses";

/** How long a new interval may take to load before a range gives up. */
const SERIES_TIMEOUT_MS = 30_000;
const NOTICE_MS = 5000;

const pad = (n: number) => String(n).padStart(2, "0");

const buttonClass =
  "px-2 py-1 rounded text-xs cursor-pointer text-[color:var(--color-fg)] hover:bg-[color:var(--color-bg-elev)] disabled:opacity-50 disabled:cursor-default";
const inputClass =
  "w-full bg-[color:var(--color-bg-elev)] border border-[color:var(--color-border)] rounded-md px-3 py-2 text-sm text-[color:var(--color-fg)] focus:outline-none focus:border-[color:var(--color-accent)]";
const actionClass =
  "px-3 py-1.5 rounded-md text-sm cursor-pointer border border-[color:var(--color-border)] hover:bg-[color:var(--color-bg-elev)]";
const primaryClass =
  "px-3 py-1.5 rounded-md text-sm cursor-pointer bg-[color:var(--color-accent)] text-black font-semibold disabled:opacity-50";

const [dialogOpen, setDialogOpen] = createSignal(false);

/** Open the go-to-date dialog, for the time axis menu. */
export const openGoToDate = (): void => {
  setDialogOpen(true);
};

export interface BottomBarProps {
  chart: KLineChart | null;
  series: SeriesPayload | null;
  /** Page history back to a time (liveFeed.loadUntil). */
  loadUntil: (ts: number, isCurrent?: () => boolean) => Promise<LoadUntilResult>;
}

export function BottomBar(props: BottomBarProps): JSX.Element {
  const [now, setNow] = createSignal(Date.now());
  const [busy, setBusy] = createSignal(false);
  const [notice, setNotice] = createSignal<string | null>(null);
  // Each navigation takes a number; a later one (or a new click) cancels the
  // earlier one at its next await.
  let job = 0;
  let noticeTimer: ReturnType<typeof setTimeout> | undefined;
  let waiting: { interval: string; done: (ok: boolean) => void } | null = null;

  const tz = createMemo(() => resolveTimeZone(timeZone(), symbol()));

  onMount(() => {
    let timer: ReturnType<typeof setTimeout>;
    const tick = () => {
      setNow(Date.now());
      timer = setTimeout(tick, 1000 - (Date.now() % 1000));
    };
    timer = setTimeout(tick, 1000 - (Date.now() % 1000));
    const onKey = (e: KeyboardEvent) => {
      if (
        e.altKey &&
        !e.ctrlKey &&
        !e.metaKey &&
        e.code === "KeyG" &&
        !e.repeat &&
        props.series &&
        !isTypingContext(e)
      ) {
        e.preventDefault();
        setDialogOpen(true);
      }
    };
    window.addEventListener("keydown", onKey);
    onCleanup(() => {
      clearTimeout(timer);
      clearTimeout(noticeTimer);
      window.removeEventListener("keydown", onKey);
    });
  });

  const say = (text: string | null) => {
    clearTimeout(noticeTimer);
    setNotice(text);
    if (text !== null) noticeTimer = setTimeout(() => setNotice(null), NOTICE_MS);
  };

  // A new interval's data arrives as a new series; resolve its waiter. It
  // fails at once if the interval is changed again or the load failed
  // (series cleared), so the buttons are not held for the whole timeout.
  createEffect(() => {
    const series = props.series;
    const current = timeframe();
    if (!waiting) return;
    if (series?.interval === waiting.interval) waiting.done(true);
    else if (series === null || current !== waiting.interval) waiting.done(false);
  });

  const seriesFor = (interval: Timeframe) =>
    new Promise<boolean>((resolve) => {
      const timer = setTimeout(() => done(false), SERIES_TIMEOUT_MS);
      const done = (ok: boolean) => {
        clearTimeout(timer);
        if (waiting?.done === done) waiting = null;
        resolve(ok);
      };
      waiting = { interval, done };
    });

  /** After the data landed, once klinecharts has painted it. */
  const settled = () => new Promise<void>((r) => requestAnimationFrame(() => r()));

  /** Run a navigation; `fn` returns early once `isCurrent()` turns false. */
  const navigate = async (fn: (isCurrent: () => boolean) => Promise<void>) => {
    const my = ++job;
    say(null);
    setBusy(true);
    try {
      await fn(() => my === job);
    } finally {
      if (my === job) setBusy(false);
    }
  };

  const selectRange = (range: RangeId) =>
    navigate(async (isCurrent) => {
      const interval = RANGE_TIMEFRAME[range];
      if (interval !== timeframe()) {
        setTimeframe(interval);
        if (!(await seriesFor(interval)) || !isCurrent()) {
          if (isCurrent() && timeframe() === interval) say("The new interval did not load");
          return;
        }
      }
      const last = props.series?.bars.at(-1)?.t;
      if (last == null) return;
      const sym = symbol();
      let start = rangeStart(range, last * 1000, tz());
      // Daily and longer bars carry a date: keep the one that holds `start`.
      if (start !== null && isDateBar(interval)) start = barAlignedStart(start, tz());
      const result = await props.loadUntil(start === null ? -Infinity : start / 1000, isCurrent);
      await settled();
      const chart = props.chart;
      if (!isCurrent() || result === "aborted" || !chart) return;
      if (symbol() !== sym || timeframe() !== interval) return;
      fitRange(chart, start);
      if (result === "limit") say("History limit reached");
    });

  const goToDate = (ms: number) =>
    navigate(async (isCurrent) => {
      const sym = symbol();
      const tf = timeframe();
      const result = await props.loadUntil(ms / 1000, isCurrent);
      await settled();
      const chart = props.chart;
      if (!isCurrent() || result === "aborted" || !chart) return;
      if (symbol() !== sym || timeframe() !== tf) return;
      const first = chart.getDataList()[0]?.timestamp;
      if (first === undefined) return;
      if (ms < first) {
        say(
          result === "error"
            ? "Could not load history: showing the oldest loaded bar"
            : result === "limit"
              ? "History limit reached: showing the oldest loaded bar"
              : "Date is before the start of history: showing the first bar",
        );
      }
      centerOnTime(chart, Math.max(ms, first));
    });

  const clock = () => formatClock(now(), tz());

  return (
    <footer
      class="flex items-center gap-1 px-3 py-1 border-t border-[color:var(--color-border)] text-xs"
      data-testid="bottom-bar"
    >
      <For each={RANGES}>
        {(range) => (
          <button
            type="button"
            class={buttonClass}
            data-testid={`range-${range}`}
            title={`${range}: ${RANGE_TIMEFRAME[range]} bars`}
            disabled={busy() || !props.series}
            onClick={() => void selectRange(range)}
          >
            {range}
          </button>
        )}
      </For>
      <button
        type="button"
        class={buttonClass}
        data-testid="goto-date"
        aria-label="Go to date (Alt+G)"
        title="Go to date (Alt+G)"
        disabled={!props.series}
        onClick={() => setDialogOpen(true)}
      >
        <svg
          width="14"
          height="14"
          viewBox="0 0 14 14"
          fill="none"
          stroke="currentColor"
          stroke-width="1.3"
          aria-hidden="true"
        >
          <rect x="1.5" y="2.5" width="11" height="10" rx="1.5" />
          <path d="M1.5 5.5h11M4.5 1v3M9.5 1v3" />
        </svg>
      </button>
      <span
        class="ml-2 text-[color:var(--color-fg-muted)]"
        role="status"
        data-testid="bottom-notice"
      >
        <Show when={busy()} fallback={notice()}>
          Loading…
        </Show>
      </span>
      <span class="ml-auto tabular-nums" data-testid="clock">
        {clock()} {offsetLabel(now(), tz())}
      </span>
      <select
        aria-label="Time zone"
        data-testid="tz-select"
        class="bg-[color:var(--color-bg-elev)] border border-[color:var(--color-border)] rounded px-1 py-0.5 text-xs text-[color:var(--color-fg)] cursor-pointer"
        value={timeZone()}
        onChange={(e) => setTimeZone(e.currentTarget.value)}
      >
        <option value="utc">UTC</option>
        <option value="exchange">
          Exchange ({resolveTimeZone("exchange", symbol())})
        </option>
        <option value="local">Local ({localTimeZone()})</option>
        <optgroup label="Time zones">
          <For each={COMMON_TIME_ZONES}>{(z) => <option value={z}>{z}</option>}</For>
        </optgroup>
      </select>

      <Dialog.Root open={dialogOpen()} onOpenChange={setDialogOpen} modal>
        <Dialog.Portal>
          <Dialog.Overlay class="fixed inset-0 z-40 bg-black/40" />
          <div class="fixed inset-0 z-50 flex items-start justify-center pt-[20vh] px-4 pointer-events-none">
            <Dialog.Content class="pointer-events-auto w-80 bg-[color:var(--color-bg)] border border-[color:var(--color-border)] rounded-lg shadow-xl p-4">
              <GoToDateForm
                tz={tz()}
                onSubmit={(ms) => {
                  setDialogOpen(false);
                  void goToDate(ms);
                }}
                onCancel={() => setDialogOpen(false)}
              />
            </Dialog.Content>
          </div>
        </Dialog.Portal>
      </Dialog.Root>
    </footer>
  );
}

/** Date and optional time in `tz`; also the replay's "Select date" (ReplayBar). */
export function GoToDateForm(props: {
  tz: string;
  title?: string;
  onSubmit: (ms: number) => void;
  onCancel: () => void;
}): JSX.Element {
  const today = wallClock(Date.now(), props.tz);
  const [date, setDate] = createSignal(
    `${today.year}-${pad(today.month)}-${pad(today.day)}`,
  );
  const [time, setTime] = createSignal("");

  const target = () => {
    const d = /^(\d{4})-(\d{2})-(\d{2})$/.exec(date());
    if (!d) return null;
    const t = /^(\d{2}):(\d{2})$/.exec(time());
    return zonedToUtc(
      {
        year: Number(d[1]),
        month: Number(d[2]),
        day: Number(d[3]),
        hour: Number(t?.[1] ?? 0),
        minute: Number(t?.[2] ?? 0),
        second: 0,
      },
      props.tz,
    );
  };

  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        const ms = target();
        if (ms !== null) props.onSubmit(ms);
      }}
    >
      <Dialog.Title class="text-sm font-semibold mb-2">
        {props.title ?? "Go to date"}
      </Dialog.Title>
      <div class="flex gap-2">
        <input
          type="date"
          required
          autofocus
          aria-label="Date"
          class={inputClass}
          value={date()}
          onInput={(e) => setDate(e.currentTarget.value)}
        />
        <input
          type="time"
          aria-label="Time (optional)"
          class={inputClass}
          value={time()}
          onInput={(e) => setTime(e.currentTarget.value)}
        />
      </div>
      <div class="mt-3 flex justify-end gap-2">
        <button type="button" class={actionClass} onClick={props.onCancel}>
          Cancel
        </button>
        <button type="submit" class={primaryClass} disabled={target() === null}>
          OK
        </button>
      </div>
    </form>
  );
}
