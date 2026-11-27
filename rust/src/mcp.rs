use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    schemars, tool, tool_handler, tool_router,
};

use crate::analytics;
use crate::engine::Client;
use crate::extract::Format;
use crate::fetch::{self, RenderMode};
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
    #[schemars(
        description = "Render mode: 'auto' (default — Cronet network stack with automatic CEF escalation on 403/503 blocks and JavaScript-required interstitials like X/Twitter, CRA shells, etc), 'cef' (force the full Chromium renderer with JS execution — slower but reads any SPA), or 'cronet' (force network-only, never spawn CEF). Use 'cef' explicitly when you know the target is a JS-heavy SPA and want to skip the Cronet probe."
    )]
    pub render: Option<String>,
    #[schemars(
        description = "CSS selector to wait for before capturing the DOM. Only applies when the CEF renderer runs (force render='cef' or it auto-escalates). Use this for SPAs that lazy-load content after initial hydration — e.g. '[data-testid=\"primaryColumn\"]' for X/Twitter timelines, 'article' for blog feeds. The renderer waits up to 20s for the selector to appear. Use simple selectors (data-testid, class, id) — single quotes and backslashes are stripped server-side for safety."
    )]
    pub wait_for_selector: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RequestCapabilityInput {
    #[schemars(description = "The user's goal you could not accomplish, in one or two sentences. Describe the task, not the user: leave out names, accounts and other personal details.")]
    pub task: String,
    #[schemars(description = "What you tried, e.g. which Wick tools and options")]
    pub tried: Option<String>,
    #[schemars(description = "What went wrong: blocked, wrong content, missing feature, too slow, etc.")]
    pub failure: Option<String>,
    #[schemars(description = "The URL involved, if any. Only its hostname is sent.")]
    pub url: Option<String>,
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
pub struct CrawlInput {
    #[schemars(description = "Starting URL to crawl from")]
    pub url: String,
    #[schemars(description = "Maximum link depth to follow (default 2, max 5)")]
    pub max_depth: Option<u32>,
    #[schemars(description = "Maximum number of pages to fetch (default 10, max 50)")]
    pub max_pages: Option<u32>,
    #[schemars(description = "Output format: markdown (default), html, or text")]
    pub format: Option<String>,
    #[schemars(description = "Whether to respect robots.txt (default true)")]
    pub respect_robots: Option<bool>,
    #[schemars(description = "Only crawl pages whose path starts with this prefix (e.g., /docs/)")]
    pub path_filter: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct MapInput {
    #[schemars(description = "Starting URL or domain to map")]
    pub url: String,
    #[schemars(description = "Maximum number of URLs to discover (default 100, max 5000)")]
    pub limit: Option<u32>,
    #[schemars(description = "Whether to check sitemap.xml first (default true)")]
    pub use_sitemap: Option<bool>,
    #[schemars(description = "Whether to respect robots.txt (default true)")]
    pub respect_robots: Option<bool>,
    #[schemars(description = "Only include URLs whose path starts with this prefix")]
    pub path_filter: Option<String>,
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

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct FetchToFileInput {
    #[schemars(description = "The URL to fetch (any content type — PDFs, images, archives, etc.)")]
    pub url: String,
    #[schemars(description = "Local filesystem path to write the response body to. Will be created (and parent directories created) if missing. An existing file at this path will be overwritten.")]
    pub path: String,
    #[schemars(description = "Whether to respect robots.txt (default true)")]
    pub respect_robots: Option<bool>,
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
        description = "Fetch a web page using Chrome's network stack with browser-grade TLS fingerprinting. Returns clean, LLM-friendly content. Succeeds on sites that block standard HTTP clients. Automatically falls back to a full Chromium (CEF) renderer when the page is JavaScript-only or blocked — pass render='cef' to force it, render='cronet' to skip it. If Wick can't get you what the user needs, call wick_request_capability."
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
        let render = input
            .render
            .as_deref()
            .map(RenderMode::from_str)
            .unwrap_or_default();
        let wait_for_selector = input.wait_for_selector.as_deref();

        let result = analytics::with_tool(
            "wick_fetch",
            fetch::fetch(
                client,
                &input.url,
                format,
                respect_robots,
                render,
                wait_for_selector,
            ),
        )
        .await
        .map_err(|e| McpError::internal_error(format!("fetch failed: {}", e), None))?;

        let mut content = result.content;
        if result.status_code == 404 {
            if let Some(hint) = suggest_urls(client, &input.url).await {
                content.push_str(&hint);
            }
        }
        Ok(CallToolResult::success(vec![Content::text(content)]))
    }

    #[tool(
        name = "wick_search",
        description = "Search the web via DuckDuckGo. Returns titles, URLs, and snippets. Use wick_fetch to read the full content of any result. If Wick can't get you what the user needs, call wick_request_capability."
    )]
    async fn wick_search(
        &self,
        Parameters(input): Parameters<SearchInput>,
    ) -> Result<CallToolResult, McpError> {
        let num = input.num_results.unwrap_or(5).max(1).min(20) as usize;
        let results = analytics::with_tool("wick_search", crate::search::search(&self.client, &input.query, num))
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

    #[tool(
        name = "wick_crawl",
        description = "Crawl a website starting from a URL. Follows same-domain links, fetches multiple pages, and returns their content as clean markdown. Useful for reading documentation sites, blogs, or multi-page content. If Wick can't get you what the user needs, call wick_request_capability."
    )]
    async fn wick_crawl(
        &self,
        Parameters(input): Parameters<CrawlInput>,
    ) -> Result<CallToolResult, McpError> {
        let format = input
            .format
            .as_deref()
            .map(Format::from_str)
            .unwrap_or(Format::Markdown);
        let max_depth = input.max_depth.unwrap_or(2).min(5);
        let max_pages = input.max_pages.unwrap_or(10).min(50);
        let respect_robots = input.respect_robots.unwrap_or(true);

        let options = crate::crawl::CrawlOptions {
            max_depth,
            max_pages,
            format,
            respect_robots,
            path_filter: input.path_filter,
        };

        let result = analytics::with_tool("wick_crawl", crate::crawl::crawl(&self.client, &input.url, options))
            .await
            .map_err(|e| McpError::internal_error(format!("crawl failed: {}", e), None))?;

        let host = url::Url::parse(&input.url)
            .ok()
            .and_then(|u| u.host_str().map(|s| s.to_string()))
            .unwrap_or_else(|| input.url.clone());

        let output = crate::crawl::format_crawl_output(&result, &host);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    #[tool(
        name = "wick_map",
        description = "Discover all URLs on a website. Checks sitemap.xml first, then follows links to build a URL map. Returns a sorted list of discovered URLs. Useful for understanding site structure before targeted fetching. If Wick can't get you what the user needs, call wick_request_capability."
    )]
    async fn wick_map(
        &self,
        Parameters(input): Parameters<MapInput>,
    ) -> Result<CallToolResult, McpError> {
        let limit = input.limit.unwrap_or(100).min(5000);
        let use_sitemap = input.use_sitemap.unwrap_or(true);
        let respect_robots = input.respect_robots.unwrap_or(true);

        let options = crate::crawl::MapOptions {
            limit,
            use_sitemap,
            respect_robots,
            path_filter: input.path_filter,
        };

        let result = analytics::with_tool("wick_map", crate::crawl::map(&self.client, &input.url, options))
            .await
            .map_err(|e| McpError::internal_error(format!("map failed: {}", e), None))?;

        let host = url::Url::parse(&input.url)
            .ok()
            .and_then(|u| u.host_str().map(|s| s.to_string()))
            .unwrap_or_else(|| input.url.clone());

        let output = crate::crawl::format_map_output(&result, &host);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    #[tool(
        name = "wick_fetch_to_file",
        description = "Fetch a URL and write the raw response body to a local file. Use this for binary content that wick_fetch can't usefully return as text — PDFs, images, archives, audio, etc. Returns the saved path, byte count, status, and timing. The agent can then Read/process the file via the host filesystem (Bash, Read tool, etc.)."
    )]
    async fn wick_fetch_to_file(
        &self,
        Parameters(input): Parameters<FetchToFileInput>,
    ) -> Result<CallToolResult, McpError> {
        use std::path::Path;
        let respect_robots = input.respect_robots.unwrap_or(true);

        let result = analytics::with_tool("wick_fetch_to_file", fetch::fetch_raw(&self.client, &input.url, respect_robots))
            .await
            .map_err(|e| McpError::internal_error(format!("fetch_raw failed: {}", e), None))?;

        if result.status_code == 0 {
            return Ok(CallToolResult::success(vec![Content::text(format!(
                "Blocked by robots.txt at {}. Pass respect_robots=false to override.",
                input.url
            ))]));
        }

        // Create parent directories if missing — matches the user's mental
        // model when they specify a path like /tmp/wick-out/foo.pdf.
        let path = Path::new(&input.path);
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    McpError::internal_error(
                        format!("create_dir_all({}): {}", parent.display(), e),
                        None,
                    )
                })?;
            }
        }
        std::fs::write(path, &result.bytes).map_err(|e| {
            McpError::internal_error(format!("write {}: {}", input.path, e), None)
        })?;

        let summary = format!(
            "Saved {} bytes to {}\nStatus: {} | Time: {}ms",
            result.bytes.len(),
            input.path,
            result.status_code,
            result.timing_ms,
        );
        Ok(CallToolResult::success(vec![Content::text(summary)]))
    }

    #[tool(
        name = "wick_request_capability",
        description = "Tell the Wick team about a task Wick couldn't help you finish: a site that stayed blocked, content you needed but couldn't get, or a capability Wick lacks (a different country's view of a page, logged-in access, structured data, etc). Call it whenever Wick falls short of the user's goal, then carry on with the task. Text is scrubbed of URL paths, emails and long numbers before sending, and nothing is sent if the user has opted out of Wick telemetry."
    )]
    async fn wick_request_capability(
        &self,
        Parameters(input): Parameters<RequestCapabilityInput>,
    ) -> Result<CallToolResult, McpError> {
        if input.task.trim().is_empty() {
            return Err(McpError::invalid_params("task is required", None));
        }
        let sent = analytics::with_tool("wick_request_capability", async {
            analytics::report_feedback(analytics::Feedback {
                task: &input.task,
                tried: input.tried.as_deref(),
                failure: input.failure.as_deref(),
                url: input.url.as_deref(),
            })
        })
        .await;
        let msg = if sent {
            "Thanks, this is queued for the Wick team (best effort). Requests like this decide what Wick builds next. Carry on with the user's task."
        } else if analytics::is_opted_out() {
            "Not sent: the user has turned off Wick telemetry. Carry on with the user's task."
        } else {
            "Not sent: Wick's telemetry queue is unavailable right now. Carry on with the user's task."
        };
        Ok(CallToolResult::success(vec![Content::text(msg)]))
    }
}

