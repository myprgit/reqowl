//! curl commands in and out.
//!
//! [`parse`] reads a command the way a POSIX shell would split it (quotes,
//! `$'...'`, backslash-newline continuations) and keeps the options that
//! shape the request. Options that only affect curl's own output are
//! skipped, unknown ones are ignored. [`to_curl`] goes the other way.

use std::time::Duration;

use super::{
    ImportedWorkspace, Node, PageDraft, append_query, basic_auth, encode_component, has_header, header, multipart,
    multipart_content_type,
};
use crate::error::{Error, Result};
use crate::request::{Directives, Header, Kind, RequestSpec};

/// Long options that take a value. Anything not listed is treated as a flag.
const LONG_WITH_VALUE: &[&str] = &[
    "abstract-unix-socket",
    "alt-svc",
    "aws-sigv4",
    "cacert",
    "capath",
    "cert",
    "cert-type",
    "ciphers",
    "config",
    "connect-timeout",
    "connect-to",
    "continue-at",
    "cookie",
    "cookie-jar",
    "create-file-mode",
    "crlfile",
    "curves",
    "data",
    "data-ascii",
    "data-binary",
    "data-raw",
    "data-urlencode",
    "delegation",
    "dns-interface",
    "dns-ipv4-addr",
    "dns-ipv6-addr",
    "dns-servers",
    "doh-url",
    "dump-header",
    "ech",
    "egd-file",
    "engine",
    "etag-compare",
    "etag-save",
    "expect100-timeout",
    "form",
    "form-string",
    "ftp-account",
    "ftp-alternative-to-user",
    "ftp-method",
    "ftp-port",
    "ftp-ssl-ccc-mode",
    "happy-eyeballs-timeout-ms",
    "haproxy-clientip",
    "header",
    "hostpubmd5",
    "hostpubsha256",
    "hsts",
    "interface",
    "ip-tos",
    "json",
    "keepalive-time",
    "key",
    "key-type",
    "krb",
    "libcurl",
    "limit-rate",
    "local-port",
    "login-options",
    "mail-auth",
    "mail-from",
    "mail-rcpt",
    "max-filesize",
    "max-redirs",
    "max-time",
    "netrc-file",
    "noproxy",
    "oauth2-bearer",
    "output",
    "output-dir",
    "parallel-max",
    "pass",
    "pinnedpubkey",
    "preproxy",
    "proto",
    "proto-default",
    "proto-redir",
    "proxy",
    "proxy-cacert",
    "proxy-capath",
    "proxy-cert",
    "proxy-cert-type",
    "proxy-ciphers",
    "proxy-crlfile",
    "proxy-header",
    "proxy-key",
    "proxy-key-type",
    "proxy-pass",
    "proxy-pinnedpubkey",
    "proxy-service-name",
    "proxy-tls13-ciphers",
    "proxy-tlsauthtype",
    "proxy-tlspassword",
    "proxy-tlsuser",
    "proxy-user",
    "proxy1.0",
    "pubkey",
    "quote",
    "random-file",
    "range",
    "rate",
    "referer",
    "request",
    "request-target",
    "resolve",
    "retry",
    "retry-delay",
    "retry-max-time",
    "sasl-authzid",
    "service-name",
    "socks4",
    "socks4a",
    "socks5",
    "socks5-gssapi-service",
    "socks5-hostname",
    "speed-limit",
    "speed-time",
    "stderr",
    "telnet-option",
    "tftp-blksize",
    "time-cond",
    "tls-max",
    "tls13-ciphers",
    "tlsauthtype",
    "tlspassword",
    "tlsuser",
    "trace",
    "trace-ascii",
    "trace-config",
    "unix-socket",
    "upload-file",
    "url",
    "url-query",
    "user",
    "user-agent",
    "variable",
    "vlan-priority",
    "write-out",
];

