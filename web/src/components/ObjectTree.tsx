// Object Tree tab of the right sidebar: everything on the chart in one list.
// Main series, compare symbols, indicators (hide/show, settings, remove),
// drawings (hide/show, lock, remove; a click selects the drawing and
// scrolls to it) and the current symbol's alerts. Every action goes through
// the store or controller that already owns it.

import { For, Show, type JSX } from "solid-js";

import { CONDITION_LABELS, alertStatus } from "../lib/alerts";
import { COMPARE_PALETTE } from "../lib/compare";
import { pricePrecision } from "../lib/format";
import { describeDrawing } from "../lib/objectTree";
import { formatPrice } from "../lib/quote";
import type { SeriesPayload } from "../types/responses";
import { alerts, removeAlert } from "../stores/alerts";
import { compareEntries, removeCompare } from "../stores/compare";
import {
  activeIndicators,
  indicatorName,
  instanceKey,
  isHidden,
  paramLabels,
  removeIndicator,
  toggleIndicatorHidden,
} from "../stores/indicators";
import { symbol } from "../stores/layout";
import { paramDefaults } from "../stores/layouts";
import { openAlertDialog } from "./AlertDialog";
import { DRAWING_LABELS, type DrawingsApi } from "./DrawingToolbar";

export interface ObjectTreeProps {
  series: SeriesPayload | null;
  timeZone: string;
  /** Null until the drawing toolbar has mounted. */
  drawings: DrawingsApi | null;
  /** Open the settings dialog of the indicator named `name`. */
  onSettings: (name: string) => void;
}

const EYE =
  "M1 8 C3 4.5 5.5 3 8 3 S13 4.5 15 8 C13 11.5 10.5 13 8 13 S3 11.5 1 8 ZM8 6a2 2 0 1 0 0 4 2 2 0 0 0 0-4z";
const EYE_OFF = `${EYE}M2 14L14 2`;
const GEAR =
  "M8 6a2 2 0 1 0 0 4 2 2 0 0 0 0-4zM8 1.5V3.5M8 12.5V14.5M1.5 8H3.5M12.5 8H14.5M3.4 3.4L4.8 4.8M11.2 11.2L12.6 12.6M3.4 12.6L4.8 11.2M11.2 4.8L12.6 3.4";
const LOCK = "M4 7h8v6H4zM5.5 7V5a2.5 2.5 0 0 1 5 0v2";
const UNLOCK = "M4 7h8v6H4zM5.5 7V5a2.5 2.5 0 0 1 4.8-1";
const CLOSE = "M3.5 3.5L12.5 12.5M12.5 3.5L3.5 12.5";

function Action(props: {
  label: string;
  path: string;
  disabled?: boolean;
  onClick: () => void;
}): JSX.Element {
  return (
    <button
      type="button"
      class="inline-flex items-center justify-center w-6 h-6 rounded-md cursor-pointer disabled:cursor-default disabled:opacity-40 text-[color:var(--color-fg-muted)] hover:text-[color:var(--color-fg)] hover:bg-[color:var(--color-bg-elev)]"
      disabled={props.disabled}
      aria-label={props.label}
      title={props.label}
      onClick={(e) => {
        e.stopPropagation();
        props.onClick();
      }}
    >
      <svg
        aria-hidden="true"
        viewBox="0 0 16 16"
        width="14"
        height="14"
        fill="none"
        stroke="currentColor"
        stroke-width="1.5"
        stroke-linecap="round"
        stroke-linejoin="round"
      >
        <path d={props.path} />
      </svg>
    </button>
  );
}

function Group(props: { title: string; children: JSX.Element }): JSX.Element {
  return (
    <section aria-label={props.title}>
      <h3 class="px-3 pt-2 pb-1 text-xs font-semibold text-[color:var(--color-fg-muted)]">
        {props.title}
      </h3>
      {props.children}
    </section>
  );
}

function Item(props: {
  testId: string;
  dim?: boolean;
  title?: string;
  swatch?: string;
  label: JSX.Element;
  onClick?: () => void;
  children?: JSX.Element;
}): JSX.Element {
  return (
    <div
      data-testid={props.testId}
      title={props.title}
      class="wl-row flex items-center gap-2 px-3 py-1 text-sm"
      classList={{ "cursor-pointer": !!props.onClick, "opacity-50": props.dim }}
      onClick={() => props.onClick?.()}
    >
      <Show when={props.swatch}>
        {(c) => (
          <span class="w-2 h-2 rounded-full shrink-0" style={{ background: c() }} aria-hidden="true" />
        )}
      </Show>
      <span class="flex-1 min-w-0 truncate">{props.label}</span>
      <span class="flex shrink-0">{props.children}</span>
    </div>
  );
}

