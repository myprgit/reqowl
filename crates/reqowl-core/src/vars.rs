//! `{{variable}}` substitution.
//!
//! A name is looked up in the layers in order (page front matter, then the
//! environment, then workspace defaults). Three other forms exist:
//!
//! - `{{login.response.body.$.token}}`: JSONPath into a named block's last response.
//!   `body`, `status` and `headers.Name` work too.
//! - `{{$uuid}}`, `{{$timestamp}}`, `{{$isoTimestamp}}`, `{{$randomInt 1 100}}`.
//! - A value may itself contain `{{...}}`; it is expanded recursively.

use std::collections::{BTreeMap, HashMap};

use rand::Rng;
use serde_json::Value;
use serde_json_path::JsonPath;

use crate::error::{Error, Result};

const MAX_DEPTH: usize = 8;

#[derive(Debug, Clone, Default)]
pub struct Layer {
    /// Shown to the user, e.g. `page`, `env dev`, `workspace`.
    pub label: String,
    pub vars: BTreeMap<String, String>,
    /// Values from secret storage. Used to mask history and pinned responses.
    pub secret: bool,
}

/// The parts of a response that later blocks can reference.
#[derive(Debug, Clone)]
pub struct Recorded {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Recorded {
    fn json(&self) -> Result<Value> {
        serde_json::from_str(&self.body).map_err(|e| Error::Variable(format!("response body is not JSON: {e}")))
    }
}

#[derive(Debug, Clone, Default)]
pub struct Context {
    /// Highest precedence first.
    pub layers: Vec<Layer>,
    /// Last response of each named block.
    pub responses: HashMap<String, Recorded>,
}

impl Context {
    pub fn record(&mut self, name: &str, r: Recorded) {
        self.responses.insert(name.to_string(), r);
    }

