//! WebSocket blocks.
//!
//! ```text
//! WEBSOCKET wss://example.com/socket
//! Authorization: Bearer {{token}}
//!
//! {"subscribe": "orders"}
//! ===
//! {"subscribe": "prices"}
//! ```
//!
//! Headers go on the handshake. The body, if any, is the first message(s)
//! to send once the connection is open; a line that is exactly `===`
//! separates messages (JetBrains HTTP Client convention). After that the
//! caller sends more through [`WsHandle`] and receives [`WsEvent`]s on the
//! channel it passed to [`connect`].
//!
//! TLS uses rustls with the same crypto provider and certificate verifier
//! as the HTTP client (reqwest): the process-wide default provider if one
//! is installed, otherwise aws-lc-rs, and the platform verifier.

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use rustls::ClientConfig;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls_platform_verifier::BuilderVerifierExt;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderName, HeaderValue, header};
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::{self, Message};
use tokio_tungstenite::{Connector, WebSocketStream};

use crate::error::{Error, Result};
use crate::exec::DEFAULT_TIMEOUT;
use crate::request::{Kind, RequestSpec};

const USER_AGENT: &str = concat!("reqowl/", env!("CARGO_PKG_VERSION"));
/// How long to wait for the server's close frame after we send ours.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WsEvent {
    /// The handshake succeeded. `status` is normally 101.
    Opened {
        status: u16,
        headers: Vec<(String, String)>,
        elapsed: Duration,
    },
    /// A text message from the server.
    Text(String),
    /// A binary message from the server.
    Binary(Vec<u8>),
    /// A text message we sent (initial messages and [`WsHandle::send`]).
    Sent(String),
    /// The connection is over. Always the last event, sent exactly once.
    Closed { code: Option<u16>, reason: String },
    /// Something went wrong after the connection opened. `Closed` follows.
    Error(String),
}

#[derive(Debug)]
enum Command {
    Send(String),
    Close,
}

/// Talks to an open connection. Dropping every clone of the handle closes it.
#[derive(Debug, Clone)]
pub struct WsHandle {
    tx: mpsc::UnboundedSender<Command>,
}

impl WsHandle {
    /// Queues a text message. A [`WsEvent::Sent`] follows once it is written.
    pub fn send(&self, text: String) -> Result<()> {
        self.tx
            .send(Command::Send(text))
            .map_err(|_| Error::WebSocket("the connection is closed".into()))
    }

    /// Starts the closing handshake. [`WsEvent::Closed`] follows.
    pub fn close(&self) {
        let _ = self.tx.send(Command::Close);
    }

    /// True once the connection has ended.
    pub fn is_closed(&self) -> bool {
        self.tx.is_closed()
    }
}

/// The initial messages in a WebSocket block body: split on lines that are
/// exactly `===`, each trimmed of surrounding blank lines; empty ones dropped.
pub fn initial_messages(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    let mut flush = |cur: &mut Vec<&str>| {
        let msg = cur.join("\n");
        let msg = msg.trim_matches(|c| c == '\n' || c == '\r');
        if !msg.trim().is_empty() {
            out.push(msg.to_string());
        }
        cur.clear();
    };
    for line in body.lines() {
        if line.trim_end() == "===" {
            flush(&mut cur);
        } else {
            cur.push(line);
        }
    }
    flush(&mut cur);
    out
}

