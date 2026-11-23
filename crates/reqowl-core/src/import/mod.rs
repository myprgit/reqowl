//! Importers: turn a Postman collection or environment, an Insomnia export
//! (v4 JSON/YAML, v5 YAML) or a curl command into an [`ImportedWorkspace`],
//! which [`write_workspace`] then lays out as a reqowl folder.
//!
//! Every importer builds [`RequestSpec`]s and renders them with
//! [`request::to_block_text`], so all produced blocks go through one
//! serializer. Things reqowl can't run (scripts, file uploads, unsupported
//! auth schemes) are kept as Markdown notes next to the block.

pub mod curl;
pub mod insomnia;
pub mod postman;

use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use serde::Serialize;
use serde_json::Value;

use crate::env::{CONFIG_FILE, ENV_DIR, STATE_DIR};
use crate::error::{Error, Result};
use crate::request::{self, HTTP_METHODS, Header, Kind, RequestSpec};

pub use crate::request::to_block_text;
pub use curl::to_curl;

/// A workspace ready to be written by [`write_workspace`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportedWorkspace {
    pub name: String,
    /// Written as `default_env` in `reqowl.toml`; must name one of `envs`.
    pub default_env: Option<String>,
    /// `[vars]` in `reqowl.toml`.
    pub vars: BTreeMap<String, String>,
    pub envs: Vec<ImportedEnv>,
    pub root: Vec<Node>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportedEnv {
    pub name: String,
    /// `env/<name>.toml`.
    pub vars: BTreeMap<String, String>,
    /// `env/<name>.local.toml` (git-ignored).
    pub secrets: BTreeMap<String, String>,
}

/// A page. A node with children is also a folder: `Name.md` plus `Name/`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Node {
    pub name: String,
    pub markdown: String,
    pub children: Vec<Node>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Curl,
    PostmanCollection,
    PostmanEnvironment,
    InsomniaV4,
    InsomniaV5,
}

/// Sniffs the format of an export file (or a pasted curl command).
pub fn detect(text: &str) -> Result<Format> {
    detect_value(text).map(|(f, _)| f)
}

/// Detects the format and imports it.
pub fn detect_and_parse(text: &str) -> Result<ImportedWorkspace> {
    match detect_value(text)? {
        (Format::Curl, _) => curl::import(text),
        (Format::PostmanCollection, Some(v)) => postman::parse_collection(&v),
        (Format::PostmanEnvironment, Some(v)) => postman::parse_environment(&v),
        (Format::InsomniaV4, Some(v)) => insomnia::parse_v4(&v),
        (Format::InsomniaV5, Some(v)) => insomnia::parse_v5(&v),
        _ => unreachable!("structured formats always carry a value"),
    }
}

fn detect_value(text: &str) -> Result<(Format, Option<Value>)> {
    let t = text.trim_start_matches('\u{feff}').trim();
    if t.starts_with('{') {
        let v: Value = serde_json::from_str(t).map_err(|e| Error::Import(format!("not valid JSON: {e}")))?;
        let format = if postman::is_collection(&v) {
            Format::PostmanCollection
        } else if postman::is_environment(&v) {
            Format::PostmanEnvironment
        } else if insomnia::is_v4(&v) {
            Format::InsomniaV4
        } else if postman::is_v1_collection(&v) {
            return Err(Error::Import(
                "Postman v1 collections are not supported; re-export as Collection v2.1".into(),
            ));
        } else {
            return Err(Error::Import(
                "unrecognized JSON: expected a Postman collection or environment, or an Insomnia export".into(),
            ));
        };
        return Ok((format, Some(v)));
    }
    if curl::looks_like_curl(t) {
        return Ok((Format::Curl, None));
    }
    // YAML: only trust an explicit type marker, since almost any text parses as YAML.
    let has_marker = t
        .lines()
        .any(|l| (l.starts_with("type:") && l.contains("insomnia.rest/5")) || l.starts_with("_type:"));
    if has_marker && let Ok(v) = serde_yaml_ng::from_str::<Value>(t) {
        if insomnia::is_v5(&v) {
            return Ok((Format::InsomniaV5, Some(v)));
        }
        if insomnia::is_v4(&v) {
            return Ok((Format::InsomniaV4, Some(v)));
        }
    }
    Err(Error::Import(
        "unrecognized input: expected a Postman collection or environment, an Insomnia export, or a curl command"
            .into(),
    ))
}

