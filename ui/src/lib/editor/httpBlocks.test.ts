import { describe, expect, it } from "vitest";
import { EditorState } from "@codemirror/state";
import { ensureSyntaxTree } from "@codemirror/language";
import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
import { findBlocks } from "./httpBlocks";

function blocks(doc: string) {
  const state = EditorState.create({ doc, extensions: [markdown({ base: markdownLanguage })] });
  ensureSyntaxTree(state, state.doc.length, 5000);
  return findBlocks(state);
}

describe("findBlocks", () => {
  it("finds only http fences, with 0-based fence lines like the backend", () => {
    const doc = [
      "# Page", // 0
      "", // 1
      "```http", // 2
      "GET http://h/a", // 3
      "```", // 4
      "", // 5
      "```json", // 6
      "{}", // 7
      "```", // 8
      "~~~HTTP title", // 9
      "POST http://h/b", // 10
      "~~~", // 11
    ].join("\n");
    const b = blocks(doc);
    expect(b.map((x) => [x.index, x.fenceLine, x.lastLine])).toEqual([
      [0, 2, 4],
      [1, 9, 11],
    ]);
  });

  it("treats an unclosed fence as running to the end", () => {
    const b = blocks("text\n```http\nGET http://h\n");
    expect(b).toHaveLength(1);
    expect(b[0].fenceLine).toBe(1);
  });
});
