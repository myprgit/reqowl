//! Running blocks of one page: workspace, environment, variables and the
//! responses of earlier named blocks, in one place. The CLI and the desktop
//! app both go through this.

use std::path::Path;

use crate::env::{self, Env, Workspace};
use crate::error::{Error, Result};
use crate::exec::{Client, Response};
use crate::page::{Block, Page};
use crate::request::{self, RequestSpec};
use crate::vars::Context;

pub struct Session {
    pub workspace: Option<Workspace>,
    pub env: Option<Env>,
    pub ctx: Context,
    client: Client,
}

pub struct Outcome {
    /// The request as sent: variables filled in, auth applied.
    pub request: RequestSpec,
    pub response: Response,
}

impl Session {
    /// `env_name` overrides the workspace's `default_env`.
    pub fn open(page_path: &Path, page: &Page, env_name: Option<&str>) -> Result<Session> {
        let workspace = Workspace::discover(page_path)?;
        let env_name = env_name
            .map(str::to_string)
            .or_else(|| workspace.as_ref()?.config.default_env.clone());

        let (env, layers) = match &workspace {
            Some(ws) => {
                let env = env_name.map(|n| ws.load_env(&n)).transpose()?;
                let layers = ws.layers(&page.front_matter, env.as_ref())?;
                (env, layers)
            }
            None => {
                if let Some(n) = env_name {
                    return Err(Error::Config {
                        path: page_path.to_path_buf(),
                        msg: format!(
                            "environment `{n}` requested, but no {} was found above this page",
                            env::CONFIG_FILE
                        ),
                    });
                }
                (None, vec![env::page_layer(&page.front_matter)])
            }
        };

        Ok(Session {
            workspace,
            env,
            ctx: Context {
                layers,
                ..Default::default()
            },
            client: Client::new(),
        })
    }

    /// Parses and resolves a block without sending it.
    pub fn prepare(&self, block: &Block) -> Result<RequestSpec> {
        let spec = request::parse(&block.content, block.content_line)?;
        let mut spec = spec.resolve(&self.ctx)?;
        spec.apply_auth()?;
        Ok(spec)
    }

    /// Sends a block. If it has `@name`, its response becomes available to
    /// later blocks as `{{name.response...}}`.
    pub async fn run(&mut self, block: &Block) -> Result<Outcome> {
        let spec = self.prepare(block)?;
        let response = self.client.execute(&spec).await?;
        if let Some(name) = spec.name() {
            self.ctx.record(name, response.record());
        }
        Ok(Outcome {
            request: spec,
            response,
        })
    }
}

/// Names used as `{{name.response...}}` in a block.
pub fn referenced_names(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = content;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else { break };
        if let Some((name, _)) = after[..end].trim().split_once(".response.")
            && !out.iter().any(|n| n == name)
        {
            out.push(name.to_string());
        }
        rest = &after[end + 2..];
    }
    out
}

/// The blocks to run, in order, so that `targets` have every response they
/// reference: named dependencies first (recursively), then the targets.
/// Each block appears once. Unknown names are left for resolution to report.
pub fn with_dependencies<'p>(page: &'p Page, targets: &[&'p Block]) -> Vec<&'p Block> {
    fn visit<'p>(page: &'p Page, b: &'p Block, out: &mut Vec<&'p Block>, stack: &mut Vec<usize>) {
        if out.iter().any(|o| o.index == b.index) || stack.contains(&b.index) {
            return;
        }
        stack.push(b.index);
        for name in referenced_names(&b.content) {
            if let Some(dep) = find_block(page, &name) {
                visit(page, dep, out, stack);
            }
        }
        stack.pop();
        out.push(b);
    }
    let mut out = Vec::new();
    for t in targets {
        visit(page, t, &mut out, &mut Vec::new());
    }
    out
}

/// Finds a block by `@name` or by 1-based position.
pub fn find_block<'p>(page: &'p Page, key: &str) -> Option<&'p Block> {
    if let Ok(n) = key.parse::<usize>() {
        return n.checked_sub(1).and_then(|i| page.blocks.get(i));
    }
    page.blocks
        .iter()
        .find(|b| request::parse(&b.content, b.content_line).is_ok_and(|s| s.name() == Some(key)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page;

    #[test]
    fn dependencies_come_first() {
        let p = page::parse(
            "```http\n# @name a\nGET http://h/a\n```\n\
             ```http\n# @name b\nGET http://h/{{a.response.body.$.x}}\n```\n\
             ```http\nGET http://h/{{b.response.status}}/{{a.response.status}}/{{zz.response.status}}\n```\n\
             ```http\n# @name loop\nGET http://h/{{loop.response.status}}\n```\n",
        );
        let order = |keys: &[&str]| -> Vec<usize> {
            let t: Vec<&Block> = keys.iter().map(|k| find_block(&p, k).unwrap()).collect();
            with_dependencies(&p, &t).iter().map(|b| b.index + 1).collect()
        };
        assert_eq!(order(&["3"]), [1, 2, 3]);
        assert_eq!(order(&["b", "a"]), [1, 2]);
        assert_eq!(order(&["loop"]), [4]);
        assert_eq!(referenced_names("{{a.response.x}} {{ a.response.y }} {{c}}"), ["a"]);
    }
}