// ---------------------------------------------------------------------------
// Writing

#[derive(Serialize)]
struct ConfigOut<'a> {
    name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    default_env: Option<&'a str>,
    vars: &'a BTreeMap<String, String>,
}

const GITIGNORE_LINES: &[&str] = &[".reqowl/", "env/*.local.toml"];

/// Writes the workspace into `dir` (created if missing): `reqowl.toml`,
/// `env/*.toml`, pages, `.order` files and a `.gitignore`. Refuses a
/// directory that already holds a workspace. Existing files in `dir` are
/// never overwritten; colliding page names get a ` (2)` suffix.
pub fn write_workspace(ws: &ImportedWorkspace, dir: &Path) -> Result<()> {
    let config_path = dir.join(CONFIG_FILE);
    if config_path.exists() {
        return Err(Error::Import(format!(
            "{} already contains a reqowl workspace ({CONFIG_FILE})",
            dir.display()
        )));
    }
    fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;

    // Environments first, so default_env can point at the sanitized name.
    let mut env_names = Names::default();
    let mut written_envs = Vec::new();
    let mut default_env = None;
    for env in &ws.envs {
        let mut base = sanitize_file_name(&env.name);
        while let Some(s) = base.strip_suffix(".local") {
            base = s.trim_end().to_string();
        }
        if base.is_empty() {
            base = "env".into();
        }
        let name = env_names.unique(&base);
        if ws.default_env.as_deref() == Some(env.name.as_str()) && default_env.is_none() {
            default_env = Some(name.clone());
        }
        written_envs.push((name, env));
    }
    if !written_envs.is_empty() {
        let env_dir = dir.join(ENV_DIR);
        fs::create_dir_all(&env_dir).map_err(|e| Error::io(&env_dir, e))?;
        for (name, env) in &written_envs {
            write_file(&env_dir.join(format!("{name}.toml")), &toml_table(&env.vars)?)?;
            if !env.secrets.is_empty() {
                write_file(&env_dir.join(format!("{name}.local.toml")), &toml_table(&env.secrets)?)?;
            }
        }
    }

    let config = ConfigOut {
        name: &ws.name,
        default_env: default_env.as_deref(),
        vars: &ws.vars,
    };
    let text = toml::to_string(&config).map_err(|e| Error::Import(format!("writing {CONFIG_FILE}: {e}")))?;
    write_file(&config_path, &text)?;

    write_gitignore(dir)?;
    write_nodes(dir, &ws.root, true)
}

fn write_gitignore(dir: &Path) -> Result<()> {
    let path = dir.join(".gitignore");
    let existing = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(Error::io(path, e)),
    };
    let mut out = existing.clone();
    for line in GITIGNORE_LINES {
        if !existing.lines().any(|l| l.trim() == *line) {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(line);
            out.push('\n');
        }
    }
    if out != existing {
        fs::write(&path, out).map_err(|e| Error::io(path, e))?;
    }
    Ok(())
}

