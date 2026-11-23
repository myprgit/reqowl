//! Postman Collection v2.0 / v2.1 and Postman environment exports.
//!
//! Field shapes follow the official schemas at
//! `https://schema.postman.com/json/collection/v2.{0,1}.0/collection.json`.
//! The two versions differ mainly in auth: v2.1 stores auth attributes as
//! `[{key, value, type}]`, v2.0 as a plain object.
//!
//! Mapping: a folder becomes `Folder.md` (its description) plus `Folder/`;
//! each request becomes its own page with one block. Folder and collection
//! auth is inherited by requests set to "inherit" (or with no auth at all).

use std::collections::BTreeMap;

use serde_json::Value;

use super::{
    BlockNames, ImportedEnv, ImportedWorkspace, Node, PageDraft, append_query, array_field, basic_auth, bool_field,
    encode_component, fill_path_params, form_encode, graphql_body, has_header, header, multipart,
    multipart_content_type, str_field, text_of,
};
use crate::error::{Error, Result};
use crate::request::{Directives, Header, Kind, RequestSpec};

pub fn is_collection(v: &Value) -> bool {
    v.get("info").is_some_and(Value::is_object) && v.get("item").is_some_and(Value::is_array)
}

pub fn is_v1_collection(v: &Value) -> bool {
    v.get("requests").is_some_and(Value::is_array) && v.get("order").is_some()
}

pub fn is_environment(v: &Value) -> bool {
    v.get("values").is_some_and(Value::is_array)
        && (v.get("_postman_variable_scope").is_some() || v.get("name").is_some())
        && v.get("item").is_none()
}

/// A Postman environment (or globals) export. `type: "secret"` values go to
/// the git-ignored `.local.toml`; disabled values are dropped.
pub fn parse_environment(v: &Value) -> Result<ImportedWorkspace> {
    if !is_environment(v) {
        return Err(Error::Import("not a Postman environment export".into()));
    }
    let name = match str_field(v, "name").trim() {
        "" => "Postman environment".to_string(),
        n => n.to_string(),
    };
    let mut env = ImportedEnv {
        name: name.clone(),
        ..Default::default()
    };
    for var in array_field(v, "values") {
        let key = str_field(var, "key");
        if key.is_empty() || var.get("enabled").and_then(Value::as_bool) == Some(false) {
            continue;
        }
        let value = convert_dynamic(&text_of(var.get("value").unwrap_or(&Value::Null)));
        if str_field(var, "type") == "secret" {
            env.secrets.insert(key.to_string(), value);
        } else {
            env.vars.insert(key.to_string(), value);
        }
    }
    Ok(ImportedWorkspace {
        name,
        default_env: Some(env.name.clone()),
        envs: vec![env],
        ..Default::default()
    })
}

struct Ctx {
    names: BlockNames,
}

/// Inherited state while walking folders.
#[derive(Clone, Default)]
struct Scope<'a> {
    auth: Option<&'a Value>,
    vars: BTreeMap<String, String>,
}

pub fn parse_collection(v: &Value) -> Result<ImportedWorkspace> {
    if !is_collection(v) {
        return Err(Error::Import("not a Postman collection (v2.0/v2.1)".into()));
    }
    let info = &v["info"];
    let name = match str_field(info, "name").trim() {
        "" => "Postman collection".to_string(),
        n => n.to_string(),
    };

    let mut vars = BTreeMap::new();
    collect_variables(v, &mut vars);

    let scope = Scope {
        auth: auth_of(v),
        vars: BTreeMap::new(),
    };
    let mut ctx = Ctx {
        names: BlockNames::default(),
    };
    let mut root = Vec::new();

    // Collection-level notes and scripts have no page of their own.
    let description = description_of(info.get("description")).or_else(|| description_of(v.get("description")));
    let scripts = scripts_of(v);
    if description.is_some() || !scripts.is_empty() {
        let mut page = PageDraft::new("About");
        page.description = description.unwrap_or_default();
        page.scripts = scripts;
        root.push(Node {
            name: "About".into(),
            markdown: page.render(),
            children: vec![],
        });
    }

    for item in array_field(v, "item") {
        if let Some(node) = item_node(item, &scope, &mut ctx) {
            root.push(node);
        }
    }

    Ok(ImportedWorkspace {
        name,
        vars,
        root,
        ..Default::default()
    })
}

