// The Setups page at /setups (ADR-0091), reached as the operator page is (ADR-0063):
// a flag for the tab survives the sign-in, which returns to `/`. Whether the page shows
// is the server's answer: `list_setups` with this tab's token, which answers with the
// caller's own setups only. A server without setups, or no token, is the app at `/`.

// Loaded before the stores (ADR-0050): it imports none of them.
const PATH = "/setups";
// Only this flag is kept across the sign-in, never an address. Per tab.
const FLAG = "chartgen.setups-page";

/** Note that this tab was opened at /setups. Before sign-in, which returns to `/`. */
export function noteSetupsPath(): void {
  try {
    if (window.location.pathname === PATH) sessionStorage.setItem(FLAG, "1");
  } catch {
    // No session storage: the page is asked for at /setups itself only.
  }
}

/** Whether this tab was opened at /setups. */
export function wantsSetupsPage(): boolean {
  try {
    return sessionStorage.getItem(FLAG) === "1";
  } catch {
    return window.location.pathname === PATH;
  }
}

/** The page shows: the address says /setups, and the flag has done its work. */
export function enterSetupsPage(): void {
  try {
    sessionStorage.removeItem(FLAG);
  } catch {
    // Nothing was kept.
  }
  if (window.location.pathname !== PATH) window.history.replaceState({}, "", PATH);
}

/** This tab is the app again, at `/`. */
export function forgetSetupsPage(): void {
  try {
    sessionStorage.removeItem(FLAG);
  } catch {
    // Nothing was kept.
  }
  if (window.location.pathname === PATH) {
    window.history.replaceState({}, "", "/" + window.location.search + window.location.hash);
  }
}

/** The app's address that opens one setup on its chart, with its drawings. */
export const setupChartHref = (id: string): string => `/?setup=${encodeURIComponent(id)}`;

// The setup to open, kept for the tab until it is open: a server without sign-in sends the
// app through the OAuth redirect after it started, which loses the address.
const OPENING = "chartgen.setups.opening";

/**
 * The setup `setupChartHref` asks the app to open. It leaves the address at once and is
 * kept for the tab until `setupOpened`, through a redirect for a token.
 */
export function pendingSetup(): string | null {
  const params = new URLSearchParams(window.location.search);
  const id = params.get("setup");
  if (id !== null) {
    params.delete("setup");
    const search = params.toString();
    window.history.replaceState({}, "", window.location.pathname + (search ? `?${search}` : "") + window.location.hash);
  }
  try {
    if (id !== null && id !== "") sessionStorage.setItem(OPENING, id);
    return sessionStorage.getItem(OPENING);
  } catch {
    return id === "" ? null : id;
  }
}

/** The pending setup was opened, or could not be: it is not asked for again. */
export function setupOpened(): void {
  try {
    sessionStorage.removeItem(OPENING);
  } catch {
    // Nothing was kept.
  }
}
