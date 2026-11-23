//! Tauri commands. The UI owns the editor buffer; the backend owns files,
//! running requests, history and the variable state between runs.

mod tree;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use notify_debouncer_mini::{Debouncer, new_debouncer, notify::RecommendedWatcher, notify::RecursiveMode};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::oneshot;

use reqowl_core::env::{CONFIG_FILE, ENV_DIR, STATE_DIR, Workspace};
use reqowl_core::exec::Response;
use reqowl_core::history::{self, Entry, History};
use reqowl_core::mask::Masker;
use reqowl_core::oauth::TokenCache;
use reqowl_core::page::{self, Block};
use reqowl_core::request::{self, RequestSpec};
use reqowl_core::session::{Session, with_dependencies};
use reqowl_core::vars::Recorded;
use reqowl_core::ws::{self, WsEvent, WsHandle};
use tauri_plugin_opener::OpenerExt;

use tree::{Node, Res, check_id, md_path};

#[derive(Default)]
struct AppState {
    root: Mutex<Option<PathBuf>>,
    history: Mutex<Option<History>>,
    /// Named responses per page id, so chained blocks work across runs.
    responses: Mutex<HashMap<String, HashMap<String, Recorded>>>,
    runs: Mutex<HashMap<u64, oneshot::Sender<()>>>,
    watcher: Mutex<Option<Debouncer<RecommendedWatcher>>>,
    /// OAuth2 tokens, shared by every run so sign-in happens once.
    tokens: Arc<TokenCache>,
    sockets: Mutex<HashMap<u64, WsHandle>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn root(state: &AppState) -> Res<PathBuf> {
    lock(&state.root)
        .clone()
        .ok_or_else(|| "no notebook is open".to_string())
}

fn s<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

#[derive(Serialize)]
struct WorkspaceInfo {
    root: String,
    name: String,
    envs: Vec<String>,
    default_env: Option<String>,
    tree: Vec<Node>,
}

fn info(root: &Path) -> Res<WorkspaceInfo> {
    let ws = Workspace::open(root).map_err(s)?;
    let name = ws.config.name.clone().unwrap_or_else(|| {
        root.file_name()
            .map_or_else(|| "Notebook".into(), |n| n.to_string_lossy().into_owned())
    });
    Ok(WorkspaceInfo {
        root: root.display().to_string(),
        name,
        envs: ws.env_names().map_err(s)?,
        default_env: ws.config.default_env.clone(),
        tree: tree::scan(root)?,
    })
}

#[tauri::command]
fn open_workspace(app: AppHandle, state: State<AppState>, path: String) -> Res<WorkspaceInfo> {
    let root = PathBuf::from(&path)
        .canonicalize()
        .map_err(|e| format!("{path}: {e}"))?;
    if !root.join(CONFIG_FILE).is_file() {
        return Err(format!("not_workspace:{}", root.display()));
    }
    let info = info(&root)?;
    *lock(&state.history) = Some(History::open(&root).map_err(s)?);
    *lock(&state.root) = Some(root.clone());
    lock(&state.responses).clear();
    *lock(&state.watcher) = watch(app, &root);
    Ok(info)
}

/// Turns a folder into a notebook: config, one environment, a first page.
#[tauri::command]
fn init_workspace(path: String) -> Res<()> {
    let root = PathBuf::from(&path);
    if root.join(CONFIG_FILE).exists() {
        return Ok(());
    }
    let name = root
        .file_name()
        .map_or_else(|| "Notebook".into(), |n| n.to_string_lossy().into_owned());
    let w = |rel: &str, text: &str| -> Res<()> {
        let p = root.join(rel);
        if p.exists() {
            return Ok(());
        }
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
        }
        std::fs::write(&p, text).map_err(|e| format!("{}: {e}", p.display()))
    };
    w(
        CONFIG_FILE,
        &format!("name = {name:?}\ndefault_env = \"dev\"\n\n[vars]\n"),
    )?;
    w(
        "env/dev.toml",
        "# Variables for the dev environment. Use them as {{base}} in requests.\n\
         # Secrets go in dev.local.toml next to this file; it stays out of git.\n\
         base = \"http://localhost:8080\"\n",
    )?;
    w(".gitignore", ".reqowl/\nenv/*.local.toml\n")?;
    w("Start here.md", include_str!("../../examples/start-here.md"))?;
    Ok(())
}