fn collect_variables(v: &Value, out: &mut BTreeMap<String, String>) {
    for var in array_field(v, "variable") {
        if bool_field(var, "disabled") {
            continue;
        }
        let key = match str_field(var, "key") {
            "" => str_field(var, "id"),
            k => k,
        };
        if key.is_empty() {
            continue;
        }
        out.insert(
            key.to_string(),
            convert_dynamic(&text_of(var.get("value").unwrap_or(&Value::Null))),
        );
    }
}

fn item_node<'a>(item: &'a Value, scope: &Scope<'a>, ctx: &mut Ctx) -> Option<Node> {
    let name = match str_field(item, "name").trim() {
        "" => "Untitled",
        n => n,
    };
    if let Some(children) = item.get("item").and_then(Value::as_array) {
        let mut inner = scope.clone();
        if let Some(a) = auth_of(item) {
            inner.auth = Some(a);
        }
        collect_variables(item, &mut inner.vars);

        let mut page = PageDraft::new(name);
        page.description = description_of(item.get("description")).unwrap_or_default();
        page.scripts = scripts_of(item);
        let children = children.iter().filter_map(|c| item_node(c, &inner, ctx)).collect();
        return Some(Node {
            name: name.to_string(),
            markdown: page.render(),
            children,
        });
    }
    let request = item.get("request")?;

    let mut page = PageDraft::new(name);
    page.front_matter = scope.vars.clone();
    collect_variables(item, &mut page.front_matter);
    page.description = description_of(item.get("description"))
        .or_else(|| description_of(request.get("description")))
        .unwrap_or_default();
    page.scripts = scripts_of(item);
    let mut spec = request_spec(request, scope, &mut page.notes);
    spec.directives.name = Some(ctx.names.unique(name));
    page.request = Some(spec);
    Some(Node {
        name: name.to_string(),
        markdown: page.render(),
        children: vec![],
    })
}

fn request_spec(request: &Value, scope: &Scope<'_>, notes: &mut Vec<String>) -> RequestSpec {
    // "If a string, the string is assumed to be the request URL and the method is assumed to be 'GET'."
    if let Some(url) = request.as_str() {
        return RequestSpec {
            kind: Kind::Http,
            method: "GET".into(),
            url: convert_dynamic(url),
            headers: vec![],
            body: None,
            directives: Directives {
                auth: scope.auth.and_then(|a| auth_directive(a, notes)),
                ..Default::default()
            },
        };
    }

    let method = match str_field(request, "method").trim() {
        "" => "GET".to_string(),
        m => m.to_ascii_uppercase(),
    };
    let mut url = url_of(request.get("url").unwrap_or(&Value::Null));

    let mut headers = headers_of(request.get("header"));

    let auth_value = match request.get("auth") {
        Some(a) if a.is_object() && str_field(a, "type") != "inherit" => Some(a),
        _ => scope.auth,
    };
    let auth = auth_value.and_then(|a| auth_directive(a, notes));

    let (kind, body) = body_of(request.get("body"), &mut headers, notes);
    url = convert_dynamic(&url);

    RequestSpec {
        kind,
        method,
        url,
        headers: headers
            .into_iter()
            .map(|h| Header {
                name: h.name,
                value: convert_dynamic(&h.value),
            })
            .collect(),
        body: body.map(|b| convert_dynamic(&b)),
        directives: Directives {
            auth: auth.map(|a| convert_dynamic(&a)),
            ..Default::default()
        },
    }
}

