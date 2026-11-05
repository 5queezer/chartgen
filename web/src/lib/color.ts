// Color helpers for the plot color picker: split a stored CSS color into an
// opaque `#rrggbb` and an alpha, and join them back.

const CHANNEL = "(25[0-5]|2[0-4]\\d|1\\d\\d|[1-9]?\\d)";
const COLOR_RE = new RegExp(
  `^(#([0-9a-f]{3}|[0-9a-f]{6}|[0-9a-f]{8})|rgba\\(\\s*${CHANNEL}\\s*,\\s*${CHANNEL}\\s*,\\s*${CHANNEL}\\s*,\\s*(0|1|0?\\.\\d+|1\\.0+)\\s*\\))$`,
  "i",
);
/** True for the color forms the stores accept: `#rgb`, `#rrggbb`, `#rrggbbaa`, `rgba(r, g, b, a)`. */
export function isColor(x: unknown): x is string {
  return typeof x === "string" && COLOR_RE.test(x);
}

const clamp255 = (n: number) => Math.min(255, Math.max(0, Math.round(n)));
const hex2 = (n: number) => clamp255(n).toString(16).padStart(2, "0");

/** Split `#rgb`, `#rrggbb`, `#rrggbbaa` or `rgba(...)` into `#rrggbb` and alpha 0-1. */
export function parseColor(color: string): { hex: string; alpha: number } {
  const c = color.trim().toLowerCase();
  const rgba = /^rgba?\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)\s*(?:,\s*([\d.]+)\s*)?\)$/.exec(c);
  if (rgba) {
    return {
      hex: `#${hex2(+rgba[1]!)}${hex2(+rgba[2]!)}${hex2(+rgba[3]!)}`,
      alpha: rgba[4] === undefined ? 1 : Math.min(1, Number(rgba[4])),
    };
  }
  if (/^#[0-9a-f]{3}$/.test(c)) {
    return { hex: `#${c[1]}${c[1]}${c[2]}${c[2]}${c[3]}${c[3]}`, alpha: 1 };
  }
  if (/^#[0-9a-f]{6}([0-9a-f]{2})?$/.test(c)) {
    const a = c.length === 9 ? parseInt(c.slice(7), 16) / 255 : 1;
    return { hex: c.slice(0, 7), alpha: a };
  }
  return { hex: "#000000", alpha: 1 };
}

/** Join an opaque `#rrggbb` and alpha 0-1: `#rrggbb` when opaque, else `rgba()`. */
export function formatColor(hex: string, alpha: number): string {
  const a = Math.round(Math.min(1, Math.max(0, alpha)) * 100) / 100;
  if (a >= 1) return hex;
  const n = parseInt(hex.slice(1), 16);
  return `rgba(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}, ${a})`;
}