/// Short options that take a value, with their long names.
const SHORT_WITH_VALUE: &[(char, &str)] = &[
    ('A', "user-agent"),
    ('b', "cookie"),
    ('c', "cookie-jar"),
    ('C', "continue-at"),
    ('d', "data"),
    ('D', "dump-header"),
    ('e', "referer"),
    ('E', "cert"),
    ('F', "form"),
    ('H', "header"),
    ('K', "config"),
    ('m', "max-time"),
    ('o', "output"),
    ('P', "ftp-port"),
    ('Q', "quote"),
    ('r', "range"),
    ('t', "telnet-option"),
    ('T', "upload-file"),
    ('u', "user"),
    ('U', "proxy-user"),
    ('w', "write-out"),
    ('x', "proxy"),
    ('X', "request"),
    ('y', "speed-time"),
    ('Y', "speed-limit"),
    ('z', "time-cond"),
];

/// Short flags we act on; other single letters are ignored.
const SHORT_FLAGS: &[(char, &str)] = &[('G', "get"), ('I', "head"), ('k', "insecure"), ('L', "location")];

/// True if the text starts with a `curl` command word.
pub fn looks_like_curl(text: &str) -> bool {
    let t = text.trim_start();
    let t = t.strip_prefix("$ ").unwrap_or(t).trim_start();
    let word = t.split(|c: char| c.is_whitespace()).next().unwrap_or("");
    matches!(word, "curl" | "curl.exe")
}

#[derive(Default)]
struct Parsed {
    method: Option<String>,
    urls: Vec<String>,
    headers: Vec<Header>,
    data: Vec<String>,
    json: Vec<String>,
    form: Vec<(String, String)>,
    query: Vec<String>,
    cookies: Vec<String>,
    auth: Option<String>,
    get: bool,
    head: bool,
    upload: bool,
    insecure: bool,
    timeout: Option<Duration>,
}

/// Parses one curl command into a request.
pub fn parse(cmd: &str) -> Result<RequestSpec> {
    let tokens = tokenize(cmd)?;
    let mut iter = tokens.into_iter().peekable();
    // A leading prompt and the command word itself.
    if iter.peek().is_some_and(|t| t == "$") {
        iter.next();
    }
    match iter.next() {
        Some(t) if t == "curl" || t == "curl.exe" || t.ends_with("/curl") => {}
        _ => return Err(Error::Import("expected a command starting with `curl`".into())),
    }

    let mut p = Parsed::default();
    let mut only_positional = false;
    while let Some(tok) = iter.next() {
        if only_positional || !tok.starts_with('-') || tok == "-" {
            p.urls.push(tok);
            continue;
        }
        if tok == "--" {
            only_positional = true;
            continue;
        }
        if let Some(long) = tok.strip_prefix("--") {
            let (name, inline) = match long.split_once('=') {
                Some((n, v)) if LONG_WITH_VALUE.contains(&n) => (n.to_string(), Some(v.to_string())),
                _ => (long.to_string(), None),
            };
            if LONG_WITH_VALUE.contains(&name.as_str()) {
                let value = inline.or_else(|| iter.next()).unwrap_or_default();
                apply(&mut p, &name, Some(value));
            } else {
                apply(&mut p, &name, None);
            }
            continue;
        }
        // A cluster of short options: `-sSL`, `-XPOST`, `-H'X: y'`.
        let chars: Vec<char> = tok[1..].chars().collect();
        for (i, c) in chars.iter().enumerate() {
            if let Some((_, long)) = SHORT_WITH_VALUE.iter().find(|(s, _)| s == c) {
                let rest: String = chars[i + 1..].iter().collect();
                let value = if rest.is_empty() {
                    iter.next().unwrap_or_default()
                } else {
                    rest
                };
                apply(&mut p, long, Some(value));
                break;
            }
            if let Some((_, long)) = SHORT_FLAGS.iter().find(|(s, _)| s == c) {
                apply(&mut p, long, None);
            }
        }
    }
    build(p)
}