/// Opens a WebSocket for a resolved block (after [`RequestSpec::resolve`] and
/// auth). Returns once the handshake is done; [`WsEvent::Opened`] is the first
/// event on `events`, then [`WsEvent::Sent`] for each initial message.
/// `@timeout` limits the handshake only; `@insecure` accepts any certificate.
pub async fn connect(spec: &RequestSpec, events: mpsc::UnboundedSender<WsEvent>) -> Result<WsHandle> {
    if spec.kind != Kind::WebSocket {
        return Err(Error::WebSocket(format!(
            "not a WEBSOCKET block: {} {}",
            spec.method, spec.url
        )));
    }
    let mut req = spec
        .url
        .as_str()
        .into_client_request()
        .map_err(|e| Error::WebSocket(format!("invalid URL `{}`: {e}", spec.url)))?;
    let headers = req.headers_mut();
    for h in &spec.headers {
        let name = HeaderName::from_bytes(h.name.trim().as_bytes())
            .map_err(|_| Error::WebSocket(format!("invalid header name `{}`", h.name.trim())))?;
        let value = HeaderValue::from_str(&h.value)
            .map_err(|_| Error::WebSocket(format!("invalid value for header `{}`", h.name.trim())))?;
        headers.append(name, value);
    }
    if !headers.contains_key(header::USER_AGENT) {
        headers.insert(header::USER_AGENT, HeaderValue::from_static(USER_AGENT));
    }

    let connector = if req.uri().scheme_str() == Some("wss") {
        Connector::Rustls(tls_config(spec.directives.insecure)?)
    } else {
        Connector::Plain
    };
    let timeout = spec.directives.timeout.unwrap_or(DEFAULT_TIMEOUT);
    let started = Instant::now();
    let handshake = tokio_tungstenite::connect_async_tls_with_config(req, None, true, Some(connector));
    let (stream, resp) = tokio::time::timeout(timeout, handshake)
        .await
        .map_err(|_| Error::WebSocket(format!("handshake timed out after {timeout:?}")))?
        .map_err(handshake_error)?;

    let _ = events.send(WsEvent::Opened {
        status: resp.status().as_u16(),
        headers: resp
            .headers()
            .iter()
            .map(|(k, v)| {
                (
                    k.as_str().to_string(),
                    String::from_utf8_lossy(v.as_bytes()).into_owned(),
                )
            })
            .collect(),
        elapsed: started.elapsed(),
    });

    let initial = spec.body.as_deref().map(initial_messages).unwrap_or_default();
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(pump(stream, initial, rx, events));
    Ok(WsHandle { tx })
}

fn handshake_error(e: tungstenite::Error) -> Error {
    match e {
        tungstenite::Error::Http(resp) => {
            let status = resp.status();
            let body = resp
                .body()
                .as_deref()
                .map(|b| String::from_utf8_lossy(b).trim().to_string())
                .filter(|b| !b.is_empty())
                .map(|b| format!(": {b}"))
                .unwrap_or_default();
            Error::WebSocket(format!(
                "server answered {} {} instead of switching protocols{body}",
                status.as_u16(),
                status.canonical_reason().unwrap_or("")
            ))
        }
        e => Error::WebSocket(e.to_string()),
    }
}

