//! Local HTTP API server for Wick.
//! Exposes wick_fetch, wick_crawl, wick_map, wick_search as REST endpoints.
//! Runs on localhost — makes Wick accessible to any tool, not just MCP clients.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use tower_http::cors::CorsLayer;

use crate::crawl;
use crate::engine::Client;
use crate::extract::Format;
use crate::fetch;
use crate::search;

#[derive(Clone)]
struct AppState {
    client: Arc<Client>,
}

// ── Request types ────────────────────────────────────────────

#[derive(Deserialize)]
pub struct FetchParams {
    url: String,
    format: Option<String>,
    respect_robots: Option<bool>,
}

#[derive(Deserialize)]
pub struct CrawlParams {
    url: String,
    max_depth: Option<u32>,
    max_pages: Option<u32>,
    format: Option<String>,
    respect_robots: Option<bool>,
    path_filter: Option<String>,
}

#[derive(Deserialize)]
pub struct MapParams {
    url: String,
    limit: Option<u32>,
    use_sitemap: Option<bool>,
    respect_robots: Option<bool>,
    path_filter: Option<String>,
}

#[derive(Deserialize)]
pub struct SearchParams {
    q: String,
    num: Option<usize>,
}

// ── Response types ───────────────────────────────────────────

#[derive(Serialize)]
struct FetchResponse {
    url: String,
    status: u16,
    content: String,
    title: Option<String>,
    timing_ms: u64,
}

#[derive(Serialize)]
struct CrawlResponse {
    pages: Vec<CrawlPageResponse>,
    urls_discovered: usize,
    timing_ms: u64,
}

#[derive(Serialize)]
struct CrawlPageResponse {
    url: String,
    title: Option<String>,
    content: String,
}

#[derive(Serialize)]
struct MapResponse {
    urls: Vec<String>,
    count: usize,
    from_sitemap: usize,
    timing_ms: u64,
}

#[derive(Serialize)]
struct SearchResponse {
    results: Vec<SearchResultResponse>,
}

#[derive(Serialize)]
struct SearchResultResponse {
    title: String,
    url: String,
    snippet: String,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

// ── Handlers ─────────────────────────────────────────────────

async fn handle_fetch(
    State(state): State<AppState>,
    Query(params): Query<FetchParams>,
) -> impl IntoResponse {
    let format = params.format.as_deref().map(Format::from_str).unwrap_or(Format::Markdown);
    let respect_robots = params.respect_robots.unwrap_or(true);

    match fetch::fetch(&state.client, &params.url, format, respect_robots).await {
        Ok(result) => Json(FetchResponse {
            url: params.url,
            status: result.status_code,
            content: result.content,
            title: result.title,
            timing_ms: result.timing_ms,
        }).into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(ErrorResponse { error: e.to_string() }),
        ).into_response(),
    }
}

async fn handle_crawl(
    State(state): State<AppState>,
    Query(params): Query<CrawlParams>,
) -> impl IntoResponse {
    let format = params.format.as_deref().map(Format::from_str).unwrap_or(Format::Markdown);
    let options = crawl::CrawlOptions {
        max_depth: params.max_depth.unwrap_or(2).min(5),
        max_pages: params.max_pages.unwrap_or(10).min(50),
        format,
        respect_robots: params.respect_robots.unwrap_or(true),
        path_filter: params.path_filter,
    };

    match crawl::crawl(&state.client, &params.url, options).await {
        Ok(result) => Json(CrawlResponse {
            pages: result.pages.iter().map(|p| CrawlPageResponse {
                url: p.url.clone(),
                title: p.title.clone(),
                content: p.content.clone(),
            }).collect(),
            urls_discovered: result.urls_discovered,
            timing_ms: result.timing_ms,
        }).into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(ErrorResponse { error: e.to_string() }),
        ).into_response(),
    }
}

async fn handle_map(
    State(state): State<AppState>,
    Query(params): Query<MapParams>,
) -> impl IntoResponse {
    let options = crawl::MapOptions {
        limit: params.limit.unwrap_or(100).min(5000),
        use_sitemap: params.use_sitemap.unwrap_or(true),
        respect_robots: params.respect_robots.unwrap_or(true),
        path_filter: params.path_filter,
    };

    match crawl::map(&state.client, &params.url, options).await {
        Ok(result) => {
            let count = result.urls.len();
            Json(MapResponse {
                urls: result.urls,
                count,
                from_sitemap: result.from_sitemap,
                timing_ms: result.timing_ms,
            }).into_response()
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(ErrorResponse { error: e.to_string() }),
        ).into_response(),
    }
}

async fn handle_search(
    State(state): State<AppState>,
    Query(params): Query<SearchParams>,
) -> impl IntoResponse {
    let num = params.num.unwrap_or(5).max(1).min(20);

    match search::search(&state.client, &params.q, num).await {
        Ok(results) => Json(SearchResponse {
            results: results.iter().map(|r| SearchResultResponse {
                title: r.title.clone(),
                url: r.url.clone(),
                snippet: r.snippet.clone(),
            }).collect(),
        }).into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(ErrorResponse { error: e.to_string() }),
        ).into_response(),
    }
}

async fn handle_health() -> impl IntoResponse {
    let pro = if crate::cef::is_available() { " + Pro" } else { "" };
    Json(serde_json::json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "pro": crate::cef::is_available(),
        "label": format!("wick {}{}", env!("CARGO_PKG_VERSION"), pro),
    }))
}

// ── Server ───────────────────────────────────────────────────

pub async fn serve(port: u16, proxy: Option<&str>) -> anyhow::Result<()> {
    let client = Client::new(proxy)?;
    let state = AppState {
        client: Arc::new(client),
    };

    let app = Router::new()
        .route("/v1/fetch", axum::routing::get(handle_fetch))
        .route("/v1/crawl", axum::routing::get(handle_crawl))
        .route("/v1/map", axum::routing::get(handle_map))
        .route("/v1/search", axum::routing::get(handle_search))
        .route("/health", axum::routing::get(handle_health))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = format!("127.0.0.1:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    let pro = if crate::cef::is_available() { " + Pro" } else { "" };
    eprintln!("Wick {}{} API server running at http://{}", env!("CARGO_PKG_VERSION"), pro, addr);
    eprintln!("");
    eprintln!("  GET /v1/fetch?url=...          Fetch a page as markdown");
    eprintln!("  GET /v1/crawl?url=...          Crawl a site");
    eprintln!("  GET /v1/map?url=...            Discover URLs on a site");
    eprintln!("  GET /v1/search?q=...           Web search");
    eprintln!("  GET /health                    Status check");
    eprintln!("");

    axum::serve(listener, app).await?;
    Ok(())
}
