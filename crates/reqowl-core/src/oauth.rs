//! OAuth2 access tokens for `# @auth oauth2 <profile>`.
//!
//! A profile is a set of ordinary variables named `<profile>.<setting>`, so
//! it can differ per environment and keep its secret in `*.local.toml`:
//!
//! ```toml
//! # env/dev.toml (quote the keys: unquoted dotted keys are TOML tables)
//! "api.grant" = "client_credentials"   # or authorization_code, password
//! "api.token_url" = "https://auth.example.com/oauth/token"
//! "api.client_id" = "reqowl"
//! "api.scope" = "orders:read"
//!
//! # env/dev.local.toml
//! "api.client_secret" = "..."
//! ```
//!
//! Other settings: `auth_url` (authorization_code), `username` and `password`
//! (password grant), `redirect_port` (authorization_code loopback port,
//! default 0 = any free port), `audience` (sent as an extra parameter).
//!
//! Client credentials go in the form body (`client_secret_post`). The
//! authorization code flow uses PKCE (S256) and a one-shot HTTP listener on
//! `http://127.0.0.1:<port>/callback`; the caller opens the browser.
//!
//! Tokens are cached in memory per profile until shortly before they expire;
//! then the refresh token is used if there is one, else a new token is fetched.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::error::{Error, Result};
use crate::request::RequestSpec;
use crate::vars::Context;

