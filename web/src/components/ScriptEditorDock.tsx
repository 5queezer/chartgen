// Entry points of the script editor (ADR-0041). App renders these only when the
// server lists the scripting tools; the panel and CodeMirror load on first open,
// so nobody else downloads them.

import { ErrorBoundary, Show, Suspense, createSignal, lazy, type JSX } from "solid-js";

import type { McpSession } from "../mcp";
import type { IndicatorError } from "../types/responses";

const ScriptEditor = lazy(() => import("./ScriptEditor"));

const [open, setOpen] = createSignal(false);

export const openScriptEditor = (): void => {
  setOpen(true);
};

export function ScriptEditorButton(): JSX.Element {
  return (
    <button
      type="button"
      onClick={() => setOpen(!open())}
      aria-pressed={open()}
      data-testid="script-editor-open"
      title="Script editor"
      class="bg-[color:var(--color-bg-elev)] border border-[color:var(--color-border)] text-[color:var(--color-fg)] px-3 py-1.5 rounded-md text-sm hover:border-[color:var(--color-accent)] cursor-pointer inline-flex items-center gap-2 shrink-0 compact:hidden"
    >
      <span class="max-[1399px]:sr-only">Scripts</span>
      <span aria-hidden="true" class="font-mono leading-none min-[1400px]:hidden">{"{ }"}</span>
    </button>
  );
}

export function ScriptEditorDock(props: {
  session: McpSession;
  chartErrors: Record<string, IndicatorError> | undefined;
}): JSX.Element {
  const close = () => setOpen(false);
  return (
    <Show when={open()}>
      <ErrorBoundary
        fallback={
          <section
            aria-label="Script editor"
            data-testid="script-editor-failed"
            class="shrink-0 flex items-center gap-3 px-3 py-2 border-t border-[color:var(--color-border)] text-xs text-[color:var(--color-danger)]"
          >
            Could not load the script editor. Reload the page and try again.
            <button type="button" class="cursor-pointer underline" onClick={close}>
              Close
            </button>
          </section>
        }
      >
        <Suspense
          fallback={
            <div class="shrink-0 px-3 py-2 border-t border-[color:var(--color-border)] text-xs text-[color:var(--color-fg-muted)]">
              Loading the script editor…
            </div>
          }
        >
          <ScriptEditor session={props.session} chartErrors={props.chartErrors} onClose={close} />
        </Suspense>
      </ErrorBoundary>
    </Show>
  );
}
