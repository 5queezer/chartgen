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

/** WCAG relative luminance of an `#rrggbb` color. */
function luminance(hex: string): number {
  const n = parseInt(hex.slice(1), 16);
  const lin = (c: number) => {
    const v = c / 255;
    return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * lin((n >> 16) & 255) + 0.7152 * lin((n >> 8) & 255) + 0.0722 * lin(n & 255);
}

/** WCAG contrast ratio (1-21) of two `#rrggbb` colors. */
export function contrastRatio(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x) as [number, number];
  return (hi + 0.05) / (lo + 0.05);
}

/**
 * `color` made legible as text on `bg`, and opaque (a translucent plot color
 * would blend into the background). On a light background a color with
 * under 3:1 contrast is darkened (toward black, hue kept) until it reaches
 * it. Unparsable colors are left as they are.
 */
export function readableOn(color: string, bg: string, min = 3): string {
  if (!isColor(color) || !isColor(bg)) return color;
  const back = parseColor(bg).hex;
  const { hex } = parseColor(color);
  if (luminance(back) <= 0.5) return hex;
  const n = parseInt(hex.slice(1), 16);
  const rgb = [(n >> 16) & 255, (n >> 8) & 255, n & 255];
  for (let k = 1; k >= 0; k -= 0.05) {
    const out = `#${rgb.map((c) => hex2(c * k)).join("")}`;
    if (contrastRatio(out, back) >= min) return out;
  }
  return "#000000";
}
