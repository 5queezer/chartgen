// Focus guard shared by the toolbar's type-to-act shortcuts and the chart
// keyboard navigation.

/** True when a keystroke belongs to a text field or an open dialog/menu. */
export function isTypingContext(e: KeyboardEvent): boolean {
  if (e.defaultPrevented) return true;
  const t = e.target;
  if (
    t instanceof HTMLElement &&
    (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName))
  ) {
    return true;
  }
  return document.querySelector("[role=dialog], [role=menu]") !== null;
}