#[tauri::command]
fn workspace_info(state: State<AppState>) -> Res<WorkspaceInfo> {
    info(&root(&state)?)
}

#[tauri::command]
fn read_page(state: State<AppState>, id: String) -> Res<String> {
    check_id(&id)?;
    let p = md_path(&root(&state)?, &id);
    match std::fs::read_to_string(&p) {
        Ok(t) => Ok(t),
        // A folder without its own page: an empty buffer, created on first save.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(format!("{}: {e}", p.display())),
    }
}

#[tauri::command]
fn write_page(state: State<AppState>, id: String, text: String) -> Res<()> {
    check_id(&id)?;
    tree::write_atomic(&md_path(&root(&state)?, &id), &text)
}

/// Raw text files inside the workspace that aren't pages: env/*.toml, reqowl.toml.
fn text_path(root: &Path, rel: &str) -> Res<PathBuf> {
    check_id(rel)?;
    let ok = rel == CONFIG_FILE || (rel.starts_with(&format!("{ENV_DIR}/")) && rel.ends_with(".toml"));
    if !ok {
        return Err(format!("`{rel}` is not an editable settings file"));
    }
    Ok(root.join(rel))
}

#[tauri::command]
fn read_text(state: State<AppState>, rel: String) -> Res<String> {
    let p = text_path(&root(&state)?, &rel)?;
    match std::fs::read_to_string(&p) {
        Ok(t) => Ok(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(format!("{}: {e}", p.display())),
    }
}

#[tauri::command]
fn write_text(state: State<AppState>, rel: String, text: String) -> Res<()> {
    let p = text_path(&root(&state)?, &rel)?;
    tree::write_atomic(&p, &text)
}

#[tauri::command]
fn create_env(state: State<AppState>, name: String) -> Res<String> {
    let clean = tree::clean_title(&name)?.replace(' ', "-");
    if clean.ends_with(".local") {
        return Err("names ending in .local are reserved for secret files".into());
    }
    let rel = format!("{ENV_DIR}/{clean}.toml");
    let p = text_path(&root(&state)?, &rel)?;
    if !p.exists() {
        tree::write_atomic(&p, &format!("# Variables for the {clean} environment.\n"))?;
    }
    Ok(rel)
}

#[tauri::command]
fn create_page(state: State<AppState>, parent: Option<String>, title: String) -> Res<String> {
    tree::create_page(&root(&state)?, parent.as_deref(), &title)
}

fn move_history(state: &AppState, from: &str, to: &str) {
    if from == to {
        return;
    }
    if let Some(h) = lock(&state.history).as_ref() {
        let _ = h.rename_page(&format!("{from}.md"), &format!("{to}.md"));
    }
    let mut r = lock(&state.responses);
    let moved: Vec<String> = r
        .keys()
        .filter(|k| *k == from || k.starts_with(&format!("{from}/")))
        .cloned()
        .collect();
    for k in moved {
        if let Some(v) = r.remove(&k) {
            r.insert(format!("{to}{}", &k[from.len()..]), v);
        }
    }
}

#[tauri::command]
fn rename_page(state: State<AppState>, id: String, title: String) -> Res<String> {
    let new_id = tree::rename_page(&root(&state)?, &id, &title)?;
    move_history(&state, &id, &new_id);
    Ok(new_id)
}

#[tauri::command]
fn move_page(state: State<AppState>, id: String, parent: Option<String>, before: Option<String>) -> Res<String> {
    let new_id = tree::move_page(&root(&state)?, &id, parent.as_deref(), before.as_deref())?;
    move_history(&state, &id, &new_id);
    Ok(new_id)
}

#[tauri::command]
fn trash_page(state: State<AppState>, id: String) -> Res<()> {
    tree::trash_page(&root(&state)?, &id).map(|_| ())
}

/// A session for one run: workspace variables, the page's earlier responses,
/// shared OAuth2 tokens, and a browser for sign-in pages.
fn open_session(app: &AppHandle, state: &AppState, id: &str, parsed: &page::Page, env: Option<&str>) -> Res<Session> {
    let root = root(state)?;
    let mut session = Session::open(&md_path(&root, id), parsed, env).map_err(s)?;
    session.ctx.responses = lock(&state.responses).get(id).cloned().unwrap_or_default();
    session.set_token_cache(state.tokens.clone());
    let app = app.clone();
    session.set_browser_opener(Box::new(move |url: &str| {
        let _ = app.opener().open_url(url, None::<&str>);
    }));
    Ok(session)
}

#[derive(Serialize)]
struct BlockInfo {
    /// 0-based line of the opening fence.
    line: usize,
    key: String,
    name: Option<String>,
    /// `http`, `graphql` or `websocket`.
    kind: Option<&'static str>,
    method: Option<String>,
    url: Option<String>,
    error: Option<String>,
}

fn fence_line(b: &Block) -> usize {
    // content_line is 1-based and points at the line after the fence.
    b.content_line.saturating_sub(2)
}

/// The request blocks of a buffer, as the backend sees them.
#[tauri::command]
fn page_blocks(text: String) -> Vec<BlockInfo> {
    page::parse(&text)
        .blocks
        .iter()
        .map(|b| {
            let parsed = request::parse(&b.content, b.content_line);
            BlockInfo {
                line: fence_line(b),
                key: history::block_key(b),
                name: parsed.as_ref().ok().and_then(|s| s.name().map(str::to_string)),
                kind: parsed.as_ref().ok().map(|s| match s.kind {
                    request::Kind::Http => "http",
                    request::Kind::GraphQl => "graphql",
                    request::Kind::WebSocket => "websocket",
                }),
                method: parsed.as_ref().ok().map(|s| s.method.clone()),
                url: parsed.as_ref().ok().map(|s| s.url.clone()),
                error: parsed.err().map(|e| e.to_string()),
            }
        })
        .collect()
}

#[derive(Serialize)]
struct RequestView {
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body: Option<String>,
}

#[derive(Serialize)]
struct ResponseView {
    status: u16,
    reason: String,
    version: String,
    url: String,
    headers: Vec<(String, String)>,
    body: String,
    /// `data:` URL for image responses, so the UI can show them.
    image: Option<String>,
    size: usize,
    truncated: bool,
    elapsed_ms: u64,
    content_type: Option<String>,
}

#[derive(Serialize)]
struct RunResult {
    line: usize,
    key: String,
    history_id: Option<i64>,
    request: Option<RequestView>,
    response: Option<ResponseView>,
    error: Option<String>,
}

fn request_view(spec: &RequestSpec, m: &Masker) -> RequestView {
    RequestView {
        method: spec.method.clone(),
        url: m.mask(&spec.url),
        headers: spec
            .headers
            .iter()
            .map(|h| (h.name.clone(), m.mask(&h.value)))
            .collect(),
        body: spec.body.as_deref().map(|b| m.mask(b)),
    }
}

fn response_view(r: &Response) -> ResponseView {
    let content_type = r.header("content-type").map(str::to_string);
    let image = content_type
        .as_deref()
        .filter(|ct| ct.starts_with("image/") && !r.truncated && r.body.len() <= 16 * 1024 * 1024)
        .map(|ct| {
            let mime = ct.split(';').next().unwrap_or(ct).trim();
            format!(
                "data:{mime};base64,{}",
                base64::engine::general_purpose::STANDARD.encode(&r.body)
            )
        });
    ResponseView {
        status: r.status,
        reason: r.reason.clone(),
        version: r.version.clone(),
        url: r.url.clone(),
        headers: r.headers.clone(),
        body: if image.is_some() { String::new() } else { r.body_text() },
        image,
        size: r.body.len(),
        truncated: r.truncated,
        elapsed_ms: r.elapsed.as_millis() as u64,
        content_type,
    }
}

/// Runs the block whose opening fence is on `line` (0-based). Named blocks it
/// depends on run first when their responses aren't known yet.
#[tauri::command]
async fn run_block(
    app: AppHandle,
    state: State<'_, AppState>,
    run_id: u64,
    id: String,
    text: String,
    line: usize,
    env: Option<String>,
) -> Res<Vec<RunResult>> {
    check_id(&id)?;
    let root = root(&state)?;
    let page_path = md_path(&root, &id);
    if !page_path.exists() {
        tree::write_atomic(&page_path, &text)?;
    }
    let parsed = page::parse(&text);
    let target = parsed
        .blocks
        .iter()
        .find(|b| fence_line(b) == line)
        .ok_or_else(|| "there's no http block on that line".to_string())?;

    let mut session = open_session(&app, &state, &id, &parsed, env.as_deref())?;

    let order: Vec<&Block> = with_dependencies(&parsed, &[target])
        .into_iter()
        .filter(|b| {
            b.index == target.index
                || request::parse(&b.content, b.content_line)
                    .ok()
                    .and_then(|s| s.name().map(|n| !session.ctx.responses.contains_key(n)))
                    .unwrap_or(false)
        })
        .collect();

    let (tx, mut rx) = oneshot::channel();
    lock(&state.runs).insert(run_id, tx);
    let page_rel = format!("{id}.md");
    let mut results = Vec::new();

    for b in order {
        let masker = Masker::new(&session.ctx);
        let key = history::block_key(b);
        let mut result = RunResult {
            line: fence_line(b),
            key: key.clone(),
            history_id: None,
            request: None,
            response: None,
            error: None,
        };

        let spec = match session.prepare(b) {
            Ok(spec) => spec,
            Err(e) => {
                result.error = Some(masker.mask(&e.to_string()));
                results.push(result);
                break;
            }
        };
        result.request = Some(request_view(&spec, &masker));

        let outcome = tokio::select! {
            r = session.run(b) => Some(r),
            _ = &mut rx => None,
        };
        let failed = match outcome {
            None => {
                result.error = Some("Cancelled".into());
                results.push(result);
                break;
            }
            Some(Ok(out)) => {
                result.history_id = lock(&state.history).as_ref().and_then(|h| {
                    h.record(
                        &page_rel,
                        &key,
                        env.as_deref(),
                        &out.request,
                        Ok(&out.response),
                        &masker,
                    )
                    .ok()
                });
                result.response = Some(response_view(&out.response));
                false
            }
            Some(Err(e)) => {
                let msg = masker.mask(&e.to_string());
                result.history_id = lock(&state.history).as_ref().and_then(|h| {
                    h.record(&page_rel, &key, env.as_deref(), &spec, Err(&msg), &masker)
                        .ok()
                });
                result.error = Some(msg);
                true
            }
        };
        results.push(result);
        if failed {
            break;
        }
    }

    lock(&state.runs).remove(&run_id);
    lock(&state.responses).insert(id, session.ctx.responses);
    Ok(results)
}

#[tauri::command]
fn cancel_run(state: State<AppState>, run_id: u64) {
    if let Some(tx) = lock(&state.runs).remove(&run_id) {
        let _ = tx.send(());
    }
}

/// Forgets chained responses for a page (e.g. after switching environment).
#[tauri::command]
fn reset_page_state(state: State<AppState>, id: String) {
    lock(&state.responses).remove(&id);
}

#[tauri::command]
fn latest_runs(state: State<AppState>, id: String) -> Res<Vec<Entry>> {
    let h = lock(&state.history);
    let Some(h) = h.as_ref() else { return Ok(Vec::new()) };
    h.latest_per_block(&format!("{id}.md")).map_err(s)
}

#[tauri::command]
fn history_list(state: State<AppState>, id: String, key: Option<String>, limit: Option<usize>) -> Res<Vec<Entry>> {
    let h = lock(&state.history);
    let Some(h) = h.as_ref() else { return Ok(Vec::new()) };
    h.list(&format!("{id}.md"), key.as_deref(), limit.unwrap_or(50))
        .map_err(s)
}

/// Copies a block as a curl command, with variables filled in for `env`.
#[tauri::command]
fn block_as_curl(
    app: AppHandle,
    state: State<AppState>,
    id: String,
    text: String,
    line: usize,
    env: Option<String>,
) -> Res<String> {
    let parsed = page::parse(&text);
    let b = parsed
        .blocks
        .iter()
        .find(|b| fence_line(b) == line)
        .ok_or("there's no http block on that line")?;
    let session = open_session(&app, &state, &id, &parsed, env.as_deref())?;
    let spec = session.prepare(b).map_err(s)?;
    Ok(reqowl_core::export::to_curl(&spec))
}

/// Turns a pasted curl command into the text of an http block, or None.
#[tauri::command]
fn curl_to_block(text: String) -> Option<String> {
    if !reqowl_core::import::curl::looks_like_curl(&text) {
        return None;
    }
    reqowl_core::import::curl::parse(&text)
        .ok()
        .map(|spec| request::to_block_text(&spec))
}

/// Imports a Postman/Insomnia export or curl command into a new notebook folder.
#[tauri::command]
fn import_file(source: String, target: String) -> Res<String> {
    let text = std::fs::read_to_string(&source).map_err(|e| format!("{source}: {e}"))?;
    let imported = reqowl_core::import::detect_and_parse(&text).map_err(s)?;
    let dir = PathBuf::from(&target);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{target}: {e}"))?;
    reqowl_core::import::write_workspace(&imported, &dir).map_err(s)?;
    Ok(dir.display().to_string())
}

/// Variable names for `{{` completion. Values are not sent to the UI.
#[tauri::command]
fn variable_names(state: State<AppState>, id: Option<String>, text: String, env: Option<String>) -> Res<Vec<String>> {
    let root = root(&state)?;
    let parsed = page::parse(&text);
    let ws = Workspace::open(&root).map_err(s)?;
    let env = env.map(|e| ws.load_env(&e)).transpose().ok().flatten();
    let mut names: Vec<String> = ws
        .layers(&parsed.front_matter, env.as_ref())
        .map_err(s)?
        .into_iter()
        .flat_map(|l| l.vars.into_keys())
        .collect();
    for b in &parsed.blocks {
        if let Ok(spec) = request::parse(&b.content, b.content_line)
            && let Some(n) = spec.name()
        {
            names.extend([
                format!("{n}.response.body.$."),
                format!("{n}.response.status"),
                format!("{n}.response.headers."),
            ]);
        }
    }
    let _ = id;
    names.sort();
    names.dedup();
    Ok(names)
}

#[derive(Serialize, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum SocketEvent {
    Opened {
        status: u16,
        headers: Vec<(String, String)>,
        elapsed_ms: u64,
    },
    Received {
        text: String,
    },
    ReceivedBinary {
        size: usize,
    },
    Sent {
        text: String,
    },
    Closed {
        code: Option<u16>,
        reason: String,
    },
    Error {
        message: String,
    },
}

