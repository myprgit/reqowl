<script lang="ts">
  import { onMount } from "svelte";
  import { SvelteMap, SvelteSet } from "svelte/reactivity";
  import { listen } from "@tauri-apps/api/event";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import {
    api,
    errorText,
    fromHistory,
    fromRun,
    type BlockInfo,
    type HistoryEntry,
    type RequestForm,
    type Shown,
    type Socket,
    type SocketMessage,
    type TreeNode,
    type WorkspaceInfo,
  } from "./lib/api";
  import Editor from "./components/Editor.svelte";
  import Sidebar from "./components/Sidebar.svelte";
  import Panel, { type Tab } from "./components/Panel.svelte";
  import Welcome from "./components/Welcome.svelte";
  import QuickOpen from "./components/QuickOpen.svelte";
  import SettingsDialog from "./components/Settings.svelte";
  import {
    BUILTIN_THEMES,
    applyAppearance,
    customThemes,
    loadSettings,
    resolveTheme,
    saveSettings,
    type Settings,
    type Theme,
  } from "./lib/themes";

  type Doc = { kind: "page"; id: string } | { kind: "text"; rel: string };

  let ws = $state<WorkspaceInfo | null>(null);
  let doc = $state<Doc | null>(null);
  let text = $state("");
  let savedText = $state("");
  let env = $state<string | null>(null);
  let settings = $state<Settings>(loadSettings());
  let customThemeList = $state<Theme[]>([]);
  let themeErrors = $state<string[]>([]);
  let themesDir = $state<string | null>(null);
  let settingsOpen = $state(false);
  let systemDark = $state(window.matchMedia("(prefers-color-scheme: dark)").matches);
  const allThemes = $derived([...BUILTIN_THEMES, ...customThemeList]);
  let blocks = $state<BlockInfo[]>([]);
  let varNames: string[] = [];
  const shown = new SvelteMap<string, Shown>();
  const running = new SvelteSet<string>();
  /** Folded responses on the open page, remembered per page. */
  const collapsed = new SvelteSet<string>();
  let sockets = $state<Socket[]>([]);
  /** The block the panel shows: its key (for responses) and fence line (for editing). */
  let panel = $state<{ key: string; line: number } | null>(null);
  let panelTab = $state<Tab>("body");
  let panelForm = $state<RequestForm | null>(null);
  let formError = $state<string | null>(null);
  let formFocused = $state(false);
  let formTimer: ReturnType<typeof setTimeout> | undefined;
  let formWrite: Promise<void> | null = null;
  let panelHistory = $state<HistoryEntry[]>([]);
  let panelHeight = $state(300);
  let toast = $state<{ text: string; error: boolean } | null>(null);
  let diskChanged = $state(false);
  let quickOpen = $state(false);
  let welcomeError = $state<string | null>(null);
  let recent = $state<string[]>(JSON.parse(localStorage.getItem("reqowl.recent") ?? "[]"));
  let sidebarWidth = $state(Number(localStorage.getItem("reqowl.sidebar") ?? 250));

  let editor: Editor | undefined = $state();
  let saveTimer: ReturnType<typeof setTimeout> | undefined;
  let blocksTimer: ReturnType<typeof setTimeout> | undefined;
  let runSeq = 0;
  let activeRun: number | null = null;
  let stopPageRun = false;

  const dirty = $derived(text !== savedText);
  const collapseKey = () => `reqowl.collapsed.${ws?.root}::${pageId}`;
  const allFolded = $derived(shown.size > 0 && [...shown.keys()].every((k) => collapsed.has(k)));

  function saveCollapsed() {
    localStorage.setItem(collapseKey(), JSON.stringify([...collapsed]));
  }

  function toggleFold(key: string) {
    if (collapsed.has(key)) collapsed.delete(key);
    else collapsed.add(key);
    saveCollapsed();
  }

  function foldAll(fold: boolean) {
    collapsed.clear();
    if (fold) for (const k of shown.keys()) collapsed.add(k);
    saveCollapsed();
  }
  const pageId = $derived(doc?.kind === "page" ? doc.id : null);
  const panelShown = $derived(panel ? shown.get(panel.key) : undefined);
  const panelSocket = $derived(panel ? sockets.findLast((s) => s.key === panel!.key) : undefined);
  const title = $derived(
    doc?.kind === "page" ? doc.id.split("/").join(" / ") : doc?.kind === "text" ? doc.rel : "",
  );

  function notify(text: string, error = false) {
    if (error) api.logUi("error", text);
    toast = { text, error };
    setTimeout(() => {
      if (toast?.text === text) toast = null;
    }, error ? 6000 : 2600);
  }

  // ---- theme ------------------------------------------------------------

  // Colours and fonts follow the settings, the chosen theme and the system mode.
  $effect(() => {
    applyAppearance(settings, resolveTheme(settings, allThemes, systemDark));
    // Wait for the new styles (and any newly used font) before measuring.
    requestAnimationFrame(() => editor?.remeasure());
    document.fonts?.ready.then(() => editor?.remeasure());
  });

  function changeSettings(next: Settings) {
    settings = next;
    saveSettings(next);
  }

  async function loadCustomThemes(announce = false) {
    try {
      const r = customThemes(await api.listCustomThemes());
      customThemeList = r.themes;
      themeErrors = r.errors;
      if (announce) notify(`${r.themes.length} custom theme${r.themes.length === 1 ? "" : "s"} loaded`);
    } catch (e) {
      themeErrors = [errorText(e)];
    }
  }

  async function openThemesFolder() {
    try {
      themesDir = await api.openThemesFolder();
    } catch (e) {
      notify(errorText(e), true);
    }
  }

  // ---- workspace ----------------------------------------------------------

  async function openWorkspace(path: string) {
    welcomeError = null;
    try {
      await flushSave();
      const info = await api.openWorkspace(path);
      ws = info;
      recent = [info.root, ...recent.filter((r) => r !== info.root)].slice(0, 8);
      localStorage.setItem("reqowl.recent", JSON.stringify(recent));
      localStorage.setItem("reqowl.last", info.root);
      const savedEnv = localStorage.getItem(`reqowl.env.${info.root}`);
      env = savedEnv && info.envs.includes(savedEnv) ? savedEnv : (info.default_env ?? info.envs[0] ?? null);
      shown.clear();
      sockets = [];
      panel = null;
      const last = localStorage.getItem(`reqowl.page.${info.root}`);
      const first = last && findNode(info.tree, last) ? last : firstPage(info.tree);
      if (first) await openPage(first);
      else doc = null;
    } catch (e) {
      const msg = errorText(e);
      api.logUi("error", `opening ${path}: ${msg} ${e instanceof Error ? e.stack : ""}`);
      if (msg.startsWith("not_workspace:")) {
        const dir = msg.slice("not_workspace:".length);
        welcomeError = `${dir} isn't a notebook yet. Use "Create a notebook" to set it up.`;
      } else {
        welcomeError = msg;
      }
      ws = null;
    }
  }

  /** Back to the start screen; the next launch opens it too. */
  async function closeWorkspace() {
    await flushSave();
    doc = null;
    text = savedText = "";
    panel = null;
    shown.clear();
    sockets = [];
    ws = null;
    localStorage.removeItem("reqowl.last");
  }

  async function refreshTree() {
    if (!ws) return;
    try {
      ws = await api.workspaceInfo();
    } catch (e) {
      notify(errorText(e), true);
    }
  }

  function findNode(nodes: TreeNode[], id: string): TreeNode | undefined {
    for (const n of nodes) {
      if (n.id === id) return n;
      const c = findNode(n.children, id);
      if (c) return c;
    }
  }

  function firstPage(nodes: TreeNode[]): string | null {
    for (const n of nodes) {
      if (n.has_page) return n.id;
      const c = firstPage(n.children);
      if (c) return c;
    }
    return nodes[0]?.id ?? null;
  }

  function allPages(nodes: TreeNode[], out: TreeNode[] = []): TreeNode[] {
    for (const n of nodes) {
      out.push(n);
      allPages(n.children, out);
    }
    return out;
  }

  async function chooseFolder(title: string): Promise<string | null> {
    const picked = await openDialog({ directory: true, multiple: false, title });
    return typeof picked === "string" ? picked : null;
  }

  async function pickAndOpen() {
    const dir = await chooseFolder("Open a notebook folder");
    if (dir) await openWorkspace(dir);
  }

  async function pickAndCreate() {
    const dir = await chooseFolder("Choose a folder for the new notebook");
    if (!dir) return;
    try {
      await api.initWorkspace(dir);
      await openWorkspace(dir);
    } catch (e) {
      welcomeError = errorText(e);
    }
  }

  async function pickAndImport() {
    const file = await openDialog({
      multiple: false,
      title: "Import a Postman or Insomnia export",
      filters: [{ name: "Exports", extensions: ["json", "yaml", "yml", "txt", "sh"] }],
    });
    if (typeof file !== "string") return;
    const dir = await chooseFolder("Choose an empty folder for the imported notebook");
    if (!dir) return;
    try {
      const root = await api.importFile(file, dir);
      await openWorkspace(root);
      notify("Imported");
    } catch (e) {
      const msg = errorText(e);
      if (ws) notify(msg, true);
      else welcomeError = msg;
    }
  }

  // ---- documents -------------------------------------------------------------

  async function openPage(id: string) {
    await flushSave();
    try {
      const t = await api.readPage(id);
      doc = { kind: "page", id };
      text = savedText = t;
      diskChanged = false;
      shown.clear();
      panel = null;
      editor?.load(t, "markdown");
      localStorage.setItem(`reqowl.page.${ws?.root}`, id);
      collapsed.clear();
      for (const k of JSON.parse(localStorage.getItem(collapseKey()) ?? "[]") as string[]) collapsed.add(k);
      await refreshBlocks(t);
      const latest = await api.latestRuns(id);
      for (const e of latest) shown.set(e.block_key, fromHistory(e));
    } catch (e) {
      notify(errorText(e), true);
    }
  }

  async function openText(rel: string) {
    await flushSave();
    try {
      const t = await api.readText(rel);
      doc = { kind: "text", rel };
      text = savedText = t;
      diskChanged = false;
      blocks = [];
      shown.clear();
      panel = null;
      editor?.load(t, "toml");
    } catch (e) {
      notify(errorText(e), true);
    }
  }

  function onChange(t: string) {
    text = t;
    clearTimeout(saveTimer);
    saveTimer = setTimeout(save, 450);
    if (doc?.kind === "page") {
      clearTimeout(blocksTimer);
      blocksTimer = setTimeout(() => refreshBlocks(t), 250);
    }
  }

  async function refreshBlocks(t: string) {
    const b = await api.pageBlocks(t);
    if (t !== text) return; // a newer edit is on its way
    blocks = b;
    syncPanel();
    try {
      varNames = await api.variableNames(pageId, t, env);
    } catch {
      /* completion is best effort */
    }
  }

  async function save() {
    clearTimeout(saveTimer);
    if (!doc || !dirty) return;
    const t = text;
    try {
      if (doc.kind === "page") await api.writePage(doc.id, t);
      else {
        await api.writeText(doc.rel, t);
        if (doc.rel.startsWith("env/") || doc.rel === "reqowl.toml") refreshTree();
      }
      savedText = t;
    } catch (e) {
      notify(`Couldn't save: ${errorText(e)}`, true);
    }
  }

  async function flushSave() {
    if (dirty) await save();
  }

  async function onDiskChange(paths: string[]) {
    if (paths.some((p) => p === "*" || p.endsWith(".md") || p.endsWith(".order") || p.startsWith("env/")))
      await refreshTree();
    if (!doc) return;
    const mine = doc.kind === "page" ? `${doc.id}.md` : doc.rel;
    if (!paths.includes(mine) && !paths.includes("*")) return;
    const disk = doc.kind === "page" ? await api.readPage(doc.id) : await api.readText(doc.rel);
    if (disk === savedText || disk === text) return;
    if (!dirty) {
      text = savedText = disk;
      editor?.replaceText(disk);
      if (doc.kind === "page") refreshBlocks(disk);
    } else {
      diskChanged = true;
    }
  }

  async function reloadFromDisk() {
    if (!doc) return;
    const disk = doc.kind === "page" ? await api.readPage(doc.id) : await api.readText(doc.rel);
    text = savedText = disk;
    editor?.replaceText(disk);
    diskChanged = false;
    if (doc.kind === "page") refreshBlocks(disk);
  }

  async function keepMine() {
    diskChanged = false;
    savedText = "";
    await save();
  }

  // ---- tree operations ---------------------------------------------------

  async function createPage(parent: string | null) {
    try {
      const id = await api.createPage(parent, "Untitled");
      await refreshTree();
      await openPage(id);
      return id;
    } catch (e) {
      notify(errorText(e), true);
    }
  }

  function remapOpen(oldId: string, newId: string) {
    if (doc?.kind !== "page") return;
    if (doc.id === oldId) doc = { kind: "page", id: newId };
    else if (doc.id.startsWith(oldId + "/")) doc = { kind: "page", id: newId + doc.id.slice(oldId.length) };
    else return;
    localStorage.setItem(`reqowl.page.${ws?.root}`, doc.id);
  }

  async function renamePage(id: string, title: string) {
    try {
      await flushSave();
      const newId = await api.renamePage(id, title);
      remapOpen(id, newId);
      await refreshTree();
    } catch (e) {
      notify(errorText(e), true);
    }
  }

  async function movePage(id: string, parent: string | null, before: string | null) {
    try {
      await flushSave();
      const newId = await api.movePage(id, parent, before);
      remapOpen(id, newId);
      await refreshTree();
    } catch (e) {
      notify(errorText(e), true);
    }
  }

  async function trashPage(id: string) {
    try {
      const openInside = doc?.kind === "page" && (doc.id === id || doc.id.startsWith(id + "/"));
      if (openInside) {
        clearTimeout(saveTimer);
        doc = null;
        text = savedText = "";
      }
      await api.trashPage(id);
      await refreshTree();
      notify("Moved to .reqowl/trash");
      if (openInside && ws) {
        const next = firstPage(ws.tree);
        if (next) await openPage(next);
      }
    } catch (e) {
      notify(errorText(e), true);
    }
  }

  // window.prompt is blocked in some webviews, so names are asked inline.
  let promptState = $state<{ label: string; value: string; resolve: (v: string | null) => void } | null>(null);
  function askName(label: string, value: string): Promise<string | null> {
    return new Promise((resolve) => (promptState = { label, value, resolve }));
  }
  async function createEnvAsk() {
    const name = await askName("New environment", "staging");
    if (!name) return;
    try {
      const rel = await api.createEnv(name);
      await refreshTree();
      await openText(rel);
    } catch (e) {
      notify(errorText(e), true);
    }
  }

  function changeEnv(next: string | null) {
    env = next;
    if (ws) localStorage.setItem(`reqowl.env.${ws.root}`, next ?? "");
    // Responses captured in another environment shouldn't feed this one.
    if (pageId) api.resetPageState(pageId);
    refreshBlocks(text);
  }

  // ---- running ---------------------------------------------------------------

  function keyForLine(line: number): string | undefined {
    return blocks.find((b) => b.line === line)?.key;
  }

  async function runAt(line: number, openPanel = false) {
    if (doc?.kind !== "page") return;
    const id = doc.id;
    await flushSave();
    const t = text;
    const info = (await api.pageBlocks(t)).find((b) => b.line === line);
    if (!info) return;
    if (info.kind === "websocket") return connectSocket(info, t);
    const runId = ++runSeq;
    activeRun = runId;
    running.add(info.key);
    if (openPanel) panel = { key: info.key, line: info.line };
    try {
      const results = await api.runBlock(runId, id, t, line, env);
      if (pageId !== id) return;
      for (const r of results) shown.set(r.key, fromRun(r, env));
      if (panel?.key === info.key) loadPanelHistory();
      const failed = results.find((r) => r.error);
      if (failed && failed.key !== info.key && !results.some((r) => r.key === info.key)) {
        shown.set(info.key, { ...fromRun(failed, env), key: info.key, error: `A block it depends on failed: ${failed.error}` });
      }
    } catch (e) {
      if (pageId === id)
        shown.set(info.key, { ...fromRun({ line, key: info.key, history_id: null, request: null, response: null, error: errorText(e) }, env) });
    } finally {
      running.delete(info.key);
      if (activeRun === runId) activeRun = null;
    }
  }

  async function runPage() {
    if (doc?.kind !== "page") return;
    stopPageRun = false;
    await flushSave();
    const lines = (await api.pageBlocks(text)).filter((b) => b.kind !== "websocket").map((b) => b.line);
    for (const line of lines) {
      if (stopPageRun) break;
      await runAt(line);
      const key = keyForLine(line);
      const s = key ? shown.get(key) : undefined;
      if (s?.error === "Cancelled") break;
    }
  }

  function cancel() {
    stopPageRun = true;
    if (activeRun != null) api.cancelRun(activeRun);
  }

  async function connectSocket(info: BlockInfo, t: string) {
    if (!pageId) return;
    const conn = ++runSeq;
    const sock: Socket = { conn, key: info.key, line: info.line, url: info.url ?? "", open: false, log: [] };
    sockets = [...sockets.filter((s) => !(s.key === info.key && !s.open)), sock];
    panel = { key: info.key, line: info.line };
    if (panelTab === "edit") panelTab = "messages";
    running.add(info.key);
    try {
      await api.wsConnect(conn, pageId, t, info.line, env);
    } catch (e) {
      sockets = sockets.map((s) =>
        s.conn === conn
          ? { ...s, log: [...s.log, { conn, ts: Date.now(), event: { kind: "error", message: errorText(e) } }] }
          : s,
      );
    } finally {
      running.delete(info.key);
    }
  }

  function onSocketMessage(m: SocketMessage) {
    sockets = sockets.map((s) => {
      if (s.conn !== m.conn) return s;
      const open = m.event.kind === "opened" ? true : m.event.kind === "closed" ? false : s.open;
      return { ...s, open, log: [...s.log, m].slice(-2000) };
    });
  }

  // ---- panel -------------------------------------------------------------------

  async function openPanel(key: string) {
    const line = blocks.find((b) => b.key === key)?.line ?? panel?.line ?? 0;
    panel = { key, line };
    if (panelTab === "edit") panelTab = "body";
    await loadPanelHistory();
  }

  async function openEdit(line: number) {
    const key = blocks.find((b) => b.line === line)?.key ?? `#line${line}`;
    panel = { key, line };
    panelTab = "edit";
    await loadForm();
    loadPanelHistory();
  }

  /** Keeps the panel on its block when lines shift or a form edit changes the block's key. */
  function syncPanel() {
    if (!panel) return;
    const b = blocks.find((b) => b.key === panel!.key) ?? blocks.find((b) => b.line === panel!.line);
    if (!b) return;
    if (b.key !== panel.key || b.line !== panel.line) panel = { key: b.key, line: b.line };
    if (panelTab === "edit" && !formFocused && !formWrite) loadForm();
  }

  async function loadForm() {
    if (!panel || doc?.kind !== "page") return;
    try {
      panelForm = await api.blockForm(text, panel.line);
      formError = null;
    } catch (e) {
      panelForm = null;
      formError = errorText(e);
    }
  }

  function onFormChange(f: RequestForm) {
    panelForm = f;
    clearTimeout(formTimer);
    formTimer = setTimeout(() => {
      formWrite = writeForm(f).finally(() => (formWrite = null));
    }, 150);
  }

  async function writeForm(f: RequestForm) {
    if (!panel) return;
    try {
      const content = await api.formToBlock(f);
      formError = null;
      editor?.replaceBlockContent(panel.line, content);
    } catch (e) {
      formError = errorText(e);
    }
  }

  async function sendFromForm() {
    if (!panel) return;
    clearTimeout(formTimer);
    if (panelForm) await writeForm(panelForm);
    if (formWrite) await formWrite;
    if (formError) return;
    await refreshBlocks(text);
    await runAt(panel.line);
  }

  async function newRequest() {
    if (doc?.kind !== "page") return;
    const line = editor?.insertRequest();
    if (line == null) return;
    await refreshBlocks(text);
    await openEdit(line);
  }

  async function loadPanelHistory() {
    if (!panel || !pageId) return;
    try {
      panelHistory = await api.historyList(pageId, panel.key, 50);
    } catch {
      panelHistory = [];
    }
  }

  function showHistory(e: HistoryEntry) {
    shown.set(e.block_key, fromHistory(e));
  }

  async function copyCurl() {
    if (!panel || !pageId) return;
    const b = blocks.find((b) => b.key === panel!.key);
    if (!b) return notify("That block isn't on the page any more", true);
    try {
      const curl = await api.blockAsCurl(pageId, text, b.line, env);
      await navigator.clipboard.writeText(curl);
      notify("Copied as curl");
    } catch (e) {
      notify(errorText(e), true);
    }
  }

  /** Writes the response into the page as a ```response block, from the masked history copy. */
  async function pin() {
    if (!panel || !pageId) return;
    const s = shown.get(panel.key);
    const b = blocks.find((b) => b.key === panel!.key);
    if (!s || !b) return;
    let entry = s.historyId != null ? panelHistory.find((h) => h.id === s.historyId) : undefined;
    if (!entry && s.historyId != null) {
      entry = (await api.historyList(pageId, panel.key, 50)).find((h) => h.id === s.historyId);
    }
    if (!entry) return notify("Only responses saved in history can be pinned", true);
    const body = (() => {
      const raw = entry.response_body ?? "";
      try {
        return JSON.stringify(JSON.parse(raw), null, 2);
      } catch {
        return raw;
      }
    })();
    const lines = body.split("\n");
    const clipped = lines.length > 200 ? [...lines.slice(0, 200), `… ${lines.length - 200} more lines`] : lines;
    const status = entry.error ? `error: ${entry.error}` : `${entry.status} ${entry.reason ?? ""}`.trim();
    const fence = body.includes("```") ? "~~~~" : "```";
    editor?.insertAfterBlock(
      b.line,
      `\n${fence}response\n${status} · ${new Date(entry.ts).toLocaleString()}\n\n${clipped.join("\n")}\n${fence}\n`,
    );
    notify("Pinned to the page. Secrets from .local files are hidden.");
  }

  // ---- pasting curl -------------------------------------------------------------

  async function pasteCurl(t: string): Promise<string | null> {
    const block = await api.curlToBlock(t).catch(() => null);
    if (!block) return null;
    notify("Pasted curl as an http block");
    return "```http\n" + block.trimEnd() + "\n```\n";
  }

  // ---- keys and layout ------------------------------------------------------------

  function onKey(e: KeyboardEvent) {
    const mod = e.ctrlKey || e.metaKey;
    if (e.key === "Escape" && (activeRun != null || running.size)) {
      cancel();
      return;
    }
    if (e.key === "Escape" && quickOpen) quickOpen = false;
    if (!mod) return;
    const k = e.key.toLowerCase();
    if (k === ",") {
      e.preventDefault();
      settingsOpen = true;
    } else if (k === "s") {
      e.preventDefault();
      save();
    } else if (k === "p" || k === "k") {
      if (ws) {
        e.preventDefault();
        quickOpen = true;
      }
    } else if (k === "n" && ws) {
      e.preventDefault();
      createPage(null);
    } else if (k === "enter" && e.shiftKey) {
      e.preventDefault();
      runPage();
    } else if (k === "j") {
      e.preventDefault();
      if (panel) panel = null;
      else if (blocks[0]) openPanel(blocks[0].key);
    } else if (k === "e" && doc?.kind === "page" && !e.defaultPrevented) {
      // Ctrl+E inside a block is handled by the editor; elsewhere open the panel's block or the first one.
      const line = panel?.line ?? blocks[0]?.line;
      if (line != null) {
        e.preventDefault();
        openEdit(line);
      }
    }
  }

  function dragSplit(e: PointerEvent, kind: "sidebar" | "panel") {
    e.preventDefault();
    const start = kind === "sidebar" ? e.clientX : e.clientY;
    const startSize = kind === "sidebar" ? sidebarWidth : panelHeight;
    const move = (ev: PointerEvent) => {
      if (kind === "sidebar") sidebarWidth = Math.min(480, Math.max(170, startSize + ev.clientX - start));
      else panelHeight = Math.min(window.innerHeight - 160, Math.max(120, startSize - (ev.clientY - start)));
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      localStorage.setItem("reqowl.sidebar", String(sidebarWidth));
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }

  // Push responses and running state into the editor whenever they change.
  $effect(() => {
    const keys = blocks.map((b) => b.key);
    const map = new Map(shown);
    const run = new Set(running);
    const folded = new Set(collapsed);
    editor?.setResponses({ keys, shown: map, running: run, collapsed: folded });
  });

  onMount(() => {
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const onScheme = () => (systemDark = media.matches);
    media.addEventListener("change", onScheme);
    loadCustomThemes();
    api
      .startupPath()
      .catch(() => null)
      .then((arg) => {
        const last = arg ?? localStorage.getItem("reqowl.last");
        if (last) openWorkspace(last);
      });
    const unlisten = [
      listen<string[]>("fs-changed", (e) => onDiskChange(e.payload)),
      listen<SocketMessage>("ws-event", (e) => onSocketMessage(e.payload)),
    ];
    const beforeUnload = () => {
      if (dirty) save();
    };
    window.addEventListener("beforeunload", beforeUnload);
    return () => {
      unlisten.forEach((u) => u.then((f) => f()));
      window.removeEventListener("beforeunload", beforeUnload);
    };
  });
</script>

<svelte:window onkeydown={onKey} />

{#if !ws}
  <Welcome {recent} error={welcomeError} onOpen={pickAndOpen} onCreate={pickAndCreate} onImport={pickAndImport} onOpenRecent={openWorkspace} />
{:else}
  <svelte:boundary onerror={(e) => api.logUi("error", `render: ${e instanceof Error ? e.stack : e}`)}>
  <div class="app" style:--sidebar-w="{sidebarWidth}px">
    <Sidebar
      {ws}
      current={doc}
      onSelectPage={openPage}
      onSelectText={openText}
      onCreate={createPage}
      onRename={renamePage}
      onMove={movePage}
      onTrash={trashPage}
      onCreateEnv={createEnvAsk}
      onImport={pickAndImport}
      {recent}
      onOpenNotebook={pickAndOpen}
      onCreateNotebook={pickAndCreate}
      onOpenRecent={openWorkspace}
      onCloseNotebook={closeWorkspace}
    />
    <div class="split-x" role="separator" aria-orientation="vertical" onpointerdown={(e) => dragSplit(e, "sidebar")}></div>
    <main>
      <header class="bar">
        <div class="crumbs" title={title}>
          {title}{#if dirty}<span class="dot" title="Unsaved changes">●</span>{/if}
        </div>
        <div class="tools">
          {#if doc?.kind === "page"}
            <button class="quiet-btn" onclick={newRequest} title="Add a request block below the cursor">+ Request</button>
          {/if}
          {#if doc?.kind === "page" && shown.size}
            <button class="quiet-btn" onclick={() => foldAll(!allFolded)} title="Fold or unfold every response on this page">
              {allFolded ? "Show responses" : "Hide responses"}
            </button>
          {/if}
          {#if doc?.kind === "page" && blocks.length}
            {#if running.size}
              <button class="quiet-btn" onclick={cancel} title="Esc">Stop</button>
            {:else}
              <button class="quiet-btn" onclick={runPage} title="Ctrl+Shift+Enter">Run page</button>
            {/if}
          {/if}
          <label class="env" title="Environment: variables from env/<name>.toml">
            <span>env</span>
            <select value={env ?? ""} onchange={(e) => changeEnv((e.currentTarget as HTMLSelectElement).value || null)}>
              <option value="">none</option>
              {#each ws.envs as name (name)}
                <option value={name}>{name}</option>
              {/each}
            </select>
          </label>
          <button class="quiet-btn" onclick={() => (settingsOpen = true)} title="Theme, fonts and text size (Ctrl+,)">
            Settings
          </button>
        </div>
      </header>

      {#if diskChanged}
        <div class="banner">
          This file changed on disk while you were editing it.
          <button class="quiet-btn" onclick={reloadFromDisk}>Load the disk version</button>
          <button class="quiet-btn" onclick={keepMine}>Keep mine</button>
        </div>
      {/if}

      <section class="page" class:hidden={!doc}>
        <Editor
          bind:this={editor}
          {onChange}
          variableNames={() => varNames}
          transformPaste={pasteCurl}
          handlers={{
            run: (line) => runAt(line),
            open: (key) => openPanel(key),
            edit: (line) => openEdit(line),
            toggle: toggleFold,
          }}
        />
      </section>
      {#if !doc}
        <div class="empty">
          <img src="/owl.png" alt="" width="120" height="120" />
          <p>This notebook has no pages yet.</p>
          <button class="ink-btn" onclick={() => createPage(null)}>New page</button>
        </div>
      {/if}

      {#if panel}
        <div class="split-y" role="separator" aria-orientation="horizontal" onpointerdown={(e) => dragSplit(e, "panel")}></div>
        <div class="panel-wrap" style:height="{panelHeight}px">
          <Panel
            shown={panelShown}
            socket={panelSocket}
            history={panelHistory}
            blockLabel={blocks.find((b) => b.key === panel!.key)}
            onClose={() => (panel = null)}
            onPin={pin}
            onCurl={copyCurl}
            onPickHistory={showHistory}
            onSendMessage={(t) => panelSocket && api.wsSend(panelSocket.conn, t).catch((e) => notify(errorText(e), true))}
            bind:tab={panelTab}
            bind:formFocused
            form={panelForm}
            {formError}
            running={running.has(panel.key)}
            {onFormChange}
            onSend={sendFromForm}
            onDisconnect={() => panelSocket && api.wsClose(panelSocket.conn)}
            onReconnect={() => {
              const b = blocks.find((b) => b.key === panel!.key);
              if (b) runAt(b.line);
            }}
          />
        </div>
      {/if}
    </main>
  </div>
  {#snippet failed(error, reset)}
    <div class="crash">
      <p>Something broke while drawing this notebook: {error instanceof Error ? error.message : String(error)}</p>
      <button class="ink-btn" onclick={reset}>Try again</button>
    </div>
  {/snippet}
  </svelte:boundary>
{/if}

{#if quickOpen && ws}
  <QuickOpen
    pages={allPages(ws.tree)}
    onPick={(id) => {
      quickOpen = false;
      openPage(id);
    }}
    onClose={() => (quickOpen = false)}
  />
{/if}

{#if promptState}
  <div class="scrim" role="presentation" onclick={() => { promptState!.resolve(null); promptState = null; }}></div>
  <form
    class="ask"
    onsubmit={(e) => {
      e.preventDefault();
      const v = promptState!.value.trim();
      promptState!.resolve(v || null);
      promptState = null;
    }}
  >
    <label>
      {promptState.label}
      <!-- svelte-ignore a11y_autofocus -->
      <input bind:value={promptState.value} autofocus />
    </label>
    <div class="ask-row">
      <button type="button" class="quiet-btn" onclick={() => { promptState!.resolve(null); promptState = null; }}>Cancel</button>
      <button class="ink-btn">Create</button>
    </div>
  </form>
{/if}

{#if settingsOpen}
  <SettingsDialog
    {settings}
    themes={allThemes}
    errors={themeErrors}
    {themesDir}
    onchange={changeSettings}
    onOpenFolder={openThemesFolder}
    onReload={() => loadCustomThemes(true)}
    onClose={() => (settingsOpen = false)}
  />
{/if}

{#if toast}
  <div class="toast" class:error={toast.error} role="status">{toast.text}</div>
{/if}

<style>
  .app {
    display: grid;
    grid-template-columns: var(--sidebar-w) 4px 1fr;
    height: 100%;
  }

  .split-x {
    cursor: col-resize;
    background: var(--rule);
  }

  .split-x:hover,
  .split-y:hover {
    background: var(--accent);
  }

  main {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }

  .bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 7px 14px 7px 18px;
    border-bottom: 1px solid var(--rule-strong);
    min-height: 44px;
  }

  .crumbs {
    font-family: var(--font-note);
    font-variation-settings: "MONO" 0, "CASL" 1, "slnt" 0, "CRSV" 1;
    font-size: 15px;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .dot {
    color: var(--accent);
    margin-left: 6px;
    font-size: 10px;
    vertical-align: middle;
  }

  .tools {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
  }

  .env {
    display: flex;
    align-items: center;
    gap: 6px;
    border: 1px solid var(--rule-strong);
    border-radius: 6px;
    padding: 2px 4px 2px 9px;
    color: var(--ink-faint);
    font-size: 12px;
  }

  .env select {
    border: none;
    background: transparent;
    color: var(--ink);
    font-weight: 700;
    font-size: 13px;
    padding: 2px;
  }

  .env select option {
    background: var(--paper);
  }

  .banner {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 18px;
    background: var(--accent-wash);
    border-bottom: 1px solid var(--accent);
    font-size: 13px;
  }

  .page {
    flex: 1;
    min-height: 0;
  }

  .page.hidden {
    display: none;
  }

  .empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    color: var(--ink-soft);
  }

  .split-y {
    height: 4px;
    cursor: row-resize;
    background: var(--rule-strong);
    flex-shrink: 0;
  }

  .panel-wrap {
    flex-shrink: 0;
    min-height: 0;
  }

  .toast {
    position: fixed;
    bottom: 18px;
    left: 50%;
    transform: translateX(-50%);
    background: var(--ink);
    color: var(--paper);
    padding: 8px 16px;
    border-radius: 8px;
    font-size: 13px;
    max-width: min(640px, 90vw);
    box-shadow: 0 6px 24px color-mix(in srgb, var(--ink) 25%, transparent);
    z-index: 50;
  }

  .toast.error {
    background: var(--err);
    color: #fff;
  }

  .scrim {
    position: fixed;
    inset: 0;
    background: color-mix(in srgb, var(--ink) 25%, transparent);
    z-index: 40;
  }

  .ask {
    position: fixed;
    top: 22%;
    left: 50%;
    transform: translateX(-50%);
    width: 340px;
    background: var(--paper-raised);
    border: 1px solid var(--rule-strong);
    border-radius: 10px;
    padding: 16px;
    z-index: 41;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .ask label {
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-weight: 700;
  }

  .ask input {
    border: 1px solid var(--rule-strong);
    background: var(--paper);
    border-radius: 6px;
    padding: 6px 8px;
    font-weight: 400;
  }

  .ask-row {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
  }
</style>
