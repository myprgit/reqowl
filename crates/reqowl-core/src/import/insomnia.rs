//! Insomnia exports: v4 (`_type: export`, a flat `resources` list linked by
//! `parentId`, JSON or YAML) and v5 (the YAML files Insomnia 10+ writes,
//! `type: collection.insomnia.rest/5.0`, with a nested `collection` tree).
//!
//! v5 field names follow Kong/insomnia `packages/insomnia/src/common/import-v5-parser.ts`.
//! v4 resources are first arranged into the same nested shape, so one walker
//! handles both.
//!
//! Template tags: `{{ _.name }}`, `{{ _['name'] }}` and `{{ name }}` become
//! `{{name}}`. `{% ... %}` tags (`response`, `faker`, ...) have no reqowl
//! equivalent; they are left as written and listed in a note on the page.

use std::collections::BTreeMap;

use serde_json::Value;

use super::{
    BlockNames, ImportedEnv, ImportedWorkspace, Node, PageDraft, append_query, array_field, basic_auth, bool_field,
    encode_component, fill_path_params, flatten_into, form_encode, graphql_body, has_header, header, multipart,
    multipart_content_type, str_field, text_of,
};
use crate::error::{Error, Result};
use crate::request::{Directives, Header, Kind, RequestSpec};

const V5_TYPES: &[&str] = &[
    "collection.insomnia.rest/5",
    "spec.insomnia.rest/5",
    "environment.insomnia.rest/5",
];

pub fn is_v4(v: &Value) -> bool {
    str_field(v, "_type") == "export" && v.get("resources").is_some_and(Value::is_array)
}

pub fn is_v5(v: &Value) -> bool {
    let t = str_field(v, "type");
    V5_TYPES.iter().any(|p| t.starts_with(p))
}

/// Inherited state while walking folders.
#[derive(Clone, Default)]
struct Scope {
    auth: Option<Value>,
    headers: Vec<Header>,
    vars: BTreeMap<String, String>,
}

struct Ctx {
    names: BlockNames,
}

pub fn parse_v5(v: &Value) -> Result<ImportedWorkspace> {
    if !is_v5(v) {
        return Err(Error::Import(format!(
            "unsupported Insomnia v5 file type `{}`; expected a collection, design document or environment",
            str_field(v, "type")
        )));
    }
    let name = match str_field(v, "name").trim() {
        "" => "Insomnia import".to_string(),
        n => n.to_string(),
    };
    let mut ws = ImportedWorkspace {
        name,
        ..Default::default()
    };
    if let Some(envs) = v.get("environments").filter(|e| e.is_object()) {
        if str_field(v, "type").starts_with("environment.") {
            // A standalone environment file: its base environment is an environment too.
            let mut base = envs.clone();
            if str_field(&base, "name").trim().is_empty() {
                base["name"] = Value::String(ws.name.clone());
            }
            add_sub_env(&mut ws, &base);
        } else {
            add_base_env(&mut ws, envs);
        }
        for sub in array_field(envs, "subEnvironments") {
            add_sub_env(&mut ws, sub);
        }
    }
    ws.default_env = ws.envs.first().map(|e| e.name.clone());

    let mut tags = Vec::new();
    let scope = Scope {
        auth: own_auth(v.get("authentication")).cloned(),
        headers: headers_of(v, &mut tags),
        vars: BTreeMap::new(),
    };
    if let Some(about) = about_page(v.pointer("/meta/description"), &scripts_of(v), &tags) {
        ws.root.push(about);
    }
    let mut ctx = Ctx {
        names: BlockNames::default(),
    };
    ws.root.extend(walk(array_field(v, "collection"), &scope, &mut ctx));
    Ok(ws)
}

