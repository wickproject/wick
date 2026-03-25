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

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DownloadInput {
    #[schemars(description = "The URL of the page or video to download media from")]
    pub url: String,
    #[schemars(description = "Output directory path (default: current directory)")]
    pub output_dir: Option<String>,
    #[schemars(description = "If true, just return info about the media without downloading")]
    pub info_only: Option<bool>,
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
    pub fn new(proxy: Option<&str>) -> Result<Self, anyhow::Error> {
        let client = Client::new(proxy)?;
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
        description = "Search the web via DuckDuckGo. Returns titles, URLs, and snippets. Use wick_fetch to read the full content of any result."
    )]
    async fn wick_search(
        &self,
        Parameters(input): Parameters<SearchInput>,
    ) -> Result<CallToolResult, McpError> {
        let num = input.num_results.unwrap_or(5).max(1).min(20) as usize;
        let results = crate::search::search(&self.client, &input.query, num)
            .await
            .map_err(|e| McpError::internal_error(format!("search failed: {}", e), None))?;

        let formatted = crate::search::format_results(&results);
        Ok(CallToolResult::success(vec![Content::text(formatted)]))
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

    #[tool(
        name = "wick_download",
        description = "Download video or audio from a URL (Reddit, YouTube, Twitter, and 1000+ sites). Returns the file path of the downloaded media. Requires yt-dlp installed."
    )]
    async fn wick_download(
        &self,
        Parameters(input): Parameters<DownloadInput>,
    ) -> Result<CallToolResult, McpError> {
        if input.info_only.unwrap_or(false) {
            let vi = crate::download::info(&input.url).await.map_err(|e| {
                McpError::internal_error(format!("info failed: {}", e), None)
            })?;
            let mut info = format!("Title: {}\nFormat: {}\nSize: {}", vi.title, vi.format, vi.size_approx);
            if let Some(dur) = vi.duration_secs {
                info.push_str(&format!("\nDuration: {}:{:02}", dur as u64 / 60, dur as u64 % 60));
            }
            Ok(CallToolResult::success(vec![Content::text(info)]))
        } else {
            let result = crate::download::download(&input.url, input.output_dir.as_deref())
                .await
                .map_err(|e| {
                    McpError::internal_error(format!("download failed: {}", e), None)
                })?;
            Ok(CallToolResult::success(vec![Content::text(format!(
                "Downloaded: {}\nSize: {:.1} MB",
                result.path, result.size_mb
            ))]))
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
