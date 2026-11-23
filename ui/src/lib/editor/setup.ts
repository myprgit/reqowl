import { EditorState, type Extension, Compartment } from "@codemirror/state";
import {
  EditorView,
  drawSelection,
  highlightActiveLine,
  highlightSpecialChars,
  keymap,
  placeholder,
  rectangularSelection,
  dropCursor,
  MatchDecorator,
  ViewPlugin,
  Decoration,
  type DecorationSet,
  type ViewUpdate,
} from "@codemirror/view";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { searchKeymap, highlightSelectionMatches } from "@codemirror/search";
import {
  HighlightStyle,
  LanguageDescription,
  StreamLanguage,
  bracketMatching,
  indentOnInput,
  syntaxHighlighting,
} from "@codemirror/language";
import { autocompletion, closeBrackets, closeBracketsKeymap, completionKeymap, type CompletionSource } from "@codemirror/autocomplete";
import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
import { json } from "@codemirror/lang-json";
import { http } from "@codemirror/legacy-modes/mode/http";
import { toml } from "@codemirror/legacy-modes/mode/toml";
import { tags as t } from "@lezer/highlight";

const httpLang = StreamLanguage.define(http);

const codeLanguages = [
  LanguageDescription.of({ name: "http", alias: ["rest"], support: httpLang }),
  LanguageDescription.of({ name: "json", support: json() }),
];

const highlight = HighlightStyle.define([
  { tag: t.heading1, fontSize: "1.55em", fontWeight: "700", class: "cm-h" },
  { tag: t.heading2, fontSize: "1.3em", fontWeight: "700", class: "cm-h" },
  { tag: t.heading3, fontSize: "1.12em", fontWeight: "700", class: "cm-h" },
  { tag: [t.heading4, t.heading5, t.heading6], fontWeight: "700", class: "cm-h" },
  { tag: t.strong, fontWeight: "700" },
  { tag: t.emphasis, fontStyle: "italic" },
  { tag: t.strikethrough, textDecoration: "line-through" },
  { tag: t.link, color: "var(--ink)", textDecoration: "underline" },
  { tag: t.url, color: "var(--ink-soft)" },
  { tag: [t.processingInstruction, t.monospace], class: "cm-inline-code" },
  { tag: t.quote, color: "var(--ink-soft)", fontStyle: "italic" },
  { tag: [t.meta, t.comment, t.contentSeparator], color: "var(--ink-faint)" },
  { tag: t.keyword, color: "var(--amber-ink)", fontWeight: "700" },
  { tag: [t.atom, t.bool, t.null], color: "var(--amber-ink)" },
  { tag: t.number, color: "var(--ok)" },
  { tag: t.string, color: "var(--ink)" },
  { tag: [t.propertyName, t.attributeName, t.atom], color: "var(--ink-soft)", fontWeight: "600" },
  { tag: t.variableName, color: "var(--amber-ink)" },
  { tag: t.invalid, color: "var(--err)" },
]);

/** `{{name}}` stands out everywhere, including inside code blocks. */
const varMatcher = new MatchDecorator({
  regexp: /\{\{\s*[^{}\s][^{}]*?\s*\}\}/g,
  decoration: Decoration.mark({ class: "cm-var" }),
});
const varHighlight = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    constructor(view: EditorView) {
      this.decorations = varMatcher.createDeco(view);
    }
    update(u: ViewUpdate) {
      this.decorations = varMatcher.updateDeco(u, this.decorations);
    }
  },
  { decorations: (v) => v.decorations },
);

