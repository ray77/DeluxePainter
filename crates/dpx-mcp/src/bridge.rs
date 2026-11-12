//! The stdio bridge (ADR-43) for clients that start a local stdio server (Claude Desktop's
//! `.mcpb` extension, `claude mcp add ... -- <bridge>`): `deluxepainter mcp-bridge` or the
//! small `dpx-mcp-bridge` binary.
//!
//! It answers `initialize`, `tools/list` and `resources/list` itself with the app's own
//! definitions, and forwards every tool call and resource read to the running app over
//! Streamable HTTP (`http://127.0.0.1:<port>/mcp` only, bearer token), reading the port and
//! token from the endpoint file on every (re)connect. When the app is not running and
//! `mcp-app.json` names it, the bridge starts it and waits up to [`LAUNCH_WAIT`]; otherwise
//! the call fails with `APP_NOT_RUNNING`. Calls are forwarded one at a time in arrival
//! order. stdout carries protocol messages only.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, ClientCapabilities, ClientConfig, ErrorCode,
    Implementation, InitializeRequestParams, InitializeResult, ListResourcesResult,
    ListToolsResult, PaginatedRequestParams, ReadResourceRequestParams, ReadResourceResponse,
    ServerConfig,
};
use rmcp::service::{RequestContext, RunningService, ServiceError};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::async_rw::AsyncRwTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::{ClientHandler, ErrorData, Peer, RoleClient, RoleServer, ServerHandler, ServiceExt};
use tokio::sync::Mutex;

use crate::endpoint::{self, AppLaunch, Endpoint};
use crate::error::{ToolError, error_result};
use crate::seq::Sequencer;
use crate::server::{Ticketed, server_config, ticket_of};
use crate::tools;

/// How long the bridge waits for an app it started.
pub const LAUNCH_WAIT: Duration = Duration::from_secs(20);
/// How long one connection attempt may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

pub const NOT_RUNNING: &str =
    "DeluxePainter is not running. Start it and turn on Prefs > AI & MCP.";

/// The client side towards the app: names the bridge and the client behind it.
#[derive(Clone, Debug)]
struct Upstream {
    name: String,
}

impl ClientHandler for Upstream {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::new(
            ClientCapabilities::default(),
            Implementation::new(self.name.clone(), env!("CARGO_PKG_VERSION")),
        )
    }
}

struct Conn {
    endpoint: Endpoint,
    client: RunningService<RoleClient, Upstream>,
}

struct Inner {
    dir: Option<PathBuf>,
    seq: Arc<Sequencer>,
    conn: Mutex<Option<Conn>>,
    /// The name of the client that started the bridge (from its `initialize`).
    client: std::sync::Mutex<Option<String>>,
    launch_wait: Duration,
}

/// The bridge's MCP server.
#[derive(Clone)]
pub struct Bridge {
    inner: Arc<Inner>,
}

impl Bridge {
    /// A bridge that looks for the endpoint and app files in `dir` (the config directory).
    pub fn new(dir: Option<PathBuf>) -> Self {
        Self::with_launch_wait(dir, LAUNCH_WAIT)
    }

    pub fn with_launch_wait(dir: Option<PathBuf>, launch_wait: Duration) -> Self {
        Self {
            inner: Arc::new(Inner {
                dir,
                seq: Sequencer::new(),
                conn: Mutex::new(None),
                client: std::sync::Mutex::new(None),
                launch_wait,
            }),
        }
    }

    pub fn sequencer(&self) -> &Arc<Sequencer> {
        &self.inner.seq
    }

    fn upstream(&self) -> Upstream {
        let client = self
            .inner
            .client
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        Upstream {
            name: match client {
                Some(c) => format!("{c} via deluxepainter-bridge"),
                None => "deluxepainter-bridge".into(),
            },
        }
    }

    async fn connect_to(
        &self,
        e: &Endpoint,
    ) -> Result<RunningService<RoleClient, Upstream>, String> {
        // Only ever 127.0.0.1, whatever the file says.
        let config = StreamableHttpClientTransportConfig::with_uri(endpoint::url_for(e.port))
            .auth_header(e.token.clone())
            .max_concurrent_requests(1);
        let transport = StreamableHttpClientTransport::from_config(config);
        match tokio::time::timeout(CONNECT_TIMEOUT, self.upstream().serve(transport)).await {
            Ok(Ok(c)) => Ok(c),
            Ok(Err(e)) => Err(e.to_string()),
            Err(_) => Err("timed out".into()),
        }
    }

