//! The Streamable HTTP server (ADR-43): rmcp's `StreamableHttpService` hosted with axum on
//! `127.0.0.1` only, path `/mcp`, on a tokio runtime thread of its own.
//!
//! Every request passes [`guard`] first: a `Host` other than `127.0.0.1:<port>` or
//! `localhost:<port>` gets 403 (DNS rebinding), any `Origin` header gets 403 (no browser may
//! call it), a missing or wrong `Authorization: Bearer <token>` gets 401 (constant-time
//! compare), a body over [`MAX_BODY`] gets 413. Tool calls and resource reads get their
//! arrival-order ticket there. The server is stateless (no MCP sessions) and answers with
//! plain JSON, so every POST is one request and its answer.

use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::path::PathBuf;
use std::sync::Arc;
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
/// Ports tried after a busy one (then any free port).
pub const FALLBACK_PORTS: u16 = 10;
/// Largest request body (a base64 `import_image` of 12 MiB fits).
pub const MAX_BODY: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Default)]
pub struct HttpOptions {
    /// The port to try first: `0` takes any free port, otherwise the next
    /// [`FALLBACK_PORTS`] ports and then any free port are tried when it is busy.
    pub port: u16,
    /// The bearer token; `None` makes a random one.
    pub token: Option<String>,
    /// Where to write the endpoint file (the config directory), if anywhere.
    pub endpoint_dir: Option<PathBuf>,
}

/// A running server; stopped by [`HttpServer::stop`] or when dropped.
pub struct HttpServer {
    port: u16,
    token: String,
    cancel: CancellationToken,
    thread: Option<std::thread::JoinHandle<()>>,
    endpoint_dir: Option<PathBuf>,
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
    token: Arc<[u8]>,
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
    if !bearer(&req).is_some_and(|t| same_secret(t.as_bytes(), &g.token)) {
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
    /// `opts.endpoint_dir` is set (a failure to write it stops the server again).
    pub fn start(
        exec: Executor,
        state: Arc<ServerState>,
        opts: HttpOptions,
    ) -> std::io::Result<HttpServer> {
        let token = match opts.token {
            Some(t) => t,
            None => endpoint::new_token()?,
        };
        let listener = bind(opts.port)?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let seq = Sequencer::new();
        let service = McpService::new(exec, state.clone(), seq.clone(), "http");
        let gate = Gate {
            token: Arc::from(token.as_bytes()),
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
            cancel,
            thread: Some(thread),
            endpoint_dir: None,
        };
        ready?;
        if let Some(dir) = opts.endpoint_dir {
            endpoint::write_endpoint(&dir, &Endpoint::new(port, server.token.clone()))?;
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

    /// The bearer token. Never show it in the UI or write it to project files.
    pub fn token(&self) -> &str {
        &self.token
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