#[derive(Serialize, Clone)]
struct SocketMessage {
    conn: u64,
    ts: u64,
    event: SocketEvent,
}

/// Opens the WEBSOCKET block on `line`. Events arrive as `ws-event`.
#[tauri::command]
async fn ws_connect(
    app: AppHandle,
    state: State<'_, AppState>,
    conn: u64,
    id: String,
    text: String,
    line: usize,
    env: Option<String>,
) -> Res<()> {
    check_id(&id)?;
    let parsed = page::parse(&text);
    let b = parsed
        .blocks
        .iter()
        .find(|b| fence_line(b) == line)
        .ok_or("there's no block on that line")?;
    let session = open_session(&app, &state, &id, &parsed, env.as_deref())?;
    let mut spec = session.prepare(b).map_err(s)?;
    session.authorize(&mut spec).await.map_err(s)?;

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let handle = ws::connect(&spec, tx).await.map_err(s)?;
    lock(&state.sockets).insert(conn, handle);

    let emitter = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(e) = rx.recv().await {
            let closed = matches!(e, WsEvent::Closed { .. });
            let event = match e {
                WsEvent::Opened {
                    status,
                    headers,
                    elapsed,
                } => SocketEvent::Opened {
                    status,
                    headers,
                    elapsed_ms: elapsed.as_millis() as u64,
                },
                WsEvent::Text(text) => SocketEvent::Received { text },
                WsEvent::Binary(b) => SocketEvent::ReceivedBinary { size: b.len() },
                WsEvent::Sent(text) => SocketEvent::Sent { text },
                WsEvent::Closed { code, reason } => SocketEvent::Closed { code, reason },
                WsEvent::Error(message) => SocketEvent::Error { message },
            };
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as u64);
            let _ = emitter.emit("ws-event", SocketMessage { conn, ts, event });
            if closed {
                if let Some(state) = emitter.try_state::<AppState>() {
                    lock(&state.sockets).remove(&conn);
                }
                break;
            }
        }
    });
    Ok(())
}

