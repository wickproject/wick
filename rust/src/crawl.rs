//! BFS crawl engine for wick_crawl and wick_map.
//! Follows same-domain links, respects rate limits, returns markdown per page.

use std::collections::{HashSet, VecDeque};
use std::time::{Duration, Instant};

use anyhow::Result;
use scraper::{Html, Selector};
use url::Url;

use crate::engine::Client;
use crate::extract::{self, Format};
use crate::fetch;

// ── Crawl ────────────────────────────────────────────────────

pub struct CrawlOptions {
    pub max_depth: u32,
    pub max_pages: u32,
    pub format: Format,
    pub respect_robots: bool,
    pub path_filter: Option<String>,
}

pub struct CrawlPage {
    pub url: String,
    pub title: Option<String>,
    pub content: String,
    pub depth: u32,
}

pub struct CrawlResult {
    pub pages: Vec<CrawlPage>,
    pub urls_discovered: usize,
    pub timing_ms: u64,
}

pub async fn crawl(
    client: &Client,
    start_url: &str,
    options: CrawlOptions,
) -> Result<CrawlResult> {
    let start = Instant::now();
    let parsed = Url::parse(start_url)?;
    let origin = get_origin(&parsed);

    let mut visited: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<(String, u32)> = VecDeque::new();
    let mut pages: Vec<CrawlPage> = Vec::new();
    let mut urls_discovered: usize = 0;

    let normalized = normalize_url(start_url);
    visited.insert(normalized);
    queue.push_back((start_url.to_string(), 0));

    while let Some((url, depth)) = queue.pop_front() {
        if pages.len() >= options.max_pages as usize {
            break;
        }

        // Rate limit: 500ms between requests
        if !pages.is_empty() {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        let html_result = fetch::fetch_html(client, &url, options.respect_robots).await;
        let html_result = match html_result {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!("crawl skip {}: {}", url, e);
                continue;
            }
        };

        if html_result.status_code >= 400 || html_result.html.is_empty() {
            continue;
        }

        // Extract links for BFS (before depth limit check on children)
        if depth < options.max_depth {
            let page_url = Url::parse(&url).unwrap_or(parsed.clone());
            let links = extract_links(&html_result.html, &page_url);
            for link in links {
                let link_str = link.to_string();
                let norm = normalize_url(&link_str);

                if !is_same_origin(&link, &origin) {
                    continue;
                }
                if let Some(ref prefix) = options.path_filter {
                    if !link.path().starts_with(prefix.as_str()) {
                        continue;
                    }
                }
                if !is_crawlable_extension(&link) {
                    continue;
                }
                if visited.contains(&norm) {
                    continue;
                }

                visited.insert(norm);
                urls_discovered += 1;
                queue.push_back((link_str, depth + 1));
            }
        }

        // Extract content
        let page_url = Url::parse(&url).unwrap_or(parsed.clone());
        let extracted = extract::extract(&html_result.html, &page_url, options.format)
            .unwrap_or(extract::Extracted {
                content: String::new(),
                title: None,
            });

        // Cap per-page content at 10K chars
        let content = if extracted.content.len() > 10_000 {
            let mut truncated = extracted.content[..10_000].to_string();
            truncated.push_str("\n\n[... truncated]");
            truncated
        } else {
            extracted.content
        };

        pages.push(CrawlPage {
            url: url.clone(),
            title: extracted.title,
            content,
            depth,
        });
    }

    Ok(CrawlResult {
        pages,
        urls_discovered,
        timing_ms: start.elapsed().as_millis() as u64,
    })
}

