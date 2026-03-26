use anyhow::Result;
use std::time::Instant;

use crate::captcha;
use crate::engine::Client;
use crate::extract::{self, Format};
use crate::robots;

pub struct FetchResult {
    pub content: String,
    pub title: Option<String>,
    pub url: String,
    pub status_code: u16,
    pub timing_ms: u64,
}

/// Raw HTML fetch result (used by crawl engine for link extraction).
pub struct FetchHtmlResult {
    pub html: String,
    pub url: String,
    pub status_code: u16,
}

/// Full fetch pipeline: validate → robots.txt → fetch → CAPTCHA → extract.
pub async fn fetch(
    client: &Client,
    url: &str,
    format: Format,
    respect_robots: bool,
) -> Result<FetchResult> {
    let start = Instant::now();
    crate::analytics::ping("fetch");

    let parsed = url::Url::parse(url)
        .map_err(|e| anyhow::anyhow!("invalid URL: {}", e))?;

    match parsed.scheme() {
        "http" | "https" => {}
        s => anyhow::bail!("unsupported scheme {:?} (only http and https)", s),
    }

    let host = parsed.host_str().ok_or_else(|| anyhow::anyhow!("missing host"))?;

    // Rewrite www.reddit.com → old.reddit.com for better content extraction.
    if host == "www.reddit.com" || host == "reddit.com" {
        let old_url = url
            .replace("://www.reddit.com", "://old.reddit.com")
            .replace("://reddit.com", "://old.reddit.com");
        return Box::pin(fetch(client, &old_url, format, respect_robots)).await;
    }

    // robots.txt check
    if respect_robots && !robots::check(client, url).await {
        return Ok(FetchResult {
            content: format!(
                "Blocked by robots.txt: {} disallows this path for automated agents.\n\
                 Use respect_robots=false to override (the user takes responsibility).",
                host
            ),
            title: None,
            url: url.to_string(),
            status_code: 0,
            timing_ms: start.elapsed().as_millis() as u64,
        });
    }

    // If Pro renderer is installed, use it (JS rendering + stealth patches)
    if crate::cef::is_available() {
        match crate::cef::render(url).await {
            Ok(html) => {
                let extracted = extract::extract(&html, &parsed, format)?;
                let content = append_media(&extracted.content, &html, &parsed);
                return Ok(FetchResult {
                    content,
                    title: extracted.title,
                    url: url.to_string(),
                    status_code: 200,
                    timing_ms: start.elapsed().as_millis() as u64,
                });
            }
            Err(e) => {
                tracing::warn!("CEF renderer failed: {}. Falling back to Cronet.", e);
                crate::analytics::report_failure(host, 0, "cef_failed");
            }
        }
    }

    // Free tier: fetch via Cronet (Chrome TLS fingerprint)
    let resp = client.get(url).await?;
    let status = resp.status;
    let body = resp.body;

    // CAPTCHA detection → user-in-the-loop solving
    if (status == 403 || status == 503) && is_challenge(&body) {
        if captcha::is_available() {
            tracing::info!("CAPTCHA detected on {}. Launching solver...", host);
            match captcha::solve(url).await {
                Ok(cookies) => {
                    tracing::info!(
                        "CAPTCHA solved! Got {} cookies. Retrying request...",
                        cookies.len()
                    );
                    // Retry the request — Cronet should pick up cookies from
                    // its persistent store, but also add them as a header
                    // in case the cookie store doesn't sync immediately.
                    let retry = client.get(url).await?;
                    if retry.status < 400 {
                        let extracted = extract::extract(&retry.body, &parsed, format)?;
                        return Ok(FetchResult {
                            content: extracted.content,
                            title: extracted.title,
                            url: url.to_string(),
                            status_code: retry.status,
                            timing_ms: start.elapsed().as_millis() as u64,
                        });
                    }
                    // Retry still failed — return the retry response
                    return Ok(FetchResult {
                        content: format!("HTTP {} after CAPTCHA solve: {}", retry.status, retry.body),
                        title: None,
                        url: url.to_string(),
                        status_code: retry.status,
                        timing_ms: start.elapsed().as_millis() as u64,
                    });
                }
                Err(e) => {
                    tracing::warn!("CAPTCHA solving failed: {}", e);
                    // Fall through to return the challenge response
                }
            }
        }

        return Ok(FetchResult {
            content: "This page returned a CAPTCHA or browser challenge. \
                      The content could not be extracted automatically.\n\
                      Install wick-captcha to solve CAPTCHAs interactively."
                .to_string(),
            title: None,
            url: url.to_string(),
            status_code: status,
            timing_ms: start.elapsed().as_millis() as u64,
        });
    }

    if status == 403 || status == 503 {
        crate::analytics::report_failure(host, status, "blocked");

        // Smart upsell: suggest Pro when it would help
        if !crate::cef::is_available() {
            let hint = format!(
                "HTTP {status}\n\n\
                 This site blocked the request. Wick Pro ($20/mo) bypasses\n\
                 anti-bot protection with JS rendering and advanced stealth.\n\n\
                 Activate: wick pro activate"
            );
            return Ok(FetchResult {
                content: hint,
                title: None,
                url: url.to_string(),
                status_code: status,
                timing_ms: start.elapsed().as_millis() as u64,
            });
        }
        return Ok(FetchResult {
            content: format!("HTTP {}: {}", status, body),
            title: None,
            url: url.to_string(),
            status_code: status,
            timing_ms: start.elapsed().as_millis() as u64,
        });
    }

    if status >= 400 {
        return Ok(FetchResult {
            content: format!("HTTP {}: {}", status, body),
            title: None,
            url: url.to_string(),
            status_code: status,
            timing_ms: start.elapsed().as_millis() as u64,
        });
    }

    let extracted = extract::extract(&body, &parsed, format)?;
    let content = append_media(&extracted.content, &body, &parsed);
    Ok(FetchResult {
        content,
        title: extracted.title,
        url: url.to_string(),
        status_code: status,
        timing_ms: start.elapsed().as_millis() as u64,
    })
}