fn apply(p: &mut Parsed, name: &str, value: Option<String>) {
    let v = value.unwrap_or_default();
    match name {
        "request" => p.method = Some(v.to_ascii_uppercase()),
        "header" => {
            if v.starts_with('@') {
                return; // headers from a file
            }
            if let Some((n, val)) = v.split_once(':') {
                // `Name:` with nothing after removes a header in curl.
                if !val.trim().is_empty() {
                    p.headers.push(header(n.trim(), val.trim()));
                }
            } else if let Some(n) = v.trim().strip_suffix(';') {
                p.headers.push(header(n.trim(), ""));
            }
        }
        "data" | "data-ascii" | "data-binary" | "data-raw" => p.data.push(v),
        "data-urlencode" => p.data.push(urlencode_arg(&v)),
        "json" => p.json.push(v),
        "form" | "form-string" => {
            if let Some((k, val)) = v.split_once('=') {
                // `@file` / `<file` uploads can't be represented; keep text fields.
                if name == "form-string" || !val.starts_with(['@', '<']) {
                    p.form.push((k.to_string(), val.to_string()));
                }
            }
        }
        "url-query" => p.query.push(urlencode_arg(&v)),
        "user" => {
            let (u, pw) = v.split_once(':').unwrap_or((&v, ""));
            p.auth = Some(basic_auth(u, pw));
        }
        "oauth2-bearer" => p.auth = Some(format!("bearer {v}")),
        "user-agent" => p.headers.push(header("User-Agent", v)),
        "referer" => {
            let r = v.strip_suffix(";auto").unwrap_or(&v);
            if !r.is_empty() {
                p.headers.push(header("Referer", r));
            }
        }
        "cookie" => {
            // Without `=` it names a cookie file.
            if v.contains('=') {
                p.cookies.push(v.trim().trim_end_matches(';').to_string());
            }
        }
        "url" => p.urls.push(v),
        "max-time" => {
            if let Ok(secs) = v.trim().parse::<f64>()
                && secs > 0.0
                && secs.is_finite()
            {
                p.timeout = Some(Duration::from_secs_f64(secs));
            }
        }
        "upload-file" => p.upload = true,
        "get" => p.get = true,
        "head" => p.head = true,
        "insecure" => p.insecure = true,
        // `location` (reqowl follows redirects by default), `compressed`
        // (reqwest decodes gzip/brotli itself) and everything else.
        _ => {}
    }
}

/// `--data-urlencode` forms: `content`, `=content`, `name=content`,
/// `@file`, `name@file` (file forms are kept verbatim).
fn urlencode_arg(v: &str) -> String {
    if let Some(rest) = v.strip_prefix('=') {
        return encode_component(rest);
    }
    match v.find(['=', '@']) {
        Some(i) if v.as_bytes()[i] == b'=' => format!("{}={}", &v[..i], encode_component(&v[i + 1..])),
        Some(_) => v.to_string(),
        None => encode_component(v),
    }
}

fn build(p: Parsed) -> Result<RequestSpec> {
    let url = p
        .urls
        .iter()
        .find(|u| u.contains("://"))
        .or_else(|| p.urls.first())
        .cloned()
        .ok_or_else(|| Error::Import("curl command has no URL".into()))?;
    let mut url = if url.contains("://") || url.starts_with("{{") {
        url
    } else {
        format!("http://{url}")
    };
    let mut headers = p.headers;
    let mut body = None;
    let mut data = p.data;
    if p.get {
        append_query(&mut url, &data.join("&"));
        data.clear();
    }
    for q in &p.query {
        append_query(&mut url, q);
    }
    if !p.json.is_empty() {
        body = Some(p.json.concat());
        if !has_header(&headers, "content-type") {
            headers.push(header("Content-Type", "application/json"));
        }
        if !has_header(&headers, "accept") {
            headers.push(header("Accept", "application/json"));
        }
    } else if !data.is_empty() {
        body = Some(data.join("&"));
        if !has_header(&headers, "content-type") {
            headers.push(header("Content-Type", "application/x-www-form-urlencoded"));
        }
    } else if !p.form.is_empty() {
        body = Some(multipart(&p.form));
        if !has_header(&headers, "content-type") {
            headers.push(header("Content-Type", multipart_content_type()));
        }
    }
    if !p.cookies.is_empty() {
        headers.push(header("Cookie", p.cookies.join("; ")));
    }
    let method = match p.method {
        Some(m) => m,
        None if p.head => "HEAD".into(),
        None if body.is_some() => "POST".into(),
        None if p.upload => "PUT".into(),
        None => "GET".into(),
    };
    Ok(RequestSpec {
        kind: Kind::Http,
        method,
        url,
        headers,
        body,
        directives: Directives {
            auth: p.auth,
            timeout: p.timeout,
            insecure: p.insecure,
            ..Default::default()
        },
    })
}

