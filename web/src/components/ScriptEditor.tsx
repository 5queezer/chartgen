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
  draftAt,
  loadDrafts,
  NEW_DRAFT,
  SCRIPTING_DOCS_URL,
  STARTER_SCRIPT,
  writeDraft,
  type ScriptDraft,
} from "../lib/scripts";
import { McpToolError, type McpSession } from "../mcp";
import { refreshChartsUsing, refreshIndicatorList } from "../queries";
import { showToast } from "../services/alertEngine";
import {
  activeIndicators,
  addIndicator,
  indicatorName,
  removeIndicatorEverywhere,
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
  unknown: "On the chart",
  params: "Parameters on the chart",
  chart: "Chart reload",
};

const buttonClass =
  "px-2.5 py-1 rounded-md text-xs cursor-pointer border border-[color:var(--color-border)] text-[color:var(--color-fg)] hover:bg-[color:var(--color-bg-elev)] disabled:opacity-50 disabled:cursor-default";
const primaryClass =
  "px-2.5 py-1 rounded-md text-xs cursor-pointer bg-[color:var(--color-accent)] text-white font-semibold disabled:opacity-50 disabled:cursor-default";

// Full screen: title and close, then the name on a row of its own, then these.
const LOWER_ROW = "script-lower";

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
    // Fresh each time the panel opens: another tab or client may have changed it.
    staleTime: 0,
    retry: false,
  }));
  const scripts = () => list.data?.indicators ?? [];

  // The open script: null is one not saved yet. Its unsaved text is stored under
  // `bufferKey`: its name, NEW_DRAFT, or for a saved script that is gone, its old name.
  const [selected, setSelected] = createSignal<string | null>(null);
  const [bufferKey, setBufferKey] = createSignal(NEW_DRAFT);
  const [name, setName] = createSignal("");
  const [source, setSource] = createSignal("");
  // The text on the server; a new script compares against the starter.
  const [saved, setSaved] = createSignal(STARTER_SCRIPT);
  // The saved text this draft started from. When the server's text differs, the
  // script changed elsewhere, and saving would destroy that change unasked.
  const [draftBase, setDraftBase] = createSignal(STARTER_SCRIPT);
  const conflict = () => selected() !== null && draftBase() !== saved();
  const [errors, setErrors] = createSignal<IndicatorError[]>([]);
  const [status, setStatus] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [confirmingDelete, setConfirmingDelete] = createSignal(false);

  const dirty = createMemo(
    () => source() !== saved() || (selected() === null && name() !== ""),
  );

  // Drafts the browser refused to store: they live here until this panel closes.
  const unwritten = new Map<string, ScriptDraft>();
  const storedDrafts = () => loadDrafts().drafts;
  const draftOf = (key: string) => unwritten.get(key) ?? draftAt(storedDrafts(), key);
  const draftKeys = () => [...new Set([...Object.keys(storedDrafts()), ...unwritten.keys()])];
  const [draftNames, setDraftNames] = createSignal(draftKeys());
  // Drafts whose script is not in the list: deleted elsewhere, or saved under an
  // earlier OAuth client. Offered, so they stay reachable.
  const orphans = createMemo(() =>
    list.data
      ? draftNames().filter((k) => k !== NEW_DRAFT && !scripts().some((s) => s.name === k))
      : [],
  );

  let view: ScriptView | undefined;
  let host!: HTMLDivElement;
  // No buffer is open before the first load: nothing to mirror yet.
  let bufferOpen = false;
  let draftTimer: ReturnType<typeof setTimeout> | undefined;
  let openRequest = 0;
  // The text this panel last stored, by draft key (see releaseDraft).
  const written = new Map<string, string>();

  function storeDraft(key: string, draft: ScriptDraft | null): void {
    if (writeDraft(key, draft, selected())) {
      unwritten.delete(key);
    } else if (draft) {
      unwritten.set(key, draft);
      setStatus("This browser refused to keep a draft: save soon.");
    }
    setDraftNames(draftKeys());
  }

  function flushDraft(): void {
    clearTimeout(draftTimer);
    draftTimer = undefined;
    if (!bufferOpen) return;
    const key = bufferKey();
    if (dirty()) {
      const base = selected() === null ? {} : { base: draftBase() };
      written.set(key, source());
      storeDraft(key, { name: name(), source: source(), ...base });
    } else {
      releaseDraft(key);
    }
  }

  /**
   * This buffer has no unsaved text under `key`. Remove the stored draft only if
   * this panel wrote it: a clean tab must not erase what another tab stored.
   */
  function releaseDraft(key: string): void {
    const stored = draftOf(key) ?? null;
    const mine = stored !== null && written.get(key) === stored.source;
    if (mine) written.delete(key);
    storeDraft(key, mine ? null : stored);
  }

  function scheduleDraft(): void {
    clearTimeout(draftTimer);
    draftTimer = setTimeout(flushDraft, DRAFT_DEBOUNCE_MS);
  }

  /** Open a buffer. What the old one holds is stored first, last keystrokes included. */
  function showBuffer(
    key: string,
    script: string | null,
    bufferName: string,
    text: string,
    base: string,
    startedFrom = base,
  ): void {
    flushDraft();
    setBufferKey(key);
    setSelected(script);
    setName(bufferName);
    setSaved(base);
    setDraftBase(startedFrom);
    setErrors([]);
    view?.open(text);
    markErrors();
    bufferOpen = true;
    flushDraft();
  }

  function openNew(): void {
    flushDraft();
    openRequest++;
    const draft = draftOf(NEW_DRAFT);
    setStatus(draft ? "Unsaved draft restored." : "");
    showBuffer(NEW_DRAFT, null, draft?.name ?? "", draft?.source ?? STARTER_SCRIPT, STARTER_SCRIPT);
  }

  async function openScript(target: string): Promise<void> {
    flushDraft();
    const request = ++openRequest;
    try {
      const result = await props.session.callGetMyIndicator(target);
      if (request !== openRequest) return;
      const draft = draftOf(target);
      if (result === null) {
        // Really gone. The draft stays under its own name, and keeps any other
        // draft safe: this buffer never takes over the new script's slot.
        if (draft) {
          setStatus(`${target} is no longer saved; your draft is kept. Save creates it again.`);
          showBuffer(target, null, draft.name || target, draft.source, STARTER_SCRIPT);
        } else {
          if (!bufferOpen) openNew();
          else setErrors([{ step: "load", line: null, message: `${target} is no longer saved.` }]);
          void list.refetch();
        }
        return;
      }
      const text = result.source;
      // A draft equal to the saved text is no draft; compared as text, because
      // `updated_at` only has a resolution of one second.
      if (draft?.source === text) storeDraft(target, null);
      const open = draft && draft.source !== text ? draft : undefined;
      const changed = open !== undefined && open.base !== text;
      setStatus(open && !changed ? "Unsaved draft restored." : "");
      showBuffer(target, target, target, open?.source ?? text, text, changed ? (open.base ?? "") : text);
    } catch (err) {
      if (request !== openRequest) return;
      // A failed read says nothing about the script: keep what is open and every
      // draft. With nothing open yet, start a new script to type in.
      if (!bufferOpen) openNew();
      setErrors([{ step: "load", line: null, message: messageOf(err) }]);
    }
  }

  onMount(() => {
    view = createScriptView(host, (text) => {
      setSource(text);
      scheduleDraft();
    });
    const restore = loadDrafts().selected;
    if (restore !== null) void openScript(restore);
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
    if (busy()) {
      setStatus("Still saving the last change…");
      return;
    }
    // Only with the list known is the buffer's identity known.
    if (list.data === undefined) {
      setStatus("Not saved: your indicators have not loaded yet.");
      return;
    }
    if (conflict()) {
      setStatus("Not saved: choose which version to keep first.");
      return;
    }
    const target = name().trim();
    setBusy(true);
    setStatus("Saving…");
    // Another script may be opened while this waits: the answer is not about it.
    const request = openRequest;
    const key = bufferKey();
    const script = selected();
    const base = saved();
    const text = source();
    try {
      // The server may have moved on since this was opened. The window left
      // between this read and the write is small; a new script needs no read.
      const current = script === null ? null : await props.session.callGetMyIndicator(script);
      if (current !== null && current.source !== base) {
        if (request === openRequest) {
          setSaved(current.source);
          flushDraft();
          setStatus("Not saved: this script changed elsewhere.");
        } else {
          showToast(`${target} was not saved: it changed elsewhere.`);
        }
        return;
      }
      const result = await props.session.callRegisterIndicator({
        name: target,
        source: text,
        create_only: script === null,
      });
      if (request !== openRequest) {
        if (!result.ok) {
          showToast(`${target} was not saved: ${result.errors[0]?.message ?? "refused"}`);
          return;
        }
        if (draftOf(key)?.source === text) storeDraft(key, null);
        showToast(`Saved ${target}.`);
      } else if (!result.ok) {
        setErrors(result.errors);
        setStatus("Not saved.");
        const first = result.errors.find((e) => e.line !== null);
        if (first?.line != null) view?.reveal(first.line);
        return;
      } else {
        releaseDraft(key);
        setBufferKey(result.name);
        setSelected(result.name);
        setName(result.name);
        setSaved(text);
        setDraftBase(text);
        setErrors([]);
        flushDraft();
        setStatus(
          result.shadows_builtin
            ? `Saved. It replaces the built-in ${result.name} for you.`
            : "Saved.",
        );
      }
      void list.refetch();
      void refreshIndicatorList(queryClient);
      void refreshChartsUsing(queryClient, result.name).then((failure) => {
        if (failure === null) return;
        if (request === openRequest) {
          setErrors((shown) => [...shown, { step: "chart", line: null, message: failure }]);
          setStatus("Saved, but the chart did not reload.");
        } else {
          showToast(`Saved ${target}, but the chart did not reload.`);
        }
      });
    } catch (err) {
      if (request === openRequest) {
        setErrors([{ step: "save", line: null, message: messageOf(err) }]);
        setStatus("Not saved.");
      } else {
        showToast(`${target} was not saved: ${messageOf(err)}`);
      }
    } finally {
      setBusy(false);
    }
  }

  async function remove(): Promise<void> {
    const target = selected();
    setConfirmingDelete(false);
    if (target === null || busy()) return;
    setBusy(true);
    const request = openRequest;
    try {
      await props.session.callDeleteIndicator(target);
      releaseDraft(target);
      if (request === openRequest) {
        // Its text is deleted with it: nothing to store from this buffer.
        bufferOpen = false;
        openNew();
        setStatus(`Deleted ${target}.`);
      } else {
        showToast(`Deleted ${target}.`);
      }
      void list.refetch();
      // A built-in it shadowed is back under that name, so the chart keeps it;
      // any other name would now be reported as unknown on the chart.
      await refreshIndicatorList(queryClient);
      const registry = queryClient.getQueryData<ListIndicatorsPayload>(["list_indicators"]);
      const builtinLeft = registry?.indicators.some(
        (i) => i.source !== "user" && (i.name === target || i.aliases?.includes(target)),
      );
      if (!builtinLeft) {
        removeIndicatorEverywhere(target);
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
    setDraftBase(saved());
    view?.replace(saved());
    if (selected() === null) setName("");
    flushDraft();
  }

  /** Keep this text: the next save replaces what changed elsewhere, knowingly. */
  function keepDraft(): void {
    setDraftBase(saved());
    flushDraft();
  }

  return (
    <section
      aria-label="Script editor"
      data-testid="script-editor"
      class="script-editor flex flex-col shrink-0 border-t border-[color:var(--color-border)] bg-[color:var(--color-bg)]"
      onKeyDown={(e) => {
        if ((e.ctrlKey || e.metaKey) && !e.altKey && e.key.toLowerCase() === "s") {
          e.preventDefault();
          void save();
        }
      }}
    >
      <div class="flex flex-wrap items-center gap-2 px-3 py-2 border-b border-[color:var(--color-border)]">
        <h2 class="text-sm font-semibold">Script editor</h2>
        <button type="button" class={buttonClass} data-testid="script-new" onClick={() => openNew()}>
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
          class="script-name min-w-0 w-44 bg-[color:var(--color-bg-elev)] border border-[color:var(--color-border)] rounded-md px-2 py-1 text-xs font-mono text-[color:var(--color-fg)] focus:outline-none focus:border-[color:var(--color-accent)] read-only:opacity-70"
        />
        <button
          type="button"
          class={`${primaryClass} ${LOWER_ROW}`}
          data-testid="script-save"
          disabled={busy() || list.data === undefined || conflict()}
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
        <span class="script-end ml-auto flex items-center gap-2">
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
      <div class="script-body flex flex-1 min-h-0">
        <div class="script-list w-48 shrink-0 overflow-y-auto border-r border-[color:var(--color-border)]">
          <Show when={list.isError}>
            <p class="p-2 text-xs text-[color:var(--color-danger)]">
              {messageOf(list.error)}{" "}
              <button
                type="button"
                data-testid="script-list-retry"
                class="underline cursor-pointer"
                onClick={() => void list.refetch()}
              >
                Retry
              </button>
            </p>
          </Show>
          <Show when={list.isSuccess && scripts().length === 0 && orphans().length === 0}>
            <p class="p-2 text-xs text-[color:var(--color-fg-muted)]">No saved indicators yet.</p>
          </Show>
          <ul data-testid="script-list">
            <For each={scripts()}>
              {(s) => (
                <li>
                  <button
                    type="button"
                    data-testid={`script-row-${s.name}`}
                    aria-current={bufferKey() === s.name ? "true" : undefined}
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
            <For each={orphans()}>
              {(key) => (
                <li>
                  <button
                    type="button"
                    data-testid={`script-row-${key}`}
                    aria-current={bufferKey() === key ? "true" : undefined}
                    class="w-full text-left px-2 py-1 text-xs cursor-pointer hover:bg-[color:var(--color-bg-elev)] aria-[current]:bg-[color:var(--color-bg-elev)]"
                    onClick={() => void openScript(key)}
                  >
                    <span class="block font-mono truncate">{key}</span>
                    <span class="block truncate text-[color:var(--color-fg-muted)]">
                      Draft, not saved
                    </span>
                  </button>
                </li>
              )}
            </For>
          </ul>
        </div>
        <div class="flex flex-col flex-1 min-w-0 min-h-0">
          <Show when={conflict()}>
            <div
              role="alert"
              data-testid="script-conflict"
              class="flex flex-wrap items-center gap-2 px-3 py-1.5 border-b border-[color:var(--color-danger)] text-xs text-[color:var(--color-fg)]"
            >
              <span class="min-w-0">
                This script changed elsewhere since your draft. Saving now would replace that change.
              </span>
              <button type="button" class={buttonClass} data-testid="script-use-saved" onClick={revert}>
                Use the saved version
              </button>
              <button type="button" class={buttonClass} data-testid="script-keep-draft" onClick={keepDraft}>
                Keep my draft
              </button>
            </div>
          </Show>
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