/// How long a 404 may spend looking up alternative URLs.
const SUGGEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);
const SUGGEST_COUNT: usize = 5;

/// After a 404, look for real pages on the same site whose paths resemble
/// the one requested. Agents often guess documentation URLs, and the
/// directory above a guessed page usually links to the real ones, so this
/// reads the parent directory's page, then the grandparent's, and ranks
/// their same-site links. Bounded by `SUGGEST_TIMEOUT`. These lookups run
/// under `SUGGEST_TOOL`, which keeps them out of telemetry.
async fn suggest_urls(client: &Client, requested: &str) -> Option<String> {
    let parsed = url::Url::parse(requested).ok()?;
    let best = tokio::time::timeout(
        SUGGEST_TIMEOUT,
        analytics::with_tool(analytics::SUGGEST_TOOL, async {
            let mut dir = parsed.clone();
            for _ in 0..2 {
                dir = parent_dir(&dir)?;
                // A plain network GET: it never launches the CEF renderer
                // (so render='cronet' holds and the timeout can't strand a
                // render) and, unlike fetch_raw, never writes the strategy
                // cache, so a probe can't overwrite a host's learned CEF-first
                // routing.
                if !crate::robots::check(client, dir.as_str()).await {
                    continue;
                }
                let Ok(page) = client.get_bytes(dir.as_str()).await else {
                    continue;
                };
                if page.status >= 400 {
                    continue;
                }
                let html = String::from_utf8_lossy(&page.body);
                let links: Vec<String> = crate::crawl::extract_links(&html, &dir)
                    .into_iter()
                    .filter(|u| u.host_str() == parsed.host_str())
                    .map(|u| u.to_string())
                    .collect();
                let best = rank_similar(parsed.path(), &links, SUGGEST_COUNT);
                if !best.is_empty() {
                    return Some(best);
                }
            }
            None
        }),
    )
    .await
    .ok()??;
    let mut out = String::from("\n\n---\nThis page doesn't exist. Similar pages on this site:\n");
    for u in best {
        out.push_str(&format!("- {}\n", u));
    }
    Some(out)
}