export function ObjectTree(props: ObjectTreeProps): JSX.Element {
  const precision = () =>
    pricePrecision(props.series?.bars.slice(-50).map((b) => b.c) ?? []);
  const drawings = () => props.drawings?.list() ?? [];
  const symbolAlerts = () => alerts().filter((a) => a.symbol === symbol());
  // A repeated indicator shows its params, so its rows read apart (ADR-0028).
  const params = () => paramLabels(activeIndicators(), paramDefaults(), true);

  return (
    <div class="flex-1 min-h-0 overflow-y-auto pb-2" role="region" aria-label="Object Tree">
      <Group title="Main series">
        <Item
          testId="tree-main"
          label={
            <>
              {props.series?.symbol ?? symbol()}
              <Show when={props.series?.interval}>{(i) => <> · {i()}</>}</Show>
            </>
          }
        />
      </Group>
      <Show when={compareEntries().length > 0}>
        <Group title="Compare">
          <For each={compareEntries()}>
            {(e) => (
              <Item
                testId={`tree-compare-${e.symbol}`}
                swatch={COMPARE_PALETTE[e.slot]}
                label={e.symbol}
              >
                <Action label={`Remove ${e.symbol}`} path={CLOSE} onClick={() => removeCompare(e.symbol)} />
              </Item>
            )}
          </For>
        </Group>
      </Show>
      <Group title="Indicators">
        <Show
          when={activeIndicators().length > 0}
          fallback={<p class="px-3 py-1 text-xs text-[color:var(--color-fg-muted)]">None.</p>}
        >
          <For each={activeIndicators()}>
            {(entry, index) => {
              // Payload and callbacks address the instance (ADR-0028).
              const key = () => instanceKey(entry);
              const label = () =>
                [
                  props.series?.indicators[key()]?.label || indicatorName(entry),
                  params().get(key()),
                ]
                  .filter(Boolean)
                  .join(" ");
              return (
                <Item testId={`tree-ind-${key()}`} dim={isHidden(entry)} label={label()}>
                  <Action
                    label={`${isHidden(entry) ? "Show" : "Hide"} ${label()}`}
                    path={isHidden(entry) ? EYE_OFF : EYE}
                    onClick={() => toggleIndicatorHidden(index())}
                  />
                  <Action
                    label={`Settings for ${label()}`}
                    path={GEAR}
                    onClick={() => props.onSettings(key())}
                  />
                  <Action
                    label={`Remove ${label()}`}
                    path={CLOSE}
                    onClick={() => removeIndicator(key())}
                  />
                </Item>
              );
            }}
          </For>
        </Show>
      </Group>
      <Group title="Drawings">
        <Show
          when={drawings().length > 0}
          fallback={<p class="px-3 py-1 text-xs text-[color:var(--color-fg-muted)]">None.</p>}
        >
          <For each={drawings()}>
            {(d) => {
              const text = () =>
                describeDrawing(d, DRAWING_LABELS[d.name], props.timeZone, precision());
              return (
                <Item
                  testId={`tree-drawing-${d.id}`}
                  dim={!d.visible || d.offRange}
                  title={d.offRange ? "Not in loaded range" : undefined}
                  label={text()}
                  onClick={() => props.drawings?.select(d.id)}
                >
                  <Action
                    label={`${d.visible ? "Hide" : "Show"} ${text()}`}
                    path={d.visible ? EYE : EYE_OFF}
                    disabled={d.offRange}
                    onClick={() => props.drawings?.setVisible(d.id, !d.visible)}
                  />
                  <Action
                    label={`${d.lock ? "Unlock" : "Lock"} ${text()}`}
                    path={d.lock ? LOCK : UNLOCK}
                    disabled={d.offRange}
                    onClick={() => props.drawings?.setLocked(d.id, !d.lock)}
                  />
                  <Action
                    label={`Remove ${text()}`}
                    path={CLOSE}
                    onClick={() => props.drawings?.remove(d.id)}
                  />
                </Item>
              );
            }}
          </For>
        </Show>
      </Group>
      <Group title="Alerts">
        <Show
          when={symbolAlerts().length > 0}
          fallback={<p class="px-3 py-1 text-xs text-[color:var(--color-fg-muted)]">None.</p>}
        >
          <For each={symbolAlerts()}>
            {(a) => (
              <Item
                testId={`tree-alert-${a.id}`}
                dim={!a.active}
                label={
                  <>
                    {CONDITION_LABELS[a.condition]}{" "}
                    <span class="tabular-nums">{formatPrice(a.price)}</span>
                    <span class="ml-1 text-xs text-[color:var(--color-fg-muted)]">
                      {alertStatus(a)}
                    </span>
                  </>
                }
                onClick={() => openAlertDialog({ id: a.id })}
              >
                <Action label="Delete alert" path={CLOSE} onClick={() => removeAlert(a.id)} />
              </Item>
            )}
          </For>
        </Show>
      </Group>
    </div>
  );
}
