// CodeMirror 6 setup of the script editor (ADR-0041): line numbers, Python
// highlighting (Starlark is a Python dialect), Tab indents, undo, and the lines
// a server error points at. Only ScriptEditor imports this, so all of CodeMirror
// stays in the chunk loaded when the panel first opens.

import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { python } from "@codemirror/lang-python";
import {
  bracketMatching,
  HighlightStyle,
  indentOnInput,
  indentUnit,
  syntaxHighlighting,
} from "@codemirror/language";
import { EditorState, StateEffect, StateField } from "@codemirror/state";
import {
  Decoration,
  EditorView,
  highlightActiveLine,
  highlightActiveLineGutter,
  keymap,
  lineNumbers,
  type DecorationSet,
} from "@codemirror/view";
import { tags as t } from "@lezer/highlight";

const setErrorLines = StateEffect.define<readonly number[]>();
const errorLine = Decoration.line({ class: "cm-script-error-line" });

// Marks stay on their text while it is edited, until the next set.
const errorLines = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(marks, tr) {
    let next = marks.map(tr.changes);
    for (const e of tr.effects) {
      if (!e.is(setErrorLines)) continue;
      const doc = tr.state.doc;
      const lines = [...new Set(e.value)].filter((l) => l >= 1 && l <= doc.lines);
      next = Decoration.set(
        lines.sort((a, b) => a - b).map((l) => errorLine.range(doc.line(l).from)),
      );
    }
    return next;
  },
  provide: (f) => EditorView.decorations.from(f),
});

// Colors come from CSS variables (scriptEditor.css), so they follow the theme.
const highlight = HighlightStyle.define([
  { tag: [t.keyword, t.operatorKeyword, t.controlKeyword, t.definitionKeyword], color: "var(--script-keyword)" },
  { tag: [t.string, t.special(t.string)], color: "var(--script-string)" },
  { tag: [t.number, t.bool, t.null], color: "var(--script-number)" },
  { tag: [t.lineComment, t.comment], color: "var(--script-comment)", fontStyle: "italic" },
  { tag: [t.function(t.variableName), t.function(t.definition(t.variableName))], color: "var(--script-function)" },
  { tag: [t.propertyName, t.function(t.propertyName)], color: "var(--script-property)" },
]);

const theme = EditorView.theme({
  "&": { height: "100%", fontSize: "13px", backgroundColor: "var(--color-bg)", color: "var(--color-fg)" },
  "&.cm-focused": { outline: "none" },
  ".cm-scroller": { fontFamily: "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace", overflow: "auto" },
  ".cm-content": { caretColor: "var(--color-fg)" },
  ".cm-cursor": { borderLeftColor: "var(--color-fg)" },
  ".cm-gutters": {
    backgroundColor: "var(--color-bg)",
    color: "var(--color-fg-muted)",
    border: "none",
    borderRight: "1px solid var(--color-border)",
  },
  ".cm-activeLine, .cm-activeLineGutter": { backgroundColor: "var(--script-active-line)" },
  ".cm-script-error-line": {
    backgroundColor: "color-mix(in srgb, var(--color-danger) 22%, transparent)",
    boxShadow: "inset 2px 0 0 var(--color-danger)",
  },
});

export interface ScriptView {
  /** Open another text, with its own undo history. */
  open: (source: string) => void;
  /** Replace the text as one undoable edit. */
  replace: (source: string) => void;
  setErrors: (lines: readonly number[]) => void;
  /** Scroll to a line, put the cursor on it and focus the editor. */
  reveal: (line: number) => void;
  destroy: () => void;
}

export function createScriptView(parent: HTMLElement, onChange: (source: string) => void): ScriptView {
  const extensions = [
    lineNumbers(),
    highlightActiveLine(),
    highlightActiveLineGutter(),
    history(),
    indentOnInput(),
    bracketMatching(),
    python(),
    indentUnit.of("    "),
    EditorState.tabSize.of(4),
    syntaxHighlighting(highlight),
    // Tab indents; as CodeMirror documents, Esc then Tab leaves the editor.
    keymap.of([...defaultKeymap, ...historyKeymap, indentWithTab]),
    errorLines,
    theme,
    EditorView.contentAttributes.of({ "aria-label": "Script source" }),
    EditorView.updateListener.of((u) => {
      if (u.docChanged) onChange(u.state.doc.toString());
    }),
  ];
  const view = new EditorView({ parent, state: EditorState.create({ extensions }) });
  return {
    open(source) {
      view.setState(EditorState.create({ doc: source, extensions }));
      onChange(source);
    },
    replace(source) {
      view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: source } });
    },
    setErrors(lines) {
      view.dispatch({ effects: setErrorLines.of(lines) });
    },
    reveal(line) {
      const doc = view.state.doc;
      const pos = doc.line(Math.min(Math.max(line, 1), doc.lines)).from;
      view.dispatch({ selection: { anchor: pos }, effects: EditorView.scrollIntoView(pos, { y: "center" }) });
      view.focus();
    },
    destroy: () => view.destroy(),
  };
}