/// The directory containing `u`'s path: `/a/b/c.html` → `/a/b/`,
/// `/a/b/` → `/a/`. None at the site root.
fn parent_dir(u: &url::Url) -> Option<url::Url> {
    let path = u.path().trim_end_matches('/');
    if path.is_empty() {
        return None;
    }
    let cut = path.rfind('/')?;
    let mut d = u.clone();
    d.set_path(&path[..=cut]);
    d.set_query(None);
    d.set_fragment(None);
    Some(d)
}

/// Tokens of a path's last segment (usually the page name), lowercased,
/// without file extensions or filler like "index".
fn page_tokens(path: &str) -> Vec<String> {
    const SKIP: &[&str] = &["html", "htm", "php", "aspx", "md", "index", "www"];
    let last = path.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    let decoded = urlencoding::decode(last).map(|d| d.into_owned()).unwrap_or_else(|_| last.to_string());
    decoded
        .split(|c: char| !c.is_alphanumeric())
        .map(|t| t.to_lowercase())
        .filter(|t| t.len() > 1 && !SKIP.contains(&t.as_str()))
        .collect()
}

/// Rank `candidates` by how well their page name matches the requested
/// one: 2 per identical token, 1 when one token starts with the other
/// (`task` and `taskgroups`). Candidates with no match are dropped, so an
/// unrelated site gets no suggestions rather than noise.
fn rank_similar(path: &str, candidates: &[String], n: usize) -> Vec<String> {
    let want = page_tokens(path);
    if want.is_empty() {
        return Vec::new();
    }
    let mut seen = std::collections::HashSet::new();
    let mut scored: Vec<(usize, &String)> = candidates
        .iter()
        .filter_map(|c| {
            let cp = url::Url::parse(c).ok()?.path().to_string();
            if cp == path || !seen.insert(cp.clone()) {
                return None;
            }
            let have = page_tokens(&cp);
            let score: usize = want
                .iter()
                .map(|w| {
                    if have.contains(w) {
                        2
                    } else if have.iter().any(|h| {
                        h.len() >= 3 && w.len() >= 3 && (h.starts_with(w.as_str()) || w.starts_with(h.as_str()))
                    }) {
                        1
                    } else {
                        0
                    }
                })
                .sum();
            (score > 0).then_some((score, c))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.len().cmp(&b.1.len())));
    scored.into_iter().take(n).map(|(_, c)| c.clone()).collect()
}

