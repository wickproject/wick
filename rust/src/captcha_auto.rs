//! Automated CAPTCHA solving via CapSolver.
//!
//! Users bring their own CapSolver API key (set `CAPSOLVER_API_KEY` env var
//! or `~/.wick/capsolver-key`). Wick never proxies or subsidizes solves —
//! the user's key goes directly to CapSolver and they pay CapSolver directly.
//!
//! If no key is configured, Wick falls back to the user-in-the-loop
//! `captcha::solve` flow where the user clicks through the CAPTCHA manually.
//!
//! Supported: Cloudflare Turnstile, reCAPTCHA v2, hCaptcha, DataDome.

use anyhow::{bail, Result};
use serde::Deserialize;
use std::time::Duration;

const CAPSOLVER_API: &str = "https://api.capsolver.com";
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

/// Load the user's CapSolver API key from env var or config file.
/// Returns None if the user hasn't configured one.
pub fn load_capsolver_key() -> Option<String> {
    if let Ok(key) = std::env::var("CAPSOLVER_API_KEY") {
        if !key.is_empty() {
            return Some(key);
        }
    }
    let home = std::env::var_os("HOME")?;
    let path = std::path::PathBuf::from(home).join(".wick").join("capsolver-key");
    std::fs::read_to_string(&path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Whether auto-CAPTCHA is configured (user has a CapSolver key).
pub fn is_available() -> bool {
    load_capsolver_key().is_some()
}

/// Extract data-sitekey from a CAPTCHA widget div.
fn extract_site_key(html: &str, class_hint: &str) -> Option<String> {
    let hint_pos = html.to_lowercase().find(&class_hint.to_lowercase())?;
    let region = &html[hint_pos.saturating_sub(500)..html.len().min(hint_pos + 2000)];

    if let Some(pos) = region.find("data-sitekey=\"") {
        let start = pos + 14;
        if let Some(end) = region[start..].find('"') {
            let key = &region[start..start + end];
            if !key.is_empty() {
                return Some(key.to_string());
            }
        }
    }

    // turnstile.render({sitekey: '...'})
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

/// Solve a CAPTCHA via CapSolver using the user's own API key.
/// The user pays CapSolver directly — Wick never sees or proxies the key.
pub async fn solve(
    capsolver_key: &str,
    page_url: &str,
    captcha: &CaptchaType,
) -> Result<String> {
    let client = reqwest::Client::new();

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
            if captcha_url.contains("t=bv") {
                bail!("DataDome: IP is banned (t=bv). Residential IP required.");
            }
            let proxy = std::env::var("WICK_PROXY").unwrap_or_default();
            let mut task = serde_json::json!({
                "type": "DatadomeSliderTask",
                "websiteURL": page_url,
                "captchaUrl": captcha_url,
                "userAgent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36"
            });
            if !proxy.is_empty() {
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

    // Create task
    let resp: TaskResponse = client
        .post(format!("{}/createTask", CAPSOLVER_API))
        .json(&serde_json::json!({
            "clientKey": capsolver_key,
            "task": task
        }))
        .send()
        .await?
        .json()
        .await?;

    if resp.error_id != 0 {
        bail!(
            "CapSolver createTask error: {}",
            resp.error_code.unwrap_or_else(|| "unknown".to_string())
        );
    }

    if resp.status.as_deref() == Some("ready") {
        if let Some(token) = extract_token(&resp.solution, captcha) {
            return Ok(token);
        }
    }

    let task_id = resp
        .task_id
        .ok_or_else(|| anyhow::anyhow!("no task_id in response"))?;

    for _ in 0..MAX_POLLS {
        tokio::time::sleep(POLL_INTERVAL).await;

        let result: TaskResponse = client
            .post(format!("{}/getTaskResult", CAPSOLVER_API))
            .json(&serde_json::json!({
                "clientKey": capsolver_key,
                "taskId": task_id
            }))
            .send()
            .await?
            .json()
            .await?;

        if result.error_id != 0 {
            bail!(
                "CapSolver solve error: {}",
                result.error_code.unwrap_or_else(|| "unknown".to_string())
            );
        }

        match result.status.as_deref() {
            Some("ready") => {
                if let Some(token) = extract_token(&result.solution, captcha) {
                    return Ok(token);
                }
                bail!("CapSolver returned ready but no token");
            }
            Some("processing") => continue,
            other => bail!("unexpected status: {:?}", other),
        }
    }

    bail!("CapSolver solve timed out after {} polls", MAX_POLLS)
}

fn extract_datadome_url(html: &str) -> Option<String> {
    let marker = "captcha-delivery.com";
    let pos = html.find(marker)?;
    let before = &html[..pos];
    let src_start = before.rfind("src=\"")? + 5;
    let after = &html[src_start..];
    let src_end = after.find('"')?;
    let url = &after[..src_end];
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
            sol.get("cookie")?.as_str().map(|s| s.to_string())
        }
    }
}