#[tauri::command]
fn ws_send(state: State<AppState>, conn: u64, text: String) -> Res<()> {
    let sockets = lock(&state.sockets);
    let h = sockets.get(&conn).ok_or("this connection is closed")?;
    h.send(text).map_err(s)
}

#[tauri::command]
fn ws_close(state: State<AppState>, conn: u64) {
    if let Some(h) = lock(&state.sockets).remove(&conn) {
        h.close();
    }
}

/// Watches the workspace and tells the UI which pages changed on disk.
fn watch(app: AppHandle, root: &Path) -> Option<Debouncer<RecommendedWatcher>> {
    let base = root.to_path_buf();
    let mut deb = new_debouncer(
        Duration::from_millis(300),
        move |res: notify_debouncer_mini::DebounceEventResult| {
            let events = match res {
                Ok(notify_debouncer_mini::DebouncedEventBatch::Events(events)) => events,
                // Missed events: tell the UI to reload everything.
                Ok(notify_debouncer_mini::DebouncedEventBatch::Rescan) => {
                    let _ = app.emit("fs-changed", vec!["*".to_string()]);
                    return;
                }
                _ => return,
            };
            let mut changed: Vec<String> = events
                .iter()
                .filter_map(|e| e.path.strip_prefix(&base).ok().map(Path::to_path_buf))
                .filter(|p| !p.starts_with(STATE_DIR))
                .filter(|p| p.extension().is_some_and(|x| x == "md" || x == "toml") || p.ends_with(tree::ORDER_FILE))
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .collect();
            changed.sort();
            changed.dedup();
            if !changed.is_empty() {
                let _ = app.emit("fs-changed", changed);
            }
        },
    )
    .ok()?;
    deb.watcher().watch(root, RecursiveMode::Recursive).ok()?;
    Some(deb)
}

