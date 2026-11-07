/* @refresh reload */
import { render } from "solid-js/web";
import { QueryClient, QueryClientProvider } from "@tanstack/solid-query";

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
  const boot = await bootWorkspace();
  const [{ App }, { applyShareLink }] = await Promise.all([
    import("./App"),
    import("./services/shareLink"),
  ]);

  // Share params of the URL go into the stores before the first render (and
  // before any OAuth redirect but a sign-in, which brings them back), so they
  // persist like a manual change.
  applyShareLink();

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
  root.textContent =
    "Cannot start: this browser's storage is full or blocked, so accounts could not be kept apart.";
});
