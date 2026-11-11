// The operator page at /admin (ADR-0063): whether this tab asked for it, and its
// one read. The server answers the read to a signed-in operator only; to everyone
// else it answers as it does for any unknown path, and then this tab is the app
// as at any other address, with no word about it.

import { currentAccessToken, url } from "../oauth";
import { OperatorOverviewSchema, type OperatorOverview } from "../types/responses";

const PATH = "/admin";
// Only this flag is kept across the sign-in, never an address: nothing stored is
// ever used as a place to go. Per tab, gone when the tab closes.
const FLAG = "chartgen.operator-page";

/** Note that this tab was opened at /admin. Before sign-in, which returns to `/`. */
export function noteOperatorPath(): void {
  try {
    if (window.location.pathname === PATH) sessionStorage.setItem(FLAG, "1");
  } catch {
    // No session storage: the page is asked for at /admin itself only.
  }
}

/** Whether this tab was opened at /admin. */
export function wantsOperatorPage(): boolean {
  try {
    return sessionStorage.getItem(FLAG) === "1";
  } catch {
    return window.location.pathname === PATH;
  }
}

/** This tab is the app again, at `/`. */
export function forgetOperatorPage(): void {
  try {
    sessionStorage.removeItem(FLAG);
  } catch {
    // Nothing was kept.
  }
  if (window.location.pathname === PATH) {
    window.history.replaceState({}, "", "/" + window.location.search + window.location.hash);
  }
}

/** The address says /admin while the page shows. */
export function showOperatorPath(): void {
  if (window.location.pathname !== PATH) window.history.replaceState({}, "", PATH);
}

/**
 * The overview, or `null` when there is none for this tab: not signed in, not an
 * operator, no such page on this server, or an answer that is not the overview.
 * JSON is asked for, so that "no" is the server's short answer and not the app.
 */
export async function fetchOverview(): Promise<OperatorOverview | null> {
  const token = currentAccessToken();
  if (token === null) return null;
  try {
    const res = await fetch(url("/admin/overview"), {
      headers: { Accept: "application/json", Authorization: `Bearer ${token}` },
      cache: "no-store",
    });
    if (!res.ok) return null;
    const parsed = OperatorOverviewSchema.safeParse(await res.json());
    return parsed.success ? parsed.data : null;
  } catch {
    return null;
  }
}
