use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use clap::{Parser, Subcommand};
use reqowl_core::exec::Response;
use reqowl_core::page::{self, Block};
use reqowl_core::request;
use reqowl_core::session::{Session, find_block, with_dependencies};

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
        } => {
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
            let mut ok = true;
            for (n, b) in blocks.iter().enumerate() {
                if n > 0 && !quiet {
                    println!();
                }
                match session.run(b).await {
                    Ok(out) => {
                        print_response(&out.request, &out.response, include, quiet);
                        ok &= out.response.status < 400;
                    }
                    Err(e) => {
                        eprintln!("block {} (line {}): {e}", b.index + 1, b.content_line);
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

fn print_response(req: &request::RequestSpec, resp: &Response, include: bool, quiet: bool) {
    println!(
        "{} {}  →  {} {}  ·  {} ms  ·  {} B",
        req.method,
        req.url,
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
            println!("{k}: {v}");
        }
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
