/* @refresh reload */
import { render } from "solid-js/web";
import { QueryClient, QueryClientProvider } from "@tanstack/solid-query";

import {
  fetchOverview,
  forgetOperatorPage,
  noteOperatorPath,
  wantsOperatorPage,
} from "./services/operatorPage";
import { bootWorkspace } from "./services/workspace";
import "./styles/globals.css";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      refetchOnWindowFocus: false,
      staleTime: 60_000,
    },
  },
});

const root = document.getElementById("root");
if (!root) {
  throw new Error("#root element missing from index.html");
}

// The stores read localStorage as they are imported, so nothing that imports
// them may load before the right account's data is in place (ADR-0050).
async function start(root: HTMLElement): Promise<void> {
  // Sign-in can take a while, retries included, and the status line is not up yet.
  root.textContent = "Signing in...";
  // Before sign-in, which leaves this address and comes back to `/` (ADR-0063).
  noteOperatorPath();
  const boot = await bootWorkspace();
  if (wantsOperatorPage()) {
    const answer = boot.user ? await fetchOverview() : ({ kind: "refused" } as const);
    if (answer.kind !== "refused") {
      // The overview, or a server that failed for now: the page then offers to try
      // again, and this tab stays the operator's until the server says otherwise.
      const { OperatorPage } = await import("./components/OperatorPage");
      root.textContent = "";
      render(
        () => <OperatorPage first={answer.kind === "overview" ? answer.overview : null} />,
        root,
      );
      return;
    }
    // Not for this tab, or no such page here: the app, as at any other address.
    forgetOperatorPage();
  }
  const [{ App }, { applyShareLink }] = await Promise.all([
    import("./App"),
    import("./services/shareLink"),
  ]);

  // Share params of the URL go into the stores before the first render (and
  // before any OAuth redirect but a sign-in, which brings them back), so they
  // persist like a manual change.
  applyShareLink();

  root.textContent = "";
  render(
    () => (
      <QueryClientProvider client={queryClient}>
        <App authError={boot.authError} user={boot.user} />
      </QueryClientProvider>
    ),
    root,
  );
}

start(root).catch((err: unknown) => {
  console.error("[boot] cannot start:", err);
  root.textContent = `Cannot start: ${err instanceof Error ? err.message : String(err)}. If this browser's storage is full or blocked, accounts could not be kept apart.`;
});
