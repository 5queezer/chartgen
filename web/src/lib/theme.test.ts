import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

import {
  applyTheme,
  cssVarName,
  dark,
  light,
  type Palette,
} from "./theme";

const css = readFileSync(
  new URL("../styles/globals.css", import.meta.url),
  "utf8",
);

function cssVars(): Map<string, string> {
  const block = /@theme\s*{([\s\S]*?)\n}/.exec(css)![1]!;
  return new Map(
    [...block.matchAll(/(--color-[\w-]+):\s*([^;]+);/g)].map((m) => [
      m[1]!,
      m[2]!.trim(),
    ]),
  );
}

function luminance(hex: string): number {
  const [r, g, b] = [1, 3, 5].map((i) => {
    const c = parseInt(hex.slice(i, i + 2), 16) / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r! + 0.7152 * g! + 0.0722 * b!;
}

function contrast(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi! + 0.05) / (lo! + 0.05);
}

describe("theme tokens", () => {
  it("globals.css @theme defaults equal the dark palette", () => {
    const vars = cssVars();
    for (const key of Object.keys(dark) as (keyof Palette)[]) {
      expect(vars.get(cssVarName(key)), key).toBe(dark[key]);
    }
  });

  it("both palettes define the same keys", () => {
    expect(Object.keys(light)).toEqual(Object.keys(dark));
  });

  it("light palette uses TradingView's light values", () => {
    expect(light).toMatchObject({
      bg: "#ffffff",
      surface: "#f0f3fa",
      border: "#e0e3eb",
      fg: "#131722",
      accent: "#2962ff",
      bull: "#089981",
      bear: "#f23645",
      grid: "#f0f3fa",
      crosshair: "#9598a1",
    });
  });

  it("light text meets WCAG AA (4.5:1)", () => {
    for (const bg of [light.bg, light.surface]) {
      expect(contrast(light.fg, bg)).toBeGreaterThanOrEqual(4.5);
      expect(contrast(light.fgMuted, bg)).toBeGreaterThanOrEqual(4.5);
    }
    expect(contrast(light.accent, light.bg)).toBeGreaterThanOrEqual(4.5);
    expect(contrast(light.labelFg, light.labelBg)).toBeGreaterThanOrEqual(4.5);
  });

  it("applyTheme sets CSS variables and the scheme", () => {
    const set = new Map<string, string>();
    const root = {
      style: {
        setProperty: (k: string, v: string) => void set.set(k, v),
        colorScheme: "",
      },
      dataset: {} as Record<string, string>,
    } as unknown as HTMLElement;
    applyTheme("light", root);
    expect(set.get("--color-bg")).toBe("#ffffff");
    expect(set.get("--color-fg-muted")).toBe(light.fgMuted);
    expect(root.style.colorScheme).toBe("light");
    expect(root.dataset.theme).toBe("light");
  });
});
