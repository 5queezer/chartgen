// Object Tree text: the one-line descriptor of a drawing, e.g.
// "Trend line 2026-11-03 → 11-04" or "Horizontal line 67123.45".

import type { DrawingRecord } from "../stores/drawings";
import { formatPrice } from "./format";
import { formatDateTime } from "./timezone";

const date = (ms: number, tz: string) => formatDateTime(ms, tz).slice(0, 10);

export function describeDrawing(
  rec: Pick<DrawingRecord, "points" | "extendData">,
  label: string,
  tz: string,
  precision: number,
): string {
  const [first, last] = [rec.points[0], rec.points.at(-1)];
  if (!first || !last) return label;
  if (rec.points.length === 1) {
    return rec.extendData
      ? `${label} “${rec.extendData}”`
      : `${label} ${date(first.timestamp, tz)} @ ${formatPrice(first.value, precision)}`;
  }
  const from = date(first.timestamp, tz);
  const to = date(last.timestamp, tz);
  if (from === to) return `${label} ${from}`;
  return `${label} ${from} → ${from.slice(0, 4) === to.slice(0, 4) ? to.slice(5) : to}`;
}
