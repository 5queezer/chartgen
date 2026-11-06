// The app's right-click menu: one Kobalte menu at the pointer, opened with the
// items of whatever was clicked, so placement (stays on screen), keys, look and
// closing are the same on every surface. Mouse only: on touch a long press is
// the crosshair (ADR-0039). guides/web.md says where touch finds these actions.
//
// Not modal, so a right click elsewhere closes this menu and opens that spot's.

import { DropdownMenu } from "@kobalte/core";
import { For, Show, createSignal, onCleanup, onMount, type JSX } from "solid-js";

export interface MenuItem {
  label: string;
  /** Muted text on the right: a shortcut. */
  hint?: string;
  /** Set: a checkbox item, with a check mark when true. */
  checked?: boolean;
  disabled?: boolean;
  /** Destructive: drawn in the danger color. */
  danger?: boolean;
  onSelect: () => void;
}

export interface MenuSpec {
  /** Accessible name of the menu. */
  label: string;
  /** `null` is a separator. */
  items: (MenuItem | null)[];
}

const [menu, setMenu] = createSignal<(MenuSpec & { x: number; y: number }) | null>(null);
let returnFocus: Element | null = null;
let touchPress = false;
let overlayMenu: (() => MenuSpec | null) | null = null;

const close = () => {
  const el = returnFocus;
  returnFocus = null;
  setMenu(null);
  // A closed menu leaves focus on <body>; a press outside moves it on by itself.
  if (el instanceof HTMLElement && el.isConnected && document.activeElement === document.body) {
    el.focus({ preventScroll: true });
  }
};

/**
 * Handle a `contextmenu` event with this menu. An empty menu or a touch press
 * opens nothing, and no native menu either. No surface holds a text field, so
 * those keep the browser's menu, for paste.
 */
export function openContextMenu(e: MouseEvent, spec: MenuSpec | null): void {
  e.preventDefault();
  e.stopPropagation();
  if (touchPress || !spec?.items.length) return;
  returnFocus = document.activeElement;
  setMenu({ ...spec, x: e.clientX, y: e.clientY });
}

/**
 * For a chart overlay. klinecharts reports its right click on the mousedown,
 * ahead of the browser's `contextmenu`: the chart's next one opens this instead
 * of its own. A null menu means the click was used up (it cancelled a tool).
 */
export function setOverlayMenu(build: () => MenuSpec | null): void {
  overlayMenu = build;
}

/** The menu set by `setOverlayMenu` for this click, once. */
export function takeOverlayMenu(): (() => MenuSpec | null) | null {
  const build = overlayMenu;
  overlayMenu = null;
  return build;
}

const contentClass =
  "bg-[color:var(--color-bg-elev)] border border-[color:var(--color-border)] rounded-md shadow-lg z-50 py-1 min-w-48 overflow-y-auto outline-none max-h-[var(--kb-popper-content-available-height)]";
const itemClass =
  "px-3 py-1.5 text-sm cursor-pointer flex items-center justify-between gap-6 outline-none data-[highlighted]:bg-[color:var(--color-border)] data-[disabled]:opacity-40 data-[disabled]:cursor-default";
const hintClass = "text-[color:var(--color-fg-muted)] text-xs";

function Entry(props: { item: MenuItem }): JSX.Element {
  const shared = () => ({
    class: `${itemClass}${props.item.danger ? " text-[color:var(--color-danger)]" : ""}`,
    disabled: props.item.disabled,
    // As Kobalte does for a plain item: the action, then the close on the next task.
    closeOnSelect: true,
    onSelect: props.item.onSelect,
  });
  return (
    <Show
      when={props.item.checked !== undefined}
      fallback={
        <DropdownMenu.Item {...shared()}>
          {props.item.label}
          <Show when={props.item.hint}>
            <span class={hintClass}>{props.item.hint}</span>
          </Show>
        </DropdownMenu.Item>
      }
    >
      <DropdownMenu.CheckboxItem {...shared()} checked={props.item.checked}>
        {props.item.label}
        <span class="flex items-center gap-2">
          <DropdownMenu.ItemIndicator>✓</DropdownMenu.ItemIndicator>
          <Show when={props.item.hint}>
            <span class={hintClass}>{props.item.hint}</span>
          </Show>
        </span>
      </DropdownMenu.CheckboxItem>
    </Show>
  );
}

/** Mount once. The menu itself, and the input that decides whether it may open. */
export function ContextMenuHost(): JSX.Element {
  onMount(() => {
    const onPointerDown = (e: PointerEvent) => {
      touchPress = e.pointerType !== "mouse";
      overlayMenu = null;
      if (menu() && !(e.target instanceof Element && e.target.closest("[role=menu]"))) close();
    };
    // A press with another button held has no pointerdown.
    const onMouseDown = () => {
      overlayMenu = null;
    };
    // A keyboard-opened menu has no press of its own.
    const onKeyDown = () => {
      touchPress = false;
      overlayMenu = null;
    };
    // What it was opened on moves away under it.
    const onWheel = (e: WheelEvent) => {
      if (menu() && !(e.target instanceof Element && e.target.closest("[role=menu]"))) close();
    };
    const onResize = () => menu() && close();
    window.addEventListener("pointerdown", onPointerDown, true);
    window.addEventListener("mousedown", onMouseDown, true);
    window.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("wheel", onWheel, { capture: true, passive: true });
    window.addEventListener("resize", onResize);
    onCleanup(() => {
      window.removeEventListener("pointerdown", onPointerDown, true);
      window.removeEventListener("mousedown", onMouseDown, true);
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("wheel", onWheel, true);
      window.removeEventListener("resize", onResize);
    });
  });

  return (
    <Show when={menu()} keyed>
      {(m) => (
        <DropdownMenu.Root
          open
          modal={false}
          onOpenChange={(open) => open || close()}
          getAnchorRect={() => ({ x: m.x, y: m.y, width: 0, height: 0 })}
          placement="right-start"
          gutter={2}
          shift={2}
        >
          <DropdownMenu.Portal>
            <DropdownMenu.Content
              class={contentClass}
              aria-label={m.label}
              // Only a press closes it (the host's listener). The app's own focus
              // recovery (keys.ts) can land on the chart while this opens, after a
              // press on a plain row.
              onFocusOutside={(e: Event) => e.preventDefault()}
              // Over the menu itself the browser's would open on top of it.
              onContextMenu={(e: MouseEvent) => e.preventDefault()}
            >
              <For each={m.items}>
                {(item) =>
                  item === null ? (
                    <DropdownMenu.Separator class="my-1 border-t border-[color:var(--color-border)]" />
                  ) : (
                    <Entry item={item} />
                  )
                }
              </For>
            </DropdownMenu.Content>
          </DropdownMenu.Portal>
        </DropdownMenu.Root>
      )}
    </Show>
  );
}
