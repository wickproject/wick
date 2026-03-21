use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    schemars, tool, tool_handler, tool_router,
};

use crate::engine::Client;
use crate::extract::Format;
use crate::fetch;
use crate::session;

// ── Tool input types ──────────────────────────────────────────

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct FetchInput {
    #[schemars(description = "The URL to fetch")]
    pub url: String,
    #[schemars(description = "Output format: markdown (default), html, or text")]
    pub format: Option<String>,
    #[schemars(description = "Whether to respect robots.txt (default true)")]
    pub respect_robots: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SearchInput {
    #[schemars(description = "Search query")]
    pub query: String,
    #[schemars(description = "Number of search results to return (default 5)")]
    pub num_results: Option<i32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SessionInput {
    #[schemars(description = "Session action: 'clear' removes all cookies and cache")]
    pub action: String,
}

// ── Server ────────────────────────────────────────────────────

#[derive(Clone)]
pub struct WickServer {
    tool_router: ToolRouter<Self>,
    client: std::sync::Arc<Client>,
}

impl std::fmt::Debug for WickServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WickServer").finish()
    }
}

#[tool_router]
impl WickServer {
    pub fn new() -> Result<Self, anyhow::Error> {
        let client = Client::new()?;
        Ok(Self {
            tool_router: Self::tool_router(),
            client: std::sync::Arc::new(client),
        })
    }

    #[tool(
        name = "wick_fetch",
        description = "Fetch a web page using Chrome's network stack with browser-grade TLS fingerprinting. Returns clean, LLM-friendly content. Succeeds on sites that block standard HTTP clients."
    )]
    async fn wick_fetch(
        &self,
        Parameters(input): Parameters<FetchInput>,
    ) -> Result<CallToolResult, McpError> {
        let client = &self.client;

        let format = input
            .format
            .as_deref()
            .map(Format::from_str)
            .unwrap_or(Format::Markdown);
        let respect_robots = input.respect_robots.unwrap_or(true);

        let result = fetch::fetch(client, &input.url, format, respect_robots)
            .await
            .map_err(|e| {
                McpError::internal_error(format!("fetch failed: {}", e), None)
            })?;

        Ok(CallToolResult::success(vec![Content::text(result.content)]))
    }

    #[tool(
        name = "wick_search",
        description = "Search the web and optionally fetch top results. Note: basic implementation in v0.1."
    )]
    async fn wick_search(
        &self,
        Parameters(_input): Parameters<SearchInput>,
    ) -> String {
        "Web search is not yet implemented in v0.1. Use wick_fetch with a specific URL instead."
            .to_string()
    }

    #[tool(
        name = "wick_session",
        description = "Manage persistent browser sessions. Clear cookies and session data to start fresh."
    )]
    async fn wick_session(
        &self,
        Parameters(input): Parameters<SessionInput>,
    ) -> Result<CallToolResult, McpError> {
        match input.action.as_str() {
            "clear" => {
                session::clear().map_err(|e| {
                    McpError::internal_error(format!("clear session: {}", e), None)
                })?;
                Ok(CallToolResult::success(vec![Content::text(
                    "Session cleared. Cookies and cache data have been removed.",
                )]))
            }
            other => Err(McpError::invalid_params(
                format!("Unknown action: {:?}. Supported: clear", other),
                None,
            )),
        }
    }
}

#[tool_handler]
impl ServerHandler for WickServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(
                "Wick provides browser-grade web access for AI agents. \
                 Use wick_fetch to read any webpage with clean markdown output."
                    .to_string(),
            )
    }
}
