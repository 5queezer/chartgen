// Right-click menus of the chart (TradingView-style), ADR-0019: which one opens
// depends on what is under the pointer. A drawing or an alert line has its own
// (`setOverlayMenu`); the price axis, the time axis and a sub-pane have theirs.
// Anything else in the wrapped area (replay buttons, a load error) keeps the
// browser's menu.
//
// "Lock vertical cursor line by time" is not offered: klinecharts has no
// pinned crosshair, only the one that follows the mouse.

import type { JSX } from "solid-js";
import type { Chart as KLineChart } from "klinecharts";

import { resetChart, resetTimeScale } from "../lib/chartActions";
import { showToast } from "../services/alertEngine";
import { copyText } from "../services/shareLink";
import type { ScaleMode } from "../stores/layout";
import { openAlertDialog } from "./AlertDialog";
import { openGoToDate } from "./BottomBar";
import { openContextMenu, takeOverlayMenu, type MenuItem, type MenuSpec } from "./ContextMenu";
import type { DrawingsApi } from "./DrawingToolbar";

const CANDLE_PANE_ID = "candle_pane";
const X_AXIS_PANE_ID = "x_axis_pane";

/** What the menus read and set: the store of the chart they were opened on. */
export interface ScaleStore {
  scaleMode: () => ScaleMode;
  autoScale: () => boolean;
  lockRatio: () => boolean;
  invertScale: () => boolean;
  setScaleMode: (next: ScaleMode) => void;
  setAutoScale: (next: boolean) => void;
  setLockRatio: (next: boolean) => void;
  setInvertScale: (next: boolean) => void;
}

export interface ChartContextMenuProps {
  chart: KLineChart | null;
  /** Symbol of the chart; alerts created here are for it. */
  symbol: string;
  scale: ScaleStore;
  /** Compare lines are on: the ratio lock is paused. */
  comparing: boolean;
  /** Null until the drawing toolbar has mounted. */
  drawings: DrawingsApi | null;
  hasIndicators: boolean;
  /** The items of an indicator in a sub-pane, by instance key. */
  indicatorItems: (key: string) => (MenuItem | null)[];
  onCopyLink: () => void;
  onRemoveAllIndicators: () => void;
  onSettings: () => void;
  children: JSX.Element;
}

/** Value under the viewport y in that pane, in the chart's precision; null outside it. */
function valueAt(
  chart: KLineChart,
  paneId: string,
  clientY: number,
): { price: number; text: string } | null {
  const rect = chart.getDom()?.getBoundingClientRect();
  const pane = chart.getSize(paneId);
  if (!rect || !pane || chart.getDataList().length === 0) return null;
  const y = clientY - rect.top;
  if (y < pane.top || y > pane.top + pane.height) return null;
  const [point] = chart.convertFromPixel([{ y }], { paneId, absolute: true }) as { value?: number }[];
  const value = point?.value;
  if (value === undefined || !Number.isFinite(value)) return null;
  const decimals = paneId === CANDLE_PANE_ID ? chart.getPriceVolumePrecision().price : 2;
  const text = value.toFixed(decimals);
  return { price: Number(text), text };
}

async function copy(text: string, done: string): Promise<void> {
  showToast((await copyText(text)) ? done : "Copy failed");
}

