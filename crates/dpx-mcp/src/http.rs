//! The Streamable HTTP server (ADR-43): rmcp's `StreamableHttpService` hosted with axum on
//! `127.0.0.1` only, path `/mcp`, on a tokio runtime thread of its own.
//!
//! Every request passes [`guard`] first: a `Host` other than `127.0.0.1:<port>` or
//! `localhost:<port>` gets 403 (DNS rebinding), any `Origin` header gets 403 (no browser may
//! call it), a missing or wrong `Authorization: Bearer <token>` gets 401 (constant-time
//! compare), a body over [`MAX_BODY`] gets 413. Tool calls and resource reads get their
//! arrival-order ticket there. The server is stateless (no MCP sessions) and answers with
//! plain JSON, so every POST is one request and its answer.
//!
//! With an endpoint directory the server first takes [`endpoint::ENDPOINT_LOCK`] (another
//! server holding it makes the start fail: one server per config directory), then writes
//! the endpoint file with a token made for this start only, which it accepts besides the
//! configured one; on stop it removes the file and releases the lock.

use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderValue, Method, Request, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use rmcp::transport::streamable_http_server::session::never::NeverSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use tokio_util::sync::CancellationToken;

use crate::endpoint::{self, Endpoint};
use crate::jobs::Executor;
use crate::seq::Sequencer;
use crate::server::McpService;
use crate::state::ServerState;

/// The port tried first.
pub const DEFAULT_PORT: u16 = 8765;
/// Ports tried from the preferred one on, that one included (8765..=8774 by default), before
/// any free port.
pub const FALLBACK_PORTS: u16 = 10;
/// Largest request body (a base64 `import_image` of 12 MiB fits).
pub const MAX_BODY: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Default)]
pub struct HttpOptions {
    /// The port to try first: `0` takes any free port, otherwise the ports after it (up to
    /// [`FALLBACK_PORTS`] in all, this one included) and then any free port are tried when it
    /// is busy.
    pub port: u16,
    /// The bearer token; `None` makes a random one. With an endpoint directory the endpoint
    /// file names another token, made for this start, which is accepted as well.
    pub token: Option<String>,
    /// Where to write the endpoint file (the config directory), if anywhere.
    pub endpoint_dir: Option<PathBuf>,
}

/// The bearer tokens the guard accepts: the configured one and, with an endpoint file, the
/// one that file names (made for this start). Replaced by [`HttpServer::set_token`].
#[derive(Clone)]
struct Tokens {
    configured: Arc<[u8]>,
    endpoint: Option<Arc<[u8]>>,
}

impl Tokens {
    /// `t` is one of them; both are compared in constant time over their whole length.
    fn accept(&self, t: &[u8]) -> bool {
        let a = same_secret(t, &self.configured);
        let b = self.endpoint.as_ref().is_some_and(|e| same_secret(t, e));
        a | b
    }
}

type SharedToken = Arc<RwLock<Tokens>>;

/// A running server; stopped by [`HttpServer::stop`] or when dropped.
pub struct HttpServer {
    port: u16,
    token: String,
    /// The token in the endpoint file (this start's), if one is written.
    endpoint_token: Option<String>,
    shared_token: SharedToken,
    cancel: CancellationToken,
    thread: Option<std::thread::JoinHandle<()>>,
    endpoint_dir: Option<PathBuf>,
    /// [`endpoint::ENDPOINT_LOCK`], held while the endpoint file names this server.
    claim: Option<endpoint::Claim>,
}

impl std::fmt::Debug for HttpServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpServer")
            .field("port", &self.port)
            .finish_non_exhaustive()
    }
}

fn bind_one(port: u16) -> std::io::Result<TcpListener> {
    TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))
}

/// Binds `127.0.0.1`: `preferred`, else the next ports, else any free port.
pub fn bind(preferred: u16) -> std::io::Result<TcpListener> {
    if preferred != 0 {
        for p in preferred..=preferred.saturating_add(FALLBACK_PORTS - 1) {
            if let Ok(l) = bind_one(p) {
                return Ok(l);
            }
        }
    }
    bind_one(0)
}

