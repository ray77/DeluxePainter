//! The rmcp server handler shared by the stdio and the HTTP transport (ADR-43): tool and
//! resource lists from [`crate::tools`], every call in its arrival-order turn.

use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, ClientJsonRpcMessage, ClientRequest, GetExtensions,
    Implementation, InitializeRequestParams, InitializeResult, ListResourcesResult,
    ListToolsResult, PaginatedRequestParams, ReadResourceRequestParams, ReadResourceResponse,
    ServerCapabilities, ServerConfig,
};
use rmcp::service::{RequestContext, RxJsonRpcMessage, TxJsonRpcMessage};
use rmcp::transport::Transport;
use rmcp::{ErrorData, RoleServer, ServerHandler};

use crate::jobs::Executor;
use crate::seq::{Sequencer, Ticket};
use crate::state::ServerState;
use crate::tools::{self, ToolCtx};

/// The server's identity and capabilities (tools and resources), also used by the bridge.
pub fn server_config(instructions: &str) -> ServerConfig {
    ServerConfig::new(
        ServerCapabilities::builder()
            .enable_tools()
            .enable_resources()
            .build(),
    )
    .with_server_info(Implementation::new(
        tools::SERVER_NAME,
        env!("CARGO_PKG_VERSION"),
    ))
    .with_instructions(instructions)
}

/// One MCP server: the tools over an executor, with arrival-order turns.
#[derive(Clone, Debug)]
pub struct McpService {
    ctx: ToolCtx,
    seq: Arc<Sequencer>,
}

impl McpService {
    pub fn new(
        exec: Executor,
        state: Arc<ServerState>,
        seq: Arc<Sequencer>,
        transport: &'static str,
    ) -> Self {
        Self {
            ctx: ToolCtx {
                exec,
                state,
                transport,
            },
            seq,
        }
    }

    pub fn sequencer(&self) -> &Arc<Sequencer> {
        &self.seq
    }
}

/// The ticket a call arrived with: put into the request by [`Ticketed`] (stdio) or into the
/// HTTP request by the server's middleware.
pub fn ticket_of(context: &RequestContext<RoleServer>) -> Option<Ticket> {
    context.extensions.get::<Ticket>().cloned().or_else(|| {
        context
            .extensions
            .get::<axum::http::request::Parts>()
            .and_then(|p| p.extensions.get::<Ticket>().cloned())
    })
}

fn note_client(state: &ServerState, context: &RequestContext<RoleServer>) {
    if let Some(info) = context.meta.client_info() {
        state.activity().client(&info.name);
    }
}

impl ServerHandler for McpService {
    fn get_info(&self) -> ServerConfig {
        server_config(tools::INSTRUCTIONS)
    }

    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, ErrorData> {
        self.ctx.state.activity().client(&request.client_info.name);
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
        note_client(&self.ctx.state, &context);
        let turn = self.seq.turn(ticket_of(&context)).await;
        let result = tools::call(&self.ctx, &request.name, request.arguments).await;
        drop(turn);
        match result {
            Some(r) => Ok(r.into()),
            None => Err(ErrorData::invalid_params(
                format!("unknown tool {}", request.name),
                None,
            )),
        }
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
        note_client(&self.ctx.state, &context);
        let turn = self.seq.turn(ticket_of(&context)).await;
        let result = tools::read_resource(&self.ctx, &request.uri).await;
        drop(turn);
        match result {
            Ok(r) => Ok(r.into()),
            Err(e) if e.code == "RESOURCE_NOT_FOUND" => {
                Err(ErrorData::resource_not_found(e.message, None))
            }
            Err(e) => Err(ErrorData::invalid_request(e.to_string(), None)),
        }
    }
}

/// True for the requests that run on the workspace (and so take a ticket).
pub fn takes_ticket(request: &ClientRequest) -> bool {
    matches!(
        request,
        ClientRequest::CallToolRequest(_) | ClientRequest::ReadResourceRequest(_)
    )
}

/// A server transport that gives every tool call and resource read a ticket the moment it
/// is received, so calls keep their order on the line (stdio).
pub struct Ticketed<T> {
    inner: T,
    seq: Arc<Sequencer>,
}

impl<T> Ticketed<T> {
    pub fn new(inner: T, seq: Arc<Sequencer>) -> Self {
        Self { inner, seq }
    }
}

impl<T: Transport<RoleServer>> Transport<RoleServer> for Ticketed<T> {
    type Error = T::Error;

    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleServer>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        self.inner.send(item)
    }

    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleServer>> {
        let mut msg = self.inner.receive().await?;
        if let ClientJsonRpcMessage::Request(r) = &mut msg
            && takes_ticket(&r.request)
        {
            r.request.extensions_mut().insert(self.seq.ticket());
        }
        Some(msg)
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        self.inner.close().await
    }
}
