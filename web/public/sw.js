// Service worker for alert notifications (ADR-0045). It handles push and clicks and
// nothing else: with no fetch handler it caches nothing, so it cannot serve a stale app
// or stand between the page and /mcp. Served from the site root with no-cache, so a new
// version is picked up at once (ADR-0027).

const LABELS = {
  crossing: "crossing",
  crossing_up: "crossing up",
  crossing_down: "crossing down",
  greater: "above",
  less: "below",
};

function notificationOf(fire) {
  const label = LABELS[fire.condition] ?? fire.condition;
  const note = fire.message ? ` · ${fire.message}` : "";
  return {
    title: `${fire.symbol} ${label} ${fire.level}`,
    options: {
      body: `Price ${fire.price}${note}`,
      // A delivery sent twice (it is at least once) replaces itself.
      tag: `alert-fired-${fire.id}`,
      icon: "/icons/icon-192.png",
      data: { symbol: fire.symbol },
    },
  };
}

// A fixed worker takes over at once, not when the last tab closes. With no fetch handler
// that cannot mix versions of the app.
self.addEventListener("install", () => self.skipWaiting());

// Always shows something: a browser may end the subscription of a site whose pushes
// show nothing. A tab that is open stays quiet about these fires instead (`pushReport`).
self.addEventListener("push", (event) => {
  let shown = { title: "chartgen alert", options: { icon: "/icons/icon-192.png" } };
  try {
    shown = notificationOf(event.data.json());
  } catch {
    // Not ours, or not readable: the plain title stands.
  }
  event.waitUntil(self.registration.showNotification(shown.title, shown.options));
});

self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  const symbol = event.notification.data?.symbol;
  event.waitUntil(
    (async () => {
      const windows = await self.clients.matchAll({ type: "window", includeUncontrolled: true });
      const open = windows[0];
      if (open) {
        if (symbol) open.postMessage({ type: "chartgen.open-symbol", symbol });
        await open.focus();
        return;
      }
      await self.clients.openWindow(symbol ? `/?symbol=${encodeURIComponent(symbol)}` : "/");
    })(),
  );
});
