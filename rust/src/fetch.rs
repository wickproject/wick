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

/// Raw bytes fetch result. Returned by `fetch_raw` for binary content
/// (PDFs, archives, images). No extraction, no UTF-8 decoding.
pub struct FetchRawResult {
    pub bytes: Vec<u8>,
    pub url: String,
    pub status_code: u16,
    pub timing_ms: u64,
}

/// Caller-controlled override for transport selection.
///
/// - `Auto`: adaptive — Cronet first (or CEF first if the site cache says
///   so), with CEF escalation on 403/503 and on JS-required interstitials.
/// - `Cef`: force the CEF renderer (full Chromium with JS). Returns an
///   error if the renderer isn't installed.
/// - `Cronet`: force the network-only path. Never spawns CEF, even on
///   block or JS-shell — useful when you specifically want the raw HTML
///   the origin sends to a non-JS client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RenderMode {
    #[default]
    Auto,
    Cef,
    Cronet,
}

impl RenderMode {
    pub fn from_str(s: &str) -> RenderMode {
        match s.trim().to_ascii_lowercase().as_str() {
            "cef" | "browser" | "render" | "js" => RenderMode::Cef,
            "cronet" | "network" | "fast" | "no-js" | "nojs" => RenderMode::Cronet,
            _ => RenderMode::Auto,
        }
    }
}

