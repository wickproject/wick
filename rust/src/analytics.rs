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
//!   - User identity, IP addresses (the receiving Worker sees the caller IP
//!     at ingest, like any HTTP request, but doesn't persist it as a data
//!     point), machine IDs.
//!
//! Opt out by setting `WICK_TELEMETRY=0` or creating `<wick-home>/no-telemetry`.
//! `wick-home` is `$HOME/.wick` if `HOME` is set, otherwise `/tmp/.wick`.
//!
//! Implementation notes: a single background worker thread drains a bounded
//! channel of pending events. Reusing one `reqwest::blocking::Client` and
//! one thread keeps overhead bounded under high fetch concurrency
//! (e.g. `wick serve --api` with many in-flight requests).

use std::path::PathBuf;
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::OnceLock;
use std::time::Duration;

use serde_json::json;

const PING_URL: &str = "https://releases.getwick.dev/ping";
const EVENTS_URL: &str = "https://releases.getwick.dev/v1/events";
const HTTP_TIMEOUT: Duration = Duration::from_secs(3);
/// Bounded queue cap. If the worker can't keep up (e.g. network is slow
/// and `try_send` returns Full), we drop events on the floor — telemetry
/// must never apply backpressure to the user's fetches.
const QUEUE_CAP: usize = 512;

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

enum Job {
    PostJson(&'static str, String), // (url, JSON body)
}

fn worker_sender() -> &'static SyncSender<Job> {
    static SENDER: OnceLock<SyncSender<Job>> = OnceLock::new();
    SENDER.get_or_init(|| {
        let (tx, rx) = sync_channel::<Job>(QUEUE_CAP);
        std::thread::Builder::new()
            .name("wick-analytics".into())
            .spawn(move || {
                // One reused client for the lifetime of the worker.
                let client = reqwest::blocking::Client::builder()
                    .timeout(HTTP_TIMEOUT)
                    .build()
                    .ok();
                while let Ok(job) = rx.recv() {
                    match (&client, job) {
                        (Some(c), Job::PostJson(url, body)) => {
                            let _ = c
                                .post(url)
                                .header("Content-Type", "application/json")
                                .body(body)
                                .send();
                        }
                        (None, _) => { /* client failed to build; drop silently */ }
                    }
                }
            })
            .expect("spawn wick-analytics thread");
        tx
    })
}

/// Enqueue a JSON post. Returns immediately; on a full queue the event is
/// dropped (telemetry never applies backpressure).
fn enqueue(url: &'static str, body: String) {
    let _ = worker_sender().try_send(Job::PostJson(url, body));
}

/// Report a per-fetch outcome. Fire-and-forget.
pub fn report_fetch(ev: FetchEvent) {
    if is_opted_out() {
        return;
    }
    let payload = json!({
        "host": ev.host,
        "strategy": ev.strategy,
        "escalated_from": ev.escalated_from,
        "ok": ev.ok,
        "status": ev.status,
        "timing_ms": ev.timing_ms,
        "version": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
    });
    enqueue(EVENTS_URL, payload.to_string());
}

/// Report a fetch failure — legacy endpoint. Still useful for aggregate
/// error counts on the KV-backed dashboard. `report_fetch` supersedes it
/// for per-host/per-strategy analysis.
pub fn report_failure(domain: &str, status: u16, error_type: &str) {
    if is_opted_out() {
        return;
    }
    let payload = json!({
        "event": "error",
        "version": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
        "domain": domain,
        "status": status,
        "error": error_type,
        "pro": crate::cef::is_available(),
    });
    enqueue(PING_URL, payload.to_string());
}

/// Send a daily command-level ping.
pub fn ping(event: &str) {
    if is_opted_out() {
        return;
    }
    // Don't ping more than once per event per day.
    let marker = ping_marker(event);
    if marker.exists() {
        return;
    }
    let payload = json!({
        "event": event,
        "version": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
    });
    enqueue(PING_URL, payload.to_string());

    // Write the dedup marker after enqueueing — even if the actual POST
    // fails later, we don't want to spam the daily endpoint on retries.
    if let Some(dir) = marker.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&marker, "");
}

/// Extract the hostname from a URL. Returns `"unknown"` if parsing fails
/// or the URL has no host. Keeps subdomains (e.g. `docs.example.com`);
/// this does **not** perform PSL normalization (eTLD+1).
pub fn extract_host(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "unknown".to_string())
}

/// True if telemetry should be suppressed.
/// Checked via `WICK_TELEMETRY=0` env var or `<wick-home>/no-telemetry` marker.
pub fn is_opted_out() -> bool {
    if let Ok(v) = std::env::var("WICK_TELEMETRY") {
        let v = v.trim();
        if v == "0" || v.eq_ignore_ascii_case("false") || v.eq_ignore_ascii_case("off") {
            return true;
        }
    }
    wick_home().join("no-telemetry").exists()
}

/// Resolve `<HOME>/.wick`, falling back to `/tmp/.wick` if `HOME` is unset.
/// Used by all on-disk wick state (telemetry markers, opt-out flag, etc.)
/// so behavior is consistent across env configurations.
pub(crate) fn wick_home() -> PathBuf {
    let home = std::env::var_os("HOME").unwrap_or_else(|| "/tmp".into());
    PathBuf::from(home).join(".wick")
}

fn ping_marker(event: &str) -> PathBuf {
    wick_home().join("pings").join(format!("{}-{}", epoch_day(), event))
}

fn epoch_day() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{}", secs / 86400)
}