/// Append detected media URLs to content so agents/crawlers can discover them.
fn append_media(content: &str, html: &str, page_url: &url::Url) -> String {
    let media = crate::media::extract_media(html, page_url);
    if media.is_empty() {
        return content.to_string();
    }
    let mut result = content.to_string();
    result.push_str("\n\n---\n**Media found on this page:**\n");
    for m in &media {
        result.push_str(&format!("- [{}] {} ({})\n", m.media_type, m.url, m.source));
        result.push_str(&format!("  Download: `wick download \"{}\"`\n", m.url));
    }
    result
}

/// Fetch raw HTML from a URL. Handles robots.txt, CEF/Cronet routing, and CAPTCHA.
/// Used by the crawl engine to get HTML for link extraction.
pub async fn fetch_html(
    client: &Client,
    url: &str,
    respect_robots: bool,
) -> Result<FetchHtmlResult> {
    let parsed = url::Url::parse(url)
        .map_err(|e| anyhow::anyhow!("invalid URL: {}", e))?;

    let host = parsed.host_str().ok_or_else(|| anyhow::anyhow!("missing host"))?;

    // robots.txt check
    if respect_robots && !robots::check(client, url).await {
        return Ok(FetchHtmlResult {
            html: String::new(),
            url: url.to_string(),
            status_code: 0,
        });
    }

    // If Pro renderer is installed, use it
    if crate::cef::is_available() {
        match crate::cef::render(url).await {
            Ok(html) => {
                return Ok(FetchHtmlResult {
                    html,
                    url: url.to_string(),
                    status_code: 200,
                });
            }
            Err(e) => {
                tracing::warn!("CEF renderer failed: {}. Falling back to Cronet.", e);
            }
        }
    }

    // Fetch via Cronet/reqwest
    let resp = client.get(url).await?;

    if resp.status >= 400 {
        tracing::debug!("fetch_html {} returned HTTP {}", host, resp.status);
    }

    Ok(FetchHtmlResult {
        html: resp.body,
        url: url.to_string(),
        status_code: resp.status,
    })
}

fn is_challenge(body: &str) -> bool {
    let lower = body.to_lowercase();
    [
        "challenges.cloudflare.com",
        "cf-browser-verification",
        "just a moment...",
        "checking your browser",
        "google.com/recaptcha",
        "hcaptcha.com",
    ]
    .iter()
    .any(|sig| lower.contains(sig))
}
