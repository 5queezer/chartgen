// Right-click menu of the chart (TradingView-style), ADR-0019. The wrapped
// area is the ContextMenu trigger; the menu stands down when the click hit a
// drawing so the drawing's own menu (DrawingToolbar) wins.
//
// "Lock vertical cursor line by time" is not offered: klinecharts has no
// pinned crosshair, only the one that follows the mouse.

import { ContextMenu } from "@kobalte/core";
import {
  Show,
  createSignal,
  onCleanup,
  onMount,
  type JSX,
} from "solid-js";
import type { Chart as KLineChart } from "klinecharts";

import {
  invertScale,
  lockRatio,
  setInvertScale,
  setLockRatio,
} from "../stores/layout";
import { openAlertDialog } from "./AlertDialog";
import type { DrawingsApi } from "./DrawingToolbar";

const CANDLE_PANE_ID = "candle_pane";

const menuClass =
  "bg-[color:var(--color-bg-elev)] border border-[color:var(--color-border)] rounded-md shadow-lg z-50 py-1 min-w-56";
const itemClass =
  "px-3 py-1.5 text-sm cursor-pointer flex items-center justify-between gap-6 outline-none data-[highlighted]:bg-[color:var(--color-border)] data-[disabled]:opacity-40 data-[disabled]:cursor-default";

export interface ChartContextMenuProps {
  chart: KLineChart | null;
  /** Symbol of the chart; alerts created here are for it. */
  symbol: string;
  /** Null until the drawing toolbar has mounted. */
  drawings: DrawingsApi | null;
  hasIndicators: boolean;
  onResetView: () => void;
  onCopyLink: () => void;
  onRemoveAllIndicators: () => void;
  onSettings: () => void;
  children: JSX.Element;
}

/** Price under the viewport point, formatted to the chart's precision; null outside the price pane. */
function priceAt(
  chart: KLineChart,
  clientX: number,
  clientY: number,
): { price: number; text: string } | null {
  const rect = chart.getDom()?.getBoundingClientRect();
  const pane = chart.getSize(CANDLE_PANE_ID);
  if (!rect || !pane) return null;
  const x = clientX - rect.left;
  const y = clientY - rect.top;
  if (x < 0 || x > rect.width || y < pane.top || y > pane.top + pane.height) {
    return null;
  }
  const [point] = chart.convertFromPixel([{ y }], {
    paneId: CANDLE_PANE_ID,
    absolute: true,
  }) as { value?: number }[];
  const price = point?.value;
  if (price === undefined || !Number.isFinite(price)) return null;
  return { price, text: price.toFixed(chart.getPriceVolumePrecision().price) };
}