/// `url` is a string or `{raw, protocol, host, port, path, query, variable}`.
pub(crate) fn url_of(url: &Value) -> String {
    let Some(obj) = url.as_object() else {
        return text_of(url);
    };
    let mut out = match obj.get("raw").and_then(Value::as_str) {
        Some(raw) if !raw.trim().is_empty() => raw.trim().to_string(),
        _ => {
            let mut s = String::new();
            if let Some(p) = obj.get("protocol").and_then(Value::as_str) {
                s.push_str(p);
                s.push_str("://");
            }
            match obj.get("host") {
                Some(Value::Array(parts)) => s.push_str(&parts.iter().map(text_of).collect::<Vec<_>>().join(".")),
                Some(h) => s.push_str(&text_of(h)),
                None => {}
            }
            if let Some(port) = obj.get("port").map(text_of).filter(|p| !p.is_empty()) {
                s.push(':');
                s.push_str(&port);
            }
            match obj.get("path") {
                Some(Value::Array(parts)) => {
                    for p in parts {
                        s.push('/');
                        match p {
                            Value::Object(o) => s.push_str(&o.get("value").map(text_of).unwrap_or_default()),
                            other => s.push_str(&text_of(other)),
                        }
                    }
                }
                Some(Value::String(p)) if !p.is_empty() => {
                    if !p.starts_with('/') {
                        s.push('/');
                    }
                    s.push_str(p);
                }
                _ => {}
            }
            let query: Vec<String> = obj
                .get("query")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or(&[])
                .iter()
                .filter(|q| !bool_field(q, "disabled"))
                .filter_map(|q| {
                    let k = q.get("key").map(text_of).unwrap_or_default();
                    let v = q.get("value").filter(|v| !v.is_null()).map(text_of);
                    match (k.is_empty(), v) {
                        (true, None) => None,
                        (_, None) => Some(encode_component(&k)),
                        (_, Some(v)) => Some(format!("{}={}", encode_component(&k), encode_component(&v))),
                    }
                })
                .collect();
            append_query(&mut s, &query.join("&"));
            s
        }
    };
    let params: Vec<(String, String)> = obj
        .get("variable")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter(|v| !bool_field(v, "disabled"))
        .map(|v| {
            let key = match str_field(v, "key") {
                "" => str_field(v, "id"),
                k => k,
            };
            (key.to_string(), v.get("value").map(text_of).unwrap_or_default())
        })
        .filter(|(k, _)| !k.is_empty())
        .collect();
    out = fill_path_params(&out, &params);
    out
}

/// Header list, or a raw `Name: value` string. Disabled headers are skipped.
fn headers_of(v: Option<&Value>) -> Vec<Header> {
    match v {
        Some(Value::Array(list)) => list
            .iter()
            .filter(|h| !bool_field(h, "disabled"))
            .filter_map(|h| {
                let name = str_field(h, "key").trim();
                (!name.is_empty()).then(|| header(name, h.get("value").map(text_of).unwrap_or_default()))
            })
            .collect(),
        Some(Value::String(s)) => s
            .lines()
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| header(k.trim(), v.trim()))
            .filter(|h| !h.name.is_empty())
            .collect(),
        _ => vec![],
    }
}

/// Postman keeps the "enabled" state as `disabled` (v2.1) or `enabled` (older exports).
fn enabled(v: &Value) -> bool {
    !bool_field(v, "disabled") && v.get("enabled").and_then(Value::as_bool) != Some(false)
}

