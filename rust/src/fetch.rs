use anyhow::Result;
use std::time::Instant;

use crate::analytics::{self, FetchEvent};
use crate::captcha;
use crate::engine::Client;
use crate::extract::{self, Format};
use crate::robots;
use crate::site_cache;

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
///
/// Strategy selection:
///   - site_cache "cronet" for this host → try Cronet first; only escalate if it fails.
///   - site_cache "cef" for this host → go straight to CEF.
///   - No cache entry → prefer CEF if installed (current default), else Cronet.
///
/// Every terminal return point records a `FetchEvent` and updates `site_cache`.
pub async fn fetch(
    client: &Client,
    url: &str,
    format: Format,
    respect_robots: bool,
) -> Result<FetchResult> {
    let start = Instant::now();
    analytics::ping("fetch");

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

    // robots.txt check. Not a "strategy" outcome — don't tag as telemetry.
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

    let cef_installed = crate::cef::is_available();
    let cached = site_cache::get(host);

    // Strategy selection:
    //   Cache says "cef" → skip Cronet probe, go straight to CEF.
    //   Cache says "cronet" (or unset) → try Cronet first; CEF is the
    //     escalation path if Cronet gets blocked. Default is Cronet-first
    //     because most sites don't need JS rendering and Cronet is
    //     ~20x faster. CEF gets used when the site actually demands it.
    let cef_first = matches!(&cached, Some(s) if s.strategy == "cef") && cef_installed;

    if cef_first {
        match crate::cef::render(url).await {
            Ok(html) => {
                let extracted = extract::extract(&html, &parsed, format)?;
                let content = append_media(&extracted.content, &html, &parsed);
                let timing_ms = start.elapsed().as_millis() as u64;
                site_cache::record(host, "cef", false, timing_ms);
                analytics::report_fetch(FetchEvent {
                    host, strategy: "cef", escalated_from: None,
                    ok: true, status: 200, timing_ms,
                });
                return Ok(FetchResult {
                    content, title: extracted.title,
                    url: url.to_string(), status_code: 200, timing_ms,
                });
            }
            Err(e) => {
                tracing::warn!("CEF renderer failed: {}. Falling back to Cronet.", e);
                analytics::report_failure(host, 0, "cef_failed");
                // Fall through to Cronet.
            }
        }
    }

    // Cronet path (either by choice or as fallback from CEF).
    let escalated_from = if cef_first { Some("cef") } else { None };
    let resp = client.get(url).await?;
    let status = resp.status;
    let body = resp.body;

    // CAPTCHA detection → auto-solve (BYO CapSolver key) or user-in-the-loop.
    if (status == 403 || status == 503) && is_challenge(&body) {
        // 1. Auto-solve: if CAPSOLVER_API_KEY is set and the CAPTCHA type is supported.
        if let Some(cap_key) = crate::captcha_auto::load_capsolver_key() {
            if let Some(detected) = crate::captcha_auto::detect_captcha(&body) {
                tracing::info!("CAPTCHA detected on {}. Trying auto-solve via CapSolver...", host);
                match crate::captcha_auto::solve(&cap_key, url, &detected).await {
                    Ok(_token) => {
                        let retry = client.get(url).await?;
                        if retry.status < 400 {
                            let extracted = extract::extract(&retry.body, &parsed, format)?;
                            let timing_ms = start.elapsed().as_millis() as u64;
                            site_cache::record(host, "captcha-auto", false, timing_ms);
                            analytics::report_fetch(FetchEvent {
                                host, strategy: "captcha-auto",
                                escalated_from: Some("cronet"),
                                ok: true, status: retry.status, timing_ms,
                            });
                            return Ok(FetchResult {
                                content: extracted.content, title: extracted.title,
                                url: url.to_string(), status_code: retry.status, timing_ms,
                            });
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Auto-CAPTCHA failed ({}); falling back to interactive solver.", e);
                    }
                }
            }
        }

        // 2. Interactive fallback: wick-captcha.
        if captcha::is_available() {
            tracing::info!("CAPTCHA detected on {}. Launching solver...", host);
            match captcha::solve(url).await {
                Ok(cookies) => {
                    tracing::info!("CAPTCHA solved! Got {} cookies. Retrying request...", cookies.len());
                    let retry = client.get(url).await?;
                    let timing_ms = start.elapsed().as_millis() as u64;
                    if retry.status < 400 {
                        let extracted = extract::extract(&retry.body, &parsed, format)?;
                        site_cache::record(host, "captcha-interactive", false, timing_ms);
                        analytics::report_fetch(FetchEvent {
                            host, strategy: "captcha-interactive",
                            escalated_from: Some("cronet"),
                            ok: true, status: retry.status, timing_ms,
                        });
                        return Ok(FetchResult {
                            content: extracted.content, title: extracted.title,
                            url: url.to_string(), status_code: retry.status, timing_ms,
                        });
                    }
                    analytics::report_fetch(FetchEvent {
                        host, strategy: "captcha-interactive",
                        escalated_from: Some("cronet"),
                        ok: false, status: retry.status, timing_ms,
                    });
                    return Ok(FetchResult {
                        content: format!("HTTP {} after CAPTCHA solve: {}", retry.status, retry.body),
                        title: None, url: url.to_string(),
                        status_code: retry.status, timing_ms,
                    });
                }
                Err(e) => {
                    tracing::warn!("CAPTCHA solving failed: {}", e);
                }
            }
        }

        // CAPTCHA with no solver available.
        let timing_ms = start.elapsed().as_millis() as u64;
        analytics::report_fetch(FetchEvent {
            host, strategy: "captcha-blocked", escalated_from,
            ok: false, status, timing_ms,
        });
        return Ok(FetchResult {
            content: "This page returned a CAPTCHA or browser challenge. \
                      The content could not be extracted automatically.\n\
                      Install wick-captcha to solve CAPTCHAs interactively."
                .to_string(),
            title: None, url: url.to_string(), status_code: status, timing_ms,
        });
    }

    // Cronet blocked but CEF might work — escalate if available and we haven't tried yet.
    if (status == 403 || status == 503) && cef_installed && escalated_from.is_none() {
        match crate::cef::render(url).await {
            Ok(html) => {
                let extracted = extract::extract(&html, &parsed, format)?;
                let content = append_media(&extracted.content, &html, &parsed);
                let timing_ms = start.elapsed().as_millis() as u64;
                site_cache::record(host, "cef", false, timing_ms);
                analytics::report_fetch(FetchEvent {
                    host, strategy: "cef-after-cronet",
                    escalated_from: Some("cronet"),
                    ok: true, status: 200, timing_ms,
                });
                return Ok(FetchResult {
                    content, title: extracted.title,
                    url: url.to_string(), status_code: 200, timing_ms,
                });
            }
            Err(e) => {
                tracing::warn!("CEF escalation failed: {}", e);
                analytics::report_failure(host, 0, "cef_failed");
                // Fall through to the blocked response.
            }
        }
    }

    if status == 403 || status == 503 {
        let timing_ms = start.elapsed().as_millis() as u64;
        analytics::report_failure(host, status, "blocked");
        analytics::report_fetch(FetchEvent {
            host, strategy: "cronet-blocked", escalated_from,
            ok: false, status, timing_ms,
        });

        // If CEF is not installed, hint at wick install cef.
        if !cef_installed {
            let hint = format!(
                "HTTP {status}\n\n\
                 This site blocked the request. Install the CEF renderer\n\
                 for JS rendering and advanced stealth:\n\n\
                 wick install cef"
            );
            return Ok(FetchResult {
                content: hint, title: None,
                url: url.to_string(), status_code: status, timing_ms,
            });
        }
        return Ok(FetchResult {
            content: format!("HTTP {}: {}", status, body),
            title: None, url: url.to_string(), status_code: status, timing_ms,
        });
    }

    if status >= 400 {
        let timing_ms = start.elapsed().as_millis() as u64;
        analytics::report_fetch(FetchEvent {
            host, strategy: "cronet-error", escalated_from,
            ok: false, status, timing_ms,
        });
        return Ok(FetchResult {
            content: format!("HTTP {}: {}", status, body),
            title: None, url: url.to_string(), status_code: status, timing_ms,
        });
    }

    let extracted = extract::extract(&body, &parsed, format)?;
    let content = append_media(&extracted.content, &body, &parsed);
    let timing_ms = start.elapsed().as_millis() as u64;
    site_cache::record(host, "cronet", false, timing_ms);
    analytics::report_fetch(FetchEvent {
        host, strategy: "cronet", escalated_from,
        ok: true, status, timing_ms,
    });
    Ok(FetchResult {
        content, title: extracted.title,
        url: url.to_string(), status_code: status, timing_ms,
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

/// Fetch raw HTML. Shares the adaptive strategy selection with `fetch()`.
pub async fn fetch_html(
    client: &Client,
    url: &str,
    respect_robots: bool,
) -> Result<FetchHtmlResult> {
    let start = Instant::now();
    let parsed = url::Url::parse(url)
        .map_err(|e| anyhow::anyhow!("invalid URL: {}", e))?;

    let host = parsed.host_str().ok_or_else(|| anyhow::anyhow!("missing host"))?;

    if respect_robots && !robots::check(client, url).await {
        return Ok(FetchHtmlResult {
            html: String::new(), url: url.to_string(), status_code: 0,
        });
    }

    let cef_installed = crate::cef::is_available();
    let cached = site_cache::get(host);
    let cef_first = matches!(&cached, Some(s) if s.strategy == "cef") && cef_installed;

    if cef_first {
        match crate::cef::render(url).await {
            Ok(html) => {
                let timing_ms = start.elapsed().as_millis() as u64;
                site_cache::record(host, "cef", false, timing_ms);
                analytics::report_fetch(FetchEvent {
                    host, strategy: "cef", escalated_from: None,
                    ok: true, status: 200, timing_ms,
                });
                return Ok(FetchHtmlResult {
                    html, url: url.to_string(), status_code: 200,
                });
            }
            Err(e) => {
                tracing::warn!("CEF renderer failed: {}. Falling back to Cronet.", e);
            }
        }
    }

    let escalated_from = if cef_first { Some("cef") } else { None };
    let resp = client.get(url).await?;
    let timing_ms = start.elapsed().as_millis() as u64;
    let ok = resp.status < 400;

    if ok {
        site_cache::record(host, "cronet", false, timing_ms);
    }
    analytics::report_fetch(FetchEvent {
        host, strategy: "cronet", escalated_from,
        ok, status: resp.status, timing_ms,
    });

    if !ok {
        tracing::debug!("fetch_html {} returned HTTP {}", host, resp.status);
    }

    Ok(FetchHtmlResult {
        html: resp.body, url: url.to_string(), status_code: resp.status,
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
