// Layout menu at the right end of the top bar (ADR-0015): the active
// layout's name with an unsaved-changes dot, and a dropdown with Save
// (Ctrl/Cmd+S), Rename, Save as, Create new, Load (searchable list with
// per-row Delete) and Export/Import. Leaving a dirty layout asks first.

import { Dialog, DropdownMenu } from "@kobalte/core";
import {
  For,
  Show,
  createSignal,
  onCleanup,
  onMount,
  type JSX,
} from "solid-js";

import {
  MAX_LAYOUTS,
  MAX_IMPORT_BYTES,
  MAX_NAME_LENGTH,
  activeLayout,
  createLayout,
  deleteLayout,
  exportLayout,
  importLayout,
  layoutDirty,
  layouts,
  parseExport,
  renameLayout,
  saveLayout,
  saveLayoutAs,
  switchLayout,
} from "../stores/layouts";

type Dialogue =
  | {
      kind: "name";
      title: string;
      initial: string;
      submit: (name: string) => void;
    }
  | { kind: "load" }
  | { kind: "unsaved"; proceed: () => void }
  | { kind: "delete"; id: string; name: string };

const buttonClass =
  "px-2.5 py-1.5 rounded-md text-sm cursor-pointer text-[color:var(--color-fg)] hover:bg-[color:var(--color-bg-elev)] inline-flex items-center gap-1.5";
const menuClass =
  "bg-[color:var(--color-bg-elev)] border border-[color:var(--color-border)] rounded-md shadow-lg z-50 py-1 min-w-52";
const itemClass =
  "px-3 py-1.5 text-sm cursor-pointer flex items-center justify-between gap-6 outline-none data-[highlighted]:bg-[color:var(--color-border)]";
const dialogClass =
  "pointer-events-auto w-80 bg-[color:var(--color-bg)] border border-[color:var(--color-border)] rounded-lg shadow-xl p-4";
const inputClass =
  "w-full bg-[color:var(--color-bg-elev)] border border-[color:var(--color-border)] rounded-md px-3 py-2 text-sm text-[color:var(--color-fg)] focus:outline-none focus:border-[color:var(--color-accent)]";
const actionClass =
  "px-3 py-1.5 rounded-md text-sm cursor-pointer border border-[color:var(--color-border)] hover:bg-[color:var(--color-bg-elev)]";
const primaryClass =
  "px-3 py-1.5 rounded-md text-sm cursor-pointer bg-[color:var(--color-accent)] text-black font-semibold";

const isMac =
  typeof navigator !== "undefined" && /mac/i.test(navigator.platform);
const SAVE_HINT = isMac ? "⌘S" : "Ctrl+S";

function download(filename: string, text: string): void {
  const url = URL.createObjectURL(new Blob([text], { type: "application/json" }));
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  a.click();
  URL.revokeObjectURL(url);
}