/// A workspace with a single page, "Imported request".
pub fn import(cmd: &str) -> Result<ImportedWorkspace> {
    let spec = parse(cmd)?;
    let mut page = PageDraft::new("Imported request");
    page.request = Some(spec);
    Ok(ImportedWorkspace {
        name: "Imported request".into(),
        root: vec![Node {
            name: "Imported request".into(),
            markdown: page.render(),
            children: vec![],
        }],
        ..Default::default()
    })
}

/// Splits a command line like a POSIX shell. Stops at an unquoted `|`, `;`,
/// `&&` or `>` that starts a word (pipes and redirects after the command).
fn tokenize(s: &str) -> Result<Vec<String>> {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut i = 0;
    let unterminated = |q: &str| Error::Import(format!("unterminated {q} quote in curl command"));
    while i < n {
        let c = chars[i];
        match c {
            '\\' => {
                match chars.get(i + 1) {
                    Some('\n') => i += 2,
                    Some('\r') if chars.get(i + 2) == Some(&'\n') => i += 3,
                    Some(&next) => {
                        cur.push(next);
                        in_word = true;
                        i += 2;
                    }
                    None => i += 1,
                }
                continue;
            }
            '\'' => {
                in_word = true;
                i += 1;
                loop {
                    match chars.get(i) {
                        Some('\'') => break,
                        Some(&ch) => cur.push(ch),
                        None => return Err(unterminated("single")),
                    }
                    i += 1;
                }
                i += 1;
            }
            '"' => {
                in_word = true;
                i += 1;
                loop {
                    match chars.get(i) {
                        Some('"') => break,
                        Some('\\') => match chars.get(i + 1) {
                            Some('\n') => i += 1,
                            Some(&e @ ('"' | '\\' | '$' | '`')) => {
                                cur.push(e);
                                i += 1;
                            }
                            _ => cur.push('\\'),
                        },
                        Some(&ch) => cur.push(ch),
                        None => return Err(unterminated("double")),
                    }
                    i += 1;
                }
                i += 1;
            }
            '$' if chars.get(i + 1) == Some(&'\'') => {
                in_word = true;
                i += 2;
                loop {
                    match chars.get(i) {
                        Some('\'') => break,
                        Some('\\') => {
                            i += 1;
                            let Some(&e) = chars.get(i) else {
                                return Err(unterminated("$'"));
                            };
                            match e {
                                'n' => cur.push('\n'),
                                't' => cur.push('\t'),
                                'r' => cur.push('\r'),
                                'a' => cur.push('\u{7}'),
                                'b' => cur.push('\u{8}'),
                                'e' | 'E' => cur.push('\u{1b}'),
                                'f' => cur.push('\u{c}'),
                                'v' => cur.push('\u{b}'),
                                'x' | 'u' | 'U' => {
                                    let max = match e {
                                        'x' => 2,
                                        'u' => 4,
                                        _ => 8,
                                    };
                                    let hex: String = chars[i + 1..]
                                        .iter()
                                        .take(max)
                                        .take_while(|c| c.is_ascii_hexdigit())
                                        .collect();
                                    match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                                        Some(ch) if !hex.is_empty() => {
                                            cur.push(ch);
                                            i += hex.len();
                                        }
                                        _ => {
                                            cur.push('\\');
                                            cur.push(e);
                                        }
                                    }
                                }
                                other => cur.push(other), // \\ \' \" and unknown escapes
                            }
                        }
                        Some(&ch) => cur.push(ch),
                        None => return Err(unterminated("$'")),
                    }
                    i += 1;
                }
                i += 1;
            }
            c if c.is_whitespace() => {
                if in_word {
                    out.push(std::mem::take(&mut cur));
                    in_word = false;
                }
                i += 1;
            }
            '|' | ';' | '>' if !in_word => break,
            '&' if !in_word && chars.get(i + 1) == Some(&'&') => break,
            '#' if !in_word => {
                while i < n && chars[i] != '\n' {
                    i += 1;
                }
            }
            c => {
                cur.push(c);
                in_word = true;
                i += 1;
            }
        }
    }
    if in_word {
        out.push(cur);
    }
    Ok(out)
}

