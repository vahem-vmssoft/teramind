//! `TeramindMcpServer`: rmcp `ServerHandler` implementation that translates
//! MCP tool calls into Teramind IPC requests.

use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Content},
    schemars,
    tool, tool_handler, tool_router,
};
use serde::Deserialize;
use teramind_ipc::{
    client::{IpcClient, StreamClient},
    proto::{Request, Response},
    transport::{connect, default_socket_path},
};

/// MCP server that proxies tool calls into a running Teramind daemon.
#[derive(Clone)]
pub struct TeramindMcpServer {
    // The `tool_router` field is consumed by the rmcp `#[tool_handler]` macro
    // via `Self::tool_router()`, which the dead-code lint can't see through.
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

impl Default for TeramindMcpServer {
    fn default() -> Self {
        Self::new()
    }
}

impl TeramindMcpServer {
    /// Construct a new server with the auto-generated tool router.
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    /// Open a fresh IPC connection to the daemon, send `req`, and return the
    /// daemon's response.  Connections are not pooled: each MCP tool call
    /// performs one connect/request/close cycle, mirroring how the CLI works.
    async fn ipc_request(&self, req: Request) -> Result<Response, McpError> {
        let path = default_socket_path();
        let stream = connect(&path).await.map_err(|e| {
            McpError::internal_error(format!("connect teramind daemon: {e}"), None)
        })?;
        let mut client = StreamClient::new(stream);
        client
            .request(req)
            .await
            .map_err(|e| McpError::internal_error(format!("teramind ipc: {e}"), None))
    }
}

fn default_limit() -> u32 {
    10
}

/// Arguments to the `search` MCP tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchArgs {
    /// Free-text query searched against prior Claude session traces and skills.
    pub query: String,
    /// Maximum number of hits to return (default 10).
    #[serde(default = "default_limit")]
    pub limit: u32,
}

/// Arguments to the `recall` MCP tool.
#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
pub struct RecallArgs {
    /// Optional working-directory filter.
    #[serde(default)]
    pub cwd: Option<String>,
    /// File paths mentioned in the current context to bias recall toward.
    #[serde(default)]
    pub file_paths: Vec<String>,
    /// Symbol names (function/type/etc.) mentioned in the current context.
    #[serde(default)]
    pub symbols: Vec<String>,
    /// Stack-trace frames extracted from the current context.
    #[serde(default)]
    pub stack_traces: Vec<String>,
    /// Maximum number of hits to return (default 10).
    #[serde(default = "default_limit")]
    pub limit: u32,
}

/// Arguments to the `wiki` MCP tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct WikiArgs {
    /// Optional session id (UUID). If omitted, returns the most recent
    /// wiki page for `cwd`'s project.
    #[serde(default)]
    pub session_id: Option<String>,
    /// Optional cwd. Defaults to the daemon's notion of current project.
    #[serde(default)]
    pub cwd: Option<String>,
}

/// Arguments to the `save_skill` MCP tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SaveSkillArgs {
    /// Short kebab-case name for the skill.
    pub name: String,
    /// One-line description of what the skill does.
    pub description: String,
    /// Markdown body of the skill (the actual recipe to recall later).
    pub body: String,
}

#[tool_router]
impl TeramindMcpServer {
    /// Search prior Claude sessions and skills by free text.
    #[tool(description = "Search prior Claude sessions and skills by free text. \
        Returns ranked hits with source, score, and snippet.")]
    async fn search(
        &self,
        Parameters(args): Parameters<SearchArgs>,
    ) -> Result<CallToolResult, McpError> {
        let req = Request::Search(teramind_core::types::SearchRequest {
            query: args.query,
            limit: args.limit,
        });
        let resp = self.ipc_request(req).await?;
        let body = match resp {
            Response::SearchResults(s) => serde_json::to_string_pretty(&s).map_err(|e| {
                McpError::internal_error(format!("serialize search results: {e}"), None)
            })?,
            Response::Error(e) => return Err(McpError::internal_error(e, None)),
            other => {
                return Err(McpError::internal_error(
                    format!("unexpected daemon response: {other:?}"),
                    None,
                ));
            }
        };
        Ok(CallToolResult::success(vec![Content::text(body)]))
    }