    /// A connection to the running app: the cached one while the endpoint file is
    /// unchanged, else a new one; starts the app if it is not running and can be started.
    async fn peer(&self) -> Result<Peer<RoleClient>, ToolError> {
        let not_running = || ToolError::new("APP_NOT_RUNNING", NOT_RUNNING);
        let Some(dir) = self.inner.dir.clone() else {
            return Err(not_running());
        };
        let mut conn = self.inner.conn.lock().await;
        let current = endpoint::read_endpoint(&dir);
        if let (Some(c), Some(e)) = (conn.as_ref(), current.as_ref())
            && c.endpoint == *e
            && !c.client.is_closed()
        {
            return Ok(c.client.peer().clone());
        }
        *conn = None;
        if let Some(e) = &current
            && let Ok(client) = self.connect_to(e).await
        {
            let peer = client.peer().clone();
            *conn = Some(Conn {
                endpoint: e.clone(),
                client,
            });
            return Ok(peer);
        }
        let Some(app) = endpoint::read_app_file(&dir) else {
            return Err(not_running());
        };
        launch(&app)?;
        let until = Instant::now() + self.inner.launch_wait;
        while Instant::now() < until {
            tokio::time::sleep(Duration::from_millis(200)).await;
            if let Some(e) = endpoint::read_endpoint(&dir)
                && Some(&e) != current.as_ref()
                && let Ok(client) = self.connect_to(&e).await
            {
                let peer = client.peer().clone();
                *conn = Some(Conn {
                    endpoint: e,
                    client,
                });
                return Ok(peer);
            }
        }
        Err(ToolError::new(
            "APP_NOT_RUNNING",
            format!(
                "DeluxePainter was started but did not offer its MCP server within {} s. Turn on Prefs > AI & MCP.",
                self.inner.launch_wait.as_secs()
            ),
        ))
    }

    async fn forget(&self) {
        *self.inner.conn.lock().await = None;
    }

    async fn forward_call(
        &self,
        params: CallToolRequestParams,
    ) -> Result<CallToolResponse, ErrorData> {
        let read_only = tools::is_read_only(&params.name);
        for attempt in 0..2 {
            let peer = match self.peer().await {
                Ok(p) => p,
                Err(e) => return Ok(error_result(&e).into()),
            };
            match peer.call_tool_once(params.clone()).await {
                Ok(r) => return Ok(r),
                Err(ServiceError::McpError(e)) => return Err(e),
                Err(e) => {
                    self.forget().await;
                    // Only a call that changes nothing is sent again.
                    if read_only && attempt == 0 {
                        continue;
                    }
                    return Ok(error_result(&ToolError::new(
                        "APP_CONNECTION_LOST",
                        format!(
                            "the connection to DeluxePainter broke during the call ({e}); it may or may not have been carried out: check with get_document_info"
                        ),
                    ))
                    .into());
                }
            }
        }
        unreachable!("the loop returns")
    }

    async fn forward_read(
        &self,
        params: ReadResourceRequestParams,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let failed = |e: ToolError| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None);
        for attempt in 0..2 {
            let peer = self.peer().await.map_err(failed)?;
            match peer.read_resource_once(params.clone()).await {
                Ok(r) => return Ok(r),
                Err(ServiceError::McpError(e)) => return Err(e),
                Err(e) => {
                    self.forget().await;
                    if attempt == 0 {
                        continue;
                    }
                    return Err(failed(ToolError::new(
                        "APP_CONNECTION_LOST",
                        format!("the connection to DeluxePainter broke ({e})"),
                    )));
                }
            }
        }
        unreachable!("the loop returns")
    }
}

/// Starts the app detached (no stdin/stdout/stderr, so nothing reaches the protocol).
fn launch(app: &AppLaunch) -> Result<(), ToolError> {
    std::process::Command::new(&app.app_path)
        .args(&app.args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(drop)
        .map_err(|e| {
            ToolError::new(
                "APP_NOT_RUNNING",
                format!("{NOT_RUNNING} (starting {} failed: {e})", app.app_path),
            )
        })
}

const BRIDGE_INSTRUCTIONS: &str = tools::INSTRUCTIONS;

impl ServerHandler for Bridge {
    fn get_info(&self) -> ServerConfig {
        server_config(BRIDGE_INSTRUCTIONS)
    }

    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, ErrorData> {
        *self.inner.client.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(request.client_info.name.clone());
        context.peer.set_peer_info(request.clone());
        self.negotiate_initialize(&request)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(tools::definitions()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        if !tools::names().contains(&request.name.as_ref()) {
            return Err(ErrorData::invalid_params(
                format!("unknown tool {}", request.name),
                None,
            ));
        }
        let turn = self.inner.seq.turn(ticket_of(&context)).await;
        let r = self.forward_call(request).await;
        drop(turn);
        r
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        Ok(ListResourcesResult::with_all_items(tools::resources()))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let turn = self.inner.seq.turn(ticket_of(&context)).await;
        let r = self.forward_read(request).await;
        drop(turn);
        r
    }
}

const USAGE: &str = "usage: deluxepainter mcp-bridge (or dpx-mcp-bridge): a stdio MCP server that forwards to the running DeluxePainter";

/// Parses the bridge's arguments (none) and runs it on stdio until the client disconnects.
pub fn run_cli(args: &[String]) -> Result<(), String> {
    if let Some(a) = args.first() {
        return Err(format!("unknown argument {a}\n{USAGE}"));
    }
    run_blocking(crate::paths::config_dir()).map_err(|e| e.to_string())
}

/// Runs the bridge on stdin/stdout with the endpoint files in `dir`.
pub fn run_blocking(dir: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    rt.block_on(async {
        match &dir {
            Some(d) => eprintln!(
                "deluxepainter mcp-bridge: forwarding to the app found in {}",
                d.display()
            ),
            None => eprintln!("deluxepainter mcp-bridge: no config directory (set DPX_CONFIG_DIR)"),
        }
        let bridge = Bridge::new(dir);
        let seq = bridge.sequencer().clone();
        let (stdin, stdout) = rmcp::transport::stdio();
        let transport = Ticketed::new(AsyncRwTransport::new_server(stdin, stdout), seq);
        let service = bridge.serve(transport).await?;
        service.waiting().await?;
        Ok(())
    })
}