/// Renders a request as a curl command, one option per line. Variables are
/// left as written, so resolve the spec first for a command that runs as is.
pub fn to_curl(spec: &RequestSpec) -> String {
    let mut parts: Vec<String> = vec!["curl".into()];
    let mut headers = spec.headers.clone();
    let mut body = spec.body.clone();
    let mut method = spec.method.clone();
    if spec.kind == Kind::GraphQl {
        method = "POST".into();
        body = Some(graphql_json(body.as_deref().unwrap_or("")));
        if !has_header(&headers, "content-type") {
            headers.push(header("Content-Type", "application/json"));
        }
    }
    match method.as_str() {
        "GET" if body.is_none() => {}
        "HEAD" if body.is_none() => parts.push("-I".into()),
        m => parts.push(format!("-X {}", quote(m))),
    }
    parts.push(quote(&spec.url));
    for h in &headers {
        parts.push(format!("-H {}", quote(&format!("{}: {}", h.name, h.value))));
    }
    if let Some(auth) = &spec.directives.auth {
        let auth = auth.trim();
        let (scheme, rest) = auth.split_once(char::is_whitespace).unwrap_or((auth, ""));
        let rest = rest.trim();
        match scheme.to_ascii_lowercase().as_str() {
            "bearer" => parts.push(format!("-H {}", quote(&format!("Authorization: Bearer {rest}")))),
            "basic" => {
                let pair = match rest.split_once(char::is_whitespace) {
                    Some((u, p)) => format!("{u}:{}", p.trim()),
                    None => rest.to_string(),
                };
                parts.push(format!("-u {}", quote(&pair)));
            }
            "apikey" => {
                if let Some((name, value)) = rest.split_once(char::is_whitespace) {
                    parts.push(format!("-H {}", quote(&format!("{name}: {}", value.trim()))));
                }
            }
            _ => {}
        }
    }
    if let Some(b) = &body {
        parts.push(format!("--data-raw {}", quote(b)));
    }
    if spec.directives.insecure {
        parts.push("-k".into());
    }
    if !spec.directives.no_redirect {
        parts.push("-L".into());
    }
    if let Some(t) = spec.directives.timeout {
        parts.push(format!("-m {}", t.as_secs_f64()));
    }
    parts.join(" \\\n  ")
}

/// Mirrors the executor: query, then optionally a blank line and JSON variables.
fn graphql_json(body: &str) -> String {
    let (query, vars) = match body.rsplit_once("\n\n") {
        Some((q, v)) if v.trim_start().starts_with('{') => match serde_json::from_str::<serde_json::Value>(v) {
            Ok(v) => (q.trim(), Some(v)),
            Err(_) => (body.trim(), None),
        },
        _ => (body.trim(), None),
    };
    let mut obj = serde_json::Map::new();
    obj.insert("query".into(), query.into());
    if let Some(v) = vars {
        obj.insert("variables".into(), v);
    }
    serde_json::Value::Object(obj).to_string()
}

