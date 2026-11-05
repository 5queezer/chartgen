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

/**
 * Chromium delivers no key events to a page whose focused element was removed
 * (a closed dialog's content) and fell back to `<body>`, until the next click.
 * Moves focus to the chart host whenever DOM removal leaves it on `<body>`, so
 * shortcuts keep working right after a dialog or menu closes.
 */
export function keepKeyboardFocus(): () => void {
  const observer = new MutationObserver((records) => {
    if (!records.some((r) => r.removedNodes.length > 0)) return;
    const a = document.activeElement;
    if (a && a !== document.body) return;
    document
      .querySelector<HTMLElement>(".chart-host")
      ?.focus({ preventScroll: true });
  });
  observer.observe(document.body, { childList: true, subtree: true });
  return () => observer.disconnect();
}
