//! Auto CAPTCHA solving via CapSolver API (Wick Pro).
//! Detects CAPTCHA type from HTML, extracts site key, submits to CapSolver,
//! returns the solution token for injection into the page.

use anyhow::{bail, Result};
use serde::Deserialize;
use std::time::Duration;

const SOLVE_PROXY: &str = "https://releases.getwick.dev/solve";
const POLL_INTERVAL: Duration = Duration::from_secs(3);
const MAX_POLLS: usize = 40; // 40 × 3s = 2 minutes max

/// CAPTCHA type detected from page HTML.
#[derive(Debug)]
pub enum CaptchaType {
    Turnstile { site_key: String },
    ReCaptchaV2 { site_key: String },
    HCaptcha { site_key: String },
    DataDome { captcha_url: String },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TaskResponse {
    error_id: u32,
    #[serde(default)]
    error_code: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    task_id: Option<String>,
    #[serde(default)]
    solution: Option<serde_json::Value>,
}

/// Detect CAPTCHA type and extract site key from HTML.
pub fn detect_captcha(html: &str) -> Option<CaptchaType> {
    let lower = html.to_lowercase();

    // Cloudflare Turnstile
    if lower.contains("challenges.cloudflare.com/turnstile")
        || lower.contains("cf-turnstile")
    {
        if let Some(key) = extract_site_key(html, "cf-turnstile") {
            return Some(CaptchaType::Turnstile { site_key: key });
        }
    }

    // reCAPTCHA
    if lower.contains("google.com/recaptcha") || lower.contains("g-recaptcha") {
        if let Some(key) = extract_site_key(html, "g-recaptcha") {
            return Some(CaptchaType::ReCaptchaV2 { site_key: key });
        }
    }

    // hCaptcha
    if lower.contains("hcaptcha.com") || lower.contains("h-captcha") {
        if let Some(key) = extract_site_key(html, "h-captcha") {
            return Some(CaptchaType::HCaptcha { site_key: key });
        }
    }

    // DataDome
    if lower.contains("captcha-delivery.com") || lower.contains("geo.captcha-delivery.com") {
        if let Some(url) = extract_datadome_url(html) {
            return Some(CaptchaType::DataDome { captcha_url: url });
        }
    }

    None
}

/// Extract data-sitekey from a CAPTCHA widget div.
fn extract_site_key(html: &str, class_hint: &str) -> Option<String> {
    // Look for data-sitekey="..."
    // Find it near the class hint
    let hint_pos = html.to_lowercase().find(&class_hint.to_lowercase())?;
    let region = &html[hint_pos.saturating_sub(500)..html.len().min(hint_pos + 2000)];

    if let Some(pos) = region.find("data-sitekey=\"") {
        let start = pos + 14; // len of 'data-sitekey="'
        if let Some(end) = region[start..].find('"') {
            let key = &region[start..start + end];
            if !key.is_empty() {
                return Some(key.to_string());
            }
        }
    }

    // Also try turnstile.render({sitekey: '...'})
    if let Some(pos) = region.find("sitekey:") {
        let after = &region[pos + 8..];
        let after = after.trim_start();
        let quote = if after.starts_with('"') { '"' } else { '\'' };
        if after.starts_with(quote) {
            if let Some(end) = after[1..].find(quote) {
                return Some(after[1..1 + end].to_string());
            }
        }
    }

    None
}

/// Solve a CAPTCHA via the Wick Pro proxy (which forwards to CapSolver).
/// The `wick_key` is the customer's API key for authentication.
/// The CapSolver API key is stored server-side in the Worker, never exposed.
pub async fn solve(
    wick_key: &str,
    page_url: &str,
    captcha: &CaptchaType,
) -> Result<String> {
    let client = reqwest::Client::new();
    let proxy_url = format!("{}/{}", SOLVE_PROXY, wick_key);

    let task = match captcha {
        CaptchaType::Turnstile { site_key } => serde_json::json!({
            "type": "AntiTurnstileTaskProxyLess",
            "websiteURL": page_url,
            "websiteKey": site_key
        }),
        CaptchaType::ReCaptchaV2 { site_key } => serde_json::json!({
            "type": "ReCaptchaV2TaskProxyLess",
            "websiteURL": page_url,
            "websiteKey": site_key
        }),
        CaptchaType::HCaptcha { site_key } => serde_json::json!({
            "type": "HCaptchaTaskProxyLess",
            "websiteURL": page_url,
            "websiteKey": site_key
        }),
        CaptchaType::DataDome { captcha_url } => {
            // DataDome requires: Windows Chrome UA, proxy, and t=fe in captchaUrl
            // Check if IP is banned (t=bv means banned, solving will fail)
            if captcha_url.contains("t=bv") {
                bail!("DataDome: IP is banned (t=bv). Residential IP required.");
            }
            let proxy = std::env::var("WICK_PROXY")
                .unwrap_or_default();
            let mut task = serde_json::json!({
                "type": "DatadomeSliderTask",
                "websiteURL": page_url,
                "captchaUrl": captcha_url,
                "userAgent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36"
            });
            if !proxy.is_empty() {
                // Convert socks5://host:port to host:port format
                let proxy_clean = proxy
                    .trim_start_matches("socks5://")
                    .trim_start_matches("socks5h://")
                    .trim_start_matches("http://")
                    .trim_start_matches("https://");
                task["proxy"] = serde_json::Value::String(proxy_clean.to_string());
            }
            task
        },
    };

    // Create task via proxy
    let resp: TaskResponse = client
        .post(&proxy_url)
        .json(&serde_json::json!({
            "action": "createTask",
            "task": task
        }))
        .send()
        .await?
        .json()
        .await?;

    if resp.error_id != 0 {
        bail!(
            "CAPTCHA createTask error: {}",
            resp.error_code.unwrap_or_else(|| "unknown".to_string())
        );
    }

    // Sometimes solution comes back immediately
    if resp.status.as_deref() == Some("ready") {
        if let Some(token) = extract_token(&resp.solution, captcha) {
            return Ok(token);
        }
    }

    let task_id = resp
        .task_id
        .ok_or_else(|| anyhow::anyhow!("no task_id in response"))?;

    // Poll for result via proxy
    for _ in 0..MAX_POLLS {
        tokio::time::sleep(POLL_INTERVAL).await;

        let result: TaskResponse = client
            .post(&proxy_url)
            .json(&serde_json::json!({
                "action": "getTaskResult",
                "taskId": task_id
            }))
            .send()
            .await?
            .json()
            .await?;

        if result.error_id != 0 {
            bail!(
                "CAPTCHA solve error: {}",
                result.error_code.unwrap_or_else(|| "unknown".to_string())
            );
        }

        match result.status.as_deref() {
            Some("ready") => {
                if let Some(token) = extract_token(&result.solution, captcha) {
                    return Ok(token);
                }
                bail!("CAPTCHA returned ready but no token");
            }
            Some("processing") => continue,
            other => bail!("unexpected status: {:?}", other),
        }
    }

    bail!("CAPTCHA solve timed out after {} polls", MAX_POLLS)
}

/// Extract the DataDome captcha iframe URL from HTML.
fn extract_datadome_url(html: &str) -> Option<String> {
    // Look for iframe src containing captcha-delivery.com
    let marker = "captcha-delivery.com";
    let pos = html.find(marker)?;
    // Walk backwards to find src="
    let before = &html[..pos];
    let src_start = before.rfind("src=\"")? + 5;
    let after = &html[src_start..];
    let src_end = after.find('"')?;
    let url = &after[..src_end];
    // Decode HTML entities
    Some(url.replace("&amp;", "&"))
}

fn extract_token(solution: &Option<serde_json::Value>, captcha: &CaptchaType) -> Option<String> {
    let sol = solution.as_ref()?;
    match captcha {
        CaptchaType::ReCaptchaV2 { .. } => {
            sol.get("gRecaptchaResponse")?.as_str().map(|s| s.to_string())
        }
        CaptchaType::Turnstile { .. } | CaptchaType::HCaptcha { .. } => {
            sol.get("token")?.as_str().map(|s| s.to_string())
        }
        CaptchaType::DataDome { .. } => {
            // DataDome returns a cookie value, not a token
            sol.get("cookie")?.as_str().map(|s| s.to_string())
        }
    }
}