fn write_nodes(dir: &Path, nodes: &[Node], root: bool) -> Result<()> {
    if nodes.is_empty() {
        return Ok(());
    }
    let mut names = Names::default();
    if root {
        for reserved in [ENV_DIR, STATE_DIR, "reqowl"] {
            names.taken.insert(reserved.to_lowercase());
        }
    }
    // Never overwrite what is already there.
    if let Ok(entries) = fs::read_dir(dir) {
        for e in entries.flatten() {
            if let Some(n) = e.file_name().to_str() {
                names.taken.insert(n.strip_suffix(".md").unwrap_or(n).to_lowercase());
            }
        }
    }
    let mut order = String::new();
    for node in nodes {
        let name = names.unique(&sanitize_file_name(&node.name));
        let markdown = if node.markdown.trim().is_empty() {
            format!("# {}\n", one_line(&node.name))
        } else {
            node.markdown.clone()
        };
        write_file(&dir.join(format!("{name}.md")), &markdown)?;
        if !node.children.is_empty() {
            let sub = dir.join(&name);
            fs::create_dir_all(&sub).map_err(|e| Error::io(&sub, e))?;
            write_nodes(&sub, &node.children, false)?;
        }
        order.push_str(&name);
        order.push('\n');
    }
    write_file(&dir.join(".order"), &order)
}

fn write_file(path: &Path, text: &str) -> Result<()> {
    fs::write(path, text).map_err(|e| Error::io(path, e))
}

fn toml_table(vars: &BTreeMap<String, String>) -> Result<String> {
    toml::to_string(vars).map_err(|e| Error::Import(format!("writing environment: {e}")))
}

/// Case-insensitive name deduplication: `Foo`, `Foo (2)`, `Foo (3)`.
#[derive(Default)]
struct Names {
    taken: HashSet<String>,
}

impl Names {
    fn unique(&mut self, base: &str) -> String {
        let mut name = base.to_string();
        let mut n = 2;
        while !self.taken.insert(name.to_lowercase()) {
            name = format!("{base} ({n})");
            n += 1;
        }
        name
    }
}

/// A file name from a free-form title: drops `/\:*?"<>|` and control
/// characters, trims spaces and leading/trailing dots.
pub fn sanitize_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .filter(|c| !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        .collect();
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let cleaned = cleaned.trim_matches(|c: char| c == '.' || c.is_whitespace());
    if cleaned.is_empty() {
        "Untitled".into()
    } else {
        cleaned.to_string()
    }
}

/// Unique `@name` values across one import: ASCII letters, digits, `_`, `-`.
#[derive(Default)]
pub(crate) struct BlockNames {
    taken: HashSet<String>,
}

impl BlockNames {
    pub(crate) fn unique(&mut self, raw: &str) -> String {
        let mut base = String::new();
        for c in raw.chars() {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                base.push(c);
            } else if !base.ends_with('_') {
                base.push('_');
            }
        }
        let base = base.trim_matches('_');
        let base = if base.is_empty() { "request" } else { base };
        let mut name = base.to_string();
        let mut n = 2;
        while !self.taken.insert(name.clone()) {
            name = format!("{base}_{n}");
            n += 1;
        }
        name
    }
}

// ---------------------------------------------------------------------------
// Page rendering

/// A page under construction: title, notes, one request and any scripts.
#[derive(Debug, Default)]
pub(crate) struct PageDraft {
    pub title: String,
    pub front_matter: BTreeMap<String, String>,
    pub description: String,
    pub notes: Vec<String>,
    pub request: Option<RequestSpec>,
    /// (heading, code) pairs shown as non-runnable ```js blocks.
    pub scripts: Vec<(String, String)>,
}

impl PageDraft {
    pub fn new(title: &str) -> Self {
        PageDraft {
            title: one_line(title),
            ..Default::default()
        }
    }

