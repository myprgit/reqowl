use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context as _, Result};
use clap::{Parser, Subcommand};
use reqowl_core::exec::Response;
use reqowl_core::mask::Masker;
use reqowl_core::page::{self, Block};
use reqowl_core::request;
use reqowl_core::session::{Session, find_block, with_dependencies};
use reqowl_core::ws::{self, WsEvent};
use tokio::sync::mpsc;

/// reqowl: run the HTTP blocks of a notebook page.
#[derive(Parser)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run a page's request blocks, top to bottom.
    Run {
        page: PathBuf,
        /// Environment from `env/<name>.toml` (default: the workspace's default_env).
        #[arg(short, long)]
        env: Option<String>,
        /// Run only these blocks (by @name or 1-based position), plus the named
        /// blocks whose responses they use. Repeatable.
        #[arg(short, long)]
        block: Vec<String>,
        /// Print response headers too.
        #[arg(short = 'i', long)]
        include: bool,
        /// Print only status lines, no bodies.
        #[arg(short, long)]
        quiet: bool,
        /// Keep going after a failed block.
        #[arg(long)]
        keep_going: bool,
        /// For WEBSOCKET blocks: seconds to print incoming messages before closing.
        #[arg(long, default_value_t = 3.0, value_name = "SECS")]
        ws_wait: f64,
    },
    /// List a page's request blocks.
    Blocks { page: PathBuf },
    /// List the environments of the workspace containing PATH.
    Envs {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    match run(Cli::parse()).await {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(2)
        }
    }
}

fn load(path: &PathBuf) -> Result<page::Page> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(page::parse(&text))
}

async fn run(cli: Cli) -> Result<bool> {
    match cli.cmd {
        Cmd::Blocks { page: path } => {
            let page = load(&path)?;
            for b in &page.blocks {
                let line = match request::parse(&b.content, b.content_line) {
                    Ok(s) => {
                        let name = s.name().map(|n| format!("  @{n}")).unwrap_or_default();
                        format!("{} {}{name}", s.method, s.url)
                    }
                    Err(e) => format!("(invalid: {e})"),
                };
                println!("{:>3}  line {:<4} {line}", b.index + 1, b.content_line);
            }
            Ok(true)
        }
        Cmd::Envs { path } => {
            let ws = reqowl_core::env::Workspace::discover(&path)?.with_context(|| {
                format!(
                    "no {} found at or above {}",
                    reqowl_core::env::CONFIG_FILE,
                    path.display()
                )
            })?;
            let default = ws.config.default_env.as_deref();
            for name in ws.env_names()? {
                let mark = if Some(name.as_str()) == default {
                    " (default)"
                } else {
                    ""
                };
                println!("{name}{mark}");
            }
            Ok(true)
        }
        Cmd::Run {
            page: path,
            env,
            block,
            include,
            quiet,
            keep_going,
            ws_wait,
        } => {
            let ws_wait = Duration::try_from_secs_f64(ws_wait).context("--ws-wait must be a non-negative number")?;
            let page = load(&path)?;
            let blocks: Vec<&Block> = if block.is_empty() {
                page.blocks.iter().collect()
            } else {
                let targets: Vec<&Block> = block
                    .iter()
                    .map(|k| find_block(&page, k).with_context(|| format!("no block `{k}` in {}", path.display())))
                    .collect::<Result<_>>()?;
                // Blocks whose responses the targets reference run first.
                with_dependencies(&page, &targets)
            };
            if blocks.is_empty() {
                anyhow::bail!("{} has no ```http blocks", path.display());
            }

            let mut session = Session::open(&path, &page, env.as_deref())?;
            session.set_browser_opener(Box::new(open_browser));
            let mut ok = true;
            for (n, b) in blocks.iter().enumerate() {
                if n > 0 && !quiet {
                    println!();
                }
                let is_ws =
                    request::parse(&b.content, b.content_line).is_ok_and(|s| s.kind == request::Kind::WebSocket);
                if is_ws {
                    match run_websocket(&session, b, ws_wait, quiet).await {
                        Ok(opened) => ok &= opened,
                        Err(e) => {
                            eprintln!("block {} (line {}): {e}", b.index + 1, b.content_line);
                            ok = false;
                            if !keep_going {
                                break;
                            }
                        }
                    }
                    continue;
                }
                let masker = Masker::new(&session.ctx);
                match session.run(b).await {
                    Ok(out) => {
                        print_response(&out.request, &out.response, include, quiet, &masker);
                        ok &= out.response.status < 400;
                    }
                    Err(e) => {
                        eprintln!(
                            "block {} (line {}): {}",
                            b.index + 1,
                            b.content_line,
                            masker.mask(&e.to_string())
                        );
                        ok = false;
                        if !keep_going {
                            break;
                        }
                    }
                }
            }
            Ok(ok)
        }
    }
}

