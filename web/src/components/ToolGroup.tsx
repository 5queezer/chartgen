// One drawing-toolbar button per tool group, TradingView style: the button
// shows the group's last-used tool and arms it on click; the small chevron
// (or a press-and-hold on the button) opens a flyout naming every tool of
// the group. The flyout is a menu: arrow keys move, Enter picks, Esc closes.

import { createSignal, For, onCleanup, Show, type JSX } from "solid-js";

export interface GroupTool {
  name: string;
  label: string;
  /** Alt+<code> shortcut. */
  code?: string;
  icon: () => JSX.Element;
}

export interface ToolGroupProps {
  group: string;
  tools: readonly GroupTool[];
  /** Name of the group's last-used tool (the one the button shows). */
  last: string;
  /** The armed tool, if any, in any group. */
  armed: string | null;
  onArm: (name: string) => void;
  btnClass: string;
}

const HOLD_MS = 450;
const itemClass =
  "w-full flex items-center gap-2 px-3 py-1.5 text-sm text-left cursor-pointer outline-none whitespace-nowrap hover:bg-[color:var(--color-border)] focus:bg-[color:var(--color-border)] data-[armed=true]:text-[color:var(--color-accent)]";

export function ToolGroup(props: ToolGroupProps): JSX.Element {
  const current = () => props.tools.find((t) => t.name === props.last) ?? props.tools[0]!;
  const pressed = () => props.tools.some((t) => t.name === props.armed);
  const title = (t: GroupTool) => (t.code ? `${t.label} (Alt+${t.code.slice(3)})` : t.label);

  const [open, setOpen] = createSignal(false);
  const [pos, setPos] = createSignal({ x: 0, y: 0 });
  let wrap: HTMLDivElement | undefined;
  let chevron: HTMLButtonElement | undefined;
  let menu: HTMLDivElement | undefined;
  let holdTimer: number | undefined;
  let held = false;

  const items = () => [...(menu?.querySelectorAll<HTMLElement>("[role=menuitem]") ?? [])];

  const close = (refocus = false) => {
    setOpen(false);
    if (refocus) chevron?.focus();
  };

  const show = () => {
    const r = wrap?.getBoundingClientRect();
    if (!r) return;
    setPos({ x: r.right + 4, y: r.top });
    setOpen(true);
    queueMicrotask(() => {
      const el = menu;
      if (!el) return;
      // Keep the flyout on screen when the button is near the bottom.
      setPos((p) => ({ x: p.x, y: Math.max(4, Math.min(p.y, window.innerHeight - el.offsetHeight - 8)) }));
      const i = props.tools.findIndex((t) => t.name === current().name);
      items()[Math.max(0, i)]?.focus();
    });
  };

  const pick = (name: string) => {
    close();
    props.onArm(name);
  };

  const onMenuKey = (e: KeyboardEvent) => {
    const list = items();
    const at = list.indexOf(document.activeElement as HTMLElement);
    const go = (i: number) => {
      e.preventDefault();
      list[(i + list.length) % list.length]?.focus();
    };
    if (e.key === "ArrowDown") go(at + 1);
    else if (e.key === "ArrowUp") go(at - 1);
    else if (e.key === "Home") go(0);
    else if (e.key === "End") go(list.length - 1);
    else if (e.key === "Escape") {
      // The flyout's own Esc: closing it must not also cancel an armed tool.
      e.preventDefault();
      e.stopPropagation();
      close(true);
    } else if (e.key === "Tab") close();
  };

  const onChevronKey = (e: KeyboardEvent) => {
    if (e.key === "ArrowDown" || e.key === "ArrowRight") {
      e.preventDefault();
      show();
    }
  };

  const onOutside = (e: PointerEvent) => {
    if (!(e.target instanceof Node)) return;
    if (!menu?.contains(e.target) && !wrap?.contains(e.target)) close();
  };
  window.addEventListener("pointerdown", onOutside);
  onCleanup(() => {
    window.removeEventListener("pointerdown", onOutside);
    window.clearTimeout(holdTimer);
  });

  const single = () => props.tools.length === 1;

  return (
    <div ref={wrap} class="relative shrink-0" data-tool-group={props.group}>
      <button
        type="button"
        class={props.btnClass}
        aria-label={current().label}
        title={title(current())}
        aria-pressed={pressed() && props.armed === current().name}
        onPointerDown={() => {
          held = false;
          if (single()) return;
          holdTimer = window.setTimeout(() => {
            held = true;
            show();
          }, HOLD_MS);
        }}
        onPointerUp={() => window.clearTimeout(holdTimer)}
        onPointerLeave={() => window.clearTimeout(holdTimer)}
        onClick={() => {
          if (held) {
            held = false;
            return;
          }
          props.onArm(current().name);
        }}
      >
        {current().icon()}
      </button>
      <Show when={!single()}>
        <button
          type="button"
          ref={chevron}
          class="absolute bottom-0 right-0 w-3.5 h-3.5 inline-flex items-center justify-center rounded-sm cursor-pointer text-[color:var(--color-fg-muted)] hover:text-[color:var(--color-fg)] hover:bg-[color:var(--color-border)]"
          aria-label={`${props.group} tools`}
          aria-haspopup="menu"
          aria-expanded={open()}
          data-testid={`tool-group-more-${props.group}`}
          onClick={() => (open() ? close() : show())}
          onKeyDown={onChevronKey}
        >
          <svg width="8" height="8" viewBox="0 0 8 8" aria-hidden="true" fill="currentColor">
            <path d="M1 2.5h6L4 6.5z" />
          </svg>
        </button>
        <Show when={open()}>
          <div
            ref={menu}
            role="menu"
            aria-label={`${props.group} tools`}
            data-tool-flyout={props.group}
            class="fixed z-50 min-w-48 py-1 rounded-md border border-[color:var(--color-border)] bg-[color:var(--color-bg-elev)] shadow-lg"
            style={{ left: `${pos().x}px`, top: `${pos().y}px` }}
            onKeyDown={onMenuKey}
          >
            <For each={props.tools}>
              {(t) => (
                <button
                  type="button"
                  role="menuitem"
                  class={itemClass}
                  aria-label={t.label}
                  data-armed={props.armed === t.name}
                  tabindex="-1"
                  data-tool={t.name}
                  onClick={() => pick(t.name)}
                >
                  <span class="w-5 h-5 inline-flex items-center justify-center">{t.icon()}</span>
                  <span class="flex-1">{t.label}</span>
                  <Show when={t.code}>
                    <span class="text-xs text-[color:var(--color-fg-muted)]">
                      Alt+{t.code!.slice(3)}
                    </span>
                  </Show>
                </button>
              )}
            </For>
          </div>
        </Show>
      </Show>
    </div>
  );
}
