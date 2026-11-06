// Script editor panel (ADR-0041): list, write, save and delete your Starlark
// indicators, and add them to the chart. Loaded lazily by ScriptEditorDock.
//
// The text of an open script is mirrored to localStorage while it differs from
// what is saved, per script, so closing the panel or the tab loses nothing. The
// server stays the only judge of a script: it validates on save, and owns the
// list. Everything it returns (titles, error text) renders as text.

import { Dialog } from "@kobalte/core";
import { createQuery, useQueryClient } from "@tanstack/solid-query";
import {
  For,
  Show,
  createEffect,
  createMemo,
  createSignal,
  onCleanup,
  onMount,
  type JSX,
} from "solid-js";

import "./scriptEditor.css";
import { createScriptView, type ScriptView } from "./scriptEditorView";
import { keyName } from "../lib/instances";
import {
  loadDrafts,
  NEW_DRAFT,
  saveDrafts,
  SCRIPTING_DOCS_URL,
  STARTER_SCRIPT,
} from "../lib/scripts";
import { McpToolError, type McpSession } from "../mcp";
import { refreshChartsUsing, refreshIndicatorList } from "../queries";
import { showToast } from "../services/alertEngine";
import {
  activeIndicators,
  addIndicator,
  indicatorName,
  setActiveIndicators,
} from "../stores/indicators";
import type { IndicatorError, ListIndicatorsPayload } from "../types/responses";

/** How long typing may go unmirrored; closing and leaving the page flush at once. */
const DRAFT_DEBOUNCE_MS = 300;

const STEP_LABEL: Record<string, string> = {
  parse: "Syntax",
  static: "Check",
  presence: "Script shape",
  dry_run: "Dry run",
  runtime: "On the chart",
};

const buttonClass =
  "px-2.5 py-1 rounded-md text-xs cursor-pointer border border-[color:var(--color-border)] text-[color:var(--color-fg)] hover:bg-[color:var(--color-bg-elev)] disabled:opacity-50 disabled:cursor-default";
const primaryClass =
  "px-2.5 py-1 rounded-md text-xs cursor-pointer bg-[color:var(--color-accent)] text-white font-semibold disabled:opacity-50 disabled:cursor-default";

// Narrow screens: title and close, then the name on a row of its own, then these.
const LOWER_ROW = "max-[767px]:order-3";

export interface ScriptEditorProps {
  session: McpSession;
  /** Errors of the chart on screen, by instance key (`indicator_errors`). */
  chartErrors: Record<string, IndicatorError> | undefined;
  onClose: () => void;
}

