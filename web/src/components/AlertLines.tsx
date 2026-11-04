// Draws the chart symbol's active price alerts as klinecharts overlays: a
// dashed horizontal line across the pane with a bell label and a price tag
// on the axis. Dragging a line moves the alert's level; double-click opens
// the edit dialog. The alerts store is the source of truth: overlays are
// created, repriced and removed to match it, and are not persisted
// themselves.

import { createEffect, onCleanup, untrack } from "solid-js";
import {
  ActionType,
  registerOverlay,
  type Chart as KChart,
  type OverlayEvent,
} from "klinecharts";

import { CONDITION_LABELS, type Alert } from "../lib/alerts";
import { formatPrice, priceDecimals } from "../lib/quote";
import { PALETTES } from "../lib/theme";
import { alerts, moveAlert } from "../stores/alerts";
import { theme } from "../stores/layout";
import { openAlertDialog } from "./AlertDialog";

const ALERT_COLOR = "#ff9800";

registerOverlay({
  name: "cgAlert",
  totalStep: 2,
  needDefaultPointFigure: false,
  needDefaultXAxisFigure: false,
  needDefaultYAxisFigure: true,
  createPointFigures: ({ overlay, coordinates, bounding }) => {
    const c = coordinates[0];
    if (!c) return [];
    return [
      {
        type: "line",
        attrs: {
          coordinates: [
            { x: 0, y: c.y },
            { x: bounding.width, y: c.y },
          ],
        },
        styles: {
          style: "dashed",
          color: ALERT_COLOR,
          size: 1,
          dashedValue: [6, 4],
        },
      },
      {
        type: "text",
        attrs: {
          x: 8,
          y: c.y,
          text: typeof overlay.extendData === "string" ? overlay.extendData : "",
          baseline: "middle",
        },
        styles: {
          style: "stroke_fill",
          color: PALETTES[theme()].fg,
          backgroundColor: PALETTES[theme()].surface,
          borderColor: ALERT_COLOR,
          size: 12,
          paddingLeft: 6,
          paddingRight: 6,
          paddingTop: 3,
          paddingBottom: 3,
        },
      },
    ];
  },
});

const labelOf = (a: Alert) =>
  `🔔 ${CONDITION_LABELS[a.condition]} ${formatPrice(a.price)}`;

export interface AlertLinesProps {
  /** The klinecharts instance; null until the chart has mounted. */
  chart: KChart | null;
  /** Chart symbol — only its alerts are drawn. */
  symbol: string;
}

/** Renders nothing; manages the overlays of the symbol's active alerts. */
export function AlertLines(props: AlertLinesProps): null {
  // alert id -> overlay id
  const lines = new Map<string, string>();

  const drop = (alertId: string) => {
    const id = lines.get(alertId);
    if (id !== undefined) props.chart?.removeOverlay(id);
    lines.delete(alertId);
  };

  const sync = () => {
    const chart = props.chart;
    if (!chart) return;
    const wanted = alerts().filter((a) => a.active && a.symbol === props.symbol);
    const keep = new Set(wanted.map((a) => a.id));
    for (const alertId of [...lines.keys()]) if (!keep.has(alertId)) drop(alertId);

    // The x of the point is irrelevant (the line spans the pane) but the
    // overlay needs a timestamp on the loaded data.
    const timestamp = chart.getDataList().at(-1)?.timestamp;
    if (timestamp === undefined) return;
    for (const a of wanted) {
      const existing = lines.get(a.id);
      if (existing !== undefined) {
        const o = chart.getOverlayById(existing);
        if (o && o.points[0]?.value !== a.price) {
          chart.overrideOverlay({
            id: existing,
            points: [{ timestamp, value: a.price }],
            extendData: labelOf(a),
          });
        }
        continue;
      }
      const id = chart.createOverlay({
        name: "cgAlert",
        points: [{ timestamp, value: a.price }],
        extendData: labelOf(a),
        onPressedMoveEnd: ({ overlay }: OverlayEvent) => {
          const v = overlay.points[0]?.value;
          if (v !== undefined) moveAlert(a.id, +v.toFixed(priceDecimals(v)));
          return false;
        },
        onDoubleClick: () => {
          openAlertDialog({ id: a.id });
          return true;
        },
        onRightClick: () => true,
      });
      if (typeof id === "string") lines.set(a.id, id);
    }
  };

  // Overlays need bars: also run once data has loaded. Declared before the
  // effect below so that, when the chart instance changes, the old chart's
  // overlays are dropped before the new chart's are created.
  createEffect(() => {
    const chart = props.chart;
    if (!chart) return;
    chart.subscribeAction(ActionType.OnDataReady, sync);
    onCleanup(() => {
      chart.unsubscribeAction(ActionType.OnDataReady, sync);
      for (const id of lines.values()) chart.removeOverlay(id);
      lines.clear();
    });
  });

  // Alerts, symbol or chart changed.
  createEffect(() => {
    void alerts();
    void props.symbol;
    void props.chart;
    untrack(sync);
  });

  // E2E hook: the alert lines that exist on the chart, with their pixel y.
  if (import.meta.env.DEV) {
    const w = window as unknown as Record<string, unknown>;
    w.__chartgenAlertLines = () =>
      [...lines].flatMap(([alertId, id]) => {
        const o = props.chart?.getOverlayById(id);
        const p = o?.points[0];
        const y = p
          ? (props.chart?.convertToPixel({ value: p.value }, { paneId: o?.paneId }) as
              | { y?: number }
              | undefined)?.y
          : undefined;
        return o ? [{ alertId, value: p?.value, y }] : [];
      });
    onCleanup(() => delete w.__chartgenAlertLines);
  }

  return null;
}
