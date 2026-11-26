// The Setups page's chart (ADR-0091): the listed setups' results per day as bars, and
// their running sum as a line, in R. Plain SVG in the app's colors: two series need no
// chart library. The plot stretches to its box; its labels are text beside it, so they
// keep their size on a phone.

import { For, Show, type JSX } from "solid-js";

import type { DayResult } from "../lib/setupsPage";
import { formatR } from "../lib/setups";

const W = 1000;
const H = 200;

export function RChart(props: { days: DayResult[] }): JSX.Element {
  const scale = () => {
    const values = props.days.flatMap((d) => [d.r, d.cumulative, 0]);
    const [lo, hi] = [Math.min(...values), Math.max(...values)];
    const y = (v: number) => ((hi - v) / (hi - lo || 1)) * H;
    const step = W / Math.max(1, props.days.length);
    return { lo, hi, y, step };
  };
  const line = () => {
    const { y, step } = scale();
    return props.days.map((d, i) => `${i === 0 ? "M" : "L"}${i * step + step / 2},${y(d.cumulative)}`).join(" ");
  };
  const label = "text-[10px] text-[color:var(--color-fg-muted)] tabular-nums";

  return (
    <Show
      when={props.days.length > 0}
      fallback={<p class="text-xs text-[color:var(--color-fg-muted)] py-6 text-center">No closed setups with a result.</p>}
    >
      <div class="grid grid-cols-[auto_1fr] gap-x-1.5" data-testid="setups-r-chart">
        <div class={`flex flex-col justify-between text-right ${label}`}>
          <span>{formatR(scale().hi)}</span>
          <span>{formatR(scale().lo)}</span>
        </div>
        <svg
          viewBox={`0 0 ${W} ${H}`}
          preserveAspectRatio="none"
          class="w-full h-40 sm:h-52"
          role="img"
          aria-label="Results per day and their running sum, in R"
        >
          <line x1={0} x2={W} y1={scale().y(0)} y2={scale().y(0)} stroke="var(--color-border)" vector-effect="non-scaling-stroke" />
          <For each={props.days}>
            {(d, i) => (
              <rect
                x={i() * scale().step + scale().step * 0.15}
                width={scale().step * 0.7}
                y={Math.min(scale().y(d.r), scale().y(0))}
                height={Math.max(1, Math.abs(scale().y(d.r) - scale().y(0)))}
                fill={d.r >= 0 ? "var(--color-bull)" : "var(--color-bear)"}
                opacity="0.6"
              >
                <title>{`${d.day}: ${formatR(d.r)} over ${d.count}, ${formatR(d.cumulative)} so far`}</title>
              </rect>
            )}
          </For>
          <path d={line()} fill="none" stroke="var(--color-accent)" stroke-width="2" vector-effect="non-scaling-stroke" />
        </svg>
        <span />
        <div class={`flex justify-between ${label}`}>
          <span>{props.days[0]!.day}</span>
          <span>{props.days.at(-1)!.day}</span>
        </div>
      </div>
    </Show>
  );
}