/// The `Host` values accepted on `port`.
pub fn allowed_hosts(port: u16) -> Vec<String> {
    vec![format!("127.0.0.1:{port}"), format!("localhost:{port}")]
}

/// Equal without an early exit on the first difference.
fn same_secret(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let diff = a
        .iter()
        .zip(b)
        .fold(0u8, |acc, (x, y)| std::hint::black_box(acc | (x ^ y)));
    diff == 0
}

#[derive(Clone)]
struct Gate {
    token: SharedToken,
    hosts: Arc<[String]>,
    seq: Arc<Sequencer>,
    state: Arc<ServerState>,
}

fn refuse(status: StatusCode, text: &'static str) -> Response {
    let mut r = (status, text).into_response();
    if status == StatusCode::UNAUTHORIZED {
        r.headers_mut()
            .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    }
    r
}

fn bearer(req: &Request<Body>) -> Option<&str> {
    let v = req.headers().get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = v.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then(|| token.trim())
}

/// The client's name in a JSON-RPC message, if it sends one.
fn client_name(msg: &serde_json::Value) -> Option<&str> {
    let params = msg.get("params")?;
    if msg.get("method")?.as_str()? == "initialize" {
        return params.get("clientInfo")?.get("name")?.as_str();
    }
    params
        .get("_meta")?
        .get("io.modelcontextprotocol/clientInfo")?
        .get("name")?
        .as_str()
}

async fn guard(State(g): State<Gate>, req: Request<Body>, next: Next) -> Response {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok());
    if !host.is_some_and(|h| g.hosts.iter().any(|a| a.eq_ignore_ascii_case(h))) {
        return refuse(StatusCode::FORBIDDEN, "host not allowed");
    }
    if req.headers().contains_key(header::ORIGIN) {
        return refuse(StatusCode::FORBIDDEN, "browser requests are not allowed");
    }
    let tokens = g.token.read().unwrap_or_else(|e| e.into_inner()).clone();
    if !bearer(&req).is_some_and(|t| tokens.accept(t.as_bytes())) {
        return refuse(StatusCode::UNAUTHORIZED, "missing or wrong bearer token");
    }
    g.state.activity().request();
    if req.method() != Method::POST {
        return next.run(req).await;
    }
    let announced = req
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    if announced.is_some_and(|n| n > MAX_BODY as u64) {
        return refuse(StatusCode::PAYLOAD_TOO_LARGE, "request body too large");
    }
    let (mut parts, body) = req.into_parts();
    let Ok(bytes) = axum::body::to_bytes(body, MAX_BODY).await else {
        return refuse(StatusCode::PAYLOAD_TOO_LARGE, "request body too large");
    };
    if let Ok(msg) = serde_json::from_slice::<serde_json::Value>(&bytes) {
        if let Some(name) = client_name(&msg) {
            g.state.activity().client(name);
        }
        let method = msg.get("method").and_then(|m| m.as_str());
        if matches!(method, Some("tools/call" | "resources/read")) && msg.get("id").is_some() {
            parts.extensions.insert(g.seq.ticket());
        }
    }
    next.run(Request::from_parts(parts, Body::from(bytes)))
        .await
}

fn router(service: McpService, gate: Gate, cancel: &CancellationToken, port: u16) -> Router {
    let config = StreamableHttpServerConfig::default()
        .with_allowed_hosts(allowed_hosts(port))
        .enforce_origin_validation()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_max_request_body_bytes(MAX_BODY)
        .with_cancellation_token(cancel.child_token());
    let mcp = StreamableHttpService::new(
        move || Ok(service.clone()),
        Arc::new(NeverSessionManager::default()),
        config,
    );
    Router::new()
        .route_service("/mcp", mcp)
        .layer(middleware::from_fn_with_state(gate, guard))
}

