// The server's drawing tool table (src/drawings.rs, ADR-0054) against the web
// app's own tools: the same names in the same order, and each tool's klinecharts
// template takes as many points as the table says.

import { getOverlayClass } from "klinecharts";
import { describe, expect, it } from "vitest";

import drawingTools from "../generated/drawing-tools.json";
import { DRAWING_NAMES, FILLED } from "../stores/drawings";
import "../components/drawingOverlays";

describe("drawing tools", () => {
  it("are the server's", () => {
    expect(drawingTools.tools.map((t) => t.name)).toEqual([...DRAWING_NAMES]);
    expect(drawingTools.tools.filter((t) => t.style.includes("fill")).map((t) => t.name)).toEqual([
      ...FILLED,
    ]);
  });

  it.each(drawingTools.tools.map((t) => [t.name, t.points.length] as const))(
    "%s takes %i point(s)",
    (name, points) => {
      const Template = getOverlayClass(name);
      expect(Template).not.toBeNull();
      // A position is placed with one click; its target and stop are derived points.
      const steps = name === "cgLong" || name === "cgShort" ? 4 : new Template!().totalStep;
      expect(steps - 1).toBe(points);
    },
  );
});
