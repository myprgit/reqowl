<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { EditorView } from "@codemirror/view";
  import { createState } from "../lib/editor/setup";
  import { httpBlocks, setOverlay, blocksField, type Overlay, type HttpBlockHandlers } from "../lib/editor/httpBlocks";

  interface Props {
    handlers: HttpBlockHandlers;
    onChange: (text: string) => void;
    variableNames: () => string[];
    /** Turns pasted text into something else (e.g. curl into an http block), or null to paste as is. */
    transformPaste: (text: string) => Promise<string | null>;
  }
  let { handlers, onChange, variableNames, transformPaste }: Props = $props();

  // Pasted curl commands: claim the paste synchronously, convert, then insert.
  const pasteHandler = EditorView.domEventHandlers({
    paste(event, view) {
      const text = event.clipboardData?.getData("text/plain") ?? "";
      if (!/^\s*curl\s/.test(text)) return false;
      event.preventDefault();
      const { from, to } = view.state.selection.main;
      transformPaste(text).then((out) => {
        const insert = out ?? text;
        view.dispatch({ changes: { from, to, insert }, selection: { anchor: from + insert.length }, scrollIntoView: true });
      });
      return true;
    },
  });

  let host: HTMLDivElement;
  let view: EditorView | undefined;
  let lastOverlay: Overlay = { keys: [], shown: new Map(), running: new Set(), collapsed: new Set() };

  onMount(() => {
    view = new EditorView({ parent: host });
  });
  onDestroy(() => view?.destroy());

  /** Replaces the document (opening another page or file). */
  export function load(doc: string, language: "markdown" | "toml") {
    if (!view) view = new EditorView({ parent: host });
    view.setState(
      createState({
        doc,
        language,
        onChange: (t) => onChange(t),
        variableNames,
        extra: language === "markdown" ? [httpBlocks(handlers), pasteHandler] : [],
      }),
    );
    if (language === "markdown") view.dispatch({ effects: setOverlay.of(lastOverlay) });
    view.scrollDOM.scrollTop = 0;
  }

  /** Swaps in text changed on disk, keeping the cursor roughly in place. */
  export function replaceText(doc: string) {
    if (!view) return;
    const head = Math.min(view.state.selection.main.head, doc.length);
    view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: doc }, selection: { anchor: head } });
  }

  export function setResponses(o: Overlay) {
    lastOverlay = o;
    if (view && view.state.field(blocksField, false)) view.dispatch({ effects: setOverlay.of(o) });
  }

  export function focus() {
    view?.focus();
  }

  export function text(): string {
    return view?.state.doc.toString() ?? "";
  }

  /** Line of the block the cursor is in, if any. */
  export function blockLines(): number[] {
    return view?.state.field(blocksField, false)?.map((b) => b.fenceLine) ?? [];
  }

  /** Inserts text on a new line after the given block. */
  export function insertAfterBlock(fenceLine: number, insert: string) {
    if (!view) return;
    const b = view.state.field(blocksField, false)?.find((b) => b.fenceLine === fenceLine);
    if (!b) return;
    const end = view.state.doc.line(b.lastLine + 1).to;
    view.dispatch({ changes: { from: end, insert: "\n" + insert }, scrollIntoView: true });
  }

  /** Range of the text between a block's fences. */
  function contentRange(fenceLine: number): { from: number; to: number } | null {
    if (!view) return null;
    const b = view.state.field(blocksField, false)?.find((b) => b.fenceLine === fenceLine);
    if (!b) return null;
    const doc = view.state.doc;
    const last = doc.line(b.lastLine + 1);
    const closed = b.lastLine > b.fenceLine && /^\s*(```|~~~)/.test(last.text);
    const from = b.fenceLine + 2 <= doc.lines ? doc.line(b.fenceLine + 2).from : doc.length;
    const to = closed ? last.from : doc.length;
    return { from: Math.min(from, to), to };
  }

  /** Replaces what's between a block's fences, e.g. after a form edit. */
  export function replaceBlockContent(fenceLine: number, content: string) {
    const r = contentRange(fenceLine);
    if (!view || !r) return;
    const insert = content.endsWith("\n") ? content : content + "\n";
    if (view.state.sliceDoc(r.from, r.to) === insert) return;
    view.dispatch({ changes: { from: r.from, to: r.to, insert } });
  }

  /** Adds an empty request block below the cursor's line; returns its fence line. */
  export function insertRequest(): number | null {
    if (!view) return null;
    const state = view.state;
    const inBlock = state.field(blocksField, false)?.find((b) => {
      const pos = state.selection.main.head;
      return pos >= b.from && pos <= b.to;
    });
    const at = inBlock ? state.doc.line(inBlock.lastLine + 1).to : state.doc.lineAt(state.selection.main.head).to;
    const lineText = state.doc.lineAt(at).text;
    const prefix = lineText.trim() === "" ? "\n" : "\n\n";
    const insert = `${prefix}\`\`\`http\nGET {{base}}/\n\`\`\`\n`;
    // Cursor at the end of the URL, ready to type the path.
    const urlEnd = at + prefix.length + "```http\nGET {{base}}/".length;
    view.dispatch({ changes: { from: at, insert }, selection: { anchor: urlEnd }, scrollIntoView: true });
    view.focus();
    return state.doc.lineAt(at).number - 1 + prefix.length;
  }

  export function revealLine(line: number) {
    if (!view) return;
    const pos = view.state.doc.line(Math.min(line + 1, view.state.doc.lines)).from;
    view.dispatch({ selection: { anchor: pos }, effects: EditorView.scrollIntoView(pos, { y: "center" }) });
    view.focus();
  }
</script>

<div class="editor" bind:this={host}></div>

<style>
  .editor {
    height: 100%;
    min-height: 0;
  }

  /* Ruled paper: one rule under each text line, scrolling with the text. */
  .editor :global(.cm-scroller) {
    background-image: linear-gradient(to bottom, transparent calc(var(--line) - 1px), var(--rule) calc(var(--line) - 1px));
    background-size: 100% var(--line);
    background-position: 0 28px;
    background-attachment: local;
  }

  /* The margin: a double amber rule; run markers sit on it. */
  .editor :global(.cm-run-gutter) {
    width: var(--margin-x);
    background: linear-gradient(
      to left,
      var(--amber) 0 2px,
      transparent 2px 5px,
      var(--amber) 5px 6px,
      transparent 6px
    );
  }

  .editor :global(.cm-run-gutter .cm-gutterElement) {
    display: flex;
    justify-content: flex-end;
    align-items: center;
  }

  .editor :global(.cm-run-marker) {
    width: 30px;
    margin-right: 12px;
    text-align: center;
    color: var(--amber-ink);
    font-size: 13px;
    cursor: pointer;
    border-radius: 5px;
    user-select: none;
  }

  .editor :global(.cm-run-marker:hover) {
    background: var(--amber);
    color: var(--ink);
  }

  .editor :global(.cm-run-marker-running) {
    animation: spin 1s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .editor :global(.cm-h) {
    font-family: var(--font-note);
    font-variation-settings: "MONO" 0, "CASL" 1, "slnt" 0, "CRSV" 1;
    letter-spacing: -0.01em;
  }

  .editor :global(.cm-inline-code) {
    font-variation-settings: var(--code-axes);
    background: var(--code-bg);
    border-radius: 3px;
  }

  .editor :global(.cm-var) {
    color: var(--amber-ink);
    background: color-mix(in srgb, var(--amber-wash) 70%, transparent);
    border-radius: 3px;
  }

  /* An http block reads like an index card clipped onto the page. */
  .editor :global(.cm-line.cm-http) {
    font-variation-settings: var(--code-axes);
    font-size: 13.5px;
    background: var(--paper-raised);
    border-left: 1px solid var(--rule-strong);
    border-right: 1px solid var(--rule-strong);
    margin-left: 10px;
    margin-right: 12px;
  }

  .editor :global(.cm-line.cm-http-open) {
    border-top: 1px solid var(--rule-strong);
    border-radius: 6px 6px 0 0;
  }

  .editor :global(.cm-line.cm-http-close) {
    border-bottom: 1px solid var(--rule-strong);
    border-radius: 0 0 6px 6px;
  }

  .editor :global(.cm-line.cm-http-open.cm-http-close) {
    border-radius: 6px;
  }

  .editor :global(.cm-form-link) {
    float: right;
    margin-right: 6px;
    font-family: var(--font-ui);
    font-variation-settings: normal;
    font-size: 11.5px;
    color: var(--ink-faint);
    cursor: pointer;
    padding: 0 6px;
    border-radius: 4px;
    user-select: none;
  }

  .editor :global(.cm-form-link:hover) {
    color: var(--ink);
    background: var(--amber-wash);
  }

  .editor :global(.cm-resp) {
    margin: 4px 12px 10px 28px;
    border-left: 3px solid var(--ink-faint);
    background: var(--paper);
    padding: 4px 0 6px 12px;
    cursor: pointer;
    font-family: var(--font-code);
    font-variation-settings: var(--code-axes);
    font-size: 12.5px;
    line-height: 19px;
  }

  .editor :global(.cm-resp:hover) {
    background: var(--paper-raised);
  }

  .editor :global(.cm-resp-ok) {
    border-left-color: var(--ok);
  }
  .editor :global(.cm-resp-warn) {
    border-left-color: var(--warn);
  }
  .editor :global(.cm-resp-err) {
    border-left-color: var(--err);
  }
  .editor :global(.cm-resp-running) {
    border-left-color: var(--amber);
    border-left-style: dashed;
  }

  .editor :global(.cm-resp-stale) {
    opacity: 0.78;
  }

  .editor :global(.cm-resp-head) {
    display: flex;
    gap: 14px;
    align-items: baseline;
    white-space: nowrap;
    overflow: hidden;
  }

  .editor :global(.cm-resp-fold) {
    width: 16px;
    margin-left: -4px;
    color: var(--ink-faint);
    text-align: center;
    border-radius: 4px;
    cursor: pointer;
    user-select: none;
  }

  .editor :global(.cm-resp-fold:hover) {
    color: var(--ink);
    background: var(--amber-wash);
  }

  .editor :global(.cm-resp-collapsed) {
    padding-bottom: 4px;
  }

  .editor :global(.cm-resp-status) {
    font-weight: 700;
  }
  .editor :global(.cm-resp-ok .cm-resp-status) {
    color: var(--ok);
  }
  .editor :global(.cm-resp-warn .cm-resp-status) {
    color: var(--warn);
  }
  .editor :global(.cm-resp-err .cm-resp-status) {
    color: var(--err);
  }

  .editor :global(.cm-resp-meta) {
    color: var(--ink-faint);
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .editor :global(.cm-resp-err .cm-resp-meta:nth-child(2)) {
    color: var(--err);
  }

  .editor :global(.cm-resp-body) {
    margin: 4px 0 0;
    white-space: pre-wrap;
    word-break: break-all;
    color: var(--ink-soft);
    font: inherit;
  }

  .editor :global(.cm-resp-more) {
    color: var(--amber-ink);
    margin-top: 2px;
  }

  .editor :global(.cm-resp-img) {
    display: block;
    max-width: 320px;
    max-height: 220px;
    margin-top: 6px;
    border: 1px solid var(--rule);
  }
</style>
