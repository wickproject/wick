use anyhow::Result;
use std::time::Instant;

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

/// Full fetch pipeline: validate → robots.txt → CEF render → extract.
///
/// Always uses CEF (Chromium) for fetching + rendering. This means:
/// - Single request per page (no double-fetch fingerprint signal)
/// - JS rendering works automatically on every page
/// - Same Chrome TLS fingerprint as Cronet (CEF IS Chromium)
/// - Falls back to Cronet if CEF is not available
pub async fn fetch(
    client: &Client,
    url: &str,
    format: Format,
    respect_robots: bool,
) -> Result<FetchResult> {
    let start = Instant::now();

    let parsed = url::Url::parse(url)
        .map_err(|e| anyhow::anyhow!("invalid URL: {}", e))?;

    match parsed.scheme() {
        "http" | "https" => {}
        s => anyhow::bail!("unsupported scheme {:?} (only http and https)", s),
    }

    let host = parsed.host_str().ok_or_else(|| anyhow::anyhow!("missing host"))?;

    // robots.txt check (uses Cronet for the lightweight robots.txt fetch)
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

    // Primary path: CEF (full Chromium rendering, single request).
    // Falls back to Cronet if CEF crashes or is unavailable.
    if crate::cef::is_available() {
        match crate::cef::render(url).await {
            Ok(rendered_html) => {
                let extracted = extract::extract(&rendered_html, &parsed, format)?;
                return Ok(FetchResult {
                    content: extracted.content,
                    title: extracted.title,
                    url: url.to_string(),
                    status_code: 0, // CEF doesn't report HTTP status yet
                    timing_ms: start.elapsed().as_millis() as u64,
                });
            }
            Err(e) => {
                tracing::debug!("CEF render failed, falling back to Cronet: {}", e);
            }
        }
    }

    // Fallback: Cronet (no JS rendering, but Chrome TLS fingerprint)
    let resp = client.get(url).await?;
    let status = resp.status;
    let body = resp.body;

    if status == 403 || status == 503 {
        if is_challenge(&body) {
            return Ok(FetchResult {
                content: "This page returned a CAPTCHA or browser challenge. \
                          The content could not be extracted automatically."
                    .to_string(),
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
    Ok(FetchResult {
        content: extracted.content,
        title: extracted.title,
        url: url.to_string(),
        status_code: status,
        timing_ms: start.elapsed().as_millis() as u64,
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