export function ChartContextMenu(props: ChartContextMenuProps): JSX.Element {
  const resetView = (c: KLineChart) => resetChart(c, props.scale.setAutoScale);

  const priceItems = (at: { price: number; text: string } | null): (MenuItem | null)[] =>
    at
      ? [
          { label: `Copy price ${at.text}`, onSelect: () => void copy(at.text, "Price copied") },
          {
            label: `Add alert at ${at.text}…`,
            onSelect: () => openAlertDialog({ symbol: props.symbol, price: at.price }),
          },
          {
            label: `Add horizontal line at ${at.text}`,
            onSelect: () => props.drawings?.addHorizontalLine(at.price),
          },
        ]
      : [];

  const invertItem = (): MenuItem => ({
    label: "Invert scale",
    hint: "Alt+I",
    checked: props.scale.invertScale(),
    onSelect: () => props.scale.setInvertScale(!props.scale.invertScale()),
  });

  const lockItem = (): MenuItem => ({
    label: `Lock price to bar ratio${props.scale.lockRatio() && props.comparing ? " (paused while comparing)" : ""}`,
    checked: props.scale.lockRatio(),
    onSelect: () => props.scale.setLockRatio(!props.scale.lockRatio()),
  });

  const copyLinkItem = (): MenuItem => ({ label: "Copy link to chart", onSelect: props.onCopyLink });
  const settingsItem = (): MenuItem => ({ label: "Settings…", onSelect: props.onSettings });

  const priceAxisMenu = (c: KLineChart, clientY: number): MenuSpec => {
    const mode = (label: string, value: ScaleMode): MenuItem => ({
      label,
      checked: props.scale.scaleMode() === value,
      onSelect: () => props.scale.setScaleMode(value),
    });
    const at = priceItems(valueAt(c, CANDLE_PANE_ID, clientY));
    return {
      label: "Price scale actions",
      items: [
        {
          label: "Reset price scale",
          disabled: props.scale.autoScale(),
          onSelect: () => props.scale.setAutoScale(true),
        },
        {
          label: "Auto (fit data to screen)",
          checked: props.scale.autoScale(),
          onSelect: () => props.scale.setAutoScale(!props.scale.autoScale()),
        },
        lockItem(),
        null,
        mode("Regular", "normal"),
        mode("Percent", "percentage"),
        mode("Logarithmic", "log"),
        invertItem(),
        ...(at.length > 0 ? [null, ...at] : []),
      ],
    };
  };

  const timeAxisMenu = (c: KLineChart): MenuSpec => ({
    label: "Time scale actions",
    items: [
      { label: "Reset time scale", onSelect: () => resetTimeScale(c) },
      {
        label: "Go to date…",
        hint: "Alt+G",
        disabled: c.getDataList().length === 0,
        onSelect: openGoToDate,
      },
    ],
  });

  /** A sub-pane reads an indicator: no prices, and the price scale's options do not apply. */
  const subPaneMenu = (c: KLineChart, paneId: string, key: string, clientY: number): MenuSpec => {
    const at = valueAt(c, paneId, clientY);
    const items = props.indicatorItems(key);
    return {
      label: "Pane actions",
      items: [
        { label: "Reset chart view", hint: "Alt+R", onSelect: () => resetView(c) },
        ...(at ? [{ label: `Copy value ${at.text}`, onSelect: () => void copy(at.text, "Value copied") }] : []),
        ...(items.length > 0 ? [null, ...items] : []),
        null,
        copyLinkItem(),
        settingsItem(),
      ],
    };
  };

  const chartMenu = (c: KLineChart, clientY: number): MenuSpec => {
    const at = priceItems(valueAt(c, CANDLE_PANE_ID, clientY));
    return {
      label: "Chart actions",
      items: [
        { label: "Reset chart view", hint: "Alt+R", onSelect: () => resetView(c) },
        ...(at.length > 0 ? [null, ...at] : []),
        null,
        invertItem(),
        lockItem(),
        null,
        copyLinkItem(),
        { label: "Remove all drawings", onSelect: () => props.drawings?.removeAll() },
        {
          label: "Remove all indicators",
          disabled: !props.hasIndicators,
          onSelect: () => {
            if (window.confirm("Remove all indicators?")) props.onRemoveAllIndicators();
          },
        },
        null,
        settingsItem(),
      ],
    };
  };

  const onContextMenu = (e: MouseEvent) => {
    const overlay = takeOverlayMenu();
    if (overlay) {
      openContextMenu(e, overlay());
      return;
    }
    const t = e.target;
    const c = props.chart;
    if (
      !c ||
      !(t instanceof Element) ||
      // The host itself: a keyboard-opened menu, with the chart focused.
      !t.closest(".chart-container, .pane-header, .price-scale-controls, [data-testid='replay-cutline']") &&
        !t.matches(".chart-host")
    ) {
      return;
    }
    // Set on a sub-pane's root by Chart.tsx.
    const sub = t.closest<HTMLElement>("[data-indicator]");
    if (t.closest(".price-scale-controls") || c.getDom(CANDLE_PANE_ID, "yAxis" as never)?.contains(t)) {
      openContextMenu(e, priceAxisMenu(c, e.clientY));
    } else if (c.getDom(X_AXIS_PANE_ID, "main" as never)?.contains(t)) {
      openContextMenu(e, timeAxisMenu(c));
    } else if (sub?.dataset.paneId && sub.dataset.indicator) {
      openContextMenu(e, subPaneMenu(c, sub.dataset.paneId, sub.dataset.indicator, e.clientY));
    } else {
      openContextMenu(e, chartMenu(c, e.clientY));
    }
  };

  return (
    <div class="flex-1 min-w-0 relative" onContextMenu={onContextMenu}>
      {props.children}
    </div>
  );
}