/// How long the authorization code flow waits for the browser redirect.
pub const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);
/// Path of the loopback redirect URI.
pub const REDIRECT_PATH: &str = "/callback";
/// A token is renewed this long before it expires (at most half its lifetime).
const EXPIRY_MARGIN: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grant {
    ClientCredentials,
    AuthorizationCode,
    Password,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthProfile {
    pub name: String,
    pub grant: Grant,
    pub token_url: String,
    pub auth_url: Option<String>,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub scope: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    /// Loopback port for the authorization code redirect; 0 picks a free one.
    pub redirect_port: u16,
    pub audience: Option<String>,
}

impl OAuthProfile {
    /// Reads `<name>.<setting>` variables. Empty values count as unset.
    pub fn from_ctx(name: &str, ctx: &Context) -> Result<Self> {
        if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || "_-.".contains(c)) {
            return Err(Error::OAuth(format!(
                "bad profile name `{name}`; use letters, digits, `_`, `-` or `.`"
            )));
        }
        let get = |key: &str| -> Result<Option<String>> {
            let var = format!("{name}.{key}");
            match ctx.render(&format!("{{{{{var}}}}}")) {
                Ok(v) => Ok(Some(v.trim().to_string()).filter(|v| !v.is_empty())),
                Err(Error::UndefinedVariables(m)) if m == [var] => Ok(None),
                Err(e) => Err(e),
            }
        };
        let need = |key: &str| {
            get(key)?.ok_or_else(|| Error::OAuth(format!("profile `{name}` needs the variable `{name}.{key}`")))
        };

        let grant = match need("grant")?.as_str() {
            "client_credentials" => Grant::ClientCredentials,
            "authorization_code" => Grant::AuthorizationCode,
            "password" => Grant::Password,
            other => {
                return Err(Error::OAuth(format!(
                    "`{name}.grant` is `{other}`; expected client_credentials, authorization_code or password"
                )));
            }
        };
        let redirect_port = match get("redirect_port")? {
            None => 0,
            Some(p) => p
                .parse()
                .map_err(|_| Error::OAuth(format!("`{name}.redirect_port` must be a port number, got `{p}`")))?,
        };
        let profile = OAuthProfile {
            name: name.to_string(),
            grant,
            token_url: need("token_url")?,
            auth_url: if grant == Grant::AuthorizationCode {
                Some(need("auth_url")?)
            } else {
                get("auth_url")?
            },
            client_id: need("client_id")?,
            client_secret: get("client_secret")?,
            scope: get("scope")?,
            username: if grant == Grant::Password {
                Some(need("username")?)
            } else {
                get("username")?
            },
            password: if grant == Grant::Password {
                Some(need("password")?)
            } else {
                get("password")?
            },
            redirect_port,
            audience: get("audience")?,
        };
        Ok(profile)
    }

    /// Cache key: a profile name can mean different servers in different
    /// environments, so the endpoint and client are part of it.
    fn cache_key(&self) -> String {
        format!(
            "{}\n{}\n{}\n{}",
            self.name,
            self.token_url,
            self.client_id,
            self.scope.as_deref().unwrap_or("")
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// When to stop using the token (already moved earlier by a safety margin).
    pub expires_at: Option<Instant>,
}

impl Token {
    pub fn is_fresh(&self) -> bool {
        self.expires_at.is_none_or(|t| Instant::now() < t)
    }
}

/// In-memory tokens, keyed by profile. Share one across sessions (e.g. in an
/// `Arc`) so that opening another page does not sign in again.
#[derive(Debug, Default)]
pub struct TokenCache {
    tokens: Mutex<HashMap<String, Token>>,
}

impl TokenCache {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Token>> {
        self.tokens.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn get(&self, profile: &OAuthProfile) -> Option<Token> {
        self.lock().get(&profile.cache_key()).cloned()
    }

    pub fn insert(&self, profile: &OAuthProfile, token: Token) {
        self.lock().insert(profile.cache_key(), token);
    }

    /// Forgets the tokens of a profile (in every environment).
    pub fn remove(&self, profile_name: &str) {
        let prefix = format!("{profile_name}\n");
        self.lock().retain(|k, _| !k.starts_with(&prefix));
    }

    pub fn clear(&self) {
        self.lock().clear();
    }
}

/// Opens a sign-in URL: the app opens the system browser, the CLI prints it.
pub type BrowserOpener = Arc<dyn Fn(&str) + Send + Sync>;

/// Everything a session needs to turn `@auth oauth2 PROFILE` into a header.
#[derive(Default)]
pub struct OAuthState {
    pub cache: Arc<TokenCache>,
    /// Opens the sign-in URL for the authorization code grant. Without one,
    /// that grant fails instead of waiting for a redirect that never comes.
    pub open_browser: Option<BrowserOpener>,
    client: OnceLock<reqwest::Client>,
}

impl OAuthState {
    /// The HTTP client for token endpoints, created on first use.
    pub fn client(&self) -> &reqwest::Client {
        self.client.get_or_init(reqwest::Client::new)
    }

    /// If `spec` uses `@auth oauth2 PROFILE`, reads the profile from `ctx`,
    /// gets a token and sets `Authorization: Bearer <token>`.
    pub async fn authorize(&self, spec: &mut RequestSpec, ctx: &Context) -> Result<()> {
        let Some(name) = spec.oauth_profile() else {
            return Ok(());
        };
        let profile = OAuthProfile::from_ctx(name, ctx)?;
        let ignore = |_: &str| {};
        let open_browser: &(dyn Fn(&str) + Send + Sync) = match &self.open_browser {
            Some(f) => f.as_ref(),
            None if profile.grant == Grant::AuthorizationCode => {
                return Err(Error::OAuth(format!(
                    "profile `{}` signs in through a browser, but no browser opener is set",
                    profile.name
                )));
            }
            None => &ignore,
        };
        let token = access_token(&profile, &self.cache, self.client(), open_browser).await?;
        spec.set_header("Authorization", format!("Bearer {token}"));
        Ok(())
    }
}

/// A valid access token for `profile`: cached, refreshed, or newly fetched.
/// `open_browser` is called with the sign-in URL for the authorization code
/// grant (the app opens the system browser; the CLI prints the URL).
pub async fn access_token(
    profile: &OAuthProfile,
    cache: &TokenCache,
    client: &reqwest::Client,
    open_browser: &(dyn Fn(&str) + Send + Sync),
) -> Result<String> {
    let cached = cache.get(profile);
    if let Some(t) = &cached
        && t.is_fresh()
    {
        return Ok(t.access_token.clone());
    }

    let mut token = None;
    if let Some(refresh) = cached.and_then(|t| t.refresh_token) {
        let params = vec![
            ("grant_type", "refresh_token".to_string()),
            ("refresh_token", refresh.clone()),
        ];
        // A rejected refresh token (revoked, expired) falls through to a full fetch.
        if let Ok(mut t) = token_request(profile, client, params).await {
            t.refresh_token.get_or_insert(refresh);
            token = Some(t);
        }
    }
    let token = match token {
        Some(t) => t,
        None => fetch(profile, client, open_browser).await?,
    };
    cache.insert(profile, token.clone());
    Ok(token.access_token)
}

async fn fetch(
    profile: &OAuthProfile,
    client: &reqwest::Client,
    open_browser: &(dyn Fn(&str) + Send + Sync),
) -> Result<Token> {
    let mut params = Vec::new();
    match profile.grant {
        Grant::ClientCredentials => {
            params.push(("grant_type", "client_credentials".to_string()));
        }
        Grant::Password => {
            params.push(("grant_type", "password".to_string()));
            params.push(("username", profile.username.clone().unwrap_or_default()));
            params.push(("password", profile.password.clone().unwrap_or_default()));
        }
        Grant::AuthorizationCode => return authorization_code(profile, client, open_browser).await,
    }
    if let Some(s) = &profile.scope {
        params.push(("scope", s.clone()));
    }
    if let Some(a) = &profile.audience {
        params.push(("audience", a.clone()));
    }
    token_request(profile, client, params).await
}

/// POSTs to the token endpoint. Adds the client credentials to `params`.
async fn token_request(
    profile: &OAuthProfile,
    client: &reqwest::Client,
    mut params: Vec<(&str, String)>,
) -> Result<Token> {
    params.push(("client_id", profile.client_id.clone()));
    if let Some(s) = &profile.client_secret {
        params.push(("client_secret", s.clone()));
    }
    let url = &profile.token_url;
    let started = Instant::now();
    let resp = client
        .post(url)
        .header(reqwest::header::ACCEPT, "application/json")
        .form(&params)
        .send()
        .await
        .map_err(|e| Error::OAuth(format!("token request to {url} failed: {}", error_chain(&e))))?;
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| Error::OAuth(format!("reading token response from {url}: {}", error_chain(&e))))?;
    let json: Option<Value> = serde_json::from_str(&body).ok();

    if !status.is_success() {
        let detail = json
            .as_ref()
            .and_then(|j| {
                let err = j.get("error")?.as_str()?;
                Some(match j.get("error_description").and_then(Value::as_str) {
                    Some(d) => format!("{err}: {d}"),
                    None => err.to_string(),
                })
            })
            .unwrap_or_else(|| truncate(&body, 200));
        return Err(Error::OAuth(format!(
            "token endpoint {url} answered {}: {detail}",
            status.as_u16()
        )));
    }
    let json = json.ok_or_else(|| Error::OAuth(format!("token response from {url} is not JSON")))?;
    let str_field = |k: &str| json.get(k).and_then(Value::as_str).map(str::to_string);
    let access_token = str_field("access_token")
        .ok_or_else(|| Error::OAuth(format!("token response from {url} has no access_token")))?;
    // Some servers send expires_in as a string.
    let expires_in = json.get("expires_in").and_then(|v| match v {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    });
    let expires_at = expires_in.map(|secs| {
        let life = Duration::from_secs(secs);
        started + life - EXPIRY_MARGIN.min(life / 2)
    });
    Ok(Token {
        access_token,
        refresh_token: str_field("refresh_token"),
        expires_at,
    })
}

async fn authorization_code(
    profile: &OAuthProfile,
    client: &reqwest::Client,
    open_browser: &(dyn Fn(&str) + Send + Sync),
) -> Result<Token> {
    let auth_url = profile
        .auth_url
        .as_deref()
        .ok_or_else(|| Error::OAuth(format!("profile `{}` needs `{0}.auth_url`", profile.name)))?;
    let mut url = reqwest::Url::parse(auth_url)
        .map_err(|e| Error::OAuth(format!("`{}.auth_url` is not a URL: {e}", profile.name)))?;

    let listener = TcpListener::bind(("127.0.0.1", profile.redirect_port))
        .await
        .map_err(|e| {
            Error::OAuth(format!(
                "cannot listen on 127.0.0.1:{} for the sign-in redirect: {e}",
                profile.redirect_port
            ))
        })?;
    let port = listener
        .local_addr()
        .map_err(|e| Error::OAuth(format!("loopback listener: {e}")))?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}{REDIRECT_PATH}");

    let verifier = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>());
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 16]>());
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("response_type", "code")
            .append_pair("client_id", &profile.client_id)
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("state", &state)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256");
        if let Some(s) = &profile.scope {
            q.append_pair("scope", s);
        }
        if let Some(a) = &profile.audience {
            q.append_pair("audience", a);
        }
    }

    open_browser(url.as_str());
    let code = tokio::time::timeout(LOGIN_TIMEOUT, wait_for_code(&listener, &state))
        .await
        .map_err(|_| Error::OAuth(format!("no sign-in redirect within {} s", LOGIN_TIMEOUT.as_secs())))??;
    drop(listener);

    let params = vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("code_verifier", verifier),
    ];
    token_request(profile, client, params).await
}