fn body_of(body: Option<&Value>, headers: &mut Vec<Header>, notes: &mut Vec<String>) -> (Kind, Option<String>) {
    let Some(body) = body.filter(|b| b.is_object()) else {
        return (Kind::Http, None);
    };
    if bool_field(body, "disabled") {
        return (Kind::Http, None);
    }
    let set_type = |headers: &mut Vec<Header>, ct: &str| {
        if !has_header(headers, "content-type") {
            headers.push(header("Content-Type", ct));
        }
    };
    match str_field(body, "mode") {
        "raw" => {
            let raw = str_field(body, "raw").to_string();
            let lang = body
                .pointer("/options/raw/language")
                .and_then(Value::as_str)
                .unwrap_or("");
            let ct = match lang {
                "json" => Some("application/json"),
                "xml" => Some("application/xml"),
                "html" => Some("text/html"),
                "javascript" => Some("application/javascript"),
                "text" => Some("text/plain"),
                _ => None,
            };
            if let Some(ct) = ct.filter(|_| !raw.trim().is_empty()) {
                set_type(headers, ct);
            }
            (Kind::Http, Some(raw))
        }
        "urlencoded" => {
            let pairs: Vec<(String, String)> = array_field(body, "urlencoded")
                .iter()
                .filter(|p| enabled(p))
                .map(|p| {
                    (
                        str_field(p, "key").to_string(),
                        p.get("value").map(text_of).unwrap_or_default(),
                    )
                })
                .collect();
            if pairs.is_empty() {
                return (Kind::Http, None);
            }
            set_type(headers, "application/x-www-form-urlencoded");
            (Kind::Http, Some(form_encode(&pairs)))
        }
        "formdata" => {
            let mut fields = Vec::new();
            for p in array_field(body, "formdata").iter().filter(|p| enabled(p)) {
                let key = str_field(p, "key");
                if str_field(p, "type") == "file" {
                    let src = p.get("src").map(text_of).unwrap_or_default();
                    notes.push(format!(
                        "Form field `{key}` uploads a file (`{src}`) in Postman; file fields are not imported."
                    ));
                } else {
                    fields.push((key.to_string(), p.get("value").map(text_of).unwrap_or_default()));
                }
            }
            if fields.is_empty() {
                return (Kind::Http, None);
            }
            headers.retain(|h| !h.name.eq_ignore_ascii_case("content-type"));
            headers.push(header("Content-Type", multipart_content_type()));
            (Kind::Http, Some(multipart(&fields)))
        }
        "graphql" => {
            let g = body.get("graphql").unwrap_or(&Value::Null);
            let query = str_field(g, "query");
            (Kind::GraphQl, Some(graphql_body(query, g.get("variables"))))
        }
        "file" => {
            let src = body.pointer("/file/src").map(text_of).unwrap_or_default();
            notes.push(format!(
                "The body was a file upload (`{src}`) in Postman; it is not imported."
            ));
            (Kind::Http, None)
        }
        _ => (Kind::Http, None),
    }
}

fn auth_of(v: &Value) -> Option<&Value> {
    v.get("auth")
        .filter(|a| a.is_object() && str_field(a, "type") != "inherit")
}

/// An auth attribute in v2.1 (`[{key, value}]`) or v2.0 (`{key: value}`) form.
fn auth_attr(auth: &Value, kind: &str, key: &str) -> String {
    match auth.get(kind) {
        Some(Value::Array(attrs)) => attrs
            .iter()
            .find(|a| str_field(a, "key") == key)
            .and_then(|a| a.get("value"))
            .map(text_of)
            .unwrap_or_default(),
        Some(Value::Object(o)) => o.get(key).map(text_of).unwrap_or_default(),
        _ => String::new(),
    }
}

/// The `@auth` value for a Postman auth object, or `None` for no auth.
fn auth_directive(auth: &Value, notes: &mut Vec<String>) -> Option<String> {
    let kind = str_field(auth, "type");
    match kind {
        "" | "noauth" | "inherit" => None,
        "bearer" => {
            let token = auth_attr(auth, "bearer", "token");
            (!token.is_empty()).then(|| format!("bearer {token}"))
        }
        "basic" => {
            let user = auth_attr(auth, "basic", "username");
            let pass = auth_attr(auth, "basic", "password");
            Some(basic_auth(&user, &pass))
        }
        "apikey" => {
            let key = auth_attr(auth, "apikey", "key");
            let value = auth_attr(auth, "apikey", "value");
            let place = auth_attr(auth, "apikey", "in");
            if place == "query" {
                notes.push(format!(
                    "The API key was sent as a query parameter in Postman; add `{key}={value}` to the URL if you need it."
                ));
                None
            } else if key.is_empty() || key.contains(char::is_whitespace) {
                notes.push(format!("API key header name `{key}` is not usable; auth not imported."));
                None
            } else {
                Some(format!("apikey {key} {value}"))
            }
        }
        other => {
            notes.push(format!(
                "Postman `{other}` auth is not supported by reqowl and was not imported."
            ));
            None
        }
    }
}