/// The form view of the block whose opening fence is on `line`.
#[tauri::command]
fn block_form(text: String, line: usize) -> Res<reqowl_core::form::RequestForm> {
    let parsed = page::parse(&text);
    let b = parsed
        .blocks
        .iter()
        .find(|b| fence_line(b) == line)
        .ok_or("there's no http block on that line")?;
    reqowl_core::form::from_block(&b.content).map_err(s)
}

/// Block content (between the fences) for a form.
#[tauri::command]
fn form_to_block(form: reqowl_core::form::RequestForm) -> Res<String> {
    reqowl_core::form::to_block(&form).map_err(s)
}

/// Frontend errors end up in the terminal that started the app.
#[tauri::command]
fn log_ui(level: String, message: String) {
    eprintln!("[ui {level}] {message}");
}

/// A folder passed on the command line (`reqowl-app ~/notes/api`), if any.
#[tauri::command]
fn startup_path() -> Option<String> {
    std::env::args()
        .skip(1)
        .find(|a| !a.starts_with('-'))
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .and_then(|p| p.canonicalize().ok())
        .map(|p| p.display().to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::default())
        .setup(|app| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.set_title("reqowl");
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            open_workspace,
            init_workspace,
            workspace_info,
            read_page,
            write_page,
            read_text,
            write_text,
            create_env,
            create_page,
            rename_page,
            move_page,
            trash_page,
            page_blocks,
            run_block,
            cancel_run,
            reset_page_state,
            latest_runs,
            history_list,
            block_as_curl,
            curl_to_block,
            import_file,
            variable_names,
            ws_connect,
            ws_send,
            ws_close,
            startup_path,
            log_ui,
            block_form,
            form_to_block,
        ])
        .run(tauri::generate_context!())
        .expect("error while running reqowl");
}
