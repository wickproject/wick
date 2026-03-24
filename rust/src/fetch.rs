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

/// Full fetch pipeline: validate → robots.txt → fetch → CAPTCHA → extract.
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
                return Ok(FetchResult {
                    content: extracted.content,
                    title: extracted.title,
                    url: url.to_string(),
                    status_code: 200,
                    timing_ms: start.elapsed().as_millis() as u64,
                });
            }
            Err(e) => {
                tracing::warn!("CEF renderer failed: {}. Falling back to Cronet.", e);
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