pub fn parse_v4(v: &Value) -> Result<ImportedWorkspace> {
    if !is_v4(v) {
        return Err(Error::Import("not an Insomnia v4 export".into()));
    }
    let resources = array_field(v, "resources");
    let id = |r: &Value| str_field(r, "_id").to_string();
    let of_type = |t: &'static str| resources.iter().filter(move |r| str_field(r, "_type") == t);

    let workspaces: Vec<&Value> = of_type("workspace").collect();
    let group_ids: Vec<String> = of_type("request_group").map(id).collect();
    let env_ids: Vec<String> = of_type("environment").map(id).collect();

    let mut ws = ImportedWorkspace {
        name: workspaces
            .first()
            .map(|w| str_field(w, "name").trim())
            .filter(|n| !n.is_empty())
            .unwrap_or("Insomnia import")
            .to_string(),
        ..Default::default()
    };

    // Base environments hang off a workspace, sub-environments off a base.
    for env in of_type("environment") {
        if env_ids.iter().any(|i| i == str_field(env, "parentId")) {
            add_sub_env(&mut ws, env);
        } else {
            add_base_env(&mut ws, env);
        }
    }
    ws.default_env = ws.envs.first().map(|e| e.name.clone());

    let is_item = |r: &Value| {
        matches!(
            str_field(r, "_type"),
            "request" | "request_group" | "websocket_request" | "grpc_request"
        )
    };
    // Arrange resources into v5-style nested `children`.
    fn nest(resources: &[Value], parent: &str, is_item: &dyn Fn(&Value) -> bool) -> Vec<Value> {
        let mut items: Vec<&Value> = resources
            .iter()
            .filter(|r| is_item(r) && str_field(r, "parentId") == parent)
            .collect();
        items.sort_by(|a, b| sort_key(a).total_cmp(&sort_key(b)));
        items
            .into_iter()
            .map(|r| {
                let mut r = r.clone();
                if str_field(&r, "_type") == "request_group" {
                    let children = nest(resources, str_field(&r, "_id"), is_item);
                    r["children"] = Value::Array(children);
                }
                r
            })
            .collect()
    }

    let mut tops: Vec<&str> = Vec::new();
    for r in resources.iter().filter(|r| is_item(r)) {
        let p = str_field(r, "parentId");
        if !group_ids.iter().any(|g| g == p) && !tops.contains(&p) {
            tops.push(p);
        }
    }

    let mut ctx = Ctx {
        names: BlockNames::default(),
    };
    let scope = Scope::default();
    let desc = workspaces.first().and_then(|w| w.get("description"));
    if let Some(about) = about_page(desc, &[], &[]) {
        ws.root.push(about);
    }
    if tops.len() > 1 {
        // Several workspaces (or orphans): one top-level folder each.
        for parent in tops {
            let wname = workspaces
                .iter()
                .find(|w| str_field(w, "_id") == parent)
                .map(|w| str_field(w, "name").to_string())
                .unwrap_or_else(|| "Imported".into());
            let children = walk(&nest(resources, parent, &is_item), &scope, &mut ctx);
            ws.root.push(Node {
                markdown: PageDraft::new(&wname).render(),
                name: wname,
                children,
            });
        }
    } else if let Some(parent) = tops.first() {
        ws.root
            .extend(walk(&nest(resources, parent, &is_item), &scope, &mut ctx));
    }
    Ok(ws)
}

