// Indicator instance keys (ADR-0028). The first instance of an indicator is
// keyed by its name, later ones by `<name>#<n>`; the series payload is keyed
// the same way. No Solid here: pure libs (replay, dataWindow) use it too.

/** Indicator name of an instance key: `rsi#2` → `rsi`, `rsi` → `rsi`. */
export function keyName(key: string): string {
  const i = key.lastIndexOf("#");
  return i > 0 && /^[1-9]\d*$/.test(key.slice(i + 1)) ? key.slice(0, i) : key;
}
