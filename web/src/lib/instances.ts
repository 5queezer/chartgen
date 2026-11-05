// Indicator instance keys (ADR-0028). The first instance of an indicator is
// keyed by its name, later ones by `<name>#<n>`; the series payload is keyed
// the same way. No Solid here: pure libs (replay, dataWindow) use it too.

/** Indicator name of an instance key: `rsi#2` → `rsi`, `rsi` → `rsi`. */
export function keyName(key: string): string {
  const i = key.lastIndexOf("#");
  return i > 0 && /^[1-9]\d*$/.test(key.slice(i + 1)) ? key.slice(0, i) : key;
}

/** 1-based instance number of a key: `rsi` → 1, `rsi#2` → 2. */
export function instanceNumber(key: string): number {
  const name = keyName(key);
  return name === key ? 1 : Number(key.slice(name.length + 1));
}

// Default line colors of the instances after the first, so two MAs of one
// indicator do not share the server's color.
const INSTANCE_PALETTE = [
  "#e91e63", "#9c27b0", "#00bcd4", "#ff9800", "#8bc34a", "#795548",
];

/** Default color of line `i` for instance `key`; undefined for the first. */
export function instanceLineColor(key: string, i: number): string | undefined {
  const n = instanceNumber(key);
  return n > 1
    ? INSTANCE_PALETTE[(n - 2 + i) % INSTANCE_PALETTE.length]
    : undefined;
}
