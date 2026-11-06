// Right-click menus of things that show up in more than one place: an alert (its
// panel row, its tree row, its line on the chart) and an indicator (legend row,
// chip, tree row, sub-pane).

import type { Alert } from "../lib/alerts";
import { removeAlert, setAlertActive } from "../stores/alerts";
import { setSymbol, symbol } from "../stores/layout";
import { openAlertDialog } from "./AlertDialog";
import type { MenuItem, MenuSpec } from "./ContextMenu";

export function alertMenu(a: Alert): MenuSpec {
  return {
    label: "Alert actions",
    items: [
      { label: "Edit…", onSelect: () => openAlertDialog({ id: a.id }) },
      {
        label: a.active ? "Pause" : "Resume",
        onSelect: () => setAlertActive(a.id, !a.active),
      },
      ...(a.symbol === symbol()
        ? []
        : [{ label: `Open ${a.symbol} in chart`, onSelect: () => setSymbol(a.symbol) }]),
      null,
      { label: "Delete", danger: true, onSelect: () => removeAlert(a.id) },
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
export function indicatorItems(name: string, a: IndicatorActions): (MenuItem | null)[] {
  const open: MenuItem[] = [];
  if (a.onAddAlert) open.push({ label: `Add alert on ${name}…`, onSelect: a.onAddAlert });
  if (a.onSettings) open.push({ label: `Settings for ${name}…`, onSelect: a.onSettings });
  if (a.onToggleHidden) {
    open.push({ label: `${a.hidden ? "Show" : "Hide"} ${name}`, onSelect: a.onToggleHidden });
  }
  const remove: MenuItem[] = a.onRemove
    ? [{ label: `Remove ${name}`, danger: true, onSelect: a.onRemove }]
    : [];
  return open.length > 0 && remove.length > 0 ? [...open, null, ...remove] : [...open, ...remove];
}

export function indicatorMenu(name: string, a: IndicatorActions): MenuSpec {
  return { label: `${name} actions`, items: indicatorItems(name, a) };
}
