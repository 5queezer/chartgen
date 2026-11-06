// Right-click menus of things that show up in more than one place: an alert (its
// panel row, its tree row, its line on the chart) and an indicator (legend row,
// chip, tree row, sub-pane).

import type { Alert } from "../lib/alerts";
import { chipName } from "../lib/indicatorNames";
import { showToast } from "../services/alertEngine";
import { removeAlert, setAlertActive } from "../stores/alerts";
import {
  indicatorName,
  instanceKey,
  isHidden,
  paramLabels,
  type IndicatorStore,
} from "../stores/indicators";
import { setSymbol } from "../stores/layout";
import { paramDefaults } from "../stores/layouts";
import { deleteServerAlert, serverAlertError, updateServerAlert } from "../stores/serverAlerts";
import type { SeriesPayload, ServerAlert } from "../types/responses";
import { openAlertDialog } from "./AlertDialog";
import type { MenuItem, MenuSpec } from "./ContextMenu";

/** `chartSymbol`: what the chart shows, so another symbol's alert can offer to open it. */
export function alertMenu(a: Alert, chartSymbol: string): MenuSpec {
  return {
    label: "Alert actions",
    items: [
      { label: "Edit…", onSelect: () => openAlertDialog({ id: a.id }) },
      {
        label: a.active ? "Pause" : "Resume",
        onSelect: () => setAlertActive(a.id, !a.active),
      },
      ...(a.symbol === chartSymbol
        ? []
        : [{ label: `Open ${a.symbol} in chart`, onSelect: () => setSymbol(a.symbol) }]),
      null,
      { label: "Delete", danger: true, onSelect: () => removeAlert(a.id) },
    ],
  };
}

/** The same menu for an alert the server runs (ADR-0042). */
export function serverAlertMenu(a: ServerAlert, chartSymbol: string): MenuSpec {
  const send = (request: Promise<unknown>) =>
    void request.catch((err: unknown) => showToast(serverAlertError(err)));
  return {
    label: "Alert actions",
    items: [
      { label: "Edit…", onSelect: () => openAlertDialog({ serverId: a.id }) },
      {
        label: a.active ? "Pause" : "Resume",
        onSelect: () => send(updateServerAlert({ alert_id: a.id, active: !a.active })),
      },
      ...(a.symbol === chartSymbol
        ? []
        : [{ label: `Open ${a.symbol} in chart`, onSelect: () => setSymbol(a.symbol) }]),
      null,
      { label: "Delete", danger: true, onSelect: () => send(deleteServerAlert(a.id)) },
    ],
  };
}

export interface IndicatorActions {
  /** Hidden now: the item offers Show. */
  hidden?: boolean;
  onAddAlert?: () => void;
  onSettings?: () => void;
  onToggleHidden?: () => void;
  onRemove?: () => void;
}

/** Items for what the row can do; a compare symbol's legend row can only be removed. */
export function indicatorMenu(name: string, a: IndicatorActions): MenuSpec {
  const open: MenuItem[] = [];
  if (a.onAddAlert) open.push({ label: `Add alert on ${name}…`, onSelect: a.onAddAlert });
  if (a.onSettings) open.push({ label: `Settings for ${name}…`, onSelect: a.onSettings });
  if (a.onToggleHidden) {
    open.push({ label: `${a.hidden ? "Show" : "Hide"} ${name}`, onSelect: a.onToggleHidden });
  }
  const remove: MenuItem[] = a.onRemove
    ? [{ label: `Remove ${name}`, danger: true, onSelect: a.onRemove }]
    : [];
  return {
    label: `${name} actions`,
    items: open.length > 0 && remove.length > 0 ? [...open, null, ...remove] : [...open, ...remove],
  };
}

/** The menu of an indicator instance of the chart that owns `store`. */
export function indicatorMenuOf(
  store: IndicatorStore,
  series: SeriesPayload | null | undefined,
  key: string,
  on: { onAddAlert: (key: string) => void; onSettings: (key: string) => void },
): MenuSpec {
  const entries = store.activeIndicators();
  const index = entries.findIndex((e) => instanceKey(e) === key);
  const entry = entries[index];
  return indicatorMenu(
    entry ? chipName(indicatorName(entry), paramLabels(entries, paramDefaults(), true).get(key)) : key,
    {
      hidden: entry !== undefined && isHidden(entry),
      onAddAlert: series?.indicators[key]?.lines?.length ? () => on.onAddAlert(key) : undefined,
      onSettings: () => on.onSettings(key),
      onToggleHidden: () => store.toggleIndicatorHidden(index),
      onRemove: () => store.removeIndicator(key),
    },
  );
}