/** Completes `{{` with variable names, block response paths and built-ins. */
export function variableCompletion(names: () => string[]): CompletionSource {
  return (ctx) => {
    const m = ctx.matchBefore(/\{\{\s*[\w.$-]*/);
    if (!m) return null;
    const inner = m.text.replace(/^\{\{\s*/, "");
    const start = ctx.pos - inner.length;
    const builtins = ["$uuid", "$timestamp", "$isoTimestamp", "$randomInt 1 100"];
    return {
      from: start,
      options: [
        ...names().map((n) => ({ label: n, type: n.includes(".response.") ? "property" : "variable", apply: `${n}}}` })),
        ...builtins.map((b) => ({ label: b, type: "constant", apply: `${b}}}` })),
      ],
      validFor: /^[\w.$-]*$/,
    };
  };
}

export const themeCompartment = new Compartment();

const baseTheme = EditorView.theme({
  "&": { height: "100%", backgroundColor: "transparent", color: "var(--ink)" },
  ".cm-scroller": {
    fontFamily: "var(--font-note)",
    fontVariationSettings: "var(--note-axes)",
    fontSize: "15px",
    lineHeight: "var(--line)",
    overflow: "auto",
  },
  ".cm-content": { padding: "28px 0 40vh", maxWidth: "86ch", caretColor: "var(--ink)" },
  ".cm-line": { padding: "0 28px 0 18px" },
  "&.cm-focused .cm-cursor": { borderLeftColor: "var(--ink)", borderLeftWidth: "2px" },
  "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection": { backgroundColor: "var(--select) !important" },
  ".cm-activeLine": { backgroundColor: "color-mix(in srgb, var(--amber-wash) 35%, transparent)" },
  ".cm-gutters": { backgroundColor: "transparent", border: "none" },
  ".cm-placeholder": { color: "var(--ink-faint)" },
  ".cm-tooltip": { backgroundColor: "var(--paper-raised)", border: "1px solid var(--rule-strong)", color: "var(--ink)" },
  ".cm-tooltip-autocomplete > ul > li[aria-selected]": { backgroundColor: "var(--amber-wash)", color: "var(--ink)" },
  ".cm-panels": { backgroundColor: "var(--paper-side)", color: "var(--ink)" },
  ".cm-panels input, .cm-panels button": { fontFamily: "var(--font-ui)" },
  ".cm-searchMatch": { backgroundColor: "var(--amber-wash)", outline: "1px solid var(--amber)" },
});

export interface SetupOptions {
  doc: string;
  language: "markdown" | "toml";
  onChange: (text: string) => void;
  extra: Extension[];
  variableNames: () => string[];
  readOnly?: boolean;
}

export function createState(o: SetupOptions): EditorState {
  const lang =
    o.language === "markdown"
      ? markdown({ base: markdownLanguage, codeLanguages, addKeymap: true })
      : StreamLanguage.define(toml);
  return EditorState.create({
    doc: o.doc,
    extensions: [
      highlightSpecialChars(),
      history(),
      drawSelection(),
      dropCursor(),
      EditorState.allowMultipleSelections.of(true),
      indentOnInput(),
      bracketMatching(),
      closeBrackets(),
      rectangularSelection(),
      highlightActiveLine(),
      highlightSelectionMatches(),
      EditorView.lineWrapping,
      autocompletion({ override: [variableCompletion(o.variableNames)], icons: false }),
      keymap.of([...closeBracketsKeymap, ...defaultKeymap, ...searchKeymap, ...historyKeymap, ...completionKeymap, indentWithTab]),
      lang,
      syntaxHighlighting(highlight),
      varHighlight,
      baseTheme,
      placeholder(o.language === "markdown" ? "Write notes here. Add a request with a ```http block." : ""),
      EditorState.readOnly.of(!!o.readOnly),
      EditorView.updateListener.of((u) => {
        if (u.docChanged) o.onChange(u.state.doc.toString());
      }),
      ...o.extra,
    ],
  });
}

/** Read-only viewer for response bodies in the panel. */
export function viewerState(doc: string, asJson: boolean): EditorState {
  return EditorState.create({
    doc,
    extensions: [
      EditorState.readOnly.of(true),
      EditorView.editable.of(false),
      EditorView.contentAttributes.of({ tabindex: "0" }),
      drawSelection(),
      highlightSelectionMatches(),
      keymap.of([...searchKeymap, ...defaultKeymap]),
      asJson ? json() : [],
      syntaxHighlighting(highlight),
      EditorView.lineWrapping,
      EditorView.theme({
        "&": { height: "100%", backgroundColor: "transparent", color: "var(--ink)" },
        ".cm-scroller": {
          fontFamily: "var(--font-code)",
          fontVariationSettings: "var(--code-axes)",
          fontSize: "13px",
          lineHeight: "20px",
        },
        ".cm-content": { padding: "10px 0" },
        ".cm-line": { padding: "0 14px" },
        ".cm-panels": { backgroundColor: "var(--paper-side)", color: "var(--ink)" },
        ".cm-searchMatch": { backgroundColor: "var(--amber-wash)" },
      }),
    ],
  });
}
