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

/**
 * The page shows: the address says /admin, and the flag has done its work. A reload
 * at /admin sets it again; anywhere else this tab is the app.
 */
export function enterOperatorPage(): void {
  try {
    sessionStorage.removeItem(FLAG);
  } catch {
    // Nothing was kept.
  }
  if (window.location.pathname !== PATH) window.history.replaceState({}, "", PATH);
}

/**
 * What the server said. "refused" is its answer to everyone who is not an operator
 * in the web app, the same as for a path that does not exist, and also any answer
 * that is not the overview. "unavailable" is a failure that says nothing about who
 * asks: no answer at all, or an error of the server itself.
 */
export type OverviewAnswer =
  | { kind: "overview"; overview: OperatorOverview }
  | { kind: "refused" }
  | { kind: "unavailable" };

/**
 * Ask for the overview. JSON is asked for, so that "no" is the server's short answer
 * and not the app. A failure of the server or the network is not a "no": whoever was
 * shown the page keeps it, and whoever asked for it may try again.
 */
export async function fetchOverview(): Promise<OverviewAnswer> {
  const token = currentAccessToken();
  if (token === null) return { kind: "refused" };
  let res: Response;
  try {
    res = await fetch(url("/admin/overview"), {
      headers: { Accept: "application/json", Authorization: `Bearer ${token}` },
      cache: "no-store",
    });
  } catch {
    return { kind: "unavailable" };
  }
  if (res.status >= 500) return { kind: "unavailable" };
  if (!res.ok) return { kind: "refused" };
  const parsed = OperatorOverviewSchema.safeParse(await res.json().catch(() => null));
  return parsed.success ? { kind: "overview", overview: parsed.data } : { kind: "refused" };
}