pub fn format_crawl_output(result: &CrawlResult, host: &str) -> String {
    let mut out = format!(
        "# Crawl Results: {}\nCrawled {} pages in {:.1}s\n",
        host,
        result.pages.len(),
        result.timing_ms as f64 / 1000.0
    );

    let mut total_len = out.len();
    for (i, page) in result.pages.iter().enumerate() {
        let title = page.title.as_deref().unwrap_or("Untitled");
        let section = format!(
            "\n---\n## [{}/{}] {}\n**URL:** {}\n\n{}\n",
            i + 1,
            result.pages.len(),
            title,
            page.url,
            page.content,
        );

        // Cap total output at 100K chars
        if total_len + section.len() > 100_000 {
            out.push_str(&format!(
                "\n---\n*Output truncated. {} more pages not shown.*\n",
                result.pages.len() - i
            ));
            break;
        }
        total_len += section.len();
        out.push_str(&section);
    }

    out
}

// ── Map ──────────────────────────────────────────────────────

pub struct MapOptions {
    pub limit: u32,
    pub use_sitemap: bool,
    pub respect_robots: bool,
    pub path_filter: Option<String>,
}

pub struct MapResult {
    pub urls: Vec<String>,
    pub timing_ms: u64,
    pub from_sitemap: usize,
}

pub async fn map(
    client: &Client,
    start_url: &str,
    options: MapOptions,
) -> Result<MapResult> {
    let start = Instant::now();
    let parsed = Url::parse(start_url)?;
    let origin = get_origin(&parsed);
    let limit = options.limit as usize;

    let mut discovered: Vec<String> = Vec::new();
    let mut visited: HashSet<String> = HashSet::new();
    let mut from_sitemap = 0;

    // Try sitemap.xml first
    if options.use_sitemap {
        let sitemap_urls = fetch_sitemap(client, &parsed).await;
        for url in sitemap_urls {
            let norm = normalize_url(&url);
            if !visited.contains(&norm) {
                if let Some(ref prefix) = options.path_filter {
                    if let Ok(u) = Url::parse(&url) {
                        if !u.path().starts_with(prefix.as_str()) {
                            continue;
                        }
                    }
                }
                visited.insert(norm);
                discovered.push(url);
                from_sitemap += 1;
                if discovered.len() >= limit {
                    break;
                }
            }
        }
    }

    // BFS link crawl for additional discovery
    let mut queue: VecDeque<String> = VecDeque::new();
    let seed_norm = normalize_url(start_url);
    if !visited.contains(&seed_norm) {
        visited.insert(seed_norm);
        discovered.push(start_url.to_string());
    }
    queue.push_back(start_url.to_string());

    let mut pages_fetched = 0;
    let max_fetch = 20.min(limit); // Don't fetch more than 20 pages for map

    while let Some(url) = queue.pop_front() {
        if discovered.len() >= limit || pages_fetched >= max_fetch {
            break;
        }

        if pages_fetched > 0 {
            tokio::time::sleep(Duration::from_millis(300)).await;
        }

        let html_result = fetch::fetch_html(client, &url, options.respect_robots).await;
        let html_result = match html_result {
            Ok(r) if r.status_code < 400 && !r.html.is_empty() => r,
            _ => continue,
        };
        pages_fetched += 1;

        let page_url = Url::parse(&url).unwrap_or(parsed.clone());
        let links = extract_links(&html_result.html, &page_url);

        for link in links {
            let link_str = link.to_string();
            let norm = normalize_url(&link_str);

            if !is_same_origin(&link, &origin) {
                continue;
            }
            if let Some(ref prefix) = options.path_filter {
                if !link.path().starts_with(prefix.as_str()) {
                    continue;
                }
            }
            if !is_crawlable_extension(&link) {
                continue;
            }
            if visited.contains(&norm) {
                continue;
            }

            visited.insert(norm);
            discovered.push(link_str.clone());
            queue.push_back(link_str);

            if discovered.len() >= limit {
                break;
            }
        }
    }

    discovered.sort();

    Ok(MapResult {
        urls: discovered,
        timing_ms: start.elapsed().as_millis() as u64,
        from_sitemap,
    })
}

