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
 * True when a mouse event or touchend/touchcancel can end a chart press: the left
 * button (a right-button one does not), or the last finger. DrawingToolbar also passes
 * it a mousedown, whose left button ends a press still held.
 */
export function isRelease(e: Event): boolean {
  if (e instanceof MouseEvent) return e.button === 0;
  return !((e as Partial<TouchEvent>).touches?.length ?? 0);
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

/**
 * Undo/redo meaning of a Ctrl/Cmd keystroke, by the typed letter so QWERTZ
 * and AZERTY work. `e.code` is the fallback only for non-Latin layouts.
 */
export function undoRedoKey(
  e: Pick<KeyboardEvent, "key" | "code" | "ctrlKey" | "metaKey" | "altKey" | "shiftKey">,
): "undo" | "redo" | null {
  if (!(e.ctrlKey || e.metaKey) || e.altKey) return null;
  const k = e.key.toLowerCase();
  const letter = /^[a-z]$/.test(k) ? k : e.code === "KeyZ" ? "z" : e.code === "KeyY" ? "y" : "";
  if (letter === "z") return e.shiftKey ? "redo" : "undo";
  if (letter === "y" && !e.shiftKey) return "redo";
  return null;
}