export default function ScriptEditor(props: ScriptEditorProps): JSX.Element {
  const queryClient = useQueryClient();
  const list = createQuery(() => ({
    queryKey: ["list_my_indicators"] as const,
    queryFn: () => props.session.callListMyIndicators(),
    retry: false,
  }));
  const scripts = () => list.data?.indicators ?? [];

  // The open script: null is a new one, not saved yet.
  const [selected, setSelected] = createSignal<string | null>(null);
  const [name, setName] = createSignal("");
  const [source, setSource] = createSignal("");
  // The text on the server; a new script compares against the starter.
  const [saved, setSaved] = createSignal(STARTER_SCRIPT);
  const [errors, setErrors] = createSignal<IndicatorError[]>([]);
  const [status, setStatus] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [confirmingDelete, setConfirmingDelete] = createSignal(false);

  const stored = loadDrafts();
  const [draftNames, setDraftNames] = createSignal(Object.keys(stored.drafts));
  const dirty = createMemo(
    () => source() !== saved() || (selected() === null && name() !== ""),
  );

  let view: ScriptView | undefined;
  let host!: HTMLDivElement;
  // No buffer is open before the first load: nothing to mirror yet.
  let bufferOpen = false;
  let draftTimer: ReturnType<typeof setTimeout> | undefined;
  let openRequest = 0;

  function flushDraft(): void {
    clearTimeout(draftTimer);
    draftTimer = undefined;
    if (!bufferOpen) return;
    const key = selected() ?? NEW_DRAFT;
    if (dirty()) stored.drafts[key] = { name: name(), source: source() };
    else delete stored.drafts[key];
    stored.selected = selected();
    setDraftNames(Object.keys(stored.drafts));
    if (!saveDrafts(stored)) setStatus("This browser refused to keep a draft: save soon.");
  }

  function scheduleDraft(): void {
    clearTimeout(draftTimer);
    draftTimer = setTimeout(flushDraft, DRAFT_DEBOUNCE_MS);
  }

  function showBuffer(key: string | null, bufferName: string, text: string, base: string): void {
    setSelected(key);
    setName(bufferName);
    setSaved(base);
    setErrors([]);
    view?.open(text);
    markErrors();
    bufferOpen = true;
    flushDraft();
  }

  function openNew(): void {
    openRequest++;
    const draft = stored.drafts[NEW_DRAFT];
    setStatus(draft ? "Unsaved draft restored." : "");
    showBuffer(null, draft?.name ?? "", draft?.source ?? STARTER_SCRIPT, STARTER_SCRIPT);
  }

  function newScript(): void {
    flushDraft();
    openNew();
  }

  async function openScript(target: string): Promise<void> {
    flushDraft();
    const request = ++openRequest;
    const draft = stored.drafts[target];
    try {
      const { source: text } = await props.session.callGetMyIndicator(target);
      if (request !== openRequest) return;
      setStatus(draft ? "Unsaved draft restored." : "");
      showBuffer(target, target, draft?.source ?? text, text);
    } catch (err) {
      if (request !== openRequest) return;
      if (err instanceof McpToolError && draft) {
        // Gone from the server (deleted elsewhere): the draft is a new script.
        delete stored.drafts[target];
        setStatus(`${target} is no longer saved; your draft is kept as a new script.`);
        showBuffer(null, target, draft.source, STARTER_SCRIPT);
      } else {
        // Keep the open buffer; with none yet, start a new script to type in.
        // A script gone since the last visit needs no message; one gone from the list does.
        const restoring = !bufferOpen;
        if (restoring) openNew();
        if (!restoring || !(err instanceof McpToolError)) {
          setErrors([{ step: "load", line: null, message: messageOf(err) }]);
        }
        void list.refetch();
      }
    }
  }

  onMount(() => {
    view = createScriptView(host, (text) => {
      setSource(text);
      scheduleDraft();
    });
    if (stored.selected !== null) void openScript(stored.selected);
    else openNew();
    const onPageHide = () => flushDraft();
    window.addEventListener("pagehide", onPageHide);
    onCleanup(() => {
      window.removeEventListener("pagehide", onPageHide);
      flushDraft();
      view?.destroy();
    });
  });

  // Validation errors of the last save, then what the chart reports for the
  // saved script: its line numbers mean nothing once the text has changed.
  const chartErrors = createMemo(() =>
    Object.entries(props.chartErrors ?? {})
      .filter(([key]) => keyName(key) === selected())
      .map(([, e]) => e),
  );
  const shownErrors = createMemo(() => (dirty() ? errors() : [...errors(), ...chartErrors()]));
  function markErrors(): void {
    view?.setErrors(shownErrors().flatMap((e) => (e.line === null ? [] : [e.line])));
  }
  createEffect(markErrors);

  const onChart = (n: string) => activeIndicators().some((e) => indicatorName(e) === n);

  async function save(): Promise<void> {
    if (busy()) return;
    const target = name().trim();
    if (selected() === null && scripts().some((s) => s.name === target)) {
      setErrors([{
        step: "register",
        line: null,
        message: `You already have an indicator named ${target}. Open it, or pick another name.`,
      }]);
      setStatus("");
      return;
    }
    setBusy(true);
    setStatus("Saving…");
    try {
      const text = source();
      const result = await props.session.callRegisterIndicator({ name: target, source: text });
      if (!result.ok) {
        setErrors(result.errors);
        setStatus("Not saved.");
        const first = result.errors.find((e) => e.line !== null);
        if (first?.line != null) view?.reveal(first.line);
        return;
      }
      delete stored.drafts[selected() ?? NEW_DRAFT];
      setSelected(result.name);
      setName(result.name);
      setSaved(text);
      setErrors([]);
      flushDraft();
      setStatus(
        result.shadows_builtin
          ? `Saved. It replaces the built-in ${result.name} for you.`
          : "Saved.",
      );
      void list.refetch();
      void refreshIndicatorList(queryClient);
      void refreshChartsUsing(queryClient, result.name);
    } catch (err) {
      setErrors([{ step: "save", line: null, message: messageOf(err) }]);
      setStatus("Not saved.");
    } finally {
      setBusy(false);
    }
  }

  async function remove(): Promise<void> {
    const target = selected();
    setConfirmingDelete(false);
    if (target === null || busy()) return;
    setBusy(true);
    try {
      await props.session.callDeleteIndicator(target);
      delete stored.drafts[target];
      clearTimeout(draftTimer);
      openNew();
      setStatus(`Deleted ${target}.`);
      void list.refetch();
      // A built-in it shadowed is back under that name, so the chart keeps it;
      // any other name would now fail the whole chart request.
      await refreshIndicatorList(queryClient);
      const registry = queryClient.getQueryData<ListIndicatorsPayload>(["list_indicators"]);
      const builtinLeft = registry?.indicators.some(
        (i) => i.source !== "user" && (i.name === target || i.aliases?.includes(target)),
      );
      if (!builtinLeft) {
        setActiveIndicators(activeIndicators().filter((e) => indicatorName(e) !== target));
      }
      void refreshChartsUsing(queryClient, target);
    } catch (err) {
      setErrors([{ step: "delete", line: null, message: messageOf(err) }]);
    } finally {
      setBusy(false);
    }
  }

  function addToChart(): void {
    const target = selected();
    if (target === null) return;
    if (addIndicator(target) === null) showToast("At most 50 indicators per chart");
    else setStatus(`Added ${target} to the chart.`);
  }

  function revert(): void {
    setStatus("");
    view?.replace(saved());
    if (selected() === null) setName("");
    flushDraft();
  }

  return (
    <section
      aria-label="Script editor"
      data-testid="script-editor"
      class="script-editor flex flex-col shrink-0 h-[42vh] min-h-[280px] border-t border-[color:var(--color-border)] bg-[color:var(--color-bg)] max-[767px]:fixed max-[767px]:inset-0 max-[767px]:z-40 max-[767px]:h-dvh max-[767px]:min-h-0 max-[767px]:border-t-0"
      onKeyDown={(e) => {
        if ((e.ctrlKey || e.metaKey) && !e.altKey && e.key.toLowerCase() === "s") {
          e.preventDefault();
          void save();
        }
      }}
    >
      <div class="flex flex-wrap items-center gap-2 px-3 py-2 border-b border-[color:var(--color-border)]">
        <h2 class="text-sm font-semibold">Script editor</h2>
        <button type="button" class={buttonClass} data-testid="script-new" onClick={newScript}>
          New
        </button>
        <input
          type="text"
          value={name()}
          onInput={(e) => {
            setName(e.currentTarget.value);
            scheduleDraft();
          }}
          readOnly={selected() !== null}
          placeholder="indicator_name"
          aria-label="Indicator name"
          autocomplete="off"
          spellcheck={false}
          data-testid="script-name"
          class="min-w-0 w-44 max-[767px]:order-2 max-[767px]:w-full bg-[color:var(--color-bg-elev)] border border-[color:var(--color-border)] rounded-md px-2 py-1 text-xs font-mono text-[color:var(--color-fg)] focus:outline-none focus:border-[color:var(--color-accent)] read-only:opacity-70"
        />
        <button
          type="button"
          class={`${primaryClass} ${LOWER_ROW}`}
          data-testid="script-save"
          disabled={busy()}
          title="Validate and save (Ctrl/Cmd+S)"
          onClick={() => void save()}
        >
          Save
        </button>
        <button
          type="button"
          class={`${buttonClass} ${LOWER_ROW}`}
          data-testid="script-add"
          disabled={selected() === null || dirty()}
          title={dirty() || selected() === null ? "Save first: the chart runs the saved script" : "Add to the chart"}
          onClick={addToChart}
        >
          Add to chart
        </button>
        <Show when={dirty()}>
          <button type="button" class={`${buttonClass} ${LOWER_ROW}`} data-testid="script-revert" onClick={revert}>
            Revert
          </button>
        </Show>
        <Show when={selected() !== null}>
          <button
            type="button"
            class={`${buttonClass} ${LOWER_ROW}`}
            data-testid="script-delete"
            disabled={busy()}
            onClick={() => setConfirmingDelete(true)}
          >
            Delete
          </button>
        </Show>
        <span
          role="status"
          data-testid="script-status"
          class={`min-w-0 truncate text-xs text-[color:var(--color-fg-muted)] ${LOWER_ROW}`}
        >
          {status()}
        </span>
        <span class="ml-auto flex items-center gap-2 max-[767px]:order-1">
          <a
            href={SCRIPTING_DOCS_URL}
            target="_blank"
            rel="noopener noreferrer"
            data-testid="script-docs"
            class="text-xs text-[color:var(--color-accent)] hover:underline"
          >
            Docs
          </a>
          <button
            type="button"
            aria-label="Close script editor"
            data-testid="script-close"
            class="text-[color:var(--color-fg-muted)] hover:text-[color:var(--color-fg)] cursor-pointer px-2 py-0.5 text-sm"
            onClick={() => props.onClose()}
          >
            ×
          </button>
        </span>
      </div>
      <div class="flex flex-1 min-h-0 max-[767px]:flex-col">
        <div class="w-48 shrink-0 overflow-y-auto border-r border-[color:var(--color-border)] max-[767px]:w-full max-[767px]:max-h-24 max-[767px]:border-r-0 max-[767px]:border-b">
          <Show when={list.isError}>
            <p class="p-2 text-xs text-[color:var(--color-danger)]">{messageOf(list.error)}</p>
          </Show>
          <Show when={list.isSuccess && scripts().length === 0}>
            <p class="p-2 text-xs text-[color:var(--color-fg-muted)]">No saved indicators yet.</p>
          </Show>
          <ul data-testid="script-list">
            <For each={scripts()}>
              {(s) => (
                <li>
                  <button
                    type="button"
                    data-testid={`script-row-${s.name}`}
                    aria-current={selected() === s.name ? "true" : undefined}
                    title={s.description}
                    class="w-full text-left px-2 py-1 text-xs cursor-pointer hover:bg-[color:var(--color-bg-elev)] aria-[current]:bg-[color:var(--color-bg-elev)]"
                    onClick={() => void openScript(s.name)}
                  >
                    <span class="block font-mono truncate">
                      {s.name}
                      <Show when={draftNames().includes(s.name)}>
                        <span title="Unsaved changes"> •</span>
                      </Show>
                    </span>
                    <span class="block truncate text-[color:var(--color-fg-muted)]">
                      {s.title}
                      <Show when={onChart(s.name)}> · on chart</Show>
                    </span>
                  </button>
                </li>
              )}
            </For>
          </ul>
        </div>
        <div class="flex flex-col flex-1 min-w-0 min-h-0">
          <div ref={host} data-testid="script-code" class="flex-1 min-h-0 overflow-hidden" />
          <Show when={shownErrors().length > 0}>
            <ul
              data-testid="script-errors"
              class="max-h-28 shrink-0 overflow-y-auto border-t border-[color:var(--color-danger)] text-xs"
            >
              <For each={shownErrors()}>
                {(e) => (
                  <li>
                    <button
                      type="button"
                      disabled={e.line === null}
                      class="w-full text-left px-3 py-1 whitespace-pre-wrap break-words font-mono text-[color:var(--color-danger)] enabled:cursor-pointer enabled:hover:bg-[color:var(--color-bg-elev)]"
                      onClick={() => {
                        if (e.line !== null) view?.reveal(e.line);
                      }}
                    >
                      {errorLabel(e)}
                    </button>
                  </li>
                )}
              </For>
            </ul>
          </Show>
        </div>
      </div>
      <Dialog.Root open={confirmingDelete()} onOpenChange={setConfirmingDelete} modal>
        <Dialog.Portal>
          <Dialog.Overlay class="fixed inset-0 z-40 bg-black/60" />
          <div class="fixed inset-0 z-50 flex items-center justify-center px-4">
            <Dialog.Content
              data-testid="script-delete-dialog"
              class="w-full max-w-sm bg-[color:var(--color-bg)] border border-[color:var(--color-border)] rounded-lg shadow-xl p-4"
            >
              <Dialog.Title class="text-sm font-semibold mb-1">Delete indicator</Dialog.Title>
              <Dialog.Description class="text-sm text-[color:var(--color-fg-muted)] mb-4">
                Delete “{selected()}”? This cannot be undone.
                <Show when={selected() !== null && onChart(selected()!)}>
                  {" "}It also leaves the chart.
                </Show>
              </Dialog.Description>
              <div class="flex justify-end gap-2">
                <button type="button" class={buttonClass} onClick={() => setConfirmingDelete(false)}>
                  Cancel
                </button>
                <button
                  type="button"
                  class={primaryClass}
                  data-testid="script-delete-confirm"
                  onClick={() => void remove()}
                >
                  Delete
                </button>
              </div>
            </Dialog.Content>
          </div>
        </Dialog.Portal>
      </Dialog.Root>
    </section>
  );
}

function messageOf(err: unknown): string {
  return err instanceof McpToolError ? err.text : err instanceof Error ? err.message : String(err);
}

/** `Line 4 · Dry run: message`, as far as the server said. */
function errorLabel(e: IndicatorError): string {
  const parts = [e.line === null ? null : `Line ${e.line}`, STEP_LABEL[e.step]].filter(Boolean);
  return parts.length > 0 ? `${parts.join(" · ")}: ${e.message}` : e.message;
}