/// `event` scripts as (heading, code) pairs.
fn scripts_of(v: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for ev in array_field(v, "event") {
        let Some(script) = ev.get("script") else { continue };
        let code = match script.get("exec") {
            Some(Value::Array(lines)) => lines.iter().map(text_of).collect::<Vec<_>>().join("\n"),
            Some(Value::String(s)) => s.clone(),
            _ => continue,
        };
        if code.trim().is_empty() {
            continue;
        }
        let when = match str_field(ev, "listen") {
            "prerequest" => "pre-request script",
            "test" => "test script",
            "" => "script",
            other => other,
        };
        out.push((
            format!("**Postman script not imported** ({when}). Kept for reference; reqowl does not run it."),
            code,
        ));
    }
    out
}

/// `description` is a string, `{content, type}`, or null.
fn description_of(v: Option<&Value>) -> Option<String> {
    let s = match v? {
        Value::String(s) => s.clone(),
        Value::Object(o) => o.get("content").and_then(Value::as_str)?.to_string(),
        _ => return None,
    };
    (!s.trim().is_empty()).then_some(s)
}

/// Postman dynamic variables with a reqowl equivalent.
fn convert_dynamic(s: &str) -> String {
    if !s.contains("{{$") {
        return s.to_string();
    }
    s.replace("{{$guid}}", "{{$uuid}}")
        .replace("{{$randomUUID}}", "{{$uuid}}")
        .replace("{{$randomInt}}", "{{$randomInt 0 1000}}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn url_forms() {
        assert_eq!(url_of(&json!("http://h/x")), "http://h/x");
        assert_eq!(
            url_of(&json!({"raw": "{{base}}/u/:id", "variable": [{"key": "id", "value": "7"}]})),
            "{{base}}/u/7"
        );
        let u = json!({
            "protocol": "https", "host": ["api", "example", "com"], "port": "8443",
            "path": ["v1", "users", ":id"],
            "query": [{"key": "a", "value": "1 2"}, {"key": "off", "value": "x", "disabled": true}, {"key": "flag", "value": null}],
            "variable": [{"key": "id", "value": ""}]
        });
        assert_eq!(url_of(&u), "https://api.example.com:8443/v1/users/{{id}}?a=1%202&flag");
    }

    #[test]
    fn v20_and_v21_auth() {
        let mut notes = vec![];
        let v21 =
            json!({"type": "basic", "basic": [{"key": "username", "value": "u"}, {"key": "password", "value": "p"}]});
        assert_eq!(auth_directive(&v21, &mut notes).as_deref(), Some("basic u p"));
        let v20 = json!({"type": "bearer", "bearer": {"token": "t"}});
        assert_eq!(auth_directive(&v20, &mut notes).as_deref(), Some("bearer t"));
        let q = json!({"type": "apikey", "apikey": [{"key": "in", "value": "query"}, {"key": "key", "value": "k"}]});
        assert_eq!(auth_directive(&q, &mut notes), None);
        let d = json!({"type": "digest", "digest": []});
        assert_eq!(auth_directive(&d, &mut notes), None);
        assert_eq!(notes.len(), 2);
    }

    #[test]
    fn string_request_and_headers() {
        let col = json!({"info": {"name": "c"}, "item": [{"name": "Ping", "request": "http://h/ping"},
            {"name": "Raw headers", "request": {"method": "get", "url": "http://h", "header": "A: 1\nB: two"}}]});
        let ws = parse_collection(&col).unwrap();
        assert_eq!(ws.root.len(), 2);
        assert!(ws.root[0].markdown.contains("GET http://h/ping"));
        assert!(ws.root[1].markdown.contains("A: 1\nB: two"));
    }

    #[test]
    fn environment() {
        let env = json!({"name": "Dev", "_postman_variable_scope": "environment", "values": [
            {"key": "base", "value": "http://dev", "type": "default", "enabled": true},
            {"key": "token", "value": "s", "type": "secret", "enabled": true},
            {"key": "off", "value": "x", "enabled": false},
            {"key": "n", "value": 3}
        ]});
        let ws = parse_environment(&env).unwrap();
        let e = &ws.envs[0];
        assert_eq!(e.vars.len(), 2);
        assert_eq!(e.vars["n"], "3");
        assert_eq!(e.secrets["token"], "s");
        assert_eq!(ws.default_env.as_deref(), Some("Dev"));
    }
}