/// Connects, prints events for `wait` after the initial messages, then closes.
async fn run_websocket(session: &Session, block: &Block, wait: Duration, quiet: bool) -> reqowl_core::Result<bool> {
    let mut spec = session.prepare(block)?;
    session.authorize(&mut spec).await?;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let handle = ws::connect(&spec, tx).await?;

    let print = |e: &WsEvent| match e {
        WsEvent::Opened { status, elapsed, .. } => {
            println!(
                "WEBSOCKET {}  →  {status} open  ·  {} ms",
                spec.url,
                elapsed.as_millis()
            )
        }
        WsEvent::Sent(t) if !quiet => println!("→ {t}"),
        WsEvent::Text(t) if !quiet => println!("← {t}"),
        WsEvent::Binary(b) if !quiet => println!("← ({} B binary)", b.len()),
        WsEvent::Closed { code, reason } => match code {
            Some(c) if reason.is_empty() => println!("closed ({c})"),
            Some(c) => println!("closed ({c}: {reason})"),
            None => println!("closed"),
        },
        WsEvent::Error(msg) => eprintln!("websocket error: {msg}"),
        _ => {}
    };

    let deadline = tokio::time::Instant::now() + wait;
    let (mut closed, mut ok) = (false, true);
    while let Ok(Some(e)) = tokio::time::timeout_at(deadline, rx.recv()).await {
        print(&e);
        closed |= matches!(e, WsEvent::Closed { .. });
        ok &= !matches!(e, WsEvent::Error(_));
    }
    if !closed {
        handle.close();
        // The core gives up on a silent server after a few seconds.
        while let Some(e) = rx.recv().await {
            print(&e);
            ok &= !matches!(e, WsEvent::Error(_));
        }
    }
    Ok(ok)
}

/// Prints the sign-in URL and tries to open it in the default browser.
fn open_browser(url: &str) {
    eprintln!("Sign in to continue: {url}");
    let cmd = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(url).spawn()
    } else if cfg!(windows) {
        // Not `cmd /C start`: cmd would treat `&` in the query string as a command separator.
        std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url])
            .spawn()
    } else {
        std::process::Command::new("xdg-open").arg(url).spawn()
    };
    if cmd.is_err() {
        eprintln!("(could not open a browser; open the URL yourself)");
    }
}

/// Secret values are masked in the URL and headers; the body is printed as
/// received, since piping it somewhere is often the point.
fn print_response(req: &request::RequestSpec, resp: &Response, include: bool, quiet: bool, masker: &Masker) {
    println!(
        "{} {}  →  {} {}  ·  {} ms  ·  {} B",
        req.method,
        masker.mask(&req.url),
        resp.status,
        resp.reason,
        resp.elapsed.as_millis(),
        resp.body.len()
    );
    if quiet {
        return;
    }
    if include {
        for (k, v) in &resp.headers {
            println!("{k}: {}", masker.mask(v));
        }
    }
    if resp.truncated {
        eprintln!("(body cut off at {} MB)", reqowl_core::exec::MAX_BODY / (1024 * 1024));
    }
    if resp.body.is_empty() {
        return;
    }
    if include {
        println!();
    }
    let text = resp.body_text();
    let pretty = resp
        .is_json()
        .then(|| serde_json::from_str::<serde_json::Value>(&text).ok())
        .flatten()
        .and_then(|v| serde_json::to_string_pretty(&v).ok());
    println!("{}", pretty.unwrap_or(text));
}
