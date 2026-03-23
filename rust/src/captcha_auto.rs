//! Auto CAPTCHA solving via CapSolver API (Wick Pro).
//! Detects CAPTCHA type from HTML, extracts site key, submits to CapSolver,
//! returns the solution token for injection into the page.

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

    None
}

/// Extract data-sitekey from a CAPTCHA widget div.
fn extract_site_key(html: &str, class_hint: &str) -> Option<String> {
    // Look for data-sitekey="..."
    let search = format!("data-sitekey=\"");
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

/// Solve a CAPTCHA using the CapSolver API.
/// Returns the solution token.
pub async fn solve(
    api_key: &str,
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
    };

    // Create task
    let resp: TaskResponse = client
        .post(format!("{}/createTask", CAPSOLVER_API))
        .json(&serde_json::json!({
            "clientKey": api_key,
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

    // Sometimes solution comes back immediately
    if resp.status.as_deref() == Some("ready") {
        if let Some(token) = extract_token(&resp.solution, captcha) {
            return Ok(token);
        }
    }

    let task_id = resp
        .task_id
        .ok_or_else(|| anyhow::anyhow!("no task_id in response"))?;

    // Poll for result
    for _ in 0..MAX_POLLS {
        tokio::time::sleep(POLL_INTERVAL).await;

        let result: TaskResponse = client
            .post(format!("{}/getTaskResult", CAPSOLVER_API))
            .json(&serde_json::json!({
                "clientKey": api_key,
                "taskId": task_id
            }))
            .send()
            .await?
            .json()
            .await?;

        if result.error_id != 0 {
            bail!(
                "CapSolver error: {}",
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

fn extract_token(solution: &Option<serde_json::Value>, captcha: &CaptchaType) -> Option<String> {
    let sol = solution.as_ref()?;
    match captcha {
        CaptchaType::ReCaptchaV2 { .. } => {
            sol.get("gRecaptchaResponse")?.as_str().map(|s| s.to_string())
        }
        CaptchaType::Turnstile { .. } | CaptchaType::HCaptcha { .. } => {
            sol.get("token")?.as_str().map(|s| s.to_string())
        }
    }
}
