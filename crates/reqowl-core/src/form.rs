//! A form view of a request block, for editing without writing the syntax:
//! method and URL, query params, headers, auth, body and settings.
//!
//! The block text stays the source of truth. [`from_block`] reads a block
//! into a form; [`to_block`] writes a form back as block text. Plain comment
//! lines in the block are kept (moved to the top).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::request::{self, Directives, Header, Kind, RequestSpec};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pair {
    pub key: String,
    pub value: String,
}

impl Pair {
    fn new(k: &str, v: &str) -> Self {
        Pair {
            key: k.to_string(),
            value: v.to_string(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Auth {
    #[default]
    None,
    Bearer {
        token: String,
    },
    Basic {
        username: String,
        password: String,
    },
    ApiKey {
        header: String,
        value: String,
    },
    OAuth2 {
        profile: String,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BodyMode {
    #[default]
    None,
    Json,
    Text,
    /// `application/x-www-form-urlencoded`, edited as a table.
    Form,
    /// A GraphQL query plus optional variables.
    GraphQl,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestForm {
    /// `http`, `graphql` or `websocket`.
    pub kind: String,
    pub method: String,
    /// The URL without its query string.
    pub url: String,
    pub params: Vec<Pair>,
    pub headers: Vec<Pair>,
    pub auth: Auth,
    pub body_mode: BodyMode,
    /// Raw body for json/text, the query for GraphQL, messages for WebSocket.
    pub body: String,
    pub form: Vec<Pair>,
    /// GraphQL variables as JSON text.
    pub variables: String,
    pub name: String,
    /// e.g. `10s`; empty for the default.
    pub timeout: String,
    pub no_redirect: bool,
    pub insecure: bool,
    /// Directives reqowl doesn't know, kept as they were.
    pub other: Vec<Pair>,
    /// Plain comment lines from the block, without the leading `#`/`//`.
    pub comments: Vec<String>,
}

fn split_url(url: &str) -> (String, Vec<Pair>) {
    let Some((base, query)) = url.split_once('?') else {
        return (url.to_string(), Vec::new());
    };
    let params = query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| match p.split_once('=') {
            Some((k, v)) => Pair::new(k, v),
            None => Pair::new(p, ""),
        })
        .collect();
    (base.to_string(), params)
}

fn join_pairs(pairs: &[Pair]) -> String {
    pairs
        .iter()
        .filter(|p| !p.key.trim().is_empty())
        .map(|p| {
            if p.value.is_empty() {
                p.key.trim().to_string()
            } else {
                format!("{}={}", p.key.trim(), p.value)
            }
        })
        .collect::<Vec<_>>()
        .join("&")
}

fn parse_auth(raw: Option<&str>) -> Auth {
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Auth::None;
    };
    let (scheme, rest) = raw.split_once(char::is_whitespace).unwrap_or((raw, ""));
    let rest = rest.trim();
    match scheme.to_ascii_lowercase().as_str() {
        "bearer" => Auth::Bearer {
            token: rest.to_string(),
        },
        "basic" => {
            let (u, p) = rest
                .split_once(char::is_whitespace)
                .map(|(u, p)| (u, p.trim()))
                .or_else(|| rest.split_once(':'))
                .unwrap_or((rest, ""));
            Auth::Basic {
                username: u.to_string(),
                password: p.to_string(),
            }
        }
        "apikey" => {
            let (h, v) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
            Auth::ApiKey {
                header: h.to_string(),
                value: v.trim().to_string(),
            }
        }
        "oauth2" => Auth::OAuth2 {
            profile: rest.to_string(),
        },
        _ => Auth::None,
    }
}

fn auth_directive(auth: &Auth) -> Result<Option<String>> {
    let bad = |m: &str| Err(Error::Variable(m.to_string()));
    Ok(match auth {
        Auth::None => None,
        Auth::Bearer { token } if token.trim().is_empty() => return bad("Bearer auth needs a token"),
        Auth::Bearer { token } => Some(format!("bearer {}", token.trim())),
        Auth::Basic { username, .. } if username.contains(char::is_whitespace) => {
            return bad("Basic auth usernames can't contain spaces");
        }
        Auth::Basic { username, password } => {
            Some(format!("basic {} {}", username.trim(), password).trim_end().to_string())
        }
        Auth::ApiKey { header, value } => {
            if header.trim().is_empty() || header.contains(char::is_whitespace) {
                return bad("API key auth needs a header name without spaces");
            }
            Some(format!("apikey {} {}", header.trim(), value.trim()))
        }
        Auth::OAuth2 { profile } if profile.trim().is_empty() => return bad("OAuth2 needs a profile name"),
        Auth::OAuth2 { profile } => Some(format!("oauth2 {}", profile.trim())),
    })
}

fn content_type(headers: &[Header]) -> String {
    headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case("content-type"))
        .map(|h| h.value.to_ascii_lowercase())
        .unwrap_or_default()
}

/// Comment lines before the body that aren't directives.
fn plain_comments(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen_request = false;
    for line in content.lines() {
        let t = line.trim();
        if t.is_empty() {
            if seen_request {
                break;
            }
            continue;
        }
        let c = t.strip_prefix("//").or_else(|| t.strip_prefix('#'));
        match c {
            Some(c) if !c.trim_start().starts_with('@') => out.push(c.trim().to_string()),
            Some(_) => {}
            None => seen_request = true,
        }
    }
    out
}

pub fn from_block(content: &str) -> Result<RequestForm> {
    let spec = request::parse(content, 1)?;
    let (url, params) = split_url(&spec.url);
    let d = &spec.directives;
    let ct = content_type(&spec.headers);
    let body = spec.body.clone().unwrap_or_default();

    let mut f = RequestForm {
        kind: match spec.kind {
            Kind::Http => "http",
            Kind::GraphQl => "graphql",
            Kind::WebSocket => "websocket",
        }
        .into(),
        method: spec.method.clone(),
        url,
        params,
        headers: spec.headers.iter().map(|h| Pair::new(&h.name, &h.value)).collect(),
        auth: parse_auth(d.auth.as_deref()),
        name: d.name.clone().unwrap_or_default(),
        timeout: d.timeout.map(request::format_duration).unwrap_or_default(),
        no_redirect: d.no_redirect,
        insecure: d.insecure,
        other: d.other.iter().map(|(k, v)| Pair::new(k, v)).collect(),
        comments: plain_comments(content),
        ..Default::default()
    };

    match spec.kind {
        Kind::GraphQl => {
            f.body_mode = BodyMode::GraphQl;
            match body.rsplit_once("\n\n") {
                Some((q, v)) if v.trim_start().starts_with('{') => {
                    f.body = q.to_string();
                    f.variables = v.to_string();
                }
                _ => f.body = body,
            }
        }
        Kind::WebSocket => {
            f.body_mode = if body.is_empty() {
                BodyMode::None
            } else {
                BodyMode::Text
            };
            f.body = body;
        }
        Kind::Http if spec.body.is_none() => f.body_mode = BodyMode::None,
        Kind::Http if ct.contains("x-www-form-urlencoded") && !body.contains('\n') => {
            f.body_mode = BodyMode::Form;
            f.form = split_url(&format!("?{body}")).1;
        }
        Kind::Http => {
            f.body_mode = if ct.contains("json") {
                BodyMode::Json
            } else {
                BodyMode::Text
            };
            f.body = body;
        }
    }
    Ok(f)
}

pub fn to_block(f: &RequestForm) -> Result<String> {
    let kind = match f.kind.as_str() {
        "graphql" => Kind::GraphQl,
        "websocket" => Kind::WebSocket,
        _ => Kind::Http,
    };
    let base = f.url.trim();
    if base.is_empty() {
        return Err(Error::Variable("the request needs a URL".into()));
    }
    let query = join_pairs(&f.params);
    let url = if query.is_empty() {
        base.to_string()
    } else {
        format!("{base}?{query}")
    };

    let mut headers: Vec<Header> = f
        .headers
        .iter()
        .filter(|h| !h.key.trim().is_empty())
        .map(|h| {
            if h.key.trim().contains(char::is_whitespace) || h.key.contains(':') {
                return Err(Error::Variable(format!("`{}` isn't a valid header name", h.key.trim())));
            }
            Ok(Header {
                name: h.key.trim().to_string(),
                value: h.value.trim().to_string(),
            })
        })
        .collect::<Result<_>>()?;

    let has_ct = headers.iter().any(|h| h.name.eq_ignore_ascii_case("content-type"));
    let mut ensure_ct = |ct: &str| {
        if !has_ct {
            headers.push(Header {
                name: "Content-Type".into(),
                value: ct.into(),
            });
        }
    };
    let body = match (kind, f.body_mode) {
        (Kind::GraphQl, _) => {
            let q = f.body.trim_end();
            let v = f.variables.trim();
            match (q.is_empty(), v.is_empty()) {
                (true, _) => None,
                (false, true) => Some(q.to_string()),
                (false, false) => Some(format!("{q}\n\n{v}")),
            }
        }
        (_, BodyMode::None) => None,
        (_, BodyMode::Form) => {
            ensure_ct("application/x-www-form-urlencoded");
            Some(join_pairs(&f.form)).filter(|b| !b.is_empty())
        }
        (_, BodyMode::Json) => {
            ensure_ct("application/json");
            Some(f.body.trim_end().to_string()).filter(|b| !b.trim().is_empty())
        }
        (_, BodyMode::Text | BodyMode::GraphQl) => Some(f.body.trim_end().to_string()).filter(|b| !b.trim().is_empty()),
    };

    let name = f.name.trim();
    if !name.is_empty() && !name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
        return Err(Error::Variable(
            "a name can only use letters, digits, `_` and `-`".into(),
        ));
    }
    let timeout = match f.timeout.trim() {
        "" => None,
        t => Some(request::parse_duration(t).map_err(Error::Variable)?),
    };
    let other: BTreeMap<String, String> = f
        .other
        .iter()
        .filter(|p| !p.key.trim().is_empty())
        .map(|p| (p.key.trim().to_string(), p.value.trim().to_string()))
        .collect();

    let spec = RequestSpec {
        kind,
        method: if kind == Kind::Http {
            f.method.trim().to_ascii_uppercase()
        } else {
            f.method.clone()
        },
        url,
        headers,
        body,
        directives: Directives {
            name: (!name.is_empty()).then(|| name.to_string()),
            auth: auth_directive(&f.auth)?,
            timeout,
            no_redirect: f.no_redirect,
            insecure: f.insecure,
            other,
        },
    };
    if kind == Kind::Http && !request::HTTP_METHODS.contains(&spec.method.as_str()) {
        return Err(Error::Variable(format!(
            "`{}` isn't an HTTP method reqowl can send",
            spec.method
        )));
    }

    let mut out = String::new();
    for c in &f.comments {
        out.push_str(&format!("# {c}\n"));
    }
    out.push_str(&request::to_block_text(&spec));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_full_block() {
        let f = from_block(
            "# Create an order\n# @name create\n# @auth basic owl hoot\n# @timeout 5s\nPOST {{api}}/orders?dry=1&tag\n\
             Content-Type: application/json\n// trace it\nX-Trace: on\n\n{\"item\": 1}\n",
        )
        .unwrap();
        assert_eq!(f.method, "POST");
        assert_eq!(f.url, "{{api}}/orders");
        assert_eq!(f.params, [Pair::new("dry", "1"), Pair::new("tag", "")]);
        assert_eq!(f.headers.len(), 2);
        assert_eq!(
            f.auth,
            Auth::Basic {
                username: "owl".into(),
                password: "hoot".into()
            }
        );
        assert_eq!(f.body_mode, BodyMode::Json);
        assert_eq!(f.body, "{\"item\": 1}");
        assert_eq!((f.name.as_str(), f.timeout.as_str()), ("create", "5s"));
        assert_eq!(f.comments, ["Create an order", "trace it"]);
    }

    #[test]
    fn round_trips() {
        let blocks = [
            "# @name a\nGET https://h/x?q=1\nAccept: */*\n",
            "# keep me\n# @auth bearer {{token}}\nPOST https://h/f\nContent-Type: application/x-www-form-urlencoded\n\na=1&b={{b}}\n",
            "GRAPHQL https://h/graphql\n\nquery { me { id } }\n\n{\"id\": 1}\n",
            "# @auth apikey X-Key k1\n# @no-redirect\n# @insecure\nDELETE https://h/1\n",
            "WEBSOCKET wss://h/s\n\nhello\n===\nworld\n",
            "# @auth oauth2 api\n# @custom x\nPUT https://h\nContent-Type: text/plain\n\nplain body\n",
        ];
        for b in blocks {
            let f = from_block(b).unwrap();
            let text = to_block(&f).unwrap();
            assert_eq!(text, b, "block didn't round-trip");
            assert_eq!(from_block(&text).unwrap(), f);
        }
    }

    #[test]
    fn edits_produce_valid_blocks() {
        let mut f = from_block("GET https://h/items\n").unwrap();
        f.method = "post".into();
        f.params.push(Pair::new("page", "2"));
        f.params.push(Pair::new("", "ignored"));
        f.headers.push(Pair::new("X-A", " 1 "));
        f.body_mode = BodyMode::Json;
        f.body = "{\"n\": 1}\n\n".into();
        f.auth = Auth::Bearer { token: "{{t}}".into() };
        let text = to_block(&f).unwrap();
        assert_eq!(
            text,
            "# @auth bearer {{t}}\nPOST https://h/items?page=2\nX-A: 1\nContent-Type: application/json\n\n{\"n\": 1}\n"
        );
        request::parse(&text, 1).unwrap();

        f.body_mode = BodyMode::Form;
        f.form = vec![Pair::new("a", "1"), Pair::new("b", "")];
        assert!(to_block(&f).unwrap().ends_with("\n\na=1&b\n"));
    }

    #[test]
    fn rejects_bad_input() {
        let mut f = from_block("GET https://h\n").unwrap();
        f.url = " ".into();
        assert!(to_block(&f).is_err());
        let mut f = from_block("GET https://h\n").unwrap();
        f.name = "bad name".into();
        assert!(to_block(&f).is_err());
        let mut f = from_block("GET https://h\n").unwrap();
        f.headers.push(Pair::new("Bad Header", "x"));
        assert!(to_block(&f).is_err());
        let mut f = from_block("GET https://h\n").unwrap();
        f.timeout = "soon".into();
        assert!(to_block(&f).is_err());
        let mut f = from_block("GET https://h\n").unwrap();
        f.method = "FETCH".into();
        assert!(to_block(&f).is_err());
    }
}
