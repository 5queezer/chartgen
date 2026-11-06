// What a failed chart request says about itself, for ChartLoadError. Pure so the
// server's wording is pinned by a test.

export interface LoadErrorReason {
  /** Short reason for the headline. */
  text: string;
  /** Indicators that caused it: Retry cannot help, removing them can. */
  indicators: string[];
}

export function loadErrorReason(message: string): LoadErrorReason {
  const text = message.replace(/^MCP tool error: /, "");
  const unknown = /^Unknown indicator\(s\): (.+?)\. /.exec(text);
  if (unknown) {
    const indicators = unknown[1]!.split(", ");
    return { text: `unknown indicator ${indicators.join(", ")}`, indicators };
  }
  const invalid = /^Invalid parameter for ([^:]+): /.exec(text);
  if (invalid) return { text, indicators: [invalid[1]!] };
  // These say what to change, and Retry would only get them again.
  const advice = /(?:Yahoo has no seconds data|Box size \S+ is too small|no box size)[^]*$/.exec(text);
  if (advice) return { text: advice[0], indicators: [] };
  return {
    text: /\b404\b|not found|no data|unknown symbol|invalid symbol/i.test(message)
      ? "symbol not found"
      : "request failed",
    indicators: [],
  };
}