/// Serves the loopback listener until the redirect arrives. Other requests
/// (favicon, browser preconnects) get a 404 and are otherwise ignored.
async fn wait_for_code(listener: &TcpListener, state: &str) -> Result<String> {
    loop {
        let (mut sock, _) = listener
            .accept()
            .await
            .map_err(|e| Error::OAuth(format!("loopback listener: {e}")))?;
        let Some(target) = read_request_target(&mut sock).await else {
            continue;
        };
        let Ok(url) = reqwest::Url::parse(&format!("http://127.0.0.1{target}")) else {
            respond(&mut sock, 400, "Bad request").await;
            continue;
        };
        if url.path() != REDIRECT_PATH {
            respond(&mut sock, 404, "Not found").await;
            continue;
        }
        let params: HashMap<String, String> = url.query_pairs().into_owned().collect();
        if let Some(err) = params.get("error") {
            let msg = match params.get("error_description") {
                Some(d) => format!("{err}: {d}"),
                None => err.clone(),
            };
            respond(&mut sock, 400, &format!("Sign-in failed: {msg}")).await;
            return Err(Error::OAuth(format!("authorization server returned an error: {msg}")));
        }
        if params.get("state").map(String::as_str) != Some(state) {
            respond(&mut sock, 400, "Sign-in rejected: the state parameter does not match.").await;
            return Err(Error::OAuth(
                "sign-in redirect had the wrong state parameter; rejected it".into(),
            ));
        }
        let Some(code) = params.get("code").filter(|c| !c.is_empty()) else {
            respond(&mut sock, 400, "Sign-in failed: no code in the redirect.").await;
            return Err(Error::OAuth("sign-in redirect had no code".into()));
        };
        respond(&mut sock, 200, "Signed in. You can close this tab.").await;
        return Ok(code.clone());
    }
}