/// Runs the connection: initial messages, then commands and incoming frames
/// until either side closes.
async fn pump<S>(
    stream: WebSocketStream<S>,
    initial: Vec<String>,
    mut rx: mpsc::UnboundedReceiver<Command>,
    events: mpsc::UnboundedSender<WsEvent>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let emit = |e: WsEvent| {
        let _ = events.send(e);
    };
    let (mut sink, mut stream) = stream.split();
    let mut close: Option<(Option<u16>, String)> = None;
    let mut deadline: Option<tokio::time::Instant> = None;

    for msg in initial {
        if let Err(e) = sink.send(Message::text(msg.clone())).await {
            emit(WsEvent::Error(e.to_string()));
            emit(WsEvent::Closed {
                code: None,
                reason: String::new(),
            });
            return;
        }
        emit(WsEvent::Sent(msg));
    }

    loop {
        tokio::select! {
            cmd = rx.recv(), if deadline.is_none() => match cmd {
                Some(Command::Send(text)) => match sink.send(Message::text(text.clone())).await {
                    Ok(()) => emit(WsEvent::Sent(text)),
                    Err(e) => {
                        emit(WsEvent::Error(e.to_string()));
                        break;
                    }
                },
                // Close requested, or every handle dropped.
                Some(Command::Close) | None => {
                    let frame = CloseFrame {
                        code: CloseCode::Normal,
                        reason: "".into(),
                    };
                    if sink.send(Message::Close(Some(frame))).await.is_err() {
                        break;
                    }
                    deadline = Some(tokio::time::Instant::now() + CLOSE_TIMEOUT);
                }
            },
            msg = stream.next() => match msg {
                Some(Ok(Message::Text(t))) => emit(WsEvent::Text(t.as_str().to_string())),
                Some(Ok(Message::Binary(b))) => emit(WsEvent::Binary(b.to_vec())),
                Some(Ok(Message::Close(frame))) => {
                    // tungstenite answers the close itself; keep reading until the stream ends.
                    close = Some(match frame {
                        Some(f) => (Some(u16::from(f.code)), f.reason.as_str().to_string()),
                        None => (None, String::new()),
                    });
                }
                Some(Ok(_)) => {}
                Some(Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed)) | None => break,
                Some(Err(e)) => {
                    emit(WsEvent::Error(e.to_string()));
                    break;
                }
            },
            _ = tokio::time::sleep_until(deadline.unwrap_or_else(tokio::time::Instant::now)), if deadline.is_some() => {
                break;
            }
        }
    }
    let (code, reason) = close.unwrap_or_default();
    emit(WsEvent::Closed { code, reason });
}

/// A rustls client config built the way reqwest builds its own: the
/// process-default crypto provider if installed, else aws-lc-rs; the
/// platform verifier, or no verification for `@insecure`.
pub(crate) fn tls_config(insecure: bool) -> Result<Arc<ClientConfig>> {
    let tls_err = |e: rustls::Error| Error::WebSocket(format!("TLS setup failed: {e}"));
    let provider = CryptoProvider::get_default()
        .cloned()
        .unwrap_or_else(|| Arc::new(rustls::crypto::aws_lc_rs::default_provider()));
    let builder = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(tls_err)?;
    let mut config = if insecure {
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAnyCert(provider)))
            .with_no_client_auth()
    } else {
        builder.with_platform_verifier().map_err(tls_err)?.with_no_client_auth()
    };
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Arc::new(config))
}

/// `@insecure`: any certificate and host name. Handshake signatures are
/// still checked so the peer must hold the key of the certificate it sent.
#[derive(Debug)]
struct AcceptAnyCert(Arc<CryptoProvider>);

