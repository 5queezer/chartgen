// Price alert model and evaluation (ADR-0020). Pure: no Solid, no storage.
//
// Semantics follow TradingView:
//   crossing        prev on one side of the level, now on the other or on it
//   crossing_up     prev below the level, now at or above it
//   crossing_down   prev above the level, now at or below it
//   greater / less  the price is above / below the level (state, not event)
//
// Re-arming (trigger "every"): a crossing alert needs no extra rule, because
// firing again requires the price to leave the level and cross it anew. The
// state conditions would fire on every tick while satisfied, so after firing
// they stay disarmed until the price is back on the other side (greater: at
// or below the level; less: at or above it). `rearmed` is that test.

export const CONDITIONS = [
  "crossing",
  "crossing_up",
  "crossing_down",
  "greater",
  "less",
] as const;
export type AlertCondition = (typeof CONDITIONS)[number];

export const TRIGGERS = ["once", "every"] as const;
export type AlertTrigger = (typeof TRIGGERS)[number];

export interface Alert {
  id: string;
  symbol: string;
  condition: AlertCondition;
  price: number;
  trigger: AlertTrigger;
  message?: string;
  active: boolean;
  /** Epoch ms. */
  createdAt: number;
  /** Epoch ms of the last time it fired. */
  lastTriggeredAt?: number;
}

export const CONDITION_LABELS: Record<AlertCondition, string> = {
  crossing: "Crossing",
  crossing_up: "Crossing up",
  crossing_down: "Crossing down",
  greater: "Greater than",
  less: "Less than",
};

export const TRIGGER_LABELS: Record<AlertTrigger, string> = {
  once: "Only once",
  every: "Every time",
};

export type AlertStatus = "active" | "triggered" | "paused";

/** Inactive with a fire time: it ran its course; inactive without: paused. */
export function alertStatus(a: Alert): AlertStatus {
  return a.active
    ? "active"
    : a.lastTriggeredAt !== undefined
      ? "triggered"
      : "paused";
}

/**
 * Does moving from `prev` to `price` fire the alert? `prev` is null before
 * the first price is known: events cannot fire, states can.
 */
export function evaluate(
  alert: Pick<Alert, "condition" | "price">,
  prev: number | null,
  price: number,
): boolean {
  const level = alert.price;
  switch (alert.condition) {
    case "crossing_up":
      return prev !== null && prev < level && price >= level;
    case "crossing_down":
      return prev !== null && prev > level && price <= level;
    case "crossing":
      return (
        prev !== null &&
        ((prev < level && price >= level) || (prev > level && price <= level))
      );
    case "greater":
      return price > level;
    case "less":
      return price < level;
  }
}

/** True once a fired "every" alert may fire again (see the header). */
export function rearmed(
  alert: Pick<Alert, "condition" | "price">,
  price: number,
): boolean {
  switch (alert.condition) {
    case "greater":
      return price <= alert.price;
    case "less":
      return price >= alert.price;
    default:
      return true;
  }
}

/** True for conditions that need `rearmed` to hold before they fire again. */
export function needsRearm(condition: AlertCondition): boolean {
  return condition === "greater" || condition === "less";
}
