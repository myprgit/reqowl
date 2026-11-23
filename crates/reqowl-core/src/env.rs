//! A workspace is a folder with a `reqowl.toml`. Environments live in
//! `env/<name>.toml`; secrets go in the git-ignored `env/<name>.local.toml`.
//!
//! ```toml
//! # reqowl.toml
//! name = "Shop API"
//! default_env = "dev"
//!
//! [vars]          # defaults for every environment
//! api_version = "v1"
//! ```

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::vars::Layer;

pub const CONFIG_FILE: &str = "reqowl.toml";
pub const ENV_DIR: &str = "env";
pub const STATE_DIR: &str = ".reqowl";

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub name: Option<String>,
    pub default_env: Option<String>,
    #[serde(default)]
    pub vars: toml::Table,
}

#[derive(Debug, Clone)]
pub struct Workspace {
    pub root: PathBuf,
    pub config: Config,
}

#[derive(Debug, Clone)]
pub struct Env {
    pub name: String,
    pub vars: BTreeMap<String, String>,
    /// From `env/<name>.local.toml`.
    pub secrets: BTreeMap<String, String>,
}

impl Workspace {
    /// Walks up from `start` (a file or directory) to the nearest `reqowl.toml`.
    pub fn discover(start: &Path) -> Result<Option<Workspace>> {
        let start = if start.is_file() {
            start.parent().unwrap_or(start)
        } else {
            start
        };
        let start = start.canonicalize().map_err(|e| Error::io(start, e))?;
        for dir in start.ancestors() {
            if dir.join(CONFIG_FILE).is_file() {
                return Workspace::open(dir).map(Some);
            }
        }
        Ok(None)
    }

    pub fn open(root: &Path) -> Result<Workspace> {
        let path = root.join(CONFIG_FILE);
        let text = fs::read_to_string(&path).map_err(|e| Error::io(&path, e))?;
        let config: Config = toml::from_str(&text).map_err(|e| Error::Config {
            path,
            msg: e.to_string(),
        })?;
        Ok(Workspace {
            root: root.to_path_buf(),
            config,
        })
    }

    /// Environment names, sorted. `*.local.toml` files are overlays, not environments.
    pub fn env_names(&self) -> Result<Vec<String>> {
        let dir = self.root.join(ENV_DIR);
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(Error::io(dir, e)),
        };
        let mut names: Vec<String> = entries
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().to_str().map(str::to_string))
            .filter_map(|f| f.strip_suffix(".toml").map(str::to_string))
            .filter(|n| !n.ends_with(".local"))
            .collect();
        names.sort();
        Ok(names)
    }

    pub fn load_env(&self, name: &str) -> Result<Env> {
        let dir = self.root.join(ENV_DIR);
        let main = dir.join(format!("{name}.toml"));
        if !main.is_file() {
            return Err(Error::UnknownEnv(name.to_string()));
        }
        let vars = read_vars(&main)?;
        let local = dir.join(format!("{name}.local.toml"));
        let secrets = if local.is_file() {
            read_vars(&local)?
        } else {
            BTreeMap::new()
        };
        Ok(Env {
            name: name.to_string(),
            vars,
            secrets,
        })
    }

    pub fn defaults(&self) -> Result<BTreeMap<String, String>> {
        flatten(&self.config.vars, &self.root.join(CONFIG_FILE))
    }

    /// Variable layers for a page, highest precedence first:
    /// page front matter, env secrets, env, workspace defaults.
    pub fn layers(&self, front_matter: &BTreeMap<String, String>, env: Option<&Env>) -> Result<Vec<Layer>> {
        let mut layers = vec![page_layer(front_matter)];
        if let Some(env) = env {
            layers.push(Layer {
                label: format!("env {} (local)", env.name),
                vars: env.secrets.clone(),
                secret: true,
            });
            layers.push(Layer {
                label: format!("env {}", env.name),
                vars: env.vars.clone(),
                secret: false,
            });
        }
        layers.push(Layer {
            label: "workspace".into(),
            vars: self.defaults()?,
            secret: false,
        });
        Ok(layers)
    }
}

pub fn page_layer(front_matter: &BTreeMap<String, String>) -> Layer {
    Layer {
        label: "page".into(),
        vars: front_matter.clone(),
        secret: false,
    }
}

fn read_vars(path: &Path) -> Result<BTreeMap<String, String>> {
    let text = fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
    let table: toml::Table = toml::from_str(&text).map_err(|e| Error::Config {
        path: path.to_path_buf(),
        msg: e.to_string(),
    })?;
    flatten(&table, path)
}

fn flatten(table: &toml::Table, path: &Path) -> Result<BTreeMap<String, String>> {
    table
        .iter()
        .map(|(k, v)| {
            let s = match v {
                toml::Value::String(s) => s.clone(),
                toml::Value::Integer(i) => i.to_string(),
                toml::Value::Float(f) => f.to_string(),
                toml::Value::Boolean(b) => b.to_string(),
                _ => {
                    return Err(Error::Config {
                        path: path.to_path_buf(),
                        msg: format!("`{k}` must be a string, number or boolean"),
                    });
                }
            };
            Ok((k.clone(), s))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_envs_and_layers() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(
            root.join(CONFIG_FILE),
            "name = \"t\"\ndefault_env = \"dev\"\n[vars]\nv = 1\nbase = \"ws\"\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("env")).unwrap();
        fs::create_dir_all(root.join("Auth")).unwrap();
        fs::write(root.join("env/dev.toml"), "base = \"http://dev\"\ntoken = \"public\"\n").unwrap();
        fs::write(root.join("env/dev.local.toml"), "token = \"s3cret\"\n").unwrap();
        fs::write(root.join("env/prod.toml"), "base = \"http://prod\"\n").unwrap();
        fs::write(root.join("Auth/Login.md"), "x").unwrap();

        let ws = Workspace::discover(&root.join("Auth/Login.md")).unwrap().unwrap();
        assert_eq!(ws.config.default_env.as_deref(), Some("dev"));
        assert_eq!(ws.env_names().unwrap(), ["dev", "prod"]);
        assert!(matches!(ws.load_env("stage"), Err(Error::UnknownEnv(_))));

        let env = ws.load_env("dev").unwrap();
        let ctx = crate::vars::Context {
            layers: ws.layers(&BTreeMap::new(), Some(&env)).unwrap(),
            ..Default::default()
        };
        assert_eq!(ctx.render("{{base}} {{token}} {{v}}").unwrap(), "http://dev s3cret 1");
        assert_eq!(ctx.secret_values(), [("token", "s3cret")]);
    }

    #[test]
    fn no_workspace() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Workspace::discover(dir.path()).unwrap().is_none());
    }

    #[test]
    fn rejects_tables_in_env() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(CONFIG_FILE), "").unwrap();
        fs::create_dir_all(dir.path().join("env")).unwrap();
        fs::write(dir.path().join("env/x.toml"), "[nested]\na = 1\n").unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        assert!(matches!(ws.load_env("x"), Err(Error::Config { .. })));
    }
}
