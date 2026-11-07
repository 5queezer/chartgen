// A server that is full says so with `503` and `Retry-After` (ADR-0051). Nothing has run
// by then, so asking again is safe, and the SDK by itself does not: a refused stream would
// leave the tab polling for good, and a refused call would be a chart with an error.

/** Tries for a call before the `503` goes through to whoever asked: about two minutes at the server's 20 s wait. */
export const MAX_CALL_TRIES = 6;

const DEFAULT_RETRY_AFTER_S = 5;

const abortable = (ms: number, signal: AbortSignal | null | undefined): Promise<void> =>
  new Promise((resolve, reject) => {
    if (signal?.aborted) {
      reject(signal.reason);
      return;
    }
    const done = () => {
      signal?.removeEventListener("abort", stop);
      resolve();
    };
    const timer = setTimeout(done, ms);
    const stop = () => {
      clearTimeout(timer);
      reject(signal?.reason);
    };
    signal?.addEventListener("abort", stop, { once: true });
  });

/**
 * `fetch` that comes back when told to. A stream (`GET`) keeps asking for as long as the
 * connection is wanted, as it costs nothing while it waits and the tab belongs on it. A
 * call asks `MAX_CALL_TRIES` times. A `503` with no `Retry-After` is someone else's: a
 * proxy, say, with nothing to promise.
 */
export function fetchComingBack(base: typeof fetch): typeof fetch {
  return async (input, init) => {
    const isStream = (init?.method ?? "GET") === "GET";
    for (let tries = 1; ; tries++) {
      const response = await base(input, init);
      if (response.status !== 503 || !response.headers.has("Retry-After")) return response;
      if (!isStream && tries >= MAX_CALL_TRIES) return response;
      const seconds = Number(response.headers.get("Retry-After")) || DEFAULT_RETRY_AFTER_S;
      await response.body?.cancel();
      await abortable(seconds * 1000, init?.signal);
    }
  };
}