fn quote(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_./:=@%+,".contains(c)) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request;

    fn h(spec: &RequestSpec, name: &str) -> Option<String> {
        spec.header(name).map(str::to_string)
    }

    #[test]
    fn tokenizer() {
        let t = tokenize("curl 'a b' \"c \\\"d\\\" $x\" e\\ f \\\n  g $'h\\ni\\x41\\'' | jq .").unwrap();
        assert_eq!(t, ["curl", "a b", "c \"d\" $x", "e f", "g", "h\niA'"]);
        assert!(tokenize("curl 'open").is_err());
        assert_eq!(tokenize("curl x\r\n").unwrap(), ["curl", "x"]);
        assert_eq!(tokenize("curl a\\\r\n  b").unwrap(), ["curl", "a", "b"]);
    }

    #[test]
    fn simple_get() {
        let r = parse("curl https://api.example.com/users").unwrap();
        assert_eq!(
            (r.method.as_str(), r.url.as_str()),
            ("GET", "https://api.example.com/users")
        );
        assert!(r.headers.is_empty() && r.body.is_none());
        let r = parse("curl example.com/x").unwrap();
        assert_eq!(r.url, "http://example.com/x");
    }

    #[test]
    fn chrome_style_copy() {
        let cmd = r#"curl 'https://api.example.com/v1/orders?limit=2' \
  -H 'accept: application/json' \
  -H 'authorization: Bearer abc.def' \
  -b 'sid=123; theme=dark' \
  -H 'content-type: application/json' \
  --data-raw '{"item":"book","qty":2}' \
  --compressed"#;
        let r = parse(cmd).unwrap();
        assert_eq!(r.method, "POST");
        assert_eq!(r.url, "https://api.example.com/v1/orders?limit=2");
        assert_eq!(h(&r, "authorization").as_deref(), Some("Bearer abc.def"));
        assert_eq!(h(&r, "content-type").as_deref(), Some("application/json"));
        assert_eq!(h(&r, "cookie").as_deref(), Some("sid=123; theme=dark"));
        assert_eq!(r.body.as_deref(), Some(r#"{"item":"book","qty":2}"#));
    }

    #[test]
    fn data_forms() {
        let r = parse("curl -d a=1 --data b=2 --data-urlencode 'c=x y' --data-urlencode =z& http://h").unwrap();
        assert_eq!(r.method, "POST");
        assert_eq!(r.body.as_deref(), Some("a=1&b=2&c=x%20y&z%26"));
        assert_eq!(
            h(&r, "Content-Type").as_deref(),
            Some("application/x-www-form-urlencoded")
        );

        let r = parse("curl -G -d q=rust -d page=2 http://h/search?x=1").unwrap();
        assert_eq!(r.method, "GET");
        assert_eq!(r.url, "http://h/search?x=1&q=rust&page=2");
        assert!(r.body.is_none());

        let r = parse("curl -X PUT -d '{}' -H 'Content-Type: application/json' http://h").unwrap();
        assert_eq!(r.method, "PUT");
        assert_eq!(r.headers.len(), 1);
    }

    #[test]
    fn json_flag() {
        let r = parse(r#"curl --json '{"a":1}' https://h/x"#).unwrap();
        assert_eq!(r.method, "POST");
        assert_eq!(h(&r, "content-type").as_deref(), Some("application/json"));
        assert_eq!(h(&r, "accept").as_deref(), Some("application/json"));
        assert_eq!(r.body.as_deref(), Some(r#"{"a":1}"#));
    }

    #[test]
    fn short_clusters_and_long_equals() {
        let r = parse("curl -sSLk -XDELETE -H'X-A: 1' --user-agent=bot/1 --url=https://h/1 -m 2.5").unwrap();
        assert_eq!(r.method, "DELETE");
        assert_eq!(r.url, "https://h/1");
        assert_eq!(h(&r, "x-a").as_deref(), Some("1"));
        assert_eq!(h(&r, "user-agent").as_deref(), Some("bot/1"));
        assert!(r.directives.insecure);
        assert_eq!(r.directives.timeout, Some(Duration::from_millis(2500)));
    }

    #[test]
    fn auth_and_misc_flags() {
        let r = parse("curl -u alice:s3cret -A 'Agent X' -e https://ref http://h").unwrap();
        assert_eq!(r.directives.auth.as_deref(), Some("basic alice s3cret"));
        assert_eq!(h(&r, "user-agent").as_deref(), Some("Agent X"));
        assert_eq!(h(&r, "referer").as_deref(), Some("https://ref"));
        let r = parse("curl -u bob http://h").unwrap();
        assert_eq!(r.directives.auth.as_deref(), Some("basic bob:"));
        let r = parse("curl --oauth2-bearer tok http://h").unwrap();
        assert_eq!(r.directives.auth.as_deref(), Some("bearer tok"));
    }

    #[test]
    fn unknown_and_value_flags_do_not_become_the_url() {
        let r = parse("curl -o out.json -w '%{http_code}' --retry 3 --frobnicate -x proxy:1 https://h/ok -v").unwrap();
        assert_eq!(r.url, "https://h/ok");
        assert_eq!(r.method, "GET");
        assert!(parse("curl -v").is_err());
        assert!(parse("wget http://h").is_err());
        // Never panics on odd input.
        for odd in [
            "curl -",
            "curl --",
            "curl -H",
            "curl -X",
            "curl $'\\x'",
            "curl -F x",
            "curl -d",
        ] {
            let _ = parse(odd);
        }
    }

    #[test]
    fn head_form_and_cookie_file() {
        let r = parse("curl -I http://h").unwrap();
        assert_eq!(r.method, "HEAD");
        let r = parse("curl -F name=Ann -F photo=@me.png -b cookies.txt http://h/up").unwrap();
        assert_eq!(r.method, "POST");
        assert!(
            h(&r, "content-type")
                .unwrap()
                .starts_with("multipart/form-data; boundary=")
        );
        let body = r.body.clone().unwrap();
        assert!(body.contains("name=\"name\"\n\nAnn\n"));
        assert!(!body.contains("photo"));
        assert!(h(&r, "cookie").is_none());
    }

    #[test]
    fn to_curl_round_trips_through_parse() {
        let spec = RequestSpec {
            kind: Kind::Http,
            method: "POST".into(),
            url: "https://h/a?x=1&y=it's".into(),
            headers: vec![header("Content-Type", "application/json"), header("X-Q", "a 'b' c")],
            body: Some("{\"msg\": \"it's\"}".into()),
            directives: Directives {
                auth: Some("basic user pass".into()),
                insecure: true,
                timeout: Some(Duration::from_secs(5)),
                ..Default::default()
            },
        };
        let cmd = to_curl(&spec);
        assert!(cmd.contains("-L"));
        let back = parse(&cmd).unwrap();
        assert_eq!(back, spec, "{cmd}");

        let get = RequestSpec {
            kind: Kind::Http,
            method: "GET".into(),
            url: "http://h/x".into(),
            headers: vec![],
            body: None,
            directives: Directives {
                no_redirect: true,
                ..Default::default()
            },
        };
        assert_eq!(to_curl(&get), "curl \\\n  http://h/x");
    }

    #[test]
    fn to_curl_auth_and_graphql() {
        let mut spec = request::parse(
            "# @auth bearer T\nGRAPHQL http://h/gql\n\nquery { me }\n\n{\"a\": 1}",
            1,
        )
        .unwrap();
        let cmd = to_curl(&spec);
        assert!(cmd.contains("-X POST"));
        assert!(cmd.contains("'Authorization: Bearer T'"));
        assert!(cmd.contains(r#"{"query":"query { me }","variables":{"a":1}}"#), "{cmd}");
        spec.directives.auth = Some("apikey X-Key k1".into());
        assert!(to_curl(&spec).contains("-H 'X-Key: k1'"));
    }

    #[test]
    fn block_text_round_trip_from_curl() {
        let r = parse("curl -X PATCH https://h/u/1 -H 'Content-Type: application/json' -d '{\"a\":1}' -k").unwrap();
        let text = request::to_block_text(&r);
        assert_eq!(request::parse(&text, 1).unwrap(), r);
    }
}