    pub fn render(mut self) -> String {
        let block = self.request.take().map(|spec| {
            let spec = finish_spec(spec, &mut self.notes);
            let runnable = spec.kind != Kind::Http || HTTP_METHODS.contains(&spec.method.as_str());
            if !runnable {
                self.notes.push(format!(
                    "reqowl can't send `{}` requests, so this block is plain text and does not run.",
                    spec.method
                ));
            }
            fence(if runnable { "http" } else { "text" }, &request::to_block_text(&spec))
        });

        let mut out = String::new();
        let fm: Vec<_> = self
            .front_matter
            .iter()
            .filter(|(k, _)| !k.is_empty() && !k.contains(':') && !k.starts_with(['#', ' ', '\t']))
            .collect();
        if !fm.is_empty() {
            out.push_str("---\n");
            for (k, v) in fm {
                let v = one_line(v);
                if v != v.trim() || v.starts_with(['"', '\'']) {
                    let _ = writeln!(out, "{k}: \"{v}\"");
                } else {
                    let _ = writeln!(out, "{k}: {v}");
                }
            }
            out.push_str("---\n\n");
        }
        let _ = writeln!(out, "# {}", self.title);
        let desc = self.description.trim();
        if !desc.is_empty() {
            let _ = write!(out, "\n{}\n", desc.replace("\r\n", "\n"));
        }
        if !self.notes.is_empty() {
            out.push_str("\n> [!NOTE]\n");
            for n in &self.notes {
                let _ = writeln!(out, "> - {}", one_line(n));
            }
        }
        if let Some(block) = block {
            let _ = write!(out, "\n{block}");
        }
        for (heading, code) in &self.scripts {
            let _ = write!(out, "\n{heading}\n\n{}", fence("js", code.trim_end()));
        }
        out
    }
}

/// Puts a spec into the shape [`request::parse`] would produce, noting any
/// part that had to be dropped.
pub(crate) fn finish_spec(mut spec: RequestSpec, notes: &mut Vec<String>) -> RequestSpec {
    spec.url = spec.url.split(['\r', '\n']).map(str::trim).collect::<String>();
    if spec.url.is_empty() {
        notes.push("The request had no URL; set `{{url}}` or edit the block.".into());
        spec.url = "{{url}}".into();
    }
    spec.method = spec.method.trim().to_ascii_uppercase();
    if spec.method.is_empty() {
        spec.method = "GET".into();
    }
    let mut headers = Vec::with_capacity(spec.headers.len());
    for h in spec.headers {
        let name = h.name.trim().to_string();
        if !valid_header_name(&name) {
            notes.push(format!(
                "Skipped header `{}`: not a valid header name.",
                one_line(&h.name)
            ));
            continue;
        }
        headers.push(Header {
            name,
            value: one_line(&h.value).trim().to_string(),
        });
    }
    spec.headers = headers;
    spec.body = spec.body.and_then(|b| normalize_body(&b));
    spec.directives.auth = spec
        .directives
        .auth
        .map(|a| one_line(&a).trim().to_string())
        .filter(|a| !a.is_empty());
    spec
}

/// RFC 9110 token, minus leading characters the block parser treats specially.
fn valid_header_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with(['#', '&', '?'])
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "!#$%&'*+-.^_`|~".contains(c))
}

/// CRLF to LF, trailing blank lines dropped, empty as `None`.
pub(crate) fn normalize_body(body: &str) -> Option<String> {
    let body = body.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = body.lines().collect();
    let end = lines.iter().rposition(|l| !l.trim().is_empty())? + 1;
    Some(lines[..end].join("\n"))
}