pub fn format_map_output(result: &MapResult, host: &str) -> String {
    let mut out = format!(
        "# Site Map: {}\nFound {} URLs",
        host,
        result.urls.len(),
    );
    if result.from_sitemap > 0 {
        out.push_str(&format!(" ({} from sitemap.xml)", result.from_sitemap));
    }
    out.push_str("\n\n");

    for url in &result.urls {
        out.push_str(&format!("- {}\n", url));
    }
    out
}

// ── Sitemap ──────────────────────────────────────────────────

async fn fetch_sitemap(client: &Client, base_url: &Url) -> Vec<String> {
    let sitemap_url = format!("{}://{}/sitemap.xml", base_url.scheme(), base_url.host_str().unwrap_or(""));

    let resp = match client.get(&sitemap_url).await {
        Ok(r) if r.status < 400 => r,
        _ => return Vec::new(),
    };

    parse_sitemap_xml(&resp.body)
}

fn parse_sitemap_xml(xml: &str) -> Vec<String> {
    let mut urls = Vec::new();

    // Simple regex-free parsing: find all <loc>...</loc> tags
    let lower = xml.to_lowercase();
    let mut search_from = 0;
    while let Some(start_tag) = lower[search_from..].find("<loc>") {
        let content_start = search_from + start_tag + 5;
        if let Some(end_offset) = lower[content_start..].find("</loc>") {
            let content_end = content_start + end_offset;
            let url = xml[content_start..content_end].trim();
            if url.starts_with("http") {
                // Check if this is a nested sitemap (sitemap index)
                if url.ends_with(".xml") || url.ends_with(".xml.gz") {
                    // Skip nested sitemaps for now — keep it simple
                } else {
                    urls.push(url.to_string());
                }
            }
            search_from = content_end + 6;
        } else {
            break;
        }
    }

    urls
}

// ── Link extraction ──────────────────────────────────────────

fn extract_links(html: &str, page_url: &Url) -> Vec<Url> {
    let doc = Html::parse_document(html);
    let selector = Selector::parse("a[href]").unwrap();
    let mut links = Vec::new();
    let mut seen = HashSet::new();

    for element in doc.select(&selector) {
        if let Some(href) = element.value().attr("href") {
            let href = href.trim();
            if href.is_empty() || href.starts_with('#') || href.starts_with("javascript:") || href.starts_with("mailto:") {
                continue;
            }
            if let Ok(resolved) = page_url.join(href) {
                let norm = normalize_url(resolved.as_str());
                if !seen.contains(&norm) {
                    seen.insert(norm);
                    links.push(resolved);
                }
            }
        }
    }

    links
}

// ── URL utilities ────────────────────────────────────────────

fn normalize_url(url: &str) -> String {
    match Url::parse(url) {
        Ok(mut u) => {
            u.set_fragment(None);
            let mut s = u.to_string();
            // Remove trailing slash (except for root path)
            if s.ends_with('/') && u.path() != "/" {
                s.pop();
            }
            s
        }
        Err(_) => url.to_string(),
    }
}

fn get_origin(url: &Url) -> String {
    format!("{}://{}", url.scheme(), url.host_str().unwrap_or(""))
}

fn is_same_origin(url: &Url, origin: &str) -> bool {
    let url_origin = format!("{}://{}", url.scheme(), url.host_str().unwrap_or(""));
    url_origin == *origin
}

const SKIP_EXTENSIONS: &[&str] = &[
    ".jpg", ".jpeg", ".png", ".gif", ".svg", ".webp", ".ico", ".bmp",
    ".pdf", ".zip", ".tar", ".gz", ".rar",
    ".css", ".js", ".woff", ".woff2", ".ttf", ".eot",
    ".mp3", ".mp4", ".avi", ".mov", ".wmv", ".flv",
    ".xml", ".json", ".rss", ".atom",
];

fn is_crawlable_extension(url: &Url) -> bool {
    let path = url.path().to_lowercase();
    !SKIP_EXTENSIONS.iter().any(|ext| path.ends_with(ext))
}
