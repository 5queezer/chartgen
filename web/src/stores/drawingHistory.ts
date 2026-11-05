// Bounded undo/redo stack of whole-state snapshots. A new change clears
// the redo side, as in any editor.

export const MAX_HISTORY = 100;
/** Changes with the same key within this window are one undo step. */
export const COALESCE_MS = 800;

export interface History<T> {
  /**
   * Remember `prev`, the state a change just replaced. A change with the
   * same `key` as the previous one, within COALESCE_MS, adds no step.
   */
  record: (prev: T, key?: string) => void;
  /** The state to go back to (`current` becomes redoable), or undefined. */
  undo: (current: T) => T | undefined;
  /** The state to go forward to (`current` becomes undoable), or undefined. */
  redo: (current: T) => T | undefined;
  canUndo: () => boolean;
  canRedo: () => boolean;
  clear: () => void;
}

export function createHistory<T>(
  max = MAX_HISTORY,
  now: () => number = Date.now,
): History<T> {
  let past: T[] = [];
  let future: T[] = [];
  let lastKey: string | undefined;
  let lastAt = 0;
  return {
    record(prev, key) {
      const t = now();
      const merge = key !== undefined && key === lastKey && t - lastAt < COALESCE_MS;
      lastKey = key;
      lastAt = t;
      if (merge) return;
      past.push(prev);
      if (past.length > max) past.shift();
      future = [];
    },
    undo(current) {
      const prev = past.pop();
      if (prev === undefined) return undefined;
      lastKey = undefined;
      future.push(current);
      return prev;
    },
    redo(current) {
      const next = future.pop();
      if (next === undefined) return undefined;
      lastKey = undefined;
      past.push(current);
      if (past.length > max) past.shift();
      return next;
    },
    canUndo: () => past.length > 0,
    canRedo: () => future.length > 0,
    clear() {
      past = [];
      future = [];
      lastKey = undefined;
    },
  };
}
