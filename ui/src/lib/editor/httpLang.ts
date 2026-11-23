// Highlighting for the request syntax inside ```http blocks: directives,
// the request line, headers, then a body.
import { StreamLanguage, type StringStream } from "@codemirror/language";

interface State {
  phase: "pre" | "headers" | "body";
}

const METHODS = /^(GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS|TRACE|CONNECT|GRAPHQL|WEBSOCKET)\b/i;

function token(stream: StringStream, state: State): string | null {
  if (stream.sol() && state.phase !== "body") {
    if (stream.match(/^\s*$/)) {
      if (state.phase === "headers") state.phase = "body";
      stream.skipToEnd();
      return null;
    }
  }
  if (stream.match("{{")) {
    stream.skipTo("}}") ? stream.match("}}") : stream.skipToEnd();
    return "variableName";
  }

  if (state.phase === "body") {
    if (stream.match(/^"(?:[^"\\]|\\.)*"(?=\s*:)/)) return "propertyName";
    if (stream.match(/^"(?:[^"\\]|\\.)*"/)) return "string";
    if (stream.match(/^-?\d+(\.\d+)?([eE][+-]?\d+)?/)) return "number";
    if (stream.match(/^(true|false|null)\b/)) return "atom";
    if (stream.match(/^(query|mutation|subscription|fragment)\b/)) return "keyword";
    stream.next();
    while (!stream.eol() && !/["{\d\-tfnqmsf]/.test(stream.peek() ?? "")) stream.next();
    return null;
  }

  if (stream.sol() && stream.match(/^\s*(#|\/\/)/)) {
    if (stream.match(/^\s*@[\w-]+/)) {
      return "meta";
    }
    stream.skipToEnd();
    return "comment";
  }
  // Rest of a directive line.
  if (state.phase === "pre" && !stream.sol() && stream.string.trimStart().match(/^(#|\/\/)\s*@/)) {
    if (stream.match(/^[^{]+/)) return "string";
  }

  if (state.phase === "pre") {
    if (stream.sol() && stream.match(METHODS)) return "keyword";
    if (stream.match(/^\s*[?&][^\s{]*/)) return "url";
    if (stream.match(/^\s+HTTP\/[\d.]+\s*$/)) return "comment";
    if (stream.match(/^[^\s{]+/)) {
      if (stream.eol()) state.phase = "headers";
      return "url";
    }
    stream.next();
    if (stream.eol()) state.phase = "headers";
    return null;
  }

  // headers
  if (stream.sol() && stream.match(/^\s*[?&][^\s{]*/)) return "url";
  if (stream.sol() && stream.match(/^[^:\s]+(?=:)/)) return "propertyName";
  if (stream.match(/^[^{]+/)) return "string";
  stream.next();
  return null;
}

/** The request line ends the preamble; detect it so the next line starts headers. */
function wrapped(stream: StringStream, state: State): string | null {
  const wasSol = stream.sol();
  const isRequestLine =
    wasSol && state.phase === "pre" && !/^\s*(#|\/\/)/.test(stream.string) && stream.string.trim() !== "";
  const style = token(stream, state);
  if (isRequestLine && stream.eol()) state.phase = "headers";
  return style;
}

export const httpRequest = StreamLanguage.define<State>({
  name: "http",
  startState: () => ({ phase: "pre" }),
  copyState: (s) => ({ ...s }),
  token: wrapped,
  blankLine: (state) => {
    if (state.phase === "headers") state.phase = "body";
  },
  languageData: { commentTokens: { line: "#" } },
});