    /// Structured recall, filtering prior context by cwd, files, symbols, traces.
    #[tool(description = "Structured recall: filter prior Teramind context by cwd, \
        file paths, symbols, or stack-trace frames. Returns ranked hits.")]
    async fn recall(
        &self,
        Parameters(args): Parameters<RecallArgs>,
    ) -> Result<CallToolResult, McpError> {
        let req = Request::Recall(teramind_core::types::RecallRequest {
            cwd: args.cwd,
            file_paths: args.file_paths,
            symbols: args.symbols,
            stack_traces: args.stack_traces,
            limit: args.limit,
        });
        let resp = self.ipc_request(req).await?;
        let body = match resp {
            Response::SearchResults(s) => serde_json::to_string_pretty(&s).map_err(|e| {
                McpError::internal_error(format!("serialize recall results: {e}"), None)
            })?,
            Response::Error(e) => return Err(McpError::internal_error(e, None)),
            other => {
                return Err(McpError::internal_error(
                    format!("unexpected daemon response: {other:?}"),
                    None,
                ));
            }
        };
        Ok(CallToolResult::success(vec![Content::text(body)]))
    }

    /// Read a session's wiki page (session summary).
    #[tool(description = "Read a session's wiki page. Without session_id, returns \
        the most recent summary for the cwd's project.")]
    async fn wiki(
        &self,
        Parameters(args): Parameters<WikiArgs>,
    ) -> Result<CallToolResult, McpError> {
        let req = Request::WikiLookup {
            session_id: args.session_id,
            cwd: args.cwd,
        };
        let resp = self.ipc_request(req).await?;
        match resp {
            Response::WikiPage { session_id, cwd, model, content, generated_at } => {
                let body = serde_json::json!({
                    "session_id": session_id,
                    "cwd": cwd,
                    "model": model,
                    "content": content,
                    "generated_at": generated_at.to_string(),
                });
                let text = serde_json::to_string_pretty(&body).map_err(|e| {
                    McpError::internal_error(format!("serialize wiki page: {e}"), None)
                })?;
                Ok(CallToolResult::success(vec![Content::text(text)]))
            }
            Response::WikiNotFound => {
                Ok(CallToolResult::success(vec![
                    Content::text("{\"status\":\"not_found\"}".to_string()),
                ]))
            }
            Response::Error(e) => Err(McpError::internal_error(e, None)),
            other => Err(McpError::internal_error(
                format!("unexpected daemon response: {other:?}"),
                None,
            )),
        }
    }

    /// Persist a user-authored skill into Teramind for future recall.
    #[tool(description = "Save a user-authored skill into Teramind so future \
        sessions can recall it. Returns the skill id.")]
    async fn save_skill(
        &self,
        Parameters(args): Parameters<SaveSkillArgs>,
    ) -> Result<CallToolResult, McpError> {
        let req = Request::SaveSkill(teramind_core::types::SaveSkillRequest {
            name: args.name,
            description: args.description,
            body: args.body,
            source_session_ids: vec![],
        });
        let resp = self.ipc_request(req).await?;
        let body = match resp {
            Response::SkillRef(s) => serde_json::to_string_pretty(&s).map_err(|e| {
                McpError::internal_error(format!("serialize skill ref: {e}"), None)
            })?,
            Response::Error(e) => return Err(McpError::internal_error(e, None)),
            other => {
                return Err(McpError::internal_error(
                    format!("unexpected daemon response: {other:?}"),
                    None,
                ));
            }
        };
        Ok(CallToolResult::success(vec![Content::text(body)]))
    }
}

#[tool_handler(
    name = "teramind-mcp",
    instructions = "Teramind: search and recall prior Claude session traces and skills."
)]
impl ServerHandler for TeramindMcpServer {}