impl HttpServer {
    /// Binds and starts serving the tools of `exec`. Writes the endpoint file when
    /// `opts.endpoint_dir` is set (a failure to write it stops the server again); another
    /// server that serves that directory makes the start fail.
    pub fn start(
        exec: Executor,
        state: Arc<ServerState>,
        opts: HttpOptions,
    ) -> std::io::Result<HttpServer> {
        let token = match opts.token {
            Some(t) => t,
            None => endpoint::new_token()?,
        };
        let claim = match &opts.endpoint_dir {
            Some(dir) => Some(endpoint::claim_endpoint(dir)?.ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::AddrInUse,
                    "another DeluxePainter is already serving MCP clients",
                )
            })?),
            None => None,
        };
        let endpoint_token = match claim {
            Some(_) => Some(endpoint::new_token()?),
            None => None,
        };
        let listener = bind(opts.port)?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let seq = Sequencer::new();
        let service = McpService::new(exec, state.clone(), seq.clone(), "http");
        let shared_token: SharedToken = Arc::new(RwLock::new(Tokens {
            configured: Arc::from(token.as_bytes()),
            endpoint: endpoint_token.as_deref().map(|t| Arc::from(t.as_bytes())),
        }));
        let gate = Gate {
            token: shared_token.clone(),
            hosts: allowed_hosts(port).into(),
            seq,
            state,
        };
        let cancel = CancellationToken::new();
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("dpx-mcp-http")
            .enable_all()
            .build()?;
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let ct = cancel.clone();
        let thread = std::thread::Builder::new()
            .name("dpx-mcp-http".into())
            .spawn(move || {
                rt.block_on(async move {
                    let listener = match tokio::net::TcpListener::from_std(listener) {
                        Ok(l) => l,
                        Err(e) => {
                            let _ = ready_tx.send(Err(e));
                            return;
                        }
                    };
                    let app = router(service, gate, &ct, port);
                    let _ = ready_tx.send(Ok(()));
                    let _ = axum::serve(listener, app)
                        .with_graceful_shutdown(ct.cancelled_owned())
                        .await;
                });
                rt.shutdown_timeout(Duration::from_secs(1));
            })?;
        let ready = ready_rx
            .recv()
            .unwrap_or_else(|_| Err(std::io::Error::other("the server thread ended")));
        let mut server = HttpServer {
            port,
            token,
            endpoint_token,
            shared_token,
            cancel,
            thread: Some(thread),
            endpoint_dir: None,
            claim,
        };
        ready?;
        if let (Some(dir), Some(t)) = (opts.endpoint_dir, &server.endpoint_token) {
            endpoint::write_endpoint(&dir, &Endpoint::new(port, t.clone()))?;
            server.endpoint_dir = Some(dir);
        }
        Ok(server)
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn url(&self) -> String {
        endpoint::url_for(self.port)
    }

    /// The configured bearer token. Never show it in the UI or write it to project files.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// The token the endpoint file names (made for this start), if one is written.
    pub fn endpoint_token(&self) -> Option<&str> {
        self.endpoint_token.as_deref()
    }

    /// Replaces the bearer token while the server runs (revoking access): requests with the
    /// old one, and with the endpoint file's old token, get 401 from now on; the endpoint
    /// file names a new token for this start.
    pub fn set_token(&mut self, token: String) -> std::io::Result<()> {
        let endpoint_token = match self.endpoint_token {
            Some(_) => Some(endpoint::new_token()?),
            None => None,
        };
        *self.shared_token.write().unwrap_or_else(|e| e.into_inner()) = Tokens {
            configured: Arc::from(token.as_bytes()),
            endpoint: endpoint_token.as_deref().map(|t| Arc::from(t.as_bytes())),
        };
        self.token = token;
        self.endpoint_token = endpoint_token;
        if let (Some(dir), Some(t)) = (&self.endpoint_dir, &self.endpoint_token) {
            endpoint::write_endpoint(dir, &Endpoint::new(self.port, t.clone()))?;
        }
        Ok(())
    }

    pub fn endpoint_file(&self) -> Option<PathBuf> {
        self.endpoint_dir.as_deref().map(endpoint::endpoint_path)
    }

    /// Stops accepting requests, ends the open ones (waits at most about 3 s) and removes
    /// the endpoint file.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.cancel.cancel();
        if let Some(t) = self.thread.take() {
            let until = Instant::now() + Duration::from_secs(3);
            while !t.is_finished() && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(10));
            }
            if t.is_finished() {
                let _ = t.join();
            }
        }
        if let Some(dir) = self.endpoint_dir.take() {
            endpoint::remove_endpoint(&dir, self.port);
        }
        // Only now may another server take over the endpoint file.
        self.claim = None;
    }
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_compare_whole() {
        assert!(same_secret(b"abc", b"abc"));
        assert!(!same_secret(b"abc", b"abd"));
        assert!(!same_secret(b"abc", b"abcd"));
        assert!(!same_secret(b"", b"a"));
    }

    #[test]
    fn busy_ports_fall_back() {
        let a = bind(0).unwrap();
        let busy = a.local_addr().unwrap().port();
        let b = bind(busy).unwrap();
        let p = b.local_addr().unwrap().port();
        assert_ne!(p, busy);
        assert!(b.local_addr().unwrap().ip().is_loopback());
    }

    /// The status code of a `tools/list` POST with `token`.
    pub(crate) fn status_with(port: u16, token: &str) -> u16 {
        use std::io::{Read, Write};
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(
            s,
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\n\
             Content-Type: application/json\r\nAccept: application/json, text/event-stream\r\n\
             MCP-Protocol-Version: 2025-06-18\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        out.split(' ')
            .nth(1)
            .and_then(|c| c.parse().ok())
            .unwrap_or(0)
    }

    #[test]
    fn a_new_token_revokes_the_old_one_at_once() {
        let dir = crate::access::tests::scratch("http-token");
        let (exec, _queue) = crate::jobs::channel(|| {});
        let state = ServerState::new(crate::state::Settings::headless(Default::default()));
        let old = "o".repeat(32);
        let mut server = HttpServer::start(
            exec,
            state,
            HttpOptions {
                port: 0,
                token: Some(old.clone()),
                endpoint_dir: Some(dir.clone()),
            },
        )
        .unwrap();
        let port = server.port();
        assert_eq!(status_with(port, &old), 200);
        // The endpoint file names this start's own token, not the configured one.
        let first = endpoint::read_endpoint(&dir).unwrap().token;
        assert_ne!(first, old);
        assert_eq!(server.endpoint_token(), Some(first.as_str()));
        assert_eq!(status_with(port, &first), 200);
        let new = "n".repeat(32);
        server.set_token(new.clone()).unwrap();
        assert_eq!(status_with(port, &old), 401);
        assert_eq!(status_with(port, &first), 401, "the old endpoint token too");
        assert_eq!(status_with(port, &new), 200);
        let second = endpoint::read_endpoint(&dir).unwrap().token;
        assert!(second != first && second != new);
        assert_eq!(status_with(port, &second), 200);
        server.stop();
        assert!(endpoint::read_endpoint(&dir).is_none());
    }

    #[test]
    fn one_server_per_endpoint_directory() {
        let dir = crate::access::tests::scratch("http-one");
        let start = || {
            let (exec, _queue) = crate::jobs::channel(|| {});
            let state = ServerState::new(crate::state::Settings::headless(Default::default()));
            HttpServer::start(
                exec,
                state,
                HttpOptions {
                    port: 0,
                    token: None,
                    endpoint_dir: Some(dir.clone()),
                },
            )
        };
        let a = start().unwrap();
        assert!(endpoint::read_live_endpoint(&dir).is_some(), "served");
        let e = start().unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::AddrInUse, "{e}");
        assert_eq!(
            endpoint::read_endpoint(&dir).unwrap().port,
            a.port(),
            "the first server keeps its file"
        );
        a.stop();
        assert!(endpoint::read_live_endpoint(&dir).is_none());
        let b = start().unwrap();
        assert!(endpoint::read_live_endpoint(&dir).is_some());
        b.stop();
    }

    #[test]
    fn client_names_from_initialize_and_meta() {
        let init = serde_json::json!({"method": "initialize", "params": {"clientInfo": {"name": "claude-code"}}});
        assert_eq!(client_name(&init), Some("claude-code"));
        let call = serde_json::json!({"method": "tools/call", "params": {"_meta": {"io.modelcontextprotocol/clientInfo": {"name": "x"}}}});
        assert_eq!(client_name(&call), Some("x"));
        assert_eq!(
            client_name(&serde_json::json!({"method": "tools/list"})),
            None
        );
    }
}
