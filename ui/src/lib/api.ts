// Typed wrappers around the Tauri commands in src-tauri/src/lib.rs.
import { invoke } from "@tauri-apps/api/core";

export interface TreeNode {
  id: string;
  title: string;
  has_page: boolean;
  children: TreeNode[];
}

export interface WorkspaceInfo {
  root: string;
  name: string;
  envs: string[];
  default_env: string | null;
  tree: TreeNode[];
}

export interface BlockInfo {
  line: number;
  key: string;
  name: string | null;
  method: string | null;
  url: string | null;
  error: string | null;
}

type Pairs = [string, string][];

export interface RequestView {
  method: string;
  url: string;
  headers: Pairs;
  body: string | null;
}

export interface ResponseView {
  status: number;
  reason: string;
  version: string;
  url: string;
  headers: Pairs;
  body: string;
  image: string | null;
  size: number;
  truncated: boolean;
  elapsed_ms: number;
  content_type: string | null;
}

export interface RunResult {
  line: number;
  key: string;
  history_id: number | null;
  request: RequestView | null;
  response: ResponseView | null;
  error: string | null;
}

export interface HistoryEntry {
  id: number;
  ts: number;
  page: string;
  block_key: string;
  env: string | null;
  method: string;
  url: string;
  request_headers: Pairs;
  request_body: string | null;
  status: number | null;
  reason: string | null;
  response_headers: Pairs;
  response_body: string | null;
  body_size: number | null;
  body_truncated: boolean;
  elapsed_ms: number | null;
  error: string | null;
}

/** What the editor and panel show for a block, from a live run or from history. */
export interface Shown {
  key: string;
  historyId: number | null;
  ts: number;
  env: string | null;
  request: RequestView | null;
  status: number | null;
  reason: string;
  headers: Pairs;
  body: string;
  image: string | null;
  size: number | null;
  truncated: boolean;
  elapsedMs: number | null;
  contentType: string | null;
  error: string | null;
  /** Loaded from history rather than run in this session. */
  stale: boolean;
}

export function fromRun(r: RunResult, env: string | null): Shown {
  const resp = r.response;
  return {
    key: r.key,
    historyId: r.history_id,
    ts: Date.now(),
    env,
    request: r.request,
    status: resp?.status ?? null,
    reason: resp?.reason ?? "",
    headers: resp?.headers ?? [],
    body: resp?.body ?? "",
    image: resp?.image ?? null,
    size: resp?.size ?? null,
    truncated: resp?.truncated ?? false,
    elapsedMs: resp?.elapsed_ms ?? null,
    contentType: resp?.content_type ?? null,
    error: r.error,
    stale: false,
  };
}

export function fromHistory(e: HistoryEntry): Shown {
  const ct = e.response_headers.find(([k]) => k.toLowerCase() === "content-type")?.[1] ?? null;
  return {
    key: e.block_key,
    historyId: e.id,
    ts: e.ts,
    env: e.env,
    request: { method: e.method, url: e.url, headers: e.request_headers, body: e.request_body },
    status: e.status,
    reason: e.reason ?? "",
    headers: e.response_headers,
    body: e.response_body ?? "",
    image: null,
    size: e.body_size,
    truncated: e.body_truncated,
    elapsedMs: e.elapsed_ms,
    contentType: ct,
    error: e.error,
    stale: true,
  };
}

export const api = {
  openWorkspace: (path: string) => invoke<WorkspaceInfo>("open_workspace", { path }),
  initWorkspace: (path: string) => invoke<void>("init_workspace", { path }),
  workspaceInfo: () => invoke<WorkspaceInfo>("workspace_info"),
  readPage: (id: string) => invoke<string>("read_page", { id }),
  writePage: (id: string, text: string) => invoke<void>("write_page", { id, text }),
  readText: (rel: string) => invoke<string>("read_text", { rel }),
  writeText: (rel: string, text: string) => invoke<void>("write_text", { rel, text }),
  createEnv: (name: string) => invoke<string>("create_env", { name }),
  createPage: (parent: string | null, title: string) => invoke<string>("create_page", { parent, title }),
  renamePage: (id: string, title: string) => invoke<string>("rename_page", { id, title }),
  movePage: (id: string, parent: string | null, before: string | null) =>
    invoke<string>("move_page", { id, parent, before }),
  trashPage: (id: string) => invoke<void>("trash_page", { id }),
  pageBlocks: (text: string) => invoke<BlockInfo[]>("page_blocks", { text }),
  runBlock: (runId: number, id: string, text: string, line: number, env: string | null) =>
    invoke<RunResult[]>("run_block", { runId, id, text, line, env }),
  cancelRun: (runId: number) => invoke<void>("cancel_run", { runId }),
  resetPageState: (id: string) => invoke<void>("reset_page_state", { id }),
  latestRuns: (id: string) => invoke<HistoryEntry[]>("latest_runs", { id }),
  historyList: (id: string, key: string | null, limit = 50) =>
    invoke<HistoryEntry[]>("history_list", { id, key, limit }),
  blockAsCurl: (id: string, text: string, line: number, env: string | null) =>
    invoke<string>("block_as_curl", { id, text, line, env }),
};

export function errorText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
}

export function formatSize(n: number | null): string {
  if (n == null) return "";
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

export function isJson(ct: string | null, body: string): boolean {
  if (ct) {
    const mime = ct.split(";")[0].trim();
    if (mime === "application/json" || mime.endsWith("+json")) return true;
  }
  const t = body.trimStart();
  return (t.startsWith("{") || t.startsWith("[")) && t.length < 5_000_000;
}

export function prettyBody(s: Shown): string {
  if (isJson(s.contentType, s.body)) {
    try {
      return JSON.stringify(JSON.parse(s.body), null, 2);
    } catch {
      /* not valid JSON after all */
    }
  }
  return s.body;
}

export function statusClass(s: Shown | undefined): "ok" | "warn" | "err" | "none" {
  if (!s) return "none";
  if (s.error || s.status == null) return "err";
  if (s.status >= 400) return "err";
  if (s.status >= 300) return "warn";
  return "ok";
}