export function LayoutMenu(): JSX.Element {
  const [dialogue, setDialogue] = createSignal<Dialogue | null>(null);
  const [notice, setNotice] = createSignal<{ text: string; error: boolean } | null>(
    null,
  );
  const [query, setQuery] = createSignal("");
  let fileInput!: HTMLInputElement;
  let noticeTimer: ReturnType<typeof setTimeout> | undefined;

  const say = (text: string, error = true) => {
    setNotice({ text, error });
    clearTimeout(noticeTimer);
    noticeTimer = setTimeout(() => setNotice(null), 4000);
  };
  const full = () => say(`Layout limit reached (${MAX_LAYOUTS})`);
  // Saving with nothing changed gives the same feedback as a real save.
  const save = () => {
    saveLayout();
    say("Layout saved", false);
  };

  // Ctrl/Cmd+S saves from anywhere, including inputs and the chart, and
  // keeps the browser's own save dialog from opening.
  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (
        (e.ctrlKey || e.metaKey) &&
        !e.altKey &&
        !e.shiftKey &&
        e.code === "KeyS"
      ) {
        e.preventDefault();
        save();
      }
    };
    window.addEventListener("keydown", onKey);
    onCleanup(() => {
      window.removeEventListener("keydown", onKey);
      clearTimeout(noticeTimer);
    });
  });

  /** Run `proceed` now, or after the user decides what to do with unsaved changes. */
  const leaving = (proceed: () => void) => {
    if (layoutDirty()) setDialogue({ kind: "unsaved", proceed });
    else proceed();
  };

  const askName = (
    title: string,
    initial: string,
    submit: (name: string) => boolean | void,
  ) => setDialogue({ kind: "name", title, initial, submit });

  const openLoad = () => {
    setQuery("");
    setDialogue({ kind: "load" });
  };

  const filtered = () => {
    const q = query().trim().toLowerCase();
    return layouts().filter((l) => l.name.toLowerCase().includes(q));
  };

  const onImportFile = async () => {
    const file = fileInput.files?.[0];
    fileInput.value = "";
    if (!file) return;
    const text = file.size > MAX_IMPORT_BYTES ? "" : await file.text();
    // Validate first so a bad file never raises the unsaved-changes prompt.
    if (parseExport(text) === null) {
      say("Not a valid layout file");
      return;
    }
    leaving(() => {
      if (!importLayout(text)) full();
    });
  };

  return (
    <div class="flex items-center gap-2 min-w-0" role="group" aria-label="Layout">
      <Show when={notice()}>
        {(n) => (
          <span
            role={n().error ? "alert" : "status"}
            data-testid="layout-notice"
            class={`text-xs whitespace-nowrap ${n().error ? "text-[color:var(--color-danger)]" : "text-[color:var(--color-fg-muted)]"}`}
          >
            {n().text}
          </span>
        )}
      </Show>
      <DropdownMenu.Root>
        <DropdownMenu.Trigger
          class={`${buttonClass} min-w-0 max-w-full`}
          aria-label={`Layout: ${activeLayout().name}${layoutDirty() ? " (unsaved changes)" : ""}`}
          title="Layouts"
        >
          <span class="min-w-0 max-w-40 truncate" data-testid="layout-name">
            {activeLayout().name}
          </span>
          <Show when={layoutDirty()}>
            <span
              data-testid="layout-dirty"
              title="Unsaved changes"
              class="inline-block w-2 h-2 shrink-0 rounded-full bg-[color:var(--color-accent)]"
            />
          </Show>
          <DropdownMenu.Icon aria-hidden="true" class="shrink-0">▾</DropdownMenu.Icon>
        </DropdownMenu.Trigger>
        <DropdownMenu.Portal>
          <DropdownMenu.Content class={menuClass}>
            <DropdownMenu.Item class={itemClass} onSelect={save}>
              Save layout
              <span class="text-xs text-[color:var(--color-fg-muted)]">
                {SAVE_HINT}
              </span>
            </DropdownMenu.Item>
            <DropdownMenu.Item
              class={itemClass}
              onSelect={() =>
                askName("Rename layout", activeLayout().name, (name) =>
                  renameLayout(activeLayout().id, name),
                )
              }
            >
              Rename…
            </DropdownMenu.Item>
            <DropdownMenu.Item
              class={itemClass}
              onSelect={() =>
                askName(
                  "Save layout as",
                  `${activeLayout().name} copy`,
                  (name) => {
                    if (!saveLayoutAs(name)) full();
                  },
                )
              }
            >
              Make a copy / Save as…
            </DropdownMenu.Item>
            <DropdownMenu.Item
              class={itemClass}
              onSelect={() =>
                leaving(() =>
                  askName("Create new layout", "", (name) => {
                    if (!createLayout(name)) full();
                  }),
                )
              }
            >
              Create new layout…
            </DropdownMenu.Item>
            <DropdownMenu.Item class={itemClass} onSelect={openLoad}>
              Load layout…
            </DropdownMenu.Item>
            <DropdownMenu.Separator class="my-1 border-t border-[color:var(--color-border)]" />
            <DropdownMenu.Item
              class={itemClass}
              onSelect={() => {
                const { id, name } = activeLayout();
                const json = exportLayout(id);
                if (json) download(`${name}.chartgen-layout.json`, json);
              }}
            >
              Export saved layout
            </DropdownMenu.Item>
            <DropdownMenu.Item
              class={itemClass}
              onSelect={() => fileInput.click()}
            >
              Import layout…
            </DropdownMenu.Item>
          </DropdownMenu.Content>
        </DropdownMenu.Portal>
      </DropdownMenu.Root>
      <input
        ref={fileInput}
        type="file"
        accept="application/json,.json"
        class="hidden"
        aria-label="Import layout file"
        onChange={() => void onImportFile()}
      />

      <Dialog.Root
        open={dialogue() !== null}
        onOpenChange={(open) => !open && setDialogue(null)}
        modal
      >
        <Dialog.Portal>
          <Dialog.Overlay class="fixed inset-0 z-40 bg-black/40" />
          <div class="fixed inset-0 z-50 flex items-start justify-center pt-[20vh] px-4 pointer-events-none">
            <Dialog.Content class={dialogClass}>
              {(() => {
                const d = dialogue();
                if (d === null) return null;
                switch (d.kind) {
                  case "name":
                    return <NameForm d={d} close={() => setDialogue(null)} />;
                  case "load":
                    return (
                      <>
                        <Dialog.Title class="text-sm font-semibold mb-2">
                          Load layout
                        </Dialog.Title>
                        <input
                          type="text"
                          autofocus
                          aria-label="Search layouts"
                          placeholder="Search layouts"
                          autocomplete="off"
                          class={inputClass}
                          value={query()}
                          onInput={(e) => setQuery(e.currentTarget.value)}
                        />
                        <ul class="mt-2 max-h-64 overflow-y-auto">
                          <For
                            each={filtered()}
                            fallback={
                              <li class="px-2 py-1.5 text-sm text-[color:var(--color-fg-muted)]">
                                No layouts match
                              </li>
                            }
                          >
                            {(l) => (
                              <li class="flex items-center gap-1">
                                <button
                                  type="button"
                                  data-layout-id={l.id}
                                  aria-current={
                                    l.id === activeLayout().id
                                      ? "true"
                                      : undefined
                                  }
                                  class="flex-1 min-w-0 text-left px-2 py-1.5 text-sm rounded-md cursor-pointer hover:bg-[color:var(--color-bg-elev)] aria-[current=true]:text-[color:var(--color-accent)] flex justify-between gap-4"
                                  onClick={() => {
                                    setDialogue(null);
                                    if (l.id === activeLayout().id) return;
                                    leaving(() => switchLayout(l.id));
                                  }}
                                >
                                  <span class="truncate">{l.name}</span>
                                  <span class="text-xs text-[color:var(--color-fg-muted)] shrink-0">
                                    {l.state.symbol} · {l.state.timeframe}
                                  </span>
                                </button>
                                <button
                                  type="button"
                                  aria-label={`Delete ${l.name}`}
                                  title={
                                    l.id === activeLayout().id
                                      ? "Switch to another layout first"
                                      : "Delete layout"
                                  }
                                  disabled={l.id === activeLayout().id}
                                  class="px-2 py-1.5 text-sm rounded-md cursor-pointer text-[color:var(--color-fg-muted)] hover:text-[color:var(--color-danger)] disabled:opacity-30 disabled:cursor-not-allowed disabled:hover:text-[color:var(--color-fg-muted)]"
                                  onClick={() =>
                                    setDialogue({
                                      kind: "delete",
                                      id: l.id,
                                      name: l.name,
                                    })
                                  }
                                >
                                  ×
                                </button>
                              </li>
                            )}
                          </For>
                        </ul>
                      </>
                    );
                  case "unsaved":
                    return (
                      <>
                        <Dialog.Title class="text-sm font-semibold mb-1">
                          Unsaved changes
                        </Dialog.Title>
                        <Dialog.Description class="text-sm text-[color:var(--color-fg-muted)] mb-4">
                          Save changes to “{activeLayout().name}” first?
                        </Dialog.Description>
                        <div class="flex justify-end gap-2">
                          <button
                            type="button"
                            class={actionClass}
                            onClick={() => setDialogue(null)}
                          >
                            Cancel
                          </button>
                          <button
                            type="button"
                            class={actionClass}
                            onClick={() => {
                              setDialogue(null);
                              d.proceed();
                            }}
                          >
                            Don't save
                          </button>
                          <button
                            type="button"
                            class={primaryClass}
                            onClick={() => {
                              save();
                              setDialogue(null);
                              d.proceed();
                            }}
                          >
                            Save
                          </button>
                        </div>
                      </>
                    );
                  case "delete":
                    return (
                      <>
                        <Dialog.Title class="text-sm font-semibold mb-1">
                          Delete layout
                        </Dialog.Title>
                        <Dialog.Description class="text-sm text-[color:var(--color-fg-muted)] mb-4">
                          Delete “{d.name}”? This cannot be undone.
                        </Dialog.Description>
                        <div class="flex justify-end gap-2">
                          <button
                            type="button"
                            class={actionClass}
                            onClick={() => setDialogue(null)}
                          >
                            Cancel
                          </button>
                          <button
                            type="button"
                            class={primaryClass}
                            onClick={() => {
                              deleteLayout(d.id);
                              setDialogue(null);
                            }}
                          >
                            Delete
                          </button>
                        </div>
                      </>
                    );
                }
              })()}
            </Dialog.Content>
          </div>
        </Dialog.Portal>
      </Dialog.Root>
    </div>
  );
}

/** Name prompt shared by Rename, Save as and Create new. */
function NameForm(props: {
  d: Extract<Dialogue, { kind: "name" }>;
  close: () => void;
}): JSX.Element {
  const [value, setValue] = createSignal(props.d.initial);
  const valid = () => value().trim().length > 0;
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        if (!valid()) return;
        props.d.submit(value());
        props.close();
      }}
    >
      <Dialog.Title class="text-sm font-semibold mb-2">
        {props.d.title}
      </Dialog.Title>
      <input
        type="text"
        autofocus
        aria-label="Layout name"
        maxlength={MAX_NAME_LENGTH}
        autocomplete="off"
        spellcheck={false}
        class={inputClass}
        value={value()}
        onInput={(e) => setValue(e.currentTarget.value)}
        onFocus={(e) => e.currentTarget.select()}
      />
      <div class="flex justify-end gap-2 mt-4">
        <button type="button" class={actionClass} onClick={props.close}>
          Cancel
        </button>
        <button
          type="submit"
          class={`${primaryClass} disabled:opacity-50`}
          disabled={!valid()}
        >
          OK
        </button>
      </div>
    </form>
  );
}