/// The request target of an HTTP request (`/callback?code=...`), or `None`
/// if the client sends nothing sensible within a few seconds.
async fn read_request_target(sock: &mut TcpStream) -> Option<String> {
    let mut buf = Vec::with_capacity(1024);
    let read = async {
        let mut chunk = [0u8; 1024];
        while !buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.len() < 16 * 1024 {
            let n = sock.read(&mut chunk).await.ok()?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        Some(())
    };
    tokio::time::timeout(Duration::from_secs(5), read).await.ok()??;
    let text = String::from_utf8_lossy(&buf);
    let mut parts = text.lines().next()?.split_whitespace();
    let _method = parts.next()?;
    let target = parts.next()?;
    target.starts_with('/').then(|| target.to_string())
}

async fn respond(sock: &mut TcpStream, status: u16, message: &str) {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Bad Request",
    };
    let body = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>reqowl</title></head>\
         <body style=\"font-family: sans-serif; margin: 3em\"><p>{}</p></body></html>",
        html_escape(message)
    );
    let resp = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
         Connection: close\r\nCache-Control: no-store\r\n\r\n{body}",
        body.len()
    );
    let _ = sock.write_all(resp.as_bytes()).await;
    let _ = sock.shutdown().await;
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn truncate(s: &str, max: usize) -> String {
    let s = s.trim();
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vars::Layer;
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    fn ctx(kv: &[(&str, &str)]) -> Context {
        Context {
            layers: vec![Layer {
                label: "env".into(),
                vars: kv.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
                secret: false,
            }],
            ..Default::default()
        }
    }

    fn no_browser(url: &str) {
        panic!("browser opened unexpectedly: {url}");
    }

    fn form(req: &Request) -> HashMap<String, String> {
        reqwest::Url::parse(&format!("http://x/?{}", String::from_utf8_lossy(&req.body)))
            .unwrap()
            .query_pairs()
            .into_owned()
            .collect()
    }

    fn cc_profile(server: &MockServer) -> OAuthProfile {
        let p = OAuthProfile::from_ctx(
            "api",
            &ctx(&[
                ("api.grant", "client_credentials"),
                ("api.token_url", "{{auth_base}}/token"),
                ("auth_base", &server.uri()),
                ("api.client_id", "cid"),
                ("api.client_secret", "s3cret"),
                ("api.scope", "read write"),
                ("api.audience", "https://api"),
            ]),
        )
        .unwrap();
        // Settings are templates like any other variable.
        assert_eq!(p.token_url, format!("{}/token", server.uri()));
        p
    }

    #[test]
    fn profile_from_ctx() {
        let c = ctx(&[
            ("p.grant", "password"),
            ("p.token_url", "http://t"),
            ("p.client_id", "c"),
            ("p.username", "u"),
            ("p.password", "pw"),
            ("p.redirect_port", "8123"),
            ("p.scope", ""),
        ]);
        let p = OAuthProfile::from_ctx("p", &c).unwrap();
        assert_eq!(p.grant, Grant::Password);
        assert_eq!((p.username.as_deref(), p.password.as_deref()), (Some("u"), Some("pw")));
        assert_eq!(p.redirect_port, 8123);
        assert_eq!(p.scope, None, "empty means unset");
        assert_eq!(p.client_secret, None);

        let err = |kv: &[(&str, &str)]| OAuthProfile::from_ctx("p", &ctx(kv)).unwrap_err().to_string();
        assert!(err(&[]).contains("p.grant"));
        assert!(err(&[("p.grant", "implicit")]).contains("expected"));
        assert!(
            err(&[
                ("p.grant", "authorization_code"),
                ("p.token_url", "t"),
                ("p.client_id", "c")
            ])
            .contains("p.auth_url")
        );
        // An undefined variable inside a setting is reported as itself, not as "missing setting".
        let e = OAuthProfile::from_ctx("p", &ctx(&[("p.grant", "{{nope}}")])).unwrap_err();
        assert!(matches!(e, Error::UndefinedVariables(ref v) if v == &["nope"]), "{e}");
        assert!(OAuthProfile::from_ctx("bad name", &c).is_err());
    }

    #[tokio::test]
    async fn client_credentials_and_cache() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(|r: &Request| !r.headers.contains_key("authorization"))
            .and(|r: &Request| {
                let f = form(r);
                f.get("grant_type").map(String::as_str) == Some("client_credentials")
                    && f.get("client_id").map(String::as_str) == Some("cid")
                    && f.get("client_secret").map(String::as_str) == Some("s3cret")
                    && f.get("scope").map(String::as_str) == Some("read write")
                    && f.get("audience").map(String::as_str) == Some("https://api")
            })
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({"access_token": "at-1", "token_type": "Bearer", "expires_in": 3600}),
                ),
            )
            .expect(1)
            .mount(&server)
            .await;

        let p = cc_profile(&server);
        let cache = TokenCache::new();
        let client = reqwest::Client::new();
        assert_eq!(access_token(&p, &cache, &client, &no_browser).await.unwrap(), "at-1");
        // Cached: the mock's expect(1) fails on drop if this hits the server.
        assert_eq!(access_token(&p, &cache, &client, &no_browser).await.unwrap(), "at-1");
        assert!(cache.get(&p).unwrap().is_fresh());
        cache.remove("api");
        assert!(cache.get(&p).is_none());
    }

    #[tokio::test]
    async fn expiry_refresh_and_fallback() {
        let server = MockServer::start().await;
        // First fetch: already expired, with a refresh token.
        Mock::given(method("POST"))
            .and(body_string_contains("grant_type=client_credentials"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({"access_token": "at-1", "expires_in": "0", "refresh_token": "rt-1"}),
                ),
            )
            .up_to_n_times(1)
            .expect(1)
            .mount(&server)
            .await;
        // Refresh: no new refresh token in the answer, so rt-1 is kept.
        Mock::given(method("POST"))
            .and(body_string_contains("grant_type=refresh_token"))
            .and(body_string_contains("refresh_token=rt-1"))
            .and(body_string_contains("client_secret=s3cret"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"access_token": "at-2", "expires_in": 0})),
            )
            .up_to_n_times(1)
            .expect(1)
            .mount(&server)
            .await;

        let p = cc_profile(&server);
        let cache = TokenCache::new();
        let client = reqwest::Client::new();
        assert_eq!(access_token(&p, &cache, &client, &no_browser).await.unwrap(), "at-1");
        assert!(!cache.get(&p).unwrap().is_fresh());
        assert_eq!(access_token(&p, &cache, &client, &no_browser).await.unwrap(), "at-2");
        assert_eq!(cache.get(&p).unwrap().refresh_token.as_deref(), Some("rt-1"));

        // The refresh token is now rejected: fall back to client credentials.
        server.reset().await;
        Mock::given(method("POST"))
            .and(body_string_contains("grant_type=refresh_token"))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({"error": "invalid_grant"})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("grant_type=client_credentials"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"access_token": "at-3", "expires_in": 60})),
            )
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(access_token(&p, &cache, &client, &no_browser).await.unwrap(), "at-3");
    }

    #[tokio::test]
    async fn token_errors() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(401)
                    .set_body_json(serde_json::json!({"error": "invalid_client", "error_description": "bad secret"})),
            )
            .mount(&server)
            .await;
        let p = cc_profile(&server);
        let e = access_token(&p, &TokenCache::new(), &reqwest::Client::new(), &no_browser)
            .await
            .unwrap_err()
            .to_string();
        assert!(e.contains("401") && e.contains("invalid_client: bad secret"), "{e}");
    }

    #[tokio::test]
    async fn password_grant() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("grant_type=password"))
            .and(body_string_contains("username=alice"))
            .and(body_string_contains("password=p%40ss"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"access_token": "pw-at"})))
            .expect(1)
            .mount(&server)
            .await;
        let p = OAuthProfile::from_ctx(
            "pw",
            &ctx(&[
                ("pw.grant", "password"),
                ("pw.token_url", &format!("{}/token", server.uri())),
                ("pw.client_id", "cid"),
                ("pw.username", "alice"),
                ("pw.password", "p@ss"),
            ]),
        )
        .unwrap();
        let t = access_token(&p, &TokenCache::new(), &reqwest::Client::new(), &no_browser).await;
        assert_eq!(t.unwrap(), "pw-at");
    }

    fn code_profile(server: &MockServer) -> OAuthProfile {
        OAuthProfile::from_ctx(
            "web",
            &ctx(&[
                ("web.grant", "authorization_code"),
                ("web.auth_url", "https://auth.example/authorize?prompt=login"),
                ("web.token_url", &format!("{}/token", server.uri())),
                ("web.client_id", "web-client"),
                ("web.scope", "openid"),
            ]),
        )
        .unwrap()
    }

    /// A browser stand-in: reads the sign-in URL, then GETs the redirect URI
    /// with `code` and the given state (or the real one). Records the
    /// sign-in URL and the loopback page it got back.
    struct FakeBrowser {
        auth_url: Arc<Mutex<Option<reqwest::Url>>>,
        page: Arc<Mutex<Option<(u16, String)>>>,
    }

    impl FakeBrowser {
        fn new() -> Self {
            FakeBrowser {
                auth_url: Arc::default(),
                page: Arc::default(),
            }
        }

        fn opener(&self, state_override: Option<&'static str>) -> impl Fn(&str) + Send + Sync + 'static {
            let auth_url = self.auth_url.clone();
            let page = self.page.clone();
            move |url: &str| {
                let url = reqwest::Url::parse(url).unwrap();
                let q: HashMap<String, String> = url.query_pairs().into_owned().collect();
                *auth_url.lock().unwrap() = Some(url);
                let state = state_override.map(str::to_string).unwrap_or_else(|| q["state"].clone());
                let mut redirect = reqwest::Url::parse(&q["redirect_uri"]).unwrap();
                redirect
                    .query_pairs_mut()
                    .append_pair("code", "the-code")
                    .append_pair("state", &state);
                let page = page.clone();
                tokio::spawn(async move {
                    // Browsers often ask for a favicon first; it must not end the flow.
                    let base = redirect.origin().ascii_serialization();
                    let _ = reqwest::get(format!("{base}/favicon.ico")).await;
                    let resp = reqwest::get(redirect).await.unwrap();
                    let status = resp.status().as_u16();
                    *page.lock().unwrap() = Some((status, resp.text().await.unwrap()));
                });
            }
        }
    }

    #[tokio::test]
    async fn authorization_code_pkce() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("grant_type=authorization_code"))
            .and(body_string_contains("code=the-code"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                serde_json::json!({"access_token": "code-at", "expires_in": 3600, "refresh_token": "rt"}),
            ))
            .expect(1)
            .mount(&server)
            .await;

        let p = code_profile(&server);
        let browser = FakeBrowser::new();
        let opener = browser.opener(None);
        let cache = TokenCache::new();
        let client = reqwest::Client::new();
        assert_eq!(access_token(&p, &cache, &client, &opener).await.unwrap(), "code-at");
        // Cached: a second call neither opens the browser nor hits the server.
        assert_eq!(access_token(&p, &cache, &client, &no_browser).await.unwrap(), "code-at");

        let auth_url = browser.auth_url.lock().unwrap().clone().unwrap();
        assert!(
            auth_url
                .as_str()
                .starts_with("https://auth.example/authorize?prompt=login&")
        );
        let q: HashMap<String, String> = auth_url.query_pairs().into_owned().collect();
        assert_eq!(q["response_type"], "code");
        assert_eq!(q["client_id"], "web-client");
        assert_eq!(q["scope"], "openid");
        assert_eq!(q["code_challenge_method"], "S256");
        assert!(q["redirect_uri"].starts_with("http://127.0.0.1:") && q["redirect_uri"].ends_with("/callback"));

        // The verifier sent to the token endpoint matches the challenge.
        let reqs = server.received_requests().await.unwrap();
        let f = form(&reqs[0]);
        assert_eq!(f["redirect_uri"], q["redirect_uri"]);
        assert_eq!(f["client_id"], "web-client");
        let verifier = &f["code_verifier"];
        assert!(verifier.len() >= 43);
        assert_eq!(
            URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
            q["code_challenge"]
        );

        // The fake browser's GET may finish after the token exchange.
        for _ in 0..50 {
            if browser.page.lock().unwrap().is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let (status, page) = browser.page.lock().unwrap().clone().unwrap();
        assert_eq!(status, 200);
        assert!(page.contains("You can close this tab"), "{page}");
    }

    #[tokio::test]
    async fn authorization_code_state_mismatch() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"access_token": "x"})))
            .expect(0)
            .mount(&server)
            .await;
        let p = code_profile(&server);
        let browser = FakeBrowser::new();
        let opener = browser.opener(Some("forged"));
        let e = access_token(&p, &TokenCache::new(), &reqwest::Client::new(), &opener)
            .await
            .unwrap_err();
        assert!(e.to_string().contains("state"), "{e}");
    }
}
