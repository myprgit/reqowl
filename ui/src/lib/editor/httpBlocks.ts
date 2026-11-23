// CodeMirror extension for ```http blocks: run markers in the margin, a frame
// around each block, and the response shown as a widget under it.
//
// Responses live outside the document (the file stays clean); the app pushes
// them in with `setOverlay`. Block widgets must come from a StateField, not a
// view plugin, so positions and decorations are derived here from the syntax
// tree on every relevant transaction.

import { type EditorState, type Extension, type Range, StateEffect, StateField } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, GutterMarker, WidgetType, gutter, keymap } from "@codemirror/view";
import { syntaxTree } from "@codemirror/language";
import { type Shown, formatSize, prettyBody, statusClass } from "../api";

export interface Block {
  index: number;
  from: number;
  to: number;
  /** 0-based line of the opening fence; the backend identifies blocks by it. */
  fenceLine: number;
  /** 0-based line of the closing fence (or last line of an unclosed block). */
  lastLine: number;
}

export interface Overlay {
  /** Backend block keys, by block index. */
  keys: string[];
  shown: Map<string, Shown>;
  running: Set<string>;
}

export interface HttpBlockHandlers {
  run(fenceLine: number): void;
  open(key: string): void;
}

export const setOverlay = StateEffect.define<Overlay>();

const PREVIEW_LINES = 18;
const PREVIEW_CHARS = 4000;

export function findBlocks(state: EditorState): Block[] {
  const out: Block[] = [];
  syntaxTree(state).iterate({
    enter(node) {
      if (node.name !== "FencedCode") return;
      const info = node.node.getChild("CodeInfo");
      const lang = info ? state.sliceDoc(info.from, info.to).trim().split(/\s+/)[0].toLowerCase() : "";
      if (lang === "http") {
        out.push({
          index: out.length,
          from: node.from,
          to: node.to,
          fenceLine: state.doc.lineAt(node.from).number - 1,
          lastLine: state.doc.lineAt(node.to).number - 1,
        });
      }
      return false;
    },
  });
  return out;
}

export function blockAt(state: EditorState, pos: number): Block | undefined {
  return state.field(blocksField).find((b) => pos >= b.from && pos <= b.to);
}

function fenceAt(state: EditorState, lineFrom: number): Block | undefined {
  const n = state.doc.lineAt(lineFrom).number - 1;
  return state.field(blocksField).find((b) => b.fenceLine === n);
}

/** Blocks of the current document, recomputed when the text or the parse changes. */
export const blocksField = StateField.define<Block[]>({
  create: (state) => findBlocks(state),
  update(value, tr) {
    return tr.docChanged || syntaxTree(tr.startState) !== syntaxTree(tr.state) ? findBlocks(tr.state) : value;
  },
});

const overlayField = StateField.define<Overlay>({
  create: () => ({ keys: [], shown: new Map(), running: new Set() }),
  update(value, tr) {
    for (const e of tr.effects) if (e.is(setOverlay)) value = e.value;
    return value;
  },
});

class ResponseWidget extends WidgetType {
  constructor(
    readonly key: string,
    readonly shown: Shown | undefined,
    readonly running: boolean,
    readonly handlers: HttpBlockHandlers,
  ) {
    super();
  }

  eq(other: ResponseWidget) {
    return other.key === this.key && other.shown === this.shown && other.running === this.running;
  }

