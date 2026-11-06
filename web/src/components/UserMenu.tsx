// The signed-in user (ADR-0044): who, through which provider, and Sign out. A
// menu of its own at the right end of the top bar. The compact shell has no room
// for it next to the symbol, so there the same items sit in the toolbar's More
// tools menu. Only shown on a server with sign-in on. The name comes from the
// provider and is rendered as text.

import { DropdownMenu } from "@kobalte/core";
import type { JSX } from "solid-js";

import { signOut, type SignedInUser } from "../oauth";
import { showToast } from "../services/alertEngine";

const PROVIDERS: Record<string, string> = { github: "GitHub", google: "Google" };

const menuClass =
  "bg-[color:var(--color-bg-elev)] border border-[color:var(--color-border)] rounded-md shadow-lg z-50 py-1 min-w-52 max-w-72";
const itemClass =
  "px-3 py-1.5 text-sm cursor-pointer outline-none data-[highlighted]:bg-[color:var(--color-border)]";

/** The menu's content, for inside any `DropdownMenu.Content`. */
export function UserMenuItems(props: { user: SignedInUser }): JSX.Element {
  const provider = () => PROVIDERS[props.user.provider] ?? props.user.provider;
  const signOutNow = () => {
    void signOut().catch((e: unknown) =>
      showToast(`Could not sign out: ${e instanceof Error ? e.message : String(e)}`),
    );
  };
  return (
    <>
      <div class="px-3 py-1.5">
        <div class="text-xs text-[color:var(--color-fg-muted)]">Signed in with {provider()}</div>
        <div class="text-sm break-words" data-testid="user-name">
          {props.user.name}
        </div>
      </div>
      <DropdownMenu.Separator class="my-1 h-px bg-[color:var(--color-border)]" />
      <DropdownMenu.Item class={itemClass} onSelect={signOutNow}>
        Sign out
      </DropdownMenu.Item>
    </>
  );
}

export function UserMenu(props: { user: SignedInUser }): JSX.Element {
  return (
    <DropdownMenu.Root>
      <DropdownMenu.Trigger
        class="px-1.5 py-1 rounded-md text-sm cursor-pointer text-[color:var(--color-fg)] hover:bg-[color:var(--color-bg-elev)] inline-flex items-center gap-1.5 shrink-0 compact:hidden"
        aria-label={`Account: ${props.user.name}`}
        title={`Signed in as ${props.user.name}`}
        data-testid="user-menu"
      >
        <span
          aria-hidden="true"
          class="w-6 h-6 rounded-full inline-flex items-center justify-center text-xs font-semibold uppercase bg-[color:var(--color-border)]"
        >
          {props.user.name.charAt(0)}
        </span>
        <span class="max-w-28 truncate max-[1199px]:hidden">{props.user.name}</span>
      </DropdownMenu.Trigger>
      <DropdownMenu.Portal>
        <DropdownMenu.Content class={menuClass}>
          <UserMenuItems user={props.user} />
        </DropdownMenu.Content>
      </DropdownMenu.Portal>
    </DropdownMenu.Root>
  );
}