export function ChartContextMenu(props: ChartContextMenuProps): JSX.Element {
  let trigger!: HTMLDivElement;
  const [at, setAt] = createSignal<{ price: number; text: string } | null>(
    null,
  );

  // A touch long-press is the klinecharts crosshair, so the menu stays with
  // the mouse. guides/web.md says where touch finds its actions.
  const [touchPress, setTouchPress] = createSignal(false);

  // Capture phase: runs before Kobalte opens the menu, so the price is known
  // when the items render.
  onMount(() => {
    const onContextMenu = (e: MouseEvent) => {
      if (touchPress()) e.preventDefault();
      setAt(props.chart ? priceAt(props.chart, e.clientX, e.clientY) : null);
    };
    const onPointerDown = (e: PointerEvent) => setTouchPress(e.pointerType !== "mouse");
    trigger.addEventListener("contextmenu", onContextMenu, true);
    trigger.addEventListener("pointerdown", onPointerDown, true);
    onCleanup(() => {
      trigger.removeEventListener("contextmenu", onContextMenu, true);
      trigger.removeEventListener("pointerdown", onPointerDown, true);
    });
  });

  const [notice, setNotice] = createSignal<string | null>(null);
  let noticeTimer: number | undefined;
  onCleanup(() => window.clearTimeout(noticeTimer));

  const copyPrice = async () => {
    const p = at();
    if (!p) return;
    try {
      // Undefined on non-secure origins (plain HTTP); rejects without permission.
      if (!navigator.clipboard) throw new Error("clipboard unavailable");
      await navigator.clipboard.writeText(p.text);
    } catch {
      setNotice("Copy failed");
      window.clearTimeout(noticeTimer);
      noticeTimer = window.setTimeout(() => setNotice(null), 3000);
    }
  };

  const removeAllIndicators = () => {
    if (window.confirm("Remove all indicators?")) props.onRemoveAllIndicators();
  };

  return (
    <ContextMenu.Root>
      <ContextMenu.Trigger
        ref={trigger}
        class="flex-1 min-w-0 relative"
        disabled={touchPress() || (props.drawings?.rightClickedDrawing() ?? false)}
      >
        {props.children}
      </ContextMenu.Trigger>
      <ContextMenu.Portal>
        <ContextMenu.Content class={menuClass} aria-label="Chart actions">
          <ContextMenu.Item class={itemClass} onSelect={props.onResetView}>
            Reset chart view
            <span class="text-[color:var(--color-fg-muted)] text-xs">Alt+R</span>
          </ContextMenu.Item>
          <ContextMenu.Item class={itemClass} onSelect={props.onCopyLink}>
            Copy link to chart
          </ContextMenu.Item>
          <Show when={at()}>
            {(p) => (
              <>
                <ContextMenu.Item class={itemClass} onSelect={copyPrice}>
                  Copy price {p().text}
                </ContextMenu.Item>
                <ContextMenu.Item
                  class={itemClass}
                  onSelect={() =>
                    openAlertDialog({ symbol: props.symbol, price: p().price })
                  }
                >
                  Add alert at {p().text}…
                </ContextMenu.Item>
                <ContextMenu.Item
                  class={itemClass}
                  onSelect={() => props.drawings?.addHorizontalLine(p().price)}
                >
                  Add horizontal line at {p().text}
                </ContextMenu.Item>
              </>
            )}
          </Show>
          <ContextMenu.Separator class="my-1 border-t border-[color:var(--color-border)]" />
          <ContextMenu.CheckboxItem
            closeOnSelect
            class={itemClass}
            checked={invertScale()}
            onChange={setInvertScale}
          >
            Invert scale
            <span class="text-[color:var(--color-fg-muted)] text-xs">Alt+I</span>
          </ContextMenu.CheckboxItem>
          <ContextMenu.CheckboxItem
            closeOnSelect
            class={itemClass}
            checked={lockRatio()}
            onChange={setLockRatio}
          >
            Lock price to bar ratio
          </ContextMenu.CheckboxItem>
          <ContextMenu.Separator class="my-1 border-t border-[color:var(--color-border)]" />
          <ContextMenu.Item
            class={itemClass}
            onSelect={() => props.drawings?.removeAll()}
          >
            Remove all drawings
          </ContextMenu.Item>
          <ContextMenu.Item
            class={itemClass}
            disabled={!props.hasIndicators}
            onSelect={removeAllIndicators}
          >
            Remove all indicators
          </ContextMenu.Item>
          <ContextMenu.Separator class="my-1 border-t border-[color:var(--color-border)]" />
          <ContextMenu.Item class={itemClass} onSelect={props.onSettings}>
            Settings…
          </ContextMenu.Item>
        </ContextMenu.Content>
      </ContextMenu.Portal>
      <Show when={notice()}>
        {(n) => (
          <div
            role="status"
            class="fixed bottom-6 left-1/2 -translate-x-1/2 z-50 px-3 py-1.5 rounded-md text-sm bg-[color:var(--color-bg-elev)] border border-[color:var(--color-border)] shadow-lg"
          >
            {n()}
          </div>
        )}
      </Show>
    </ContextMenu.Root>
  );
}
