// Data Window tab of the right sidebar: date/time, OHLCV and every visible
// indicator plot of the bar under the crosshair (the last bar while the
// crosshair is off the chart), then the compare symbols. The values come
// from lib/dataWindow.ts; the crosshair index is the chart's own, already
// throttled to one update per animation frame.

import { For, Show, createMemo, type JSX } from "solid-js";

import { dataWindowRows } from "../lib/dataWindow";
import type { SeriesPayload } from "../types/responses";
import {
  activeIndicators,
  indicatorName,
  isHidden,
  type IndicatorStyle,
} from "../stores/indicators";
import { paletteColor, type CompareLine } from "./Chart";

export interface DataWindowProps {
  series: SeriesPayload | null;
  /** Bar index under the crosshair; null when it is off the chart. */
  crosshair: number | null;
  styles: Record<string, IndicatorStyle | undefined>;
  compare: CompareLine[];
  timeZone: string;
}

const DIRECTION_CLASS = {
  up: "text-[color:var(--color-bull)]",
  down: "text-[color:var(--color-bear)]",
  flat: "",
} as const;

function Row(props: {
  label: string;
  value: string;
  color?: string;
  class?: string;
  testId?: string;
}): JSX.Element {
  return (
    <div class="flex justify-between gap-2 px-3 py-0.5 text-sm">
      <span
        class="truncate text-[color:var(--color-fg-muted)]"
        style={{ color: props.color }}
      >
        {props.label}
      </span>
      <span
        class={`tabular-nums ${props.class ?? ""}`}
        style={{ color: props.color }}
        data-testid={props.testId}
      >
        {props.value}
      </span>
    </div>
  );
}

function Heading(props: { children: JSX.Element }): JSX.Element {
  return (
    <h3 class="px-3 pt-2 pb-1 text-xs font-semibold text-[color:var(--color-fg-muted)]">
      {props.children}
    </h3>
  );
}

export function DataWindow(props: DataWindowProps): JSX.Element {
  const visible = createMemo(
    () =>
      new Set(
        activeIndicators()
          .filter((e) => !isHidden(e))
          .map(indicatorName),
      ),
  );
  const data = createMemo(() =>
    props.series
      ? dataWindowRows(props.series, props.crosshair, props.styles, props.compare, {
          timeZone: props.timeZone,
          colorAt: paletteColor,
          visible: visible(),
        })
      : null,
  );

  return (
    <div class="flex-1 min-h-0 overflow-y-auto" role="region" aria-label="Data Window">
      <Show
        when={data()}
        fallback={
          <p class="px-3 py-3 text-xs text-[color:var(--color-fg-muted)]">
            No chart data yet.
          </p>
        }
      >
        {(d) => (
          <>
            <Heading>
              <span data-testid="dw-symbol">{d().symbol}</span>
              <Show when={d().interval}>{(i) => <> · {i()}</>}</Show>
            </Heading>
            <Show when={d().bar}>
              {(b) => (
                <>
                  <Row label="Time" value={d().time ?? "—"} testId="dw-time" />
                  <Row label="Open" value={b().o} testId="dw-o" />
                  <Row label="High" value={b().h} testId="dw-h" />
                  <Row label="Low" value={b().l} testId="dw-l" />
                  <Row label="Close" value={b().c} testId="dw-c" />
                  <Show when={b().change}>
                    {(ch) => (
                      <Row
                        label="Change"
                        value={`${ch().abs} (${ch().pct})`}
                        class={DIRECTION_CLASS[ch().direction]}
                        testId="dw-change"
                      />
                    )}
                  </Show>
                  <Row label="Vol" value={b().v} testId="dw-v" />
                </>
              )}
            </Show>
            <For each={d().indicators}>
              {(s) => (
                <section aria-label={s.title} data-testid={`dw-ind-${s.key}`}>
                  <Heading>{s.title}</Heading>
                  <For each={s.rows}>{(r) => <Row {...r} />}</For>
                </section>
              )}
            </For>
            <Show when={d().compare.length > 0}>
              <section aria-label="Compare">
                <Heading>Compare</Heading>
                <For each={d().compare}>{(r) => <Row {...r} />}</For>
              </section>
            </Show>
          </>
        )}
      </Show>
    </div>
  );
}
