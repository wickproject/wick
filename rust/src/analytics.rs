//! Lightweight usage analytics. No PII — just event counts and per-fetch
//! outcomes (hostname, strategy, ok/fail, status, timing). Posts to
//! releases.getwick.dev. Fire-and-forget, never blocks the caller, never
//! fails loudly.
//!
//! What IS collected:
//!   - Command events (`install`, `install_cef`, `fetch` dedup'd daily, etc.)
//!   - Per-fetch records: hostname, strategy, ok, status, timing_ms,
//!     wick version, OS.
//!   - Which surface issued the fetch (`tool`: an MCP tool name such as
//!     `wick_fetch`, or `cli` / `api`) and whether it retried a host that
//!     failed in the last few minutes (`retry`). A failure with no later
//!     successful retry is how the stats page spots abandoned tasks.
//!   - Explicit agent feedback sent through the `wick_request_capability`
//!     MCP tool: free text describing a task Wick couldn't do. It is scrubbed
//!     on this machine before sending (URLs cut to their host, emails, long
//!     numbers and key-like tokens replaced) and capped in length.
//!   - For transport failures (no HTTP response at all): a coarse `error_kind`
//!     cause — `offline` / `dns` / `timeout` / `reset` / `refused` /
//!     `unreachable` / `quic` / `connect` / `other`. Lets the stats page tell a
//!     user disconnect apart from a site actively blocking us. No error
//!     strings, paths, or IPs — just the bucket.
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

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::json;

const PING_URL: &str = "https://releases.getwick.dev/ping";
const EVENTS_URL: &str = "https://releases.getwick.dev/v1/events";
const FEEDBACK_URL: &str = "https://releases.getwick.dev/v1/feedback";
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

/// A fetch to a host that failed within this window counts as a retry.
const RETRY_WINDOW: Duration = Duration::from_secs(300);
/// Bound on hosts tracked for retry detection, so a long-running server
/// crawling many hosts can't grow the map without limit.
const RETRY_TRACK_CAP: usize = 256;

tokio::task_local! {
    /// The MCP tool (or other surface) driving the current fetch. Set with
    /// `with_tool`; events reported outside any scope fall back to the
    /// process mode from `set_mode`.
    static CURRENT_TOOL: &'static str;
}

static MODE: OnceLock<&'static str> = OnceLock::new();

/// Tool name for the URL lookups Wick runs after a 404.
pub const SUGGEST_TOOL: &str = "wick_fetch_404_suggest";

/// Record how this process was started (`mcp`, `api`, or `cli`). Used as
/// the `tool` value when no per-call scope is active, e.g. in tasks the
/// crawler spawns.
pub fn set_mode(mode: &'static str) {
    let _ = MODE.set(mode);
}

/// Run `fut` with `tool` attributed to every fetch event it reports.
pub async fn with_tool<F: std::future::Future>(tool: &'static str, fut: F) -> F::Output {
    CURRENT_TOOL.scope(tool, fut).await
}

fn current_tool() -> &'static str {
    CURRENT_TOOL
        .try_with(|t| *t)
        .unwrap_or_else(|_| MODE.get().copied().unwrap_or("cli"))
}

fn recent_failures() -> &'static Mutex<HashMap<String, Instant>> {
    static FAILURES: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    FAILURES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Returns whether this fetch retries a host that recently failed, and
/// updates the tracker with this outcome.
fn note_outcome(host: &str, ok: bool) -> bool {
    let Ok(mut map) = recent_failures().lock() else {
        return false;
    };
    let now = Instant::now();
    let retry = map
        .get(host)
        .is_some_and(|t| now.duration_since(*t) < RETRY_WINDOW);
    if ok {
        map.remove(host);
    } else {
        if map.len() >= RETRY_TRACK_CAP && !map.contains_key(host) {
            map.retain(|_, t| now.duration_since(*t) < RETRY_WINDOW);
            if map.len() >= RETRY_TRACK_CAP {
                return retry;
            }
        }
        map.insert(host.to_string(), now);
    }
    retry
}

enum Job {
    PostJson(&'static str, String), // (url, JSON body)
}

/// Returns the worker's sender, or `None` if the worker thread couldn't be
/// spawned (e.g. resource limits, unusual platforms). Telemetry must never
/// fail loudly, so a spawn error simply disables telemetry for the process.
fn worker_sender() -> Option<&'static SyncSender<Job>> {
    static SENDER: OnceLock<Option<SyncSender<Job>>> = OnceLock::new();
    SENDER
        .get_or_init(|| {
            let (tx, rx) = sync_channel::<Job>(QUEUE_CAP);
            let spawn_result = std::thread::Builder::new()
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
                });
            match spawn_result {
                Ok(_handle) => Some(tx),
                Err(_) => None,
            }
        })
        .as_ref()
}

/// Enqueue a JSON post. Returns immediately; on a full queue or if the
/// worker thread couldn't be spawned, the event is dropped silently
/// (telemetry never applies backpressure or fails loudly).
fn enqueue(url: &'static str, body: String) {
    if let Some(s) = worker_sender() {
        let _ = s.try_send(Job::PostJson(url, body));
    }
}

/// Report a per-fetch outcome. Fire-and-forget.
pub fn report_fetch(ev: FetchEvent) {
    if is_opted_out() {
        return;
    }
    let tool = current_tool();
    // Lookups Wick makes on its own behalf (404 suggestions) must not clear
    // or set the retry marker for the agent's own requests.
    let retry = tool != SUGGEST_TOOL && note_outcome(ev.host, ev.ok);
    let payload = json!({
        "host": ev.host,
        "strategy": ev.strategy,
        "escalated_from": ev.escalated_from,
        "ok": ev.ok,
        "status": ev.status,
        "timing_ms": ev.timing_ms,
        "tool": tool,
        "retry": retry,
        "version": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
    });
    enqueue(EVENTS_URL, payload.to_string());
}

