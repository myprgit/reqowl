//! Parses one request block. The syntax follows VS Code REST Client and
//! JetBrains HTTP Client, so `.http` habits carry over:
//!
//! ```text
//! # @name login            <- directives are `# @key value` comments
//! # @auth bearer {{token}}
//! POST {{base}}/login      <- METHOD URL [HTTP/1.1]; a bare URL means GET
//!   ?page=2                <- optional query continuation lines
//! Content-Type: application/json
//!
//! {"user": "a"}            <- body after the first blank line
//! ```

use std::collections::BTreeMap;
use std::time::Duration;

use base64::Engine;

use crate::error::{Error, Result};
use crate::vars::Context;

pub const HTTP_METHODS: &[&str] = &[
    "GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS", "TRACE", "CONNECT",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Http,
    GraphQl,
    WebSocket,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Directives {
    pub name: Option<String>,
    /// Raw `@auth` value, e.g. `bearer {{token}}`; applied by [`RequestSpec::apply_auth`].
    pub auth: Option<String>,
    pub timeout: Option<Duration>,
    pub no_redirect: bool,
    pub insecure: bool,
    /// Directives reqowl does not know; kept so tools can round-trip them.
    pub other: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RequestSpec {
    pub kind: Kind,
    pub method: String,
    pub url: String,
    pub headers: Vec<Header>,
    pub body: Option<String>,
    pub directives: Directives,
}

impl RequestSpec {
    pub fn name(&self) -> Option<&str> {
        self.directives.name.as_deref()
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case(name))
            .map(|h| h.value.as_str())
    }

    pub fn set_header(&mut self, name: &str, value: String) {
        self.headers.retain(|h| !h.name.eq_ignore_ascii_case(name));
        self.headers.push(Header {
            name: name.to_string(),
            value,
        });
    }

    /// Fills every `{{variable}}` in the URL, headers, body and `@auth`.
    /// Collects all undefined names into a single error.
    pub fn resolve(&self, ctx: &Context) -> Result<RequestSpec> {
        let mut missing = Vec::new();
        let mut render = |s: &str| match ctx.render(s) {
            Ok(v) => Ok(v),
            Err(Error::UndefinedVariables(names)) => {
                for n in names {
                    if !missing.contains(&n) {
                        missing.push(n);
                    }
                }
                Ok(String::new())
            }
            Err(e) => Err(e),
        };

        let url = render(&self.url)?;
        let mut headers = Vec::with_capacity(self.headers.len());
        for h in &self.headers {
            headers.push(Header {
                name: render(&h.name)?,
                value: render(&h.value)?,
            });
        }
        let body = self.body.as_deref().map(&mut render).transpose()?;
        let auth = self.directives.auth.as_deref().map(&mut render).transpose()?;

        if !missing.is_empty() {
            return Err(Error::UndefinedVariables(missing));
        }
        Ok(RequestSpec {
            kind: self.kind,
            method: self.method.clone(),
            url,
            headers,
            body,
            directives: Directives {
                auth,
                ..self.directives.clone()
            },
        })
    }

    /// The profile of `@auth oauth2 PROFILE`, if that is the block's auth.
    /// The session fetches the token; see [`crate::oauth`].
    pub fn oauth_profile(&self) -> Option<&str> {
        let auth = self.directives.auth.as_deref()?.trim();
        let (scheme, rest) = auth.split_once(char::is_whitespace)?;
        let rest = rest.trim();
        (scheme.eq_ignore_ascii_case("oauth2") && !rest.is_empty()).then_some(rest)
    }

    /// Turns the `@auth` directive into a header. Call after [`Self::resolve`].
    ///
    /// - `bearer TOKEN`
    /// - `basic USER PASSWORD` (or `basic USER:PASSWORD`)
    /// - `apikey HEADER-NAME VALUE`
    /// - `oauth2 PROFILE`: left alone here; [`crate::session::Session`] adds the token.
    pub fn apply_auth(&mut self) -> Result<()> {
        let Some(auth) = self.directives.auth.clone() else {
            return Ok(());
        };
        let (scheme, rest) = auth.trim().split_once(char::is_whitespace).unwrap_or((auth.trim(), ""));
        let rest = rest.trim();
        match scheme.to_ascii_lowercase().as_str() {
            "bearer" if !rest.is_empty() => {
                self.set_header("Authorization", format!("Bearer {rest}"));
            }
            "basic" if !rest.is_empty() => {
                let pair = match rest.split_once(char::is_whitespace) {
                    Some((u, p)) => format!("{u}:{}", p.trim()),
                    None => rest.to_string(),
                };
                let enc = base64::engine::general_purpose::STANDARD.encode(pair);
                self.set_header("Authorization", format!("Basic {enc}"));
            }
            "apikey" => {
                let Some((name, value)) = rest.split_once(char::is_whitespace) else {
                    return Err(Error::Variable("@auth apikey needs a header name and a value".into()));
                };
                self.set_header(name, value.trim().to_string());
            }
            "oauth2" if !rest.is_empty() => {}
            "none" => {}
            _ => {
                return Err(Error::Variable(format!(
                    "@auth: expected `bearer TOKEN`, `basic USER PASSWORD`, `apikey HEADER VALUE` or `oauth2 PROFILE`, got `{auth}`"
                )));
            }
        }
        Ok(())
    }
}

fn comment_text(line: &str) -> Option<&str> {
    let t = line.trim_start();
    t.strip_prefix("//").or_else(|| t.strip_prefix('#')).map(str::trim)
}

/// Parses a block. `first_line` is the page line of the block's first line,
/// so errors point into the page rather than into the block.
pub fn parse(content: &str, first_line: usize) -> Result<RequestSpec> {
    let lines: Vec<&str> = content.lines().collect();
    let err = |i: usize, msg: String| Error::Parse {
        line: first_line + i,
        msg,
    };
    let mut directives = Directives::default();
    let mut i = 0;

    // Leading blank lines, comments and directives.
    while i < lines.len() {
        let line = lines[i];
        if line.trim().is_empty() {
            i += 1;
            continue;
        }
        match comment_text(line) {
            Some(c) => {
                if let Some(d) = c.strip_prefix('@') {
                    parse_directive(d, &mut directives).map_err(|m| err(i, m))?;
                }
                i += 1;
            }
            None => break,
        }
    }
    if i == lines.len() {
        return Err(err(0, "block has no request line".into()));
    }

    // Request line.
    let line = lines[i].trim();
    let (first, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
    let upper = first.to_ascii_uppercase();
    let (kind, method, mut url) = if HTTP_METHODS.contains(&upper.as_str()) {
        (Kind::Http, upper, rest.trim().to_string())
    } else if upper == "GRAPHQL" {
        (Kind::GraphQl, "POST".to_string(), rest.trim().to_string())
    } else if upper == "WEBSOCKET" {
        (Kind::WebSocket, "GET".to_string(), rest.trim().to_string())
    } else if first.contains("://") || first.starts_with('{') || first.starts_with('/') {
        (Kind::Http, "GET".to_string(), line.to_string())
    } else {
        return Err(err(i, format!("expected `METHOD URL`, got `{line}`")));
    };
    if let Some((u, ver)) = url.rsplit_once(char::is_whitespace)
        && ver.starts_with("HTTP/")
    {
        url = u.trim_end().to_string();
    }
    if url.is_empty() {
        return Err(err(i, "request line has no URL".into()));
    }
    i += 1;

    // Query continuation lines.
    while i < lines.len() {
        let t = lines[i].trim();
        if t.starts_with('?') || t.starts_with('&') {
            url.push_str(t);
            i += 1;
        } else {
            break;
        }
    }

    // Headers, up to the first blank line.
    let mut headers = Vec::new();
    while i < lines.len() {
        let line = lines[i];
        i += 1;
        if line.trim().is_empty() {
            break;
        }
        if comment_text(line).is_some() {
            continue;
        }
        match line.split_once(':') {
            Some((name, value)) if !name.trim().is_empty() && !name.contains(char::is_whitespace) => {
                headers.push(Header {
                    name: name.to_string(),
                    value: value.trim().to_string(),
                });
            }
            _ => {
                return Err(err(
                    i - 1,
                    format!(
                        "expected `Header-Name: value` or a blank line before the body, got `{}`",
                        line.trim()
                    ),
                ));
            }
        }
    }

    // Body: the rest, minus trailing blank lines.
    let body_lines = &lines[i.min(lines.len())..];
    let end = body_lines
        .iter()
        .rposition(|l| !l.trim().is_empty())
        .map_or(0, |p| p + 1);
    let body = (end > 0).then(|| body_lines[..end].join("\n"));

    Ok(RequestSpec {
        kind,
        method,
        url,
        headers,
        body,
        directives,
    })
}

fn parse_directive(d: &str, out: &mut Directives) -> std::result::Result<(), String> {
    let (key, value) = d.split_once(char::is_whitespace).unwrap_or((d, ""));
    let value = value.trim();
    match key {
        "name" => {
            if value.is_empty() || !value.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
                return Err(format!("@name must be letters, digits, `_` or `-`, got `{value}`"));
            }
            out.name = Some(value.to_string());
        }
        "auth" => out.auth = Some(value.to_string()),
        "timeout" => out.timeout = Some(parse_duration(value)?),
        "no-redirect" => out.no_redirect = true,
        "insecure" => out.insecure = true,
        _ => {
            out.other.insert(key.to_string(), value.to_string());
        }
    }
    Ok(())
}

/// Renders a spec as block content (what goes between the fences): directives,
/// the request line, headers, a blank line and the body. [`parse`] reads it back
/// into an equal spec, as long as the spec is already in the shape `parse`
/// produces (no trailing blank body lines, trimmed header values, an empty body
/// as `None`).
pub fn to_block_text(spec: &RequestSpec) -> String {
    use std::fmt::Write;

    let mut out = String::new();
    let d = &spec.directives;
    if let Some(name) = &d.name {
        let _ = writeln!(out, "# @name {name}");
    }
    if let Some(auth) = &d.auth {
        let _ = writeln!(out, "# @auth {auth}");
    }
    if let Some(t) = d.timeout {
        let _ = writeln!(out, "# @timeout {}", format_duration(t));
    }
    if d.no_redirect {
        out.push_str("# @no-redirect\n");
    }
    if d.insecure {
        out.push_str("# @insecure\n");
    }
    for (k, v) in &d.other {
        if v.is_empty() {
            let _ = writeln!(out, "# @{k}");
        } else {
            let _ = writeln!(out, "# @{k} {v}");
        }
    }
    let verb = match spec.kind {
        Kind::Http => spec.method.as_str(),
        Kind::GraphQl => "GRAPHQL",
        Kind::WebSocket => "WEBSOCKET",
    };
    let _ = writeln!(out, "{verb} {}", spec.url);
    for h in &spec.headers {
        let _ = writeln!(out, "{}: {}", h.name, h.value);
    }
    if let Some(body) = &spec.body {
        let _ = write!(out, "\n{body}\n");
    }
    out
}

/// The inverse of [`parse_duration`]: `10s` for whole seconds, else milliseconds.
pub fn format_duration(d: Duration) -> String {
    if d.subsec_nanos() == 0 {
        format!("{}s", d.as_secs())
    } else if d.as_nanos().is_multiple_of(1_000_000) {
        format!("{}ms", d.as_millis())
    } else {
        format!("{}ms", d.as_secs_f64() * 1000.0)
    }
}

/// `500ms`, `10s`, `2m`, or a bare number of seconds.
pub fn parse_duration(s: &str) -> std::result::Result<Duration, String> {
    let bad = || format!("bad duration `{s}`, expected e.g. `500ms`, `10s` or `2m`");
    let s = s.trim();
    let (num, unit) = match s.find(|c: char| !c.is_ascii_digit() && c != '.') {
        Some(p) => (&s[..p], s[p..].trim()),
        None => (s, "s"),
    };
    let n: f64 = num.parse().map_err(|_| bad())?;
    let secs = match unit {
        "ms" => n / 1000.0,
        "s" => n,
        "m" => n * 60.0,
        _ => return Err(bad()),
    };
    Ok(Duration::from_secs_f64(secs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_request() {
        let r = parse(
            "# @name login\n// note\nPOST {{base}}/login HTTP/1.1\n  ?a=1\n  &b=2\nContent-Type: application/json\n# skipped\nX-Y:z\n\n{\n  \"u\": 1\n}\n\n\n",
            1,
        )
        .unwrap();
        assert_eq!(r.kind, Kind::Http);
        assert_eq!(r.method, "POST");
        assert_eq!(r.url, "{{base}}/login?a=1&b=2");
        assert_eq!(r.name(), Some("login"));
        assert_eq!(r.headers.len(), 2);
        assert_eq!(r.header("content-type"), Some("application/json"));
        assert_eq!(r.header("X-Y"), Some("z"));
        assert_eq!(r.body.as_deref(), Some("{\n  \"u\": 1\n}"));
    }

    #[test]
    fn bare_url_is_get() {
        let r = parse("https://example.com/x\n", 1).unwrap();
        assert_eq!(r.method, "GET");
        assert_eq!(r.url, "https://example.com/x");
        assert!(r.body.is_none());
        let r = parse("{{base}}/x", 1).unwrap();
        assert_eq!(r.url, "{{base}}/x");
    }

    #[test]
    fn lowercase_method() {
        assert_eq!(parse("delete http://h/1", 1).unwrap().method, "DELETE");
    }

    #[test]
    fn graphql_and_websocket() {
        let r = parse("GRAPHQL http://h/graphql\n\nquery { me { id } }", 1).unwrap();
        assert_eq!((r.kind, r.method.as_str()), (Kind::GraphQl, "POST"));
        assert_eq!(parse("WEBSOCKET ws://h/s", 1).unwrap().kind, Kind::WebSocket);
    }

    #[test]
    fn errors_point_at_page_lines() {
        let e = parse("\n\nnot a request", 10).unwrap_err();
        assert!(matches!(e, Error::Parse { line: 12, .. }), "{e}");
        let e = parse("GET http://h\nbad header line\n", 5).unwrap_err();
        assert!(matches!(e, Error::Parse { line: 6, .. }), "{e}");
        let e = parse("# only a comment\n", 1).unwrap_err();
        assert!(matches!(e, Error::Parse { .. }));
        assert!(parse("# @name bad name!\nGET http://h", 1).is_err());
    }

    #[test]
    fn directives() {
        let r = parse(
            "# @timeout 500ms\n# @no-redirect\n# @insecure\n# @foo bar\nGET http://h",
            1,
        )
        .unwrap();
        assert_eq!(r.directives.timeout, Some(Duration::from_millis(500)));
        assert!(r.directives.no_redirect && r.directives.insecure);
        assert_eq!(r.directives.other["foo"], "bar");
        assert_eq!(parse_duration("2m").unwrap(), Duration::from_secs(120));
        assert_eq!(parse_duration("3").unwrap(), Duration::from_secs(3));
        assert!(parse_duration("3h").is_err());
    }

    #[test]
    fn block_text_round_trips() {
        let mut other = BTreeMap::new();
        other.insert("note".to_string(), "keep me".to_string());
        other.insert("flag".to_string(), String::new());
        let specs = [
            RequestSpec {
                kind: Kind::Http,
                method: "GET".into(),
                url: "https://example.com/x?a=1&b={{b}}".into(),
                headers: vec![],
                body: None,
                directives: Directives::default(),
            },
            RequestSpec {
                kind: Kind::Http,
                method: "POST".into(),
                url: "{{base}}/login".into(),
                headers: vec![
                    Header {
                        name: "Content-Type".into(),
                        value: "application/json".into(),
                    },
                    Header {
                        name: "X-Empty".into(),
                        value: String::new(),
                    },
                ],
                body: Some("\n{\n  \"u\": \"{{user}}\"\n}".into()),
                directives: Directives {
                    name: Some("login".into()),
                    auth: Some("basic user:".into()),
                    timeout: Some(Duration::from_millis(1500)),
                    no_redirect: true,
                    insecure: true,
                    other,
                },
            },
            RequestSpec {
                kind: Kind::GraphQl,
                method: "POST".into(),
                url: "http://h/graphql".into(),
                headers: vec![],
                body: Some("query { me { id } }\n\n{\"a\": 1}".into()),
                directives: Directives {
                    auth: Some("bearer {{token}}".into()),
                    timeout: Some(Duration::from_secs(10)),
                    ..Default::default()
                },
            },
            RequestSpec {
                kind: Kind::WebSocket,
                method: "GET".into(),
                url: "ws://h/s".into(),
                headers: vec![],
                body: None,
                directives: Directives::default(),
            },
        ];
        for spec in specs {
            let text = to_block_text(&spec);
            assert_eq!(parse(&text, 1).unwrap(), spec, "{text}");
        }
    }

    #[test]
    fn auth() {
        let mut r = parse("# @auth basic user pa ss\nGET http://h", 1).unwrap();
        r.apply_auth().unwrap();
        // "user:pa ss"
        assert_eq!(r.header("authorization"), Some("Basic dXNlcjpwYSBzcw=="));

        let mut r = parse("# @auth bearer abc\nGET http://h\nAuthorization: old", 1).unwrap();
        r.apply_auth().unwrap();
        assert_eq!(r.header("Authorization"), Some("Bearer abc"));
        assert_eq!(r.headers.len(), 1);

        let mut r = parse("# @auth apikey X-Api-Key k1\nGET http://h", 1).unwrap();
        r.apply_auth().unwrap();
        assert_eq!(r.header("x-api-key"), Some("k1"));

        let mut r = parse("# @auth digest x\nGET http://h", 1).unwrap();
        assert!(r.apply_auth().is_err());

        let mut r = parse("# @auth OAuth2  api \nGET http://h", 1).unwrap();
        r.apply_auth().unwrap();
        assert_eq!(r.oauth_profile(), Some("api"));
        assert!(r.headers.is_empty());
        assert!(parse("# @auth oauth2\nGET http://h", 1).unwrap().apply_auth().is_err());
        assert_eq!(
            parse("# @auth bearer x\nGET http://h", 1).unwrap().oauth_profile(),
            None
        );
    }
}
