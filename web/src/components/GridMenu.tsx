// Chart grid picker in the top bar (ADR-0043). Fewer charts remove the last
// ones; a removed chart with changes no saved layout holds asks first.

import { DropdownMenu } from "@kobalte/core";
import { For, type JSX } from "solid-js";

import { GRID_IDS, GRID_LABELS, GRID_SHAPES, grid, type GridId } from "../stores/grid";
import { changeGrid, panesLostBy } from "../stores/layouts";

function GridIcon(props: { id: GridId }): JSX.Element {
  const cells = () => {
    const { columns, rows } = GRID_SHAPES[props.id];
    const w = (14 - columns + 1) / columns;
    const h = (14 - rows + 1) / rows;
    return Array.from({ length: columns * rows }, (_, i) => ({
      x: 1 + (i % columns) * (w + 1),
      y: 1 + Math.floor(i / columns) * (h + 1),
      w,
      h,
    }));
  };
  return (
    <svg aria-hidden="true" viewBox="0 0 16 16" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.2">
      <For each={cells()}>
        {(c) => <rect x={c.x} y={c.y} width={c.w} height={c.h} rx="0.5" />}
      </For>
    </svg>
  );
}

function choose(next: GridId): void {
  if (next === grid()) return;
  const lost = panesLostBy(next).length;
  const what = lost === 1 ? "a chart" : `${lost} charts`;
  if (lost > 0 && !window.confirm(`Remove ${what} with unsaved changes?`)) return;
  changeGrid(next);
}

export function GridMenu(): JSX.Element {
  return (
    <DropdownMenu.Root>
      <DropdownMenu.Trigger
        class="p-1.5 rounded-md cursor-pointer text-[color:var(--color-fg)] hover:bg-[color:var(--color-bg-elev)] inline-flex items-center"
        data-testid="grid-menu"
        data-grid={grid()}
        aria-label={`Chart grid: ${GRID_LABELS[grid()]}`}
        title="Chart grid"
      >
        <GridIcon id={grid()} />
      </DropdownMenu.Trigger>
      <DropdownMenu.Portal>
        <DropdownMenu.Content class="bg-[color:var(--color-bg-elev)] border border-[color:var(--color-border)] rounded-md shadow-lg z-50 py-1 min-w-52">
          <For each={GRID_IDS}>
            {(id) => (
              <DropdownMenu.Item
                class="px-3 py-1.5 text-sm cursor-pointer flex items-center gap-2 outline-none data-[highlighted]:bg-[color:var(--color-border)]"
                data-testid={`grid-option-${id}`}
                onSelect={() => choose(id)}
              >
                <GridIcon id={id} />
                {GRID_LABELS[id]}
              </DropdownMenu.Item>
            )}
          </For>
        </DropdownMenu.Content>
      </DropdownMenu.Portal>
    </DropdownMenu.Root>
  );
}
