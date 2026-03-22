use anyhow::{bail, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;
use tokio::process::Command;

const CAPTCHA_TIMEOUT: Duration = Duration::from_secs(120); // 2 minutes to solve

/// Cookies stored per domain after CAPTCHA solving.
static COOKIE_JAR: std::sync::LazyLock<Mutex<HashMap<String, Vec<Cookie>>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone, Deserialize)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
}

/// Get stored cookies for a domain (from previous CAPTCHA solves).
pub fn get_cookies(host: &str) -> Vec<Cookie> {
    let jar = COOKIE_JAR.lock().unwrap();
    // Check both exact match and parent domain
    for (domain, cookies) in jar.iter() {
        if host == domain || host.ends_with(domain) || domain.ends_with(host) {
            return cookies.clone();
        }
    }
    Vec::new()
}

/// Format stored cookies as a Cookie header value.
pub fn cookie_header(host: &str) -> Option<String> {
    let cookies = get_cookies(host);
    if cookies.is_empty() {
        return None;
    }
    Some(
        cookies
            .iter()
            .map(|c| format!("{}={}", c.name, c.value))
            .collect::<Vec<_>>()
            .join("; "),
    )
}

/// Launch the CAPTCHA solver UI and wait for the user to solve it.
/// Returns the cookies set after solving.
pub async fn solve(url: &str) -> Result<Vec<Cookie>> {
    let solver_path = find_solver()?;

    let result = tokio::time::timeout(CAPTCHA_TIMEOUT, async {
        let output = Command::new(&solver_path)
            .arg(url)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .output()
            .await
            .map_err(|e| anyhow::anyhow!("failed to start wick-captcha: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if stdout.is_empty() {
            bail!("CAPTCHA solver returned no cookies (user may have closed the window)");
        }

        let cookies: Vec<Cookie> = serde_json::from_str(&stdout)
            .map_err(|e| anyhow::anyhow!("failed to parse cookies from solver: {}", e))?;

        Ok(cookies)
    })
    .await
    .map_err(|_| anyhow::anyhow!("CAPTCHA solving timed out after 2 minutes"))?;

    let cookies = result?;

    // Store cookies for future requests
    if let Some(first) = cookies.first() {
        let domain = first.domain.trim_start_matches('.').to_string();
        COOKIE_JAR
            .lock()
            .unwrap()
            .insert(domain, cookies.clone());
    }

    Ok(cookies)
}

/// Check if the CAPTCHA solver binary is available.
pub fn is_available() -> bool {
    find_solver().is_ok()
}

fn find_solver() -> Result<PathBuf> {
    let locations = [
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("wick-captcha"))),
        std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join(".wick").join("bin").join("wick-captcha")),
    ];

    for loc in locations.iter().flatten() {
        if loc.exists() {
            return Ok(loc.clone());
        }
    }

    bail!("wick-captcha not found. Place it next to the wick binary.")
}