/// Report a transport-level failure: the request never produced an HTTP
/// response at all (DNS failure, connect refused, TLS reset, timeout, or the
/// user's own network being down). Posts the same `/v1/events` shape as
/// `report_fetch` plus an `error_kind` cause.
///
/// `error_kind` is the load-bearing addition for the self-improvement loop:
/// it lets the stats page and the probing harness tell a site *actively
/// blocking us* (`reset`, `refused`, `timeout`-while-online) apart from the
/// *user's connection dropping* (`offline`). Without it, a user whose wifi
/// died 25 times reads identically to a genuinely hard site — and would
/// poison the curated rules with a phantom "this site is hard" signal.
pub fn report_transport_error(
    host: &str,
    strategy: &str,
    error_kind: &str,
    escalated_from: Option<&str>,
    timing_ms: u64,
) {
    if is_opted_out() {
        return;
    }
    let retry = note_outcome(host, false);
    let payload = json!({
        "host": host,
        "strategy": strategy,
        "escalated_from": escalated_from,
        "ok": false,
        "status": 0,
        "timing_ms": timing_ms,
        "error_kind": error_kind,
        "tool": current_tool(),
        "retry": retry,
        "version": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
    });
    enqueue(EVENTS_URL, payload.to_string());
}

/// What an agent couldn't get done, as sent by `wick_request_capability`.
pub struct Feedback<'a> {
    pub task: &'a str,
    pub tried: Option<&'a str>,
    pub failure: Option<&'a str>,
    pub url: Option<&'a str>,
}

/// Per-field cap on feedback text, in characters.
pub const FEEDBACK_MAX_CHARS: usize = 600;

/// Send agent feedback. Returns false when telemetry is opted out, in which
/// case nothing is sent.
pub fn report_feedback(fb: Feedback) -> bool {
    if is_opted_out() {
        return false;
    }
    let payload = json!({
        "kind": "capability_request",
        "task": scrub(fb.task),
        "tried": fb.tried.map(scrub),
        "failure": fb.failure.map(scrub),
        "host": fb.url.map(extract_host),
        "tool": current_tool(),
        "version": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
    });
    enqueue(FEEDBACK_URL, payload.to_string());
    true
}

/// Strip likely personal or secret data from free text before it leaves
/// the machine: URLs keep only their host, and emails, long digit runs and
/// long key-like tokens are replaced. Output is capped at
/// `FEEDBACK_MAX_CHARS`.
pub fn scrub(text: &str) -> String {
    let mut out = Vec::new();
    for word in text.split_whitespace() {
        let trimmed = word.trim_matches(|c: char| ",.;:!?()[]{}<>\"'`".contains(c));
        let replacement = if let Ok(u) = url::Url::parse(trimmed) {
            match u.host_str() {
                Some(h) if matches!(u.scheme(), "http" | "https") => Some(format!("[url:{}]", h)),
                _ => Some("[url]".to_string()),
            }
        } else if trimmed.contains('@') && trimmed.contains('.') {
            Some("[email]".to_string())
        } else if looks_like_secret(trimmed) {
            Some("[redacted]".to_string())
        } else if trimmed.chars().filter(|c| c.is_ascii_digit()).count() >= 7 {
            Some("[number]".to_string())
        } else {
            None
        };
        out.push(replacement.unwrap_or_else(|| word.to_string()));
    }
    out.join(" ").chars().take(FEEDBACK_MAX_CHARS).collect()
}

fn looks_like_secret(s: &str) -> bool {
    if s.len() < 24 {
        return false;
    }
    let alnum = s.chars().all(|c| c.is_ascii_alphanumeric() || "-_+/=".contains(c));
    let has_digit = s.chars().any(|c| c.is_ascii_digit());
    let has_alpha = s.chars().any(|c| c.is_ascii_alphabetic());
    alnum && has_digit && has_alpha
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_cuts_urls_to_host() {
        assert_eq!(
            scrub("tried https://example.com/user/123/orders?id=9 twice"),
            "tried [url:example.com] twice"
        );
    }

    #[test]
    fn scrub_removes_emails_numbers_and_keys() {
        let s = scrub("mail jane.doe@corp.com call +1 (555) 123-4567 key key_live_51Habc123DEF456ghi789JKL");
        assert!(!s.contains("jane"), "{}", s);
        assert!(!s.contains("key_live"), "{}", s);
        assert!(s.contains("[email]"), "{}", s);
        assert!(s.contains("[redacted]"), "{}", s);
    }

    #[test]
    fn scrub_keeps_plain_text_and_caps_length() {
        assert_eq!(scrub("compare prices in Germany"), "compare prices in Germany");
        assert_eq!(scrub(&"a ".repeat(1000)).chars().count(), FEEDBACK_MAX_CHARS);
    }

    #[test]
    fn retry_detection() {
        let host = "retry-test.example";
        assert!(!note_outcome(host, false));
        assert!(note_outcome(host, false));
        assert!(note_outcome(host, true));
        assert!(!note_outcome(host, true));
    }

    #[tokio::test]
    async fn tool_scope_attribution() {
        assert_eq!(with_tool("wick_fetch", async { current_tool() }).await, "wick_fetch");
        assert_ne!(current_tool(), "wick_fetch");
    }
}