/// Full fetch pipeline: validate → robots.txt → fetch → CAPTCHA → extract.
///
/// Strategy selection:
///   - site_cache "cef" for this host → skip Cronet probe, go straight to CEF.
///   - site_cache "cronet" (or unset) → try Cronet first; CEF is the
///     escalation path if Cronet returns 403/503 and CEF is installed.
///   - No cache entry → default to Cronet first; CEF is used only when
///     escalation logic later selects it.
///
/// Terminal return points record a `FetchEvent` and update `site_cache`,
/// except for the intentional robots.txt early return, which skips both
/// (it's a user-config outcome, not a site-strategy outcome).
///
/// `site_cache::strategy` only ever holds "cef", "cronet", or "cef_timeout"
/// — see `rust/src/site_cache.rs`. CAPTCHA outcomes are reported via
/// `report_fetch` but recorded into the cache as the underlying transport
/// strategy ("cronet"), since the next visit should retry that transport.
pub async fn fetch(
    client: &Client,
    url: &str,
    format: Format,
    respect_robots: bool,
    render: RenderMode,
    wait_for_selector: Option<&str>,
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
        return Box::pin(fetch(
            client,
            &old_url,
            format,
            respect_robots,
            render,
            wait_for_selector,
        ))
        .await;
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
    // Forced CEF: bail early with a hint if the renderer isn't installed.
    if render == RenderMode::Cef && !cef_installed {
        anyhow::bail!(
            "render=cef requested but the CEF renderer is not installed. \
             Run `wick install cef` first."
        );
    }
    let cef_first = render == RenderMode::Cef
        || (render == RenderMode::Auto
            && should_use_cef_first(cached.as_ref().map(|s| s.strategy.as_str()), cef_installed));

    if cef_first {
        match cef_render_with_retry(url, wait_for_selector).await {
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
                let timing_ms = start.elapsed().as_millis() as u64;
                analytics::report_failure(host, 0, "cef_failed");
                // Forced CEF: surface the failure rather than silently
                // falling through to a transport the caller said not to use.
                if render == RenderMode::Cef {
                    site_cache::record(host, "cef_timeout", false, timing_ms);
                    return Err(anyhow::anyhow!("CEF renderer failed: {}", e));
                }
                tracing::warn!("CEF renderer failed: {}. Falling back to Cronet.", e);
                // Mark this host as cef_timeout so the next fetch won't pay
                // the CEF-first cost again until cef succeeds and overwrites it.
                site_cache::record(host, "cef_timeout", false, timing_ms);
                // Fall through to Cronet.
            }
        }
    }

    // Cronet path (either by choice or as fallback from CEF).
    let escalated_from = if cef_first { Some("cef") } else { None };
    let resp = match client.get(url).await {
        Ok(r) => r,
        Err(e) => {
            let timing_ms = start.elapsed().as_millis() as u64;
            analytics::report_fetch(FetchEvent {
                host, strategy: "cronet-transport-error", escalated_from,
                ok: false, status: 0, timing_ms,
            });
            return Err(e);
        }
    };
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
                        let retry = match client.get(url).await {
                            Ok(r) => r,
                            Err(e) => {
                                let timing_ms = start.elapsed().as_millis() as u64;
                                analytics::report_fetch(FetchEvent {
                                    host, strategy: "captcha-auto",
                                    escalated_from: Some("cronet"),
                                    ok: false, status: 0, timing_ms,
                                });
                                return Err(e);
                            }
                        };
                        if retry.status < 400 {
                            let extracted = extract::extract(&retry.body, &parsed, format)?;
                            let timing_ms = start.elapsed().as_millis() as u64;
                            // Underlying transport is still Cronet — the
                            // CAPTCHA might be gone or the cookies might
                            // persist on the next visit. Telemetry tags
                            // it as captcha-auto for our analysis.
                            site_cache::record(host, "cronet", false, timing_ms);
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
                    let retry = match client.get(url).await {
                        Ok(r) => r,
                        Err(e) => {
                            let timing_ms = start.elapsed().as_millis() as u64;
                            analytics::report_fetch(FetchEvent {
                                host, strategy: "captcha-interactive",
                                escalated_from: Some("cronet"),
                                ok: false, status: 0, timing_ms,
                            });
                            return Err(e);
                        }
                    };
                    let timing_ms = start.elapsed().as_millis() as u64;
                    if retry.status < 400 {
                        let extracted = extract::extract(&retry.body, &parsed, format)?;
                        // Cache the underlying transport, not the CAPTCHA flow.
                        site_cache::record(host, "cronet", false, timing_ms);
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

        // CAPTCHA with no solver available. The challenge was hit on the
        // Cronet transport, so we tag escalated_from accordingly to match
        // the captcha-auto / captcha-interactive branches above (the
        // `captcha-*` strategy is an enrichment of the cronet attempt, not
        // a transport switch).
        //
        // If CEF is available and the caller hasn't pinned us to Cronet,
        // don't terminate here — JS-based challenges (Cloudflare "Just a
        // moment", DataDome, AWS WAF) clear under a real browser. Fall
        // through to the 403/503 CEF escalation block below. Only emit
        // captcha-blocked telemetry / response when there's no recovery
        // path left.
        if !cef_installed || render == RenderMode::Cronet {
            let timing_ms = start.elapsed().as_millis() as u64;
            analytics::report_fetch(FetchEvent {
                host, strategy: "captcha-blocked", escalated_from: Some("cronet"),
                ok: false, status, timing_ms,
            });
            return Ok(FetchResult {
                content: "This page returned a CAPTCHA or browser challenge. \
                          The content could not be extracted automatically.\n\
                          Install wick-captcha to solve CAPTCHAs interactively, \
                          or `wick install cef` for JS-rendered challenges."
                    .to_string(),
                title: None, url: url.to_string(), status_code: status, timing_ms,
            });
        }
        // else: fall through to the 403/503 CEF escalation block below.
    }

    // Cronet blocked but CEF might work — escalate if available, we haven't
    // tried yet, and the caller didn't pin us to Cronet.
    if (status == 403 || status == 503)
        && cef_installed
        && escalated_from.is_none()
        && render != RenderMode::Cronet
    {
        match cef_render_with_retry(url, wait_for_selector).await {
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

    // JS-required shell: HTTP 200 but the body is just an interstitial
    // saying "you need JavaScript" (X/Twitter, CRA, etc). Cronet can't run
    // JS, so escalate to CEF if available and the caller didn't pin us to
    // Cronet. This is the second auto-escalation trigger alongside 403/503.
    if cef_installed
        && escalated_from.is_none()
        && render != RenderMode::Cronet
        && is_js_required_shell(&body)
    {
        match cef_render_with_retry(url, wait_for_selector).await {
            Ok(html) => {
                let extracted = extract::extract(&html, &parsed, format)?;
                let content = append_media(&extracted.content, &html, &parsed);
                let timing_ms = start.elapsed().as_millis() as u64;
                site_cache::record(host, "cef", false, timing_ms);
                analytics::report_fetch(FetchEvent {
                    host, strategy: "cef-after-js-shell",
                    escalated_from: Some("cronet"),
                    ok: true, status: 200, timing_ms,
                });
                return Ok(FetchResult {
                    content, title: extracted.title,
                    url: url.to_string(), status_code: 200, timing_ms,
                });
            }
            Err(e) => {
                tracing::warn!("CEF escalation for JS-shell failed: {}", e);
                analytics::report_failure(host, 0, "cef_failed");
                // Fall through and return the shell — at least the caller
                // sees something rather than an opaque error.
            }
        }
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

/// Render via CEF, retrying once if the first attempt fails or comes back
/// implausibly small. Most "main frame load error -3" aborts from
/// anti-bot sites (X, Discord, Cloudflare) are transient — a single retry
/// closes a noticeable chunk of the failure rate without unbounded cost.
async fn cef_render_with_retry(
    url: &str,
    wait_for_selector: Option<&str>,
) -> Result<String> {
    let opts = crate::cef::RenderOptions {
        use_residential: false,
        wait_for_selector: wait_for_selector.map(|s| s.to_string()),
    };
    match crate::cef::render_with(url, opts.clone()).await {
        Ok(html) if is_acceptable_render(&html) => Ok(html),
        first => {
            tracing::debug!(
                "cef render for {} failed first attempt ({}); retrying once",
                url,
                match &first {
                    Ok(html) if looks_like_cloudflare_interstitial(html) =>
                        "cloudflare interstitial captured before redirect".to_string(),
                    Ok(html) => format!("body too small: {} bytes", html.len()),
                    Err(e) => e.to_string(),
                }
            );
            crate::cef::render_with(url, opts).await
        }
    }
}

/// A CEF render is acceptable when it has plausible bulk AND isn't a
/// challenge interstitial captured before its post-verification redirect.
fn is_acceptable_render(html: &str) -> bool {
    looks_like_real_render(html) && !looks_like_cloudflare_interstitial(html)
}

/// Sanity check on a CEF render result. A successful page — even an empty
/// one — should produce at least a few KB of HTML (React/Vue shell +
/// inline scripts). Anything smaller is almost certainly a load abort
/// that returned a near-empty `<html><head></head><body></body></html>`.
fn looks_like_real_render(html: &str) -> bool {
    html.len() >= 2000
}

/// True when the renderer dumped Cloudflare's "Just a moment" challenge
/// page itself, rather than the post-verification destination. Hitting
/// this means the renderer's stability poller fired during the
/// (effectively static) interstitial before Cloudflare's JS redirected
/// us to the real content. Triggers a one-shot retry — by then the
/// persistent daemon has cleared cookies and the second render usually
/// lands on real content.
fn looks_like_cloudflare_interstitial(html: &str) -> bool {
    let lower = html.to_lowercase();
    lower.contains("just a moment")
        && (lower.contains("performing security verification")
            || lower.contains("checking your browser")
            || lower.contains("challenge-platform"))
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

    // Mirror fetch()'s Reddit rewrite so `wick crawl` doesn't land on
    // new-reddit's challenge wall when fetch() would have taken the
    // old-reddit happy path.
    if host == "www.reddit.com" || host == "reddit.com" {
        let old_url = url
            .replace("://www.reddit.com", "://old.reddit.com")
            .replace("://reddit.com", "://old.reddit.com");
        return Box::pin(fetch_html(client, &old_url, respect_robots)).await;
    }

    if respect_robots && !robots::check(client, url).await {
        return Ok(FetchHtmlResult {
            html: String::new(), url: url.to_string(), status_code: 0,
        });
    }

    let cef_installed = crate::cef::is_available();
    let cached = site_cache::get(host);
    let cef_first = should_use_cef_first(cached.as_ref().map(|s| s.strategy.as_str()), cef_installed);

    if cef_first {
        match cef_render_with_retry(url, None).await {
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
                let timing_ms = start.elapsed().as_millis() as u64;
                tracing::warn!("CEF renderer failed: {}. Falling back to Cronet.", e);
                site_cache::record(host, "cef_timeout", false, timing_ms);
            }
        }
    }

    let escalated_from = if cef_first { Some("cef") } else { None };
    let resp = match client.get(url).await {
        Ok(r) => r,
        Err(e) => {
            let timing_ms = start.elapsed().as_millis() as u64;
            analytics::report_fetch(FetchEvent {
                host, strategy: "cronet-transport-error", escalated_from,
                ok: false, status: 0, timing_ms,
            });
            return Err(e);
        }
    };
    let timing_ms = start.elapsed().as_millis() as u64;
    let blocked = matches!(resp.status, 403 | 503);

    // Cronet got blocked but CEF is available — escalate, mirroring fetch().
    // Without this, crawl/map silently fail on JS-heavy or stealth-required
    // sites the first time they're encountered (no cache entry yet).
    if blocked && cef_installed && escalated_from.is_none() {
        analytics::report_fetch(FetchEvent {
            host, strategy: "cronet-blocked", escalated_from: None,
            ok: false, status: resp.status, timing_ms,
        });
        match cef_render_with_retry(url, None).await {
            Ok(html) => {
                let timing_ms = start.elapsed().as_millis() as u64;
                site_cache::record(host, "cef", false, timing_ms);
                analytics::report_fetch(FetchEvent {
                    host, strategy: "cef-after-cronet",
                    escalated_from: Some("cronet"),
                    ok: true, status: 200, timing_ms,
                });
                return Ok(FetchHtmlResult {
                    html, url: url.to_string(), status_code: 200,
                });
            }
            Err(e) => {
                tracing::warn!(
                    "fetch_html {} returned HTTP {} and CEF fallback failed: {}",
                    host, resp.status, e,
                );
                // Fall through to return the original blocked response.
            }
        }
    }

    let ok = !blocked && resp.status < 400;
    if ok {
        site_cache::record(host, "cronet", false, timing_ms);
    }
    analytics::report_fetch(FetchEvent {
        host,
        strategy: if blocked { "cronet-blocked" } else { "cronet" },
        escalated_from,
        ok,
        status: resp.status,
        timing_ms,
    });

    if !ok {
        tracing::debug!("fetch_html {} returned HTTP {}", host, resp.status);
    }

    Ok(FetchHtmlResult {
        html: resp.body, url: url.to_string(), status_code: resp.status,
    })
}

/// Fetch raw response bytes. Cronet only — no CEF, no CAPTCHA detection,
/// no extraction. Used by `--format raw` for binary content (PDFs,
/// archives, images, anything that isn't text).
///
/// CEF doesn't fit this path: its renderer returns rendered HTML for any
/// URL — including PDFs (it'd return Chromium's PDF-viewer chrome, not the
/// PDF bytes). And CAPTCHA detection is string-based, so it wouldn't
/// fire on binary content anyway. Keeping this lean is the right call.
///
/// Telemetry tags the strategy as `cronet-raw` so the public stats page
/// can distinguish raw-byte fetches from text fetches.
pub async fn fetch_raw(
    client: &Client,
    url: &str,
    respect_robots: bool,
) -> Result<FetchRawResult> {
    let start = Instant::now();
    let parsed = url::Url::parse(url)
        .map_err(|e| anyhow::anyhow!("invalid URL: {}", e))?;

    match parsed.scheme() {
        "http" | "https" => {}
        s => anyhow::bail!("unsupported scheme {:?} (only http and https)", s),
    }

    let host = parsed.host_str().ok_or_else(|| anyhow::anyhow!("missing host"))?;

    if respect_robots && !robots::check(client, url).await {
        // robots.txt block on a raw fetch — return empty bytes with a
        // sentinel status so the caller can detect it without us
        // having to invent a bytes-shaped error type.
        return Ok(FetchRawResult {
            bytes: Vec::new(),
            url: url.to_string(),
            status_code: 0,
            timing_ms: start.elapsed().as_millis() as u64,
        });
    }

    let resp = match client.get_bytes(url).await {
        Ok(r) => r,
        Err(e) => {
            let timing_ms = start.elapsed().as_millis() as u64;
            analytics::report_fetch(FetchEvent {
                host, strategy: "cronet-raw-transport-error", escalated_from: None,
                ok: false, status: 0, timing_ms,
            });
            return Err(e);
        }
    };

    let timing_ms = start.elapsed().as_millis() as u64;
    let ok = resp.status >= 200 && resp.status < 400;
    if ok {
        site_cache::record(host, "cronet", false, timing_ms);
    }
    analytics::report_fetch(FetchEvent {
        host, strategy: "cronet-raw", escalated_from: None,
        ok, status: resp.status, timing_ms,
    });

    Ok(FetchRawResult {
        bytes: resp.body,
        url: url.to_string(),
        status_code: resp.status,
        timing_ms,
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

/// Detect a "you need JavaScript to view this site" interstitial returned
/// on an HTTP 200. These look like a successful response but contain no
/// real content — just a noscript message and an empty React/Vue root.
/// Used to trigger CEF escalation for SPAs (X/Twitter, CRA defaults, etc).
///
/// Conservative: only matches exact phrases known to ship in JS-required
/// shells, to avoid false positives on real pages that happen to have a
/// noscript fallback. The 2MB cap exists so a multi-megabyte article that
/// happens to quote one of these phrases doesn't cost a CEF render — real
/// shells (even X.com's bundled-script response, which clocks in around
/// 270KB) are well under this.
fn is_js_required_shell(body: &str) -> bool {
    if body.len() > 2_000_000 {
        return false;
    }
    let lower = body.to_lowercase();
    const SIGNALS: &[&str] = &[
        "javascript is not available",                   // X / Twitter
        "we've detected that javascript is disabled",    // X / Twitter
        "we’ve detected that javascript is disabled",    // X / Twitter (curly apostrophe)
        "you need to enable javascript to run this app", // Create React App default
        "please enable javascript to continue",
        "enable javascript and cookies to continue",     // Cloudflare soft-wall
        "this site requires javascript",
        "enable javascript to see google maps",          // Google Maps
        "when you have eliminated the javascript",       // Google "Sherlock" variant — Maps and a few others
    ];
    SIGNALS.iter().any(|s| lower.contains(s))
}

/// Decide whether to try CEF first based on the cached strategy and CEF
/// availability. Pure function so the strategy-selection rule is easy
/// to unit-test without spinning up a fetch pipeline.
fn should_use_cef_first(cached_strategy: Option<&str>, cef_installed: bool) -> bool {
    cef_installed && matches!(cached_strategy, Some("cef"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cef_first_only_when_cache_says_cef_and_installed() {
        assert!(should_use_cef_first(Some("cef"), true));
    }

    #[test]
    fn cef_first_false_when_cef_not_installed() {
        assert!(!should_use_cef_first(Some("cef"), false));
    }

    #[test]
    fn cef_first_false_for_cronet_cache_entry() {
        assert!(!should_use_cef_first(Some("cronet"), true));
    }

    #[test]
    fn cef_first_false_for_no_cache_entry() {
        // Default is Cronet-first. CEF is only used when later escalation
        // logic selects it.
        assert!(!should_use_cef_first(None, true));
        assert!(!should_use_cef_first(None, false));
    }

    #[test]
    fn cef_first_false_for_unknown_strategy() {
        // Anything outside the documented set falls through to the default
        // (Cronet-first). Forward-compatible with future cache-value
        // changes — won't accidentally route everything through CEF.
        assert!(!should_use_cef_first(Some("captcha-auto"), true));
        assert!(!should_use_cef_first(Some("cef_timeout"), true));
        assert!(!should_use_cef_first(Some(""), true));
    }

    #[test]
    fn js_shell_detects_x_interstitial() {
        let body = r#"<style>body{}</style>
            <div class="errorContainer">
            <h1>JavaScript is not available.</h1>
            <p>We've detected that JavaScript is disabled in this browser.</p>
            </div>"#;
        assert!(is_js_required_shell(body));
    }

    #[test]
    fn js_shell_detects_create_react_app() {
        let body = r#"<noscript>You need to enable JavaScript to run this app.</noscript>
            <div id="root"></div>"#;
        assert!(is_js_required_shell(body));
    }

    #[test]
    fn js_shell_detects_google_maps() {
        let body = r#"<div id="XvQR9b"><div class="wSgKnf">
            <div>When you have eliminated the <strong>JavaScript</strong>,
            whatever remains must be an empty page.</div>
            <a href="..." target="_blank">Enable JavaScript to see Google Maps.</a>
            </div></div>"#;
        assert!(is_js_required_shell(body));
    }

    #[test]
    fn js_shell_no_false_positive_on_real_content() {
        let body = "<html><body><h1>Welcome</h1><p>Normal page content here.</p></body></html>";
        assert!(!is_js_required_shell(body));
    }

    #[test]
    fn js_shell_ignores_huge_bodies() {
        // A 3MB page that happens to quote the phrase shouldn't trip the
        // heuristic — real shells (even X's bundled-script response) come
        // in well under 2MB.
        let mut body = String::from("javascript is not available ");
        body.push_str(&"<p>real article content</p>".repeat(120_000));
        assert!(body.len() > 2_000_000);
        assert!(!is_js_required_shell(&body));
    }

    #[test]
    fn js_shell_detects_x_size_response() {
        // X.com's JS-shell response is ~270KB (bundled JS + interstitial
        // text). The old 200KB cap dropped this case, so guard against
        // regressing the cap below 300KB.
        let mut body = String::from("<h1>JavaScript is not available.</h1>");
        body.push_str(&"<script>/* bundle */</script>".repeat(10_000));
        assert!(body.len() > 250_000);
        assert!(is_js_required_shell(&body));
    }

    #[test]
    fn real_render_rejects_empty_aborts() {
        // A CEF "main frame load error -3" produces near-empty HTML —
        // anything under 2KB is treated as a transient abort worth retrying.
        assert!(!looks_like_real_render(""));
        assert!(!looks_like_real_render("<html><head></head><body></body></html>"));
        assert!(!looks_like_real_render(&"x".repeat(1500)));
    }

    #[test]
    fn real_render_accepts_normal_pages() {
        // Even minimal real pages clear 2KB once React/Vue shells and
        // inline scripts are included.
        assert!(looks_like_real_render(&"<p>content</p>".repeat(200)));
    }

    #[test]
    fn cloudflare_interstitial_detected() {
        let body = "<title>Just a moment...</title><body>\
            <h2>Performing security verification</h2>\
            <p>This website uses a security service to protect against malicious bots.</p>\
            </body>";
        assert!(looks_like_cloudflare_interstitial(body));
        assert!(!is_acceptable_render(body));
    }

    #[test]
    fn cloudflare_interstitial_no_false_positive_on_articles() {
        // A real article that happens to mention "just a moment" but
        // doesn't have the security-verification scaffolding shouldn't
        // trip the heuristic.
        let body = "<title>Wait Just a Moment - News Article</title>\
            <body><p>Real article content goes here for many paragraphs.</p></body>";
        assert!(!looks_like_cloudflare_interstitial(body));
    }

    #[test]
    fn render_mode_parses_synonyms() {
        assert_eq!(RenderMode::from_str("cef"), RenderMode::Cef);
        assert_eq!(RenderMode::from_str("CEF"), RenderMode::Cef);
        assert_eq!(RenderMode::from_str("browser"), RenderMode::Cef);
        assert_eq!(RenderMode::from_str("js"), RenderMode::Cef);
        assert_eq!(RenderMode::from_str("cronet"), RenderMode::Cronet);
        assert_eq!(RenderMode::from_str("no-js"), RenderMode::Cronet);
        assert_eq!(RenderMode::from_str("auto"), RenderMode::Auto);
        assert_eq!(RenderMode::from_str(""), RenderMode::Auto);
        assert_eq!(RenderMode::from_str("garbage"), RenderMode::Auto);
    }
}