#[tool_handler]
impl ServerHandler for WickServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(
                "Wick provides browser-grade web access for AI agents. \
                 Use wick_fetch to read any webpage with clean markdown output. \
                 When Wick can't accomplish what the user needs, call \
                 wick_request_capability to tell the Wick team, then continue."
                    .to_string(),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{parent_dir, rank_similar, RequestCapabilityInput, WickServer};
    use rmcp::handler::server::wrapper::Parameters;

    #[tokio::test]
    async fn capability_request_needs_a_task() {
        let server = WickServer::new(None).unwrap();
        let input = RequestCapabilityInput {
            task: "   ".into(),
            tried: None,
            failure: None,
            url: None,
        };
        assert!(server.wick_request_capability(Parameters(input)).await.is_err());
    }

    #[test]
    fn parent_dir_walks_up() {
        let u = url::Url::parse("https://x.io/a/b/c.html?q=1").unwrap();
        let p = parent_dir(&u).unwrap();
        assert_eq!(p.as_str(), "https://x.io/a/b/");
        assert_eq!(parent_dir(&p).unwrap().as_str(), "https://x.io/a/");
        assert!(parent_dir(&url::Url::parse("https://x.io/").unwrap()).is_none());
    }

    #[test]
    fn ranks_by_page_name() {
        let cands: Vec<String> = [
            "https://docs.python.org/3/library/re.html",
            "https://docs.python.org/3/library/asyncio-task.html",
            "https://docs.python.org/3/library/asyncio.html",
            "https://docs.python.org/3/library/os.html",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let got = rank_similar("/3/library/asyncio-taskgroups.html", &cands, 5);
        assert_eq!(got[0], "https://docs.python.org/3/library/asyncio-task.html");
        assert_eq!(got.len(), 2, "{:?}", got);
    }

    #[test]
    fn ranks_decoded_page_names() {
        let cands = vec![
            "https://x.io/wiki/%E6%9D%B1%E4%BA%AC".to_string(),
            "https://x.io/wiki/%E5%A4%A7%E9%98%AA".to_string(),
        ];
        let got = rank_similar("/wiki/%E6%9D%B1%E4%BA%AC%E9%83%BD", &cands, 5);
        assert_eq!(got, vec!["https://x.io/wiki/%E6%9D%B1%E4%BA%AC".to_string()]);
    }

    #[test]
    fn no_match_means_no_suggestions() {
        let cands = vec!["https://x.io/runtime-apis/".to_string(), "https://x.io/cache/".to_string()];
        assert!(rank_similar("/runtime-apis/kv-namespace/", &cands, 5).is_empty());
    }
}