  toDOM() {
    const s = this.shown;
    const wrap = document.createElement("div");
    wrap.className = `cm-resp cm-resp-${this.running ? "running" : statusClass(s)}${s?.stale ? " cm-resp-stale" : ""}`;
    wrap.title = "Open in the response panel";

    const head = document.createElement("div");
    head.className = "cm-resp-head";
    const add = (cls: string, text: string) => {
      const el = document.createElement("span");
      el.className = cls;
      el.textContent = text;
      head.appendChild(el);
    };

    if (this.running) {
      add("cm-resp-status", "Running…");
      add("cm-resp-meta", "Esc cancels");
    } else if (s?.error) {
      add("cm-resp-status", s.status == null ? "Failed" : `${s.status}`);
      add("cm-resp-meta", s.error.split("\n")[0]);
    } else if (s) {
      add("cm-resp-status", `${s.status} ${s.reason}`);
      if (s.elapsedMs != null) add("cm-resp-meta", `${s.elapsedMs} ms`);
      add("cm-resp-meta", formatSize(s.size) + (s.truncated ? " (cut off)" : ""));
      if (s.stale) add("cm-resp-meta", `from ${new Date(s.ts).toLocaleString()}`);
    }
    wrap.appendChild(head);

    if (s && !this.running && !s.error) {
      if (s.image) {
        const img = document.createElement("img");
        img.src = s.image;
        img.className = "cm-resp-img";
        wrap.appendChild(img);
      } else if (s.body) {
        const text = prettyBody(s);
        const lines = text.slice(0, PREVIEW_CHARS).split("\n");
        const pre = document.createElement("pre");
        pre.className = "cm-resp-body";
        pre.textContent = lines.slice(0, PREVIEW_LINES).join("\n");
        wrap.appendChild(pre);
        const total = text.split("\n").length;
        if (total > PREVIEW_LINES || text.length > PREVIEW_CHARS) {
          const more = document.createElement("div");
          more.className = "cm-resp-more";
          more.textContent = `${total.toLocaleString()} lines. Click to open the full body.`;
          wrap.appendChild(more);
        }
      }
    }

    // A widget with no focusable children keeps typing fast in WebKit.
    wrap.addEventListener("mousedown", (e) => {
      e.preventDefault();
      if (!this.running) this.handlers.open(this.key);
    });
    return wrap;
  }

  ignoreEvent() {
    return true;
  }
}

function decorate(state: EditorState, handlers: HttpBlockHandlers): DecorationSet {
  const { keys, shown, running } = state.field(overlayField);
  const ranges: Range<Decoration>[] = [];
  for (const b of state.field(blocksField)) {
    for (let n = b.fenceLine; n <= b.lastLine; n++) {
      const line = state.doc.line(n + 1);
      let cls = "cm-http";
      if (n === b.fenceLine) cls += " cm-http-open";
      if (n === b.lastLine) cls += " cm-http-close";
      ranges.push(Decoration.line({ class: cls }).range(line.from));
    }
    const key = keys[b.index];
    const s = key ? shown.get(key) : undefined;
    const isRunning = key ? running.has(key) : false;
    if (s || isRunning) {
      const end = state.doc.line(b.lastLine + 1).to;
      ranges.push(
        Decoration.widget({ widget: new ResponseWidget(key, s, isRunning, handlers), block: true, side: 1 }).range(end),
      );
    }
  }
  return Decoration.set(ranges, true);
}

class RunMarker extends GutterMarker {
  constructor(readonly running: boolean) {
    super();
  }
  eq(other: RunMarker) {
    return other.running === this.running;
  }
  toDOM() {
    const el = document.createElement("div");
    el.className = this.running ? "cm-run-marker cm-run-marker-running" : "cm-run-marker";
    el.textContent = this.running ? "◌" : "▶";
    el.title = this.running ? "Running" : "Run (Ctrl+Enter)";
    return el;
  }
}

const idleMarker = new RunMarker(false);
const runningMarker = new RunMarker(true);

export function httpBlocks(handlers: HttpBlockHandlers): Extension {
  const decoField = StateField.define<DecorationSet>({
    create: (state) => decorate(state, handlers),
    update(value, tr) {
      const treeChanged = syntaxTree(tr.startState) !== syntaxTree(tr.state);
      const overlayChanged = tr.effects.some((e) => e.is(setOverlay));
      return tr.docChanged || treeChanged || overlayChanged ? decorate(tr.state, handlers) : value;
    },
    provide: (f) => EditorView.decorations.from(f),
  });

  const runGutter = gutter({
    class: "cm-run-gutter",
    lineMarker(view, line) {
      const b = fenceAt(view.state, line.from);
      if (!b) return null;
      const { keys, running } = view.state.field(overlayField);
      return keys[b.index] && running.has(keys[b.index]) ? runningMarker : idleMarker;
    },
    lineMarkerChange: (u) =>
      u.docChanged ||
      syntaxTree(u.startState) !== syntaxTree(u.state) ||
      u.transactions.some((t) => t.effects.some((e) => e.is(setOverlay))),
    initialSpacer: () => idleMarker,
    domEventHandlers: {
      mousedown(view, line) {
        const b = fenceAt(view.state, line.from);
        if (!b) return false;
        handlers.run(b.fenceLine);
        return true;
      },
    },
  });

  const runKeys = keymap.of([
    {
      key: "Mod-Enter",
      run(view) {
        const b = blockAt(view.state, view.state.selection.main.head);
        if (!b) return false;
        handlers.run(b.fenceLine);
        return true;
      },
    },
  ]);

  return [blocksField, overlayField, decoField, runGutter, runKeys];
}
