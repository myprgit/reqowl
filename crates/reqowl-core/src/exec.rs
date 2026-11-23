//! Sends a resolved [`RequestSpec`].

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{Map, Value};

use crate::error::{Error, Result};
use crate::request::{Kind, RequestSpec};
use crate::vars::Recorded;

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
/// Bodies beyond this are cut off; the rest of the stream is dropped.
pub const MAX_BODY: usize = 64 * 1024 * 1024;
const USER_AGENT: &str = concat!("reqowl/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub reason: String,
    pub version: String,
    /// Final URL, after redirects.
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// The body was longer than [`MAX_BODY`] and was cut off.
    pub truncated: bool,
    pub elapsed: Duration,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn body_text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn is_json(&self) -> bool {
        self.header("content-type").is_some_and(|ct| {
            let mime = ct.split(';').next().unwrap_or("").trim();
            mime == "application/json" || mime.ends_with("+json")
        })
    }

    pub fn record(&self) -> Recorded {
        Recorded {
            status: self.status,
            headers: self.headers.clone(),
            body: self.body_text(),
        }
    }
}

/// Holds HTTP clients. Redirect policy and certificate checks are per-client
/// settings in reqwest, so one client is kept for each combination.
#[derive(Default)]
pub struct Client {
    clients: Mutex<HashMap<(bool, bool), reqwest::Client>>,
}

impl Client {
    pub fn new() -> Self {
        Self::default()
    }

    fn get(&self, follow_redirects: bool, insecure: bool) -> Result<reqwest::Client> {
        let mut map = self.clients.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(c) = map.get(&(follow_redirects, insecure)) {
            return Ok(c.clone());
        }
        let policy = if follow_redirects {
            reqwest::redirect::Policy::limited(10)
        } else {
            reqwest::redirect::Policy::none()
        };
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .redirect(policy)
            .tls_danger_accept_invalid_certs(insecure)
            .build()
            .map_err(|e| Error::Http(error_chain(&e)))?;
        map.insert((follow_redirects, insecure), client.clone());
        Ok(client)
    }

    /// Sends a request that has been through [`RequestSpec::resolve`] and
    /// [`RequestSpec::apply_auth`].
    pub async fn execute(&self, spec: &RequestSpec) -> Result<Response> {
        let mut spec = spec.clone();
        match spec.kind {
            Kind::Http => {}
            Kind::GraphQl => graphql_body(&mut spec)?,
            Kind::WebSocket => return Err(Error::Unsupported("WebSocket")),
        }

        let d = &spec.directives;
        let client = self.get(!d.no_redirect, d.insecure)?;
        let method = reqwest::Method::from_bytes(spec.method.as_bytes())
            .map_err(|_| Error::Http(format!("invalid method `{}`", spec.method)))?;
        let url =
            reqwest::Url::parse(&spec.url).map_err(|e| Error::Http(format!("invalid URL `{}`: {e}", spec.url)))?;

        let mut req = client
            .request(method, url)
            .timeout(d.timeout.unwrap_or(DEFAULT_TIMEOUT));
        for h in &spec.headers {
            req = req.header(h.name.trim(), h.value.as_str());
        }
        if let Some(body) = spec.body {
            req = req.body(body);
        }

        let started = Instant::now();
        let resp = req.send().await.map_err(|e| Error::Http(error_chain(&e)))?;
        let status = resp.status();
        let version = format!("{:?}", resp.version());
        let url = resp.url().to_string();
        let headers = resp
            .headers()
            .iter()
            .map(|(k, v)| {
                (
                    k.as_str().to_string(),
                    String::from_utf8_lossy(v.as_bytes()).into_owned(),
                )
            })
            .collect();
        let mut resp = resp;
        let mut body = Vec::new();
        let mut truncated = false;
        while let Some(chunk) = resp.chunk().await.map_err(|e| Error::Http(error_chain(&e)))? {
            let room = MAX_BODY - body.len();
            if chunk.len() > room {
                body.extend_from_slice(&chunk[..room]);
                truncated = true;
                break;
            }
            body.extend_from_slice(&chunk);
        }

        Ok(Response {
            status: status.as_u16(),
            reason: status.canonical_reason().unwrap_or("").to_string(),
            version,
            url,
            headers,
            body,
            truncated,
            elapsed: started.elapsed(),
        })
    }
}

/// GraphQL block body: the query, then optionally a blank line and a JSON
/// object of variables. Becomes a standard `{"query", "variables"}` POST.
fn graphql_body(spec: &mut RequestSpec) -> Result<()> {
    let body = spec.body.take().unwrap_or_default();
    let (query, variables) = match body.rsplit_once("\n\n") {
        Some((q, v)) if v.trim_start().starts_with('{') => {
            let vars: Value = serde_json::from_str(v)
                .map_err(|e| Error::Http(format!("GraphQL variables are not valid JSON: {e}")))?;
            (q.trim().to_string(), Some(vars))
        }
        _ => (body.trim().to_string(), None),
    };
    let mut obj = Map::new();
    obj.insert("query".into(), Value::String(query));
    if let Some(v) = variables {
        obj.insert("variables".into(), v);
    }
    spec.body = Some(Value::Object(obj).to_string());
    if spec.header("content-type").is_none() {
        spec.set_header("Content-Type", "application/json".into());
    }
    Ok(())
}

/// reqwest's top-level message is often just "error sending request"; the
/// cause (DNS, refused, TLS) is further down the chain.
fn error_chain(e: &dyn std::error::Error) -> String {
    let mut msg = e.to_string();
    let mut src = e.source();
    while let Some(s) = src {
        let s_msg = s.to_string();
        if !msg.contains(&s_msg) {
            msg.push_str(": ");
            msg.push_str(&s_msg);
        }
        src = s.source();
    }
    msg
}