    /// Every value that came from a secret layer, with its name.
    pub fn secret_values(&self) -> Vec<(&str, &str)> {
        self.layers
            .iter()
            .filter(|l| l.secret)
            .flat_map(|l| l.vars.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .filter(|(_, v)| !v.is_empty())
            .collect()
    }

    /// Expands a template. Undefined names are collected into one error.
    pub fn render(&self, template: &str) -> Result<String> {
        let mut missing = Vec::new();
        let out = self.render_inner(template, 0, &mut missing)?;
        if missing.is_empty() {
            Ok(out)
        } else {
            Err(Error::UndefinedVariables(missing))
        }
    }

    fn render_inner(&self, template: &str, depth: usize, missing: &mut Vec<String>) -> Result<String> {
        if depth > MAX_DEPTH {
            return Err(Error::Variable(format!(
                "variables nest more than {MAX_DEPTH} levels deep (a cycle?)"
            )));
        }
        let mut out = String::with_capacity(template.len());
        let mut rest = template;
        while let Some(start) = rest.find("{{") {
            out.push_str(&rest[..start]);
            let after = &rest[start + 2..];
            let Some(end) = after.find("}}") else {
                out.push_str(&rest[start..]);
                return Ok(out);
            };
            let expr = after[..end].trim();
            match self.lookup(expr)? {
                // Only user-defined values are templates; responses and built-ins are literal.
                Some((v, true)) => out.push_str(&self.render_inner(&v, depth + 1, missing)?),
                Some((v, false)) => out.push_str(&v),
                None => {
                    if !missing.iter().any(|m| m == expr) {
                        missing.push(expr.to_string());
                    }
                }
            }
            rest = &after[end + 2..];
        }
        out.push_str(rest);
        Ok(out)
    }

    /// The value and whether it may itself contain `{{...}}` to expand.
    fn lookup(&self, expr: &str) -> Result<Option<(String, bool)>> {
        if let Some(b) = expr.strip_prefix('$') {
            return builtin(b).map(|v| Some((v, false)));
        }
        if let Some((name, path)) = expr.split_once(".response.") {
            return self.response_value(name, path).map(|v| Some((v, false)));
        }
        Ok(self
            .layers
            .iter()
            .find_map(|l| l.vars.get(expr).map(|v| (v.clone(), true))))
    }

    fn response_value(&self, name: &str, path: &str) -> Result<String> {
        let r = self
            .responses
            .get(name)
            .ok_or_else(|| Error::Variable(format!("`{name}` has no response yet; run that block first")))?;
        if path == "status" {
            return Ok(r.status.to_string());
        }
        if path == "body" {
            return Ok(r.body.clone());
        }
        if let Some(h) = path.strip_prefix("headers.") {
            return r
                .headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(h))
                .map(|(_, v)| v.clone())
                .ok_or_else(|| Error::Variable(format!("`{name}` response has no `{h}` header")));
        }
        if let Some(q) = path.strip_prefix("body.") {
            let q = if q.starts_with('$') {
                q.to_string()
            } else {
                format!("$.{q}")
            };
            let jp = JsonPath::parse(&q).map_err(|e| Error::Variable(format!("bad JSONPath `{q}`: {e}")))?;
            let json = r.json()?;
            let nodes = jp.query(&json).all();
            return match nodes.as_slice() {
                [] => Err(Error::Variable(format!("`{q}` matched nothing in `{name}` response"))),
                [Value::String(s)] => Ok(s.clone()),
                [v] => Ok(v.to_string()),
                many => Ok(Value::Array(many.iter().map(|v| (*v).clone()).collect()).to_string()),
            };
        }
        Err(Error::Variable(format!(
            "`{name}.response.{path}`: expected `status`, `body`, `body.$.path` or `headers.Name`"
        )))
    }
}

fn builtin(expr: &str) -> Result<String> {
    let mut parts = expr.split_whitespace();
    let name = parts.next().unwrap_or("");
    let now = time::OffsetDateTime::now_utc();
    match name {
        "uuid" | "guid" => Ok(uuid::Uuid::new_v4().to_string()),
        "timestamp" => Ok(now.unix_timestamp().to_string()),
        "isoTimestamp" => now
            .format(&time::format_description::well_known::Rfc3339)
            .map_err(|e| Error::Variable(e.to_string())),
        "randomInt" => {
            let nums: Vec<i64> = parts
                .map(str::parse)
                .collect::<std::result::Result<_, _>>()
                .map_err(|_| Error::Variable("$randomInt takes two integers: {{$randomInt MIN MAX}}".into()))?;
            let (lo, hi) = match nums.as_slice() {
                [] => (0, 1000),
                [lo, hi] if lo < hi => (*lo, *hi),
                _ => return Err(Error::Variable("$randomInt takes MIN MAX with MIN < MAX".into())),
            };
            Ok(rand::rng().random_range(lo..hi).to_string())
        }
        _ => Err(Error::Variable(format!(
            "unknown built-in `${name}`; known: $uuid, $timestamp, $isoTimestamp, $randomInt"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(label: &str, kv: &[(&str, &str)]) -> Layer {
        Layer {
            label: label.into(),
            vars: kv.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            secret: false,
        }
    }

    fn ctx() -> Context {
        Context {
            layers: vec![
                layer("page", &[("base", "{{host}}/v1")]),
                layer(
                    "env",
                    &[("host", "http://h"), ("base", "ignored"), ("loop", "{{loop}}")],
                ),
            ],
            responses: HashMap::new(),
        }
    }

    #[test]
    fn precedence_and_nesting() {
        assert_eq!(ctx().render("{{ base }}/x").unwrap(), "http://h/v1/x");
        assert_eq!(
            ctx().render("no vars {{ unterminated").unwrap(),
            "no vars {{ unterminated"
        );
    }

    #[test]
    fn undefined_collected() {
        match ctx().render("{{a}} {{b}} {{a}}") {
            Err(Error::UndefinedVariables(v)) => assert_eq!(v, ["a", "b"]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn cycle_detected() {
        assert!(matches!(ctx().render("{{loop}}"), Err(Error::Variable(_))));
    }

    #[test]
    fn builtins() {
        let c = ctx();
        assert_eq!(c.render("{{$uuid}}").unwrap().len(), 36);
        let n: i64 = c.render("{{$randomInt 5 6}}").unwrap().parse().unwrap();
        assert_eq!(n, 5);
        assert!(c.render("{{$timestamp}}").unwrap().parse::<i64>().is_ok());
        assert!(c.render("{{$nope}}").is_err());
    }

    #[test]
    fn response_refs() {
        let mut c = ctx();
        assert!(c.render("{{login.response.body.$.token}}").is_err());
        c.record(
            "login",
            Recorded {
                status: 201,
                headers: vec![("X-Req".into(), "r1".into())],
                body: r#"{"token":"t0","user":{"id":7},"ids":[1,2],"tpl":"{{base}}"}"#.into(),
            },
        );
        assert_eq!(c.render("{{login.response.body.$.token}}").unwrap(), "t0");
        assert_eq!(c.render("{{login.response.body.user.id}}").unwrap(), "7");
        assert_eq!(c.render("{{login.response.body.$.ids[*]}}").unwrap(), "[1,2]");
        assert_eq!(c.render("{{login.response.status}}").unwrap(), "201");
        assert_eq!(c.render("{{login.response.headers.x-req}}").unwrap(), "r1");
        assert!(c.render("{{login.response.body.$.missing}}").is_err());
        // Response data is never treated as a template.
        assert_eq!(c.render("{{login.response.body.$.tpl}}").unwrap(), "{{base}}");
    }
}
