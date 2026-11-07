import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { fetchComingBack, MAX_CALL_TRIES } from "./retryAfter";

const busy = (seconds = "2") =>
  new Response("full", { status: 503, headers: { "Retry-After": seconds } });
const ok = () => new Response("yes", { status: 200 });

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe("fetchComingBack", () => {
  it("asks again when told to, not before, and hands over the answer", async () => {
    const base = vi.fn<typeof fetch>().mockResolvedValueOnce(busy("2")).mockResolvedValueOnce(ok());
    const answer = fetchComingBack(base)("/mcp", { method: "POST", body: "{}" });
    await vi.advanceTimersByTimeAsync(1_900);
    expect(base).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(200);
    expect(base).toHaveBeenCalledTimes(2);
    expect(base.mock.calls[1]![1]).toEqual({ method: "POST", body: "{}" });
    expect((await answer).status).toBe(200);
  });

  it("gives a call up after so many tries, with the server's answer", async () => {
    const base = vi.fn<typeof fetch>().mockImplementation(async () => busy("1"));
    const answer = fetchComingBack(base)("/mcp", { method: "POST" });
    await vi.advanceTimersByTimeAsync(MAX_CALL_TRIES * 1_000);
    expect((await answer).status).toBe(503);
    expect(base).toHaveBeenCalledTimes(MAX_CALL_TRIES);
  });

  it("keeps asking for a stream, far past that", async () => {
    const base = vi.fn<typeof fetch>();
    for (let i = 0; i < MAX_CALL_TRIES * 3; i++) base.mockResolvedValueOnce(busy("30"));
    base.mockResolvedValueOnce(ok());
    const answer = fetchComingBack(base)("/mcp", { method: "GET" });
    await vi.advanceTimersByTimeAsync((MAX_CALL_TRIES * 3 + 1) * 30_000);
    expect((await answer).status).toBe(200);
  });

  it("stops waiting when the connection is no longer wanted", async () => {
    const controller = new AbortController();
    const base = vi.fn<typeof fetch>().mockImplementation(async () => busy("30"));
    const answer = fetchComingBack(base)("/mcp", { method: "GET", signal: controller.signal });
    const seen = expect(answer).rejects.toBe("closed");
    await vi.advanceTimersByTimeAsync(1_000);
    controller.abort("closed");
    await seen;
    expect(base).toHaveBeenCalledTimes(1);
  });

  it("leaves alone a 503 that promises nothing, and everything else", async () => {
    for (const response of [new Response("gone", { status: 503 }), new Response("no", { status: 500 }), ok()]) {
      const base = vi.fn<typeof fetch>().mockResolvedValue(response);
      expect((await fetchComingBack(base)("/mcp", { method: "GET" })).status).toBe(response.status);
      expect(base).toHaveBeenCalledTimes(1);
    }
  });
});
