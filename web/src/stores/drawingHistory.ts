// Bounded undo/redo stack of whole-state snapshots. A new change clears
// the redo side, as in any editor.

export const MAX_HISTORY = 100;

export interface History<T> {
  /** Remember `prev`, the state a change just replaced. */
  record: (prev: T) => void;
  /** The state to go back to (`current` becomes redoable), or undefined. */
  undo: (current: T) => T | undefined;
  /** The state to go forward to (`current` becomes undoable), or undefined. */
  redo: (current: T) => T | undefined;
  clear: () => void;
}

export function createHistory<T>(max = MAX_HISTORY): History<T> {
  let past: T[] = [];
  let future: T[] = [];
  return {
    record(prev) {
      past.push(prev);
      if (past.length > max) past.shift();
      future = [];
    },
    undo(current) {
      const prev = past.pop();
      if (prev === undefined) return undefined;
      future.push(current);
      return prev;
    },
    redo(current) {
      const next = future.pop();
      if (next === undefined) return undefined;
      past.push(current);
      if (past.length > max) past.shift();
      return next;
    },
    clear() {
      past = [];
      future = [];
    },
  };
}