/// A fenced code block whose fence is longer than any backtick run inside.
pub(crate) fn fence(lang: &str, content: &str) -> String {
    let mut longest = 0;
    let mut run = 0;
    for c in content.chars() {
        if c == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    let ticks = "`".repeat(longest.max(2) + 1);
    let nl = if content.ends_with('\n') { "" } else { "\n" };
    format!("{ticks}{lang}\n{content}{nl}{ticks}\n")
}

pub(crate) fn one_line(s: &str) -> String {
    s.split(['\r', '\n'])
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

// ---------------------------------------------------------------------------
// Shared helpers

/// A JSON scalar as text: strings as-is, numbers and booleans formatted,
/// null as empty, anything else as compact JSON.
pub(crate) fn text_of(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

pub(crate) fn str_field<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

pub(crate) fn bool_field(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

pub(crate) fn array_field<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

/// Flattens nested objects into dotted keys (`a.b`); arrays become JSON text.
pub(crate) fn flatten_into(prefix: &str, v: &Value, out: &mut BTreeMap<String, String>, map: &dyn Fn(&str) -> String) {
    match v {
        Value::Object(o) => {
            for (k, v) in o {
                let key = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten_into(&key, v, out, map);
            }
        }
        Value::String(s) => {
            out.insert(prefix.to_string(), map(s));
        }
        other => {
            out.insert(prefix.to_string(), text_of(other));
        }
    }
}

/// Percent-encodes for `application/x-www-form-urlencoded` and query strings,
/// leaving `{{variables}}` untouched.
pub(crate) fn encode_component(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while !rest.is_empty() {
        if rest.starts_with("{{")
            && let Some(end) = rest.find("}}")
        {
            out.push_str(&rest[..end + 2]);
            rest = &rest[end + 2..];
            continue;
        }
        let c = rest.chars().next().unwrap_or_default();
        if c.is_ascii_alphanumeric() || "-._~".contains(c) {
            out.push(c);
        } else {
            let mut buf = [0u8; 4];
            for b in c.encode_utf8(&mut buf).bytes() {
                let _ = write!(out, "%{b:02X}");
            }
        }
        rest = &rest[c.len_utf8()..];
    }
    out
}

pub(crate) fn form_encode(pairs: &[(String, String)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", encode_component(k), encode_component(v)))
        .collect::<Vec<_>>()
        .join("&")
}

pub(crate) fn append_query(url: &mut String, query: &str) {
    if query.is_empty() {
        return;
    }
    let sep = if url.contains('?') {
        if url.ends_with(['?', '&']) { "" } else { "&" }
    } else {
        "?"
    };
    url.push_str(sep);
    url.push_str(query);
}

pub(crate) const MULTIPART_BOUNDARY: &str = "reqowl-form-boundary";

/// A `multipart/form-data` body with text fields. Lines end in LF.
pub(crate) fn multipart(fields: &[(String, String)]) -> String {
    let mut out = String::new();
    for (k, v) in fields {
        let _ = write!(
            out,
            "--{MULTIPART_BOUNDARY}\nContent-Disposition: form-data; name=\"{}\"\n\n{v}\n",
            k.replace('"', "%22")
        );
    }
    let _ = write!(out, "--{MULTIPART_BOUNDARY}--");
    out
}

pub(crate) fn multipart_content_type() -> String {
    format!("multipart/form-data; boundary={MULTIPART_BOUNDARY}")
}

/// The `@auth basic` value; an empty password keeps the colon so the encoded
/// pair is still `user:`.
pub(crate) fn basic_auth(user: &str, pass: &str) -> String {
    if pass.is_empty() {
        format!("basic {user}:")
    } else {
        format!("basic {user} {pass}")
    }
}

/// A GraphQL block body: the query with blank lines removed (so the executor
/// can't mistake a paragraph for the variables), then the variables as JSON.
pub(crate) fn graphql_body(query: &str, variables: Option<&Value>) -> String {
    let query = query
        .replace("\r\n", "\n")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let vars = match variables {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if s.trim().is_empty() => None,
        Some(Value::String(s)) => match serde_json::from_str::<Value>(s) {
            Ok(v) => json_vars(&v),
            // Usually unquoted {{vars}}; keep the text, minus blank lines.
            Err(_) => Some(
                s.lines()
                    .filter(|l| !l.trim().is_empty())
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
        },
        Some(v) => json_vars(v),
    };
    match vars {
        Some(v) => format!("{query}\n\n{v}"),
        None => query,
    }
}

fn json_vars(v: &Value) -> Option<String> {
    match v {
        Value::Object(o) if o.is_empty() => None,
        Value::Object(_) => serde_json::to_string_pretty(v).ok(),
        _ => None,
    }
}

pub(crate) fn has_header(headers: &[Header], name: &str) -> bool {
    headers.iter().any(|h| h.name.eq_ignore_ascii_case(name))
}

pub(crate) fn header(name: &str, value: impl Into<String>) -> Header {
    Header {
        name: name.to_string(),
        value: value.into(),
    }
}

/// Replaces `:name` path segments with a value (or `{{name}}` when empty).
pub(crate) fn fill_path_params(url: &str, params: &[(String, String)]) -> String {
    if params.is_empty() {
        return url.to_string();
    }
    let split = url.find(['?', '#']).unwrap_or(url.len());
    let (path, tail) = url.split_at(split);
    let filled: Vec<String> = path
        .split('/')
        .map(|seg| {
            if let Some(name) = seg.strip_prefix(':')
                && let Some((_, v)) = params.iter().find(|(k, _)| k == name)
            {
                if v.is_empty() {
                    format!("{{{{{name}}}}}")
                } else {
                    v.clone()
                }
            } else {
                seg.to_string()
            }
        })
        .collect();
    format!("{}{tail}", filled.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names() {
        assert_eq!(sanitize_file_name(" a/b:c*?\"<>| d "), "abc d");
        assert_eq!(sanitize_file_name("..."), "Untitled");
        assert_eq!(sanitize_file_name(".hidden."), "hidden");
        let mut n = Names::default();
        assert_eq!(n.unique("Foo"), "Foo");
        assert_eq!(n.unique("foo"), "foo (2)");
        assert_eq!(n.unique("Foo"), "Foo (3)");
    }

    #[test]
    fn block_names() {
        let mut n = BlockNames::default();
        assert_eq!(n.unique("Get user by ID!"), "Get_user_by_ID");
        assert_eq!(n.unique("Get user by ID?"), "Get_user_by_ID_2");
        assert_eq!(n.unique("???"), "request");
        assert_eq!(n.unique("Müşteri"), "M_teri");
    }

    #[test]
    fn encoding_keeps_variables() {
        assert_eq!(encode_component("a b&{{x y}}/é"), "a%20b%26{{x y}}%2F%C3%A9");
        let mut u = "http://h/p".to_string();
        append_query(&mut u, "a=1");
        append_query(&mut u, "b=2");
        assert_eq!(u, "http://h/p?a=1&b=2");
    }

    #[test]
    fn fences_grow() {
        assert_eq!(fence("js", "a"), "```js\na\n```\n");
        assert_eq!(fence("js", "x ```` y\n"), "`````js\nx ```` y\n`````\n");
    }

    #[test]
    fn path_params() {
        let p = vec![("id".to_string(), "7".to_string()), ("v".to_string(), String::new())];
        assert_eq!(fill_path_params("{{b}}/u/:id/:v?x=:id", &p), "{{b}}/u/7/{{v}}?x=:id");
    }

    #[test]
    fn graphql_bodies() {
        assert_eq!(graphql_body("query {\n\n  me\n}", None), "query {\n  me\n}");
        let v = serde_json::json!({"a": 1});
        assert_eq!(graphql_body("{ x }", Some(&v)), "{ x }\n\n{\n  \"a\": 1\n}");
        assert_eq!(graphql_body("{ x }", Some(&Value::String("{}".into()))), "{ x }");
    }

    #[test]
    fn detects_formats() {
        assert_eq!(detect("  curl https://x").unwrap(), Format::Curl);
        assert_eq!(
            detect(r#"{"info":{"name":"a","schema":"https://schema.getpostman.com/json/collection/v2.1.0/collection.json"},"item":[]}"#).unwrap(),
            Format::PostmanCollection
        );
        assert_eq!(
            detect(r#"{"name":"dev","values":[],"_postman_variable_scope":"environment"}"#).unwrap(),
            Format::PostmanEnvironment
        );
        assert_eq!(
            detect(r#"{"_type":"export","__export_format":4,"resources":[]}"#).unwrap(),
            Format::InsomniaV4
        );
        assert_eq!(
            detect("type: collection.insomnia.rest/5.0\nname: x\ncollection: []\n").unwrap(),
            Format::InsomniaV5
        );
        assert!(detect("hello: world\n").is_err());
        assert!(detect("{\"a\": 1}").is_err());
    }
}