fn sort_key(v: &Value) -> f64 {
    v.get("metaSortKey")
        .or_else(|| v.pointer("/meta/sortKey"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
}

fn about_page(description: Option<&Value>, scripts: &[(String, String)], tags: &[String]) -> Option<Node> {
    let desc = description.and_then(Value::as_str).unwrap_or("").trim();
    if desc.is_empty() && scripts.is_empty() && tags.is_empty() {
        return None;
    }
    let mut page = PageDraft::new("About");
    page.description = desc.to_string();
    page.scripts = scripts.to_vec();
    tag_note(&mut page.notes, tags);
    Some(Node {
        name: "About".into(),
        markdown: page.render(),
        children: vec![],
    })
}

fn add_base_env(ws: &mut ImportedWorkspace, env: &Value) {
    if let Some(data) = env.get("data") {
        flatten_into("", data, &mut ws.vars, &|s| convert_template(s, &mut Vec::new()));
    }
}

fn add_sub_env(ws: &mut ImportedWorkspace, env: &Value) {
    let name = match str_field(env, "name").trim() {
        "" => "env".to_string(),
        n => n.to_string(),
    };
    let mut vars = BTreeMap::new();
    if let Some(data) = env.get("data") {
        flatten_into("", data, &mut vars, &|s| convert_template(s, &mut Vec::new()));
    }
    // Private environments are not synced by Insomnia; keep them out of git too.
    let private = bool_field(env, "isPrivate") || env.pointer("/meta/isPrivate").and_then(Value::as_bool) == Some(true);
    let mut out = ImportedEnv {
        name,
        ..Default::default()
    };
    if private {
        out.secrets = vars;
    } else {
        out.vars = vars;
    }
    ws.envs.push(out);
}

enum ItemKind {
    Group,
    Request,
    WebSocket,
    Other(&'static str),
}

fn classify(v: &Value) -> ItemKind {
    match str_field(v, "_type") {
        "request_group" => return ItemKind::Group,
        "request" => return ItemKind::Request,
        "websocket_request" => return ItemKind::WebSocket,
        "grpc_request" => return ItemKind::Other("gRPC"),
        _ => {}
    }
    if v.get("children").is_some_and(Value::is_array) {
        return ItemKind::Group;
    }
    let id = v.pointer("/meta/id").and_then(Value::as_str).unwrap_or("");
    if id.starts_with("fld_") {
        ItemKind::Group
    } else if id.starts_with("ws-req") {
        ItemKind::WebSocket
    } else if id.starts_with("socketio-req") {
        ItemKind::Other("Socket.IO")
    } else if id.starts_with("greq") {
        ItemKind::Other("gRPC")
    } else if id.starts_with("mcp") {
        ItemKind::Other("MCP")
    } else if v.get("method").is_some() {
        ItemKind::Request
    } else {
        ItemKind::Other("unknown")
    }
}

fn walk(items: &[Value], scope: &Scope, ctx: &mut Ctx) -> Vec<Node> {
    items.iter().map(|item| item_node(item, scope, ctx)).collect()
}

fn item_node(item: &Value, scope: &Scope, ctx: &mut Ctx) -> Node {
    let name = match str_field(item, "name").trim() {
        "" => "Untitled".to_string(),
        n => n.to_string(),
    };
    let mut page = PageDraft::new(&name);
    page.description = description_of(item);
    page.scripts = scripts_of(item);
    let mut tags = Vec::new();

    match classify(item) {
        ItemKind::Group => {
            let mut inner = scope.clone();
            if let Some(a) = own_auth(item.get("authentication")) {
                inner.auth = Some(a.clone());
            }
            let own = headers_of(item, &mut tags);
            inner.headers = merge_headers(&inner.headers, own);
            if let Some(envdata) = item.get("environment") {
                flatten_into("", envdata, &mut inner.vars, &|s| convert_template(s, &mut Vec::new()));
            }
            tag_note(&mut page.notes, &tags);
            let children = walk(array_field(item, "children"), &inner, ctx);
            Node {
                name,
                markdown: page.render(),
                children,
            }
        }
        ItemKind::Other(kind) => {
            page.notes.push(format!(
                "Insomnia {kind} request; reqowl can't run these, so it was not imported."
            ));
            let url = str_field(item, "url");
            if !url.is_empty() {
                page.notes.push(format!("URL: `{url}`"));
            }
            Node {
                name,
                markdown: page.render(),
                children: vec![],
            }
        }
        kind @ (ItemKind::Request | ItemKind::WebSocket) => {
            page.front_matter = scope.vars.clone();
            let mut spec = request_spec(item, scope, &mut page.notes, &mut tags);
            if matches!(kind, ItemKind::WebSocket) {
                spec.kind = Kind::WebSocket;
                spec.method = "GET".into();
                spec.body = None;
            }
            spec.directives.name = Some(ctx.names.unique(&name));
            tag_note(&mut page.notes, &tags);
            page.request = Some(spec);
            Node {
                name,
                markdown: page.render(),
                children: vec![],
            }
        }
    }
}

fn request_spec(item: &Value, scope: &Scope, notes: &mut Vec<String>, tags: &mut Vec<String>) -> RequestSpec {
    let mut url = convert_template(str_field(item, "url").trim(), tags);
    let path_params: Vec<(String, String)> = array_field(item, "pathParameters")
        .iter()
        .map(|p| {
            (
                str_field(p, "name").to_string(),
                convert_template(&text_of(p.get("value").unwrap_or(&Value::Null)), tags),
            )
        })
        .filter(|(k, _)| !k.is_empty())
        .collect();
    url = fill_path_params(&url, &path_params);

    let query: Vec<String> = array_field(item, "parameters")
        .iter()
        .filter(|p| !bool_field(p, "disabled"))
        .filter(|p| !str_field(p, "name").is_empty())
        .map(|p| {
            let k = encode_component(&convert_template(str_field(p, "name"), tags));
            let v = encode_component(&convert_template(
                &text_of(p.get("value").unwrap_or(&Value::Null)),
                tags,
            ));
            format!("{k}={v}")
        })
        .collect();
    append_query(&mut url, &query.join("&"));

    let own = headers_of(item, tags);
    let mut headers = merge_headers(&scope.headers, own);

    let auth_value = own_auth(item.get("authentication")).or(scope.auth.as_ref());
    let auth = auth_value.and_then(|a| auth_directive(a, &mut headers, notes, tags));

    let (kind, body) = body_of(item.get("body"), &mut headers, notes, tags);

    let follow = item
        .pointer("/settings/followRedirects")
        .or_else(|| item.get("settingFollowRedirects"))
        .and_then(Value::as_str);

    let method = match str_field(item, "method").trim() {
        "" => "GET".to_string(),
        m => m.to_ascii_uppercase(),
    };
    RequestSpec {
        kind,
        method,
        url,
        headers,
        body,
        directives: Directives {
            auth,
            no_redirect: follow == Some("off"),
            ..Default::default()
        },
    }
}

/// Request headers win over inherited ones with the same name.
fn merge_headers(inherited: &[Header], own: Vec<Header>) -> Vec<Header> {
    let mut out: Vec<Header> = inherited
        .iter()
        .filter(|h| !own.iter().any(|o| o.name.eq_ignore_ascii_case(&h.name)))
        .cloned()
        .collect();
    out.extend(own);
    out
}

fn headers_of(v: &Value, tags: &mut Vec<String>) -> Vec<Header> {
    array_field(v, "headers")
        .iter()
        .filter(|h| !bool_field(h, "disabled"))
        .filter(|h| !str_field(h, "name").trim().is_empty())
        .map(|h| {
            header(
                &convert_template(str_field(h, "name").trim(), tags),
                convert_template(&text_of(h.get("value").unwrap_or(&Value::Null)), tags),
            )
        })
        .collect()
}

/// An `authentication` object that is set. `{}` (and a missing field) means
/// "inherit from the parent folder".
fn own_auth(v: Option<&Value>) -> Option<&Value> {
    v.filter(|a| !str_field(a, "type").is_empty())
}

fn auth_directive(
    auth: &Value,
    headers: &mut Vec<Header>,
    notes: &mut Vec<String>,
    tags: &mut Vec<String>,
) -> Option<String> {
    if bool_field(auth, "disabled") {
        return None;
    }
    let mut field = |k: &str| convert_template(&text_of(auth.get(k).unwrap_or(&Value::Null)), tags);
    match str_field(auth, "type") {
        "" | "none" => None,
        "basic" => {
            let user = field("username");
            let pass = field("password");
            Some(basic_auth(&user, &pass))
        }
        "bearer" => {
            let token = field("token");
            let prefix = field("prefix");
            let prefix = prefix.trim();
            if token.is_empty() {
                None
            } else if prefix.is_empty() || prefix.eq_ignore_ascii_case("bearer") {
                Some(format!("bearer {token}"))
            } else {
                headers.retain(|h| !h.name.eq_ignore_ascii_case("authorization"));
                headers.push(header("Authorization", format!("{prefix} {token}")));
                None
            }
        }
        "apikey" => {
            let key = field("key");
            let value = field("value");
            match str_field(auth, "addTo") {
                "queryParams" => {
                    notes.push(format!(
                        "The API key was sent as a query parameter in Insomnia; add `{key}={value}` to the URL if you need it."
                    ));
                    None
                }
                "cookie" => {
                    headers.push(header("Cookie", format!("{key}={value}")));
                    None
                }
                _ if key.is_empty() || key.contains(char::is_whitespace) => {
                    notes.push(format!("API key header name `{key}` is not usable; auth not imported."));
                    None
                }
                _ => Some(format!("apikey {key} {value}")),
            }
        }
        other => {
            notes.push(format!(
                "Insomnia `{other}` auth is not supported by reqowl and was not imported."
            ));
            None
        }
    }
}

fn body_of(
    body: Option<&Value>,
    headers: &mut Vec<Header>,
    notes: &mut Vec<String>,
    tags: &mut Vec<String>,
) -> (Kind, Option<String>) {
    let Some(body) = body.filter(|b| b.is_object()) else {
        return (Kind::Http, None);
    };
    let mime = str_field(body, "mimeType").trim();
    let params = |tags: &mut Vec<String>, notes: &mut Vec<String>| -> Vec<(String, String)> {
        let mut out = Vec::new();
        for p in array_field(body, "params")
            .iter()
            .filter(|p| !bool_field(p, "disabled"))
        {
            let name = convert_template(str_field(p, "name"), tags);
            if str_field(p, "type") == "file" {
                notes.push(format!(
                    "Form field `{name}` uploads a file (`{}`) in Insomnia; file fields are not imported.",
                    str_field(p, "fileName")
                ));
                continue;
            }
            out.push((
                name,
                convert_template(&text_of(p.get("value").unwrap_or(&Value::Null)), tags),
            ));
        }
        out
    };
    let set_type = |headers: &mut Vec<Header>, ct: &str| {
        if !has_header(headers, "content-type") {
            headers.push(header("Content-Type", ct));
        }
    };
    match mime {
        "application/x-www-form-urlencoded" => {
            let pairs = params(tags, notes);
            if pairs.is_empty() {
                return (Kind::Http, None);
            }
            set_type(headers, mime);
            (Kind::Http, Some(form_encode(&pairs)))
        }
        "multipart/form-data" => {
            let fields = params(tags, notes);
            if fields.is_empty() {
                return (Kind::Http, None);
            }
            headers.retain(|h| !h.name.eq_ignore_ascii_case("content-type"));
            headers.push(header("Content-Type", multipart_content_type()));
            (Kind::Http, Some(multipart(&fields)))
        }
        "application/graphql" => {
            let text = convert_template(str_field(body, "text"), tags);
            let body = match serde_json::from_str::<Value>(&text) {
                Ok(v) if v.is_object() => graphql_body(str_field(&v, "query"), v.get("variables")),
                _ => graphql_body(&text, None),
            };
            (Kind::GraphQl, Some(body))
        }
        _ if !str_field(body, "fileName").is_empty() => {
            notes.push(format!(
                "The body was a file (`{}`) in Insomnia; it is not imported.",
                str_field(body, "fileName")
            ));
            (Kind::Http, None)
        }
        _ => {
            let text = convert_template(str_field(body, "text"), tags);
            if text.trim().is_empty() {
                return (Kind::Http, None);
            }
            if !mime.is_empty() {
                set_type(headers, mime);
            }
            (Kind::Http, Some(text))
        }
    }
}

fn description_of(v: &Value) -> String {
    v.pointer("/meta/description")
        .or_else(|| v.get("description"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// v5 `scripts: {preRequest, afterResponse}`; v4 `preRequestScript`, `afterResponseScript`.
fn scripts_of(v: &Value) -> Vec<(String, String)> {
    let pairs = [
        ("pre-request script", "/scripts/preRequest", "preRequestScript"),
        ("after-response script", "/scripts/afterResponse", "afterResponseScript"),
    ];
    pairs
        .iter()
        .filter_map(|(label, v5, v4)| {
            let code = v.pointer(v5).or_else(|| v.get(*v4)).and_then(Value::as_str)?;
            (!code.trim().is_empty()).then(|| {
                (
                    format!("**Insomnia script not imported** ({label}). Kept for reference; reqowl does not run it."),
                    code.to_string(),
                )
            })
        })
        .collect()
}

fn tag_note(notes: &mut Vec<String>, tags: &[String]) {
    let mut uniq: Vec<&String> = Vec::new();
    for t in tags {
        if !uniq.contains(&t) {
            uniq.push(t);
        }
    }
    if !uniq.is_empty() {
        let list = uniq.iter().map(|t| format!("`{t}`")).collect::<Vec<_>>().join(", ");
        notes.push(format!(
            "Uses Insomnia template tags reqowl can't evaluate; they are left as written: {list}"
        ));
    }
}

/// Rewrites Insomnia (Nunjucks) variables to reqowl's `{{name}}`; records
/// anything it can't translate in `tags`.
pub(crate) fn convert_template(s: &str, tags: &mut Vec<String>) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    loop {
        let next = match (rest.find("{{"), rest.find("{%")) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => break,
        };
        out.push_str(&rest[..next]);
        let tail = &rest[next..];
        let is_var = tail.starts_with("{{");
        let close = if is_var { "}}" } else { "%}" };
        let Some(end) = tail[2..].find(close).map(|e| e + 2) else {
            out.push_str(tail);
            return out;
        };
        let raw = &tail[..end + 2];
        let inner = tail[2..end].trim();
        if is_var {
            match simple_var(inner) {
                Some(name) => {
                    out.push_str("{{");
                    out.push_str(&name);
                    out.push_str("}}");
                }
                None => {
                    out.push_str(raw);
                    tags.push(raw.to_string());
                }
            }
        } else if inner.split_whitespace().next() == Some("uuid") {
            out.push_str("{{$uuid}}");
        } else {
            out.push_str(raw);
            tags.push(raw.to_string());
        }
        rest = &tail[end + 2..];
    }
    out.push_str(rest);
    out
}

fn simple_var(inner: &str) -> Option<String> {
    let e = inner.strip_prefix("_.").unwrap_or(inner);
    let e = match e.strip_prefix("_[").and_then(|x| x.strip_suffix(']')) {
        Some(q) => q
            .trim()
            .strip_prefix(['\'', '"'])
            .and_then(|x| x.strip_suffix(['\'', '"']))?,
        None => e,
    };
    let ok = !e.is_empty()
        && e != "_"
        && e.chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '$'));
    ok.then(|| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standalone_environment_file() {
        let y = "type: environment.insomnia.rest/5.0\nname: Dev Environment\nenvironments:\n  name: Dev\n  data:\n    base_url: http://localhost:8000\n  subEnvironments:\n    - name: Mine\n      meta:\n        id: env_1\n        isPrivate: true\n      data:\n        token: t\n";
        let ws = parse_v5(&serde_yaml_ng::from_str(y).unwrap()).unwrap();
        assert!(ws.vars.is_empty());
        assert_eq!(ws.envs[0].name, "Dev");
        assert_eq!(ws.envs[0].vars["base_url"], "http://localhost:8000");
        assert_eq!(ws.envs[1].secrets["token"], "t");
        assert_eq!(ws.default_env.as_deref(), Some("Dev"));
    }

    #[test]
    fn templates() {
        let mut tags = vec![];
        assert_eq!(
            convert_template(
                "{{ _.base_url }}/a/{{_['test-value']}}/{{ host }}/{{ _.obj.key }}",
                &mut tags
            ),
            "{{base_url}}/a/{{test-value}}/{{host}}/{{obj.key}}"
        );
        assert!(tags.is_empty());
        let r = convert_template(
            "Bearer {% response 'body', 'req_1', '$.token' %} {% uuid 'v4' %} {{ a | upper }}",
            &mut tags,
        );
        assert_eq!(
            r,
            "Bearer {% response 'body', 'req_1', '$.token' %} {{$uuid}} {{ a | upper }}"
        );
        assert_eq!(tags.len(), 2);
        assert_eq!(convert_template("{{ open", &mut tags), "{{ open");
    }
}