impl ServerCertVerifier for AcceptAnyCert {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request;
    use std::sync::Mutex;
    use tokio::net::TcpListener;
    use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};

    /// Echo server on 127.0.0.1:0. Returns its address and the handshake
    /// headers it saw.
    #[allow(clippy::result_large_err)] // the error type is tungstenite's callback signature
    async fn echo_server() -> (String, Arc<Mutex<Vec<(String, String)>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        tokio::spawn(async move {
            while let Ok((sock, _)) = listener.accept().await {
                let seen = seen2.clone();
                tokio::spawn(async move {
                    let cb = |req: &Request, resp: Response| -> std::result::Result<Response, ErrorResponse> {
                        let mut s = seen.lock().unwrap();
                        for (k, v) in req.headers() {
                            s.push((k.to_string(), v.to_str().unwrap_or("").to_string()));
                        }
                        Ok(resp)
                    };
                    let Ok(mut ws) = tokio_tungstenite::accept_hdr_async(sock, cb).await else {
                        return;
                    };
                    while let Some(Ok(msg)) = ws.next().await {
                        match msg {
                            Message::Text(_) | Message::Binary(_) => {
                                if ws.send(msg).await.is_err() {
                                    break;
                                }
                            }
                            Message::Close(_) => {}
                            _ => {}
                        }
                    }
                });
            }
        });
        (format!("ws://{addr}"), seen)
    }

    async fn next(rx: &mut mpsc::UnboundedReceiver<WsEvent>) -> WsEvent {
        tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("timed out waiting for an event")
            .expect("event channel closed")
    }

    #[test]
    fn splits_initial_messages() {
        assert_eq!(
            initial_messages("{\"a\": 1}\n===\n\nsecond\nline\n\n===\n===\nthird"),
            ["{\"a\": 1}", "second\nline", "third"]
        );
        assert_eq!(initial_messages("a == b\n ===x"), ["a == b\n ===x"]);
        assert!(initial_messages("\n===\n").is_empty());
    }

    #[test]
    fn tls_configs_build() {
        for insecure in [false, true] {
            let c = tls_config(insecure).unwrap();
            assert_eq!(c.alpn_protocols, [b"http/1.1".to_vec()]);
        }
    }

    #[tokio::test]
    async fn echo_send_close() {
        let (url, seen) = echo_server().await;
        let block =
            format!("# @auth bearer t0k\nWEBSOCKET {url}/chat\nX-Client: reqowl-test\n\nhello\n===\n{{\"n\": 2}}\n");
        let mut spec = request::parse(&block, 1).unwrap();
        spec.apply_auth().unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let handle = connect(&spec, tx).await.unwrap();

        match next(&mut rx).await {
            WsEvent::Opened { status, headers, .. } => {
                assert_eq!(status, 101);
                assert!(headers.iter().any(|(k, v)| k == "upgrade" && v == "websocket"));
            }
            e => panic!("{e:?}"),
        }
        assert_eq!(next(&mut rx).await, WsEvent::Sent("hello".into()));
        assert_eq!(next(&mut rx).await, WsEvent::Sent("{\"n\": 2}".into()));
        assert_eq!(next(&mut rx).await, WsEvent::Text("hello".into()));
        assert_eq!(next(&mut rx).await, WsEvent::Text("{\"n\": 2}".into()));

        handle.send("more".into()).unwrap();
        assert_eq!(next(&mut rx).await, WsEvent::Sent("more".into()));
        assert_eq!(next(&mut rx).await, WsEvent::Text("more".into()));

        handle.close();
        assert_eq!(
            next(&mut rx).await,
            WsEvent::Closed {
                code: Some(1000),
                reason: String::new()
            }
        );
        assert!(rx.recv().await.is_none(), "Closed is the last event");
        assert!(handle.is_closed());
        assert!(handle.send("late".into()).is_err());

        let seen = seen.lock().unwrap();
        let get = |n: &str| seen.iter().find(|(k, _)| k == n).map(|(_, v)| v.as_str());
        assert_eq!(get("authorization"), Some("Bearer t0k"));
        assert_eq!(get("x-client"), Some("reqowl-test"));
        assert!(get("user-agent").unwrap().starts_with("reqowl/"));
    }

    #[tokio::test]
    async fn dropping_handle_closes() {
        let (url, _) = echo_server().await;
        let spec = request::parse(&format!("WEBSOCKET {url}"), 1).unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let handle = connect(&spec, tx).await.unwrap();
        assert!(matches!(next(&mut rx).await, WsEvent::Opened { .. }));
        drop(handle);
        assert!(matches!(next(&mut rx).await, WsEvent::Closed { .. }));
    }

    #[tokio::test]
    async fn connection_refused() {
        // Bind then drop to get a port nothing listens on.
        let port = TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        // Windows retries a refused connection for about two seconds before
        // giving up, so the handshake timeout has to be longer than that.
        let spec = request::parse(&format!("# @timeout 10s\nWEBSOCKET ws://127.0.0.1:{port}/"), 1).unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let err = connect(&spec, tx).await.unwrap_err();
        assert!(matches!(err, Error::WebSocket(_)), "{err}");
        assert!(err.to_string().to_lowercase().contains("refused"), "{err}");
        assert!(rx.recv().await.is_none());

        let spec = request::parse("GET http://h", 1).unwrap();
        assert!(connect(&spec, mpsc::unbounded_channel().0).await.is_err());
    }
}
