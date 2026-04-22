//! Lightweight usage analytics. No PII — just event counts and per-fetch
//! outcomes (hostname, strategy, ok/fail, status, timing). Posts to
//! releases.getwick.dev. Fire-and-forget, never blocks the caller, never
//! fails loudly.
//!
//! What IS collected:
//!   - Command events (`install`, `install_cef`, `fetch` dedup'd daily, etc.)
//!   - Per-fetch records: hostname, strategy, ok, status, timing_ms,
//!     wick version, OS.
//!
//! What is NOT collected:
//!   - URL paths or query strings, request headers, page content, titles
//!   - User identity, IP addresses (Analytics Engine sees the caller IP at
//!     ingest but doesn't store it as a data point), machine IDs.
//!
//! Opt out by setting `WICK_TELEMETRY=0` or creating `~/.wick/no-telemetry`.

use std::path::PathBuf;

const PING_URL: &str = "https://releases.getwick.dev/ping";
const EVENTS_URL: &str = "https://releases.getwick.dev/v1/events";

/// Structured per-fetch telemetry record. See module docs for what's in
/// and out of scope.
pub struct FetchEvent<'a> {
    pub host: &'a str,
    pub strategy: &'a str,
    pub escalated_from: Option<&'a str>,
    pub ok: bool,
    pub status: u16,
    pub timing_ms: u64,
}

/// Report a per-fetch outcome to releases.getwick.dev. Fire-and-forget.
pub fn report_fetch(ev: FetchEvent) {
    if is_opted_out() {
        return;
    }
    let host = ev.host.to_string();
    let strategy = ev.strategy.to_string();
    let escalated_from = ev.escalated_from.map(|s| s.to_string());
    let ok = ev.ok;
    let status = ev.status;
    let timing_ms = ev.timing_ms;
    let version = env!("CARGO_PKG_VERSION").to_string();
    let os = std::env::consts::OS.to_string();

    std::thread::spawn(move || {
        let escalated = match escalated_from {
            Some(s) => format!("\"{}\"", s),
            None => "null".to_string(),
        };
        let body = format!(
            r#"{{"host":"{}","strategy":"{}","escalated_from":{},"ok":{},"status":{},"timing_ms":{},"version":"{}","os":"{}"}}"#,
            host, strategy, escalated, ok, status, timing_ms, version, os
        );
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(3))
            .build()
            .ok();
        if let Some(c) = client {
            let _ = c.post(EVENTS_URL)
                .header("Content-Type", "application/json")
                .body(body)
                .send();
        }
    });
}

/// Report a fetch failure — legacy endpoint. Still useful for aggregate
/// error counts on the KV-backed dashboard. `report_fetch` supersedes it
/// for per-host/per-strategy analysis.
pub fn report_failure(domain: &str, status: u16, error_type: &str) {
    if is_opted_out() {
        return;
    }
    let domain = domain.to_string();
    let error_type = error_type.to_string();
    let version = env!("CARGO_PKG_VERSION").to_string();
    let os = std::env::consts::OS.to_string();
    let has_cef = crate::cef::is_available();

    std::thread::spawn(move || {
        let body = format!(
            r#"{{"event":"error","version":"{}","os":"{}","domain":"{}","status":{},"error":"{}","pro":{}}}"#,
            version, os, domain, status, error_type, has_cef
        );
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(3))
            .build()
            .ok();
        if let Some(c) = client {
            let _ = c.post(PING_URL)
                .header("Content-Type", "application/json")
                .body(body)
                .send();
        }
    });
}

/// Send a daily command-level ping (fire-and-forget).
pub fn ping(event: &str) {
    if is_opted_out() {
        return;
    }
    let event = event.to_string();
    let version = env!("CARGO_PKG_VERSION").to_string();
    let os = std::env::consts::OS.to_string();

    // Don't ping more than once per event per day.
    let marker = ping_marker(&event);
    if marker.exists() {
        return;
    }

    std::thread::spawn(move || {
        let _ = send_ping(&event, &version, &os);
        if let Some(dir) = marker.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&marker, "");
    });
}

/// Extract the registrable hostname from a URL. Falls back to the raw
/// host if parsing fails. Keeps subdomains (e.g. `docs.example.com`) —
/// we can PSL-normalize later if needed.
pub fn extract_host(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "unknown".to_string())
}

/// True if telemetry should be suppressed.
/// Checked via `WICK_TELEMETRY=0` env var or `~/.wick/no-telemetry` marker.
pub fn is_opted_out() -> bool {
    if let Ok(v) = std::env::var("WICK_TELEMETRY") {
        let v = v.trim();
        if v == "0" || v.eq_ignore_ascii_case("false") || v.eq_ignore_ascii_case("off") {
            return true;
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        if PathBuf::from(home).join(".wick").join("no-telemetry").exists() {
            return true;
        }
    }
    false
}

fn ping_marker(event: &str) -> PathBuf {
    let home = std::env::var_os("HOME").unwrap_or_else(|| "/tmp".into());
    let date = epoch_day();
    PathBuf::from(home)
        .join(".wick")
        .join("pings")
        .join(format!("{}-{}", date, event))
}

fn epoch_day() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{}", secs / 86400)
}

fn send_ping(event: &str, version: &str, os: &str) -> Result<(), Box<dyn std::error::Error>> {
    let body = format!(
        r#"{{"event":"{}","version":"{}","os":"{}"}}"#,
        event, version, os
    );

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()?;

    client.post(PING_URL)
        .header("Content-Type", "application/json")
        .body(body)
        .send()?;

    Ok(())
}
