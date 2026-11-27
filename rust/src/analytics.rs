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
//!     failed in the last few minutes (`retry`). The worker counts retries
//!     and successful retries per host, as input for spotting tasks agents
//!     gave up on; nothing reads those counts yet.
//!   - Explicit agent feedback sent through the `wick_request_capability`
//!     MCP tool: free text describing a task Wick couldn't do. It is scrubbed
//!     on this machine before sending (see `scrub`) and capped in length.
//!     Delivery is best effort, like everything here.
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
fn enqueue(url: &'static str, body: String) -> bool {
    worker_sender().is_some_and(|s| s.try_send(Job::PostJson(url, body)).is_ok())
}

/// Telemetry is skipped for opted-out users and for lookups Wick runs on
/// its own behalf (404 suggestions), which must not count as the agent's
/// fetches or touch its retry tracking.
fn should_report() -> bool {
    !is_opted_out() && current_tool() != SUGGEST_TOOL
}

/// Report a per-fetch outcome. Fire-and-forget.
pub fn report_fetch(ev: FetchEvent) {
    if !should_report() {
        return;
    }
    let retry = note_outcome(ev.host, ev.ok);
    enqueue(EVENTS_URL, fetch_payload(&ev, retry).to_string());
}

fn fetch_payload(ev: &FetchEvent, retry: bool) -> serde_json::Value {
    json!({
        "host": ev.host,
        "strategy": ev.strategy,
        "escalated_from": ev.escalated_from,
        "ok": ev.ok,
        "status": ev.status,
        "timing_ms": ev.timing_ms,
        "tool": current_tool(),
        "retry": retry,
        "version": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
    })
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
    if !should_report() {
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

/// Queue agent feedback for sending. Returns false when nothing was queued:
/// telemetry is opted out, or the send queue is full or unavailable.
/// Delivery is best effort, like all telemetry.
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
    enqueue(FEEDBACK_URL, payload.to_string())
}

/// Strip likely personal or secret data from free text before it leaves
/// the machine. Anything from a `scheme://` to the end of its
/// whitespace-separated token becomes `[url:host]`. The rest of each token
/// is split at structural characters (quotes, brackets, `:`, `=`, `,`...)
/// and each piece is checked for scheme-less `domain/path` forms, emails
/// and key-like tokens. Finally, runs of 7+ digits (phone or account
/// numbers, even when split by spaces or dashes) become `[number]`, and the
/// output is capped at `FEEDBACK_MAX_CHARS`. The worker applies the same
/// rules (`scrubText` in worker/src/index.js); both are checked against
/// worker/test/scrub-cases.json.
pub fn scrub(text: &str) -> String {
    let words: Vec<String> = text.split_whitespace().map(scrub_word).collect();
    mask_digit_runs(&words.join(" "))
        .chars()
        .take(FEEDBACK_MAX_CHARS)
        .collect()
}

/// Characters that split a token into separately checked pieces.
const SEPARATORS: &str = "\"'`()[]{}<>,;:=|";

fn scrub_word(word: &str) -> String {
    let Some(i) = word.find("://") else {
        return scrub_pieces(word);
    };
    let mut start = i;
    for (idx, c) in word[..i].char_indices().rev() {
        if c.is_ascii_alphanumeric() || "+.-".contains(c) {
            start = idx;
        } else {
            break;
        }
    }
    let after = &word[i + 3..];
    let authority_end = after
        .find(|c: char| "/?#".contains(c) || (c != ':' && SEPARATORS.contains(c)))
        .unwrap_or(after.len());
    let authority = &after[..authority_end];
    let host = authority.rsplit('@').next().unwrap_or("");
    let host = host.split(':').next().unwrap_or("");
    let tag = if !host.is_empty()
        && host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    {
        format!("[url:{}]", host)
    } else {
        "[url]".to_string()
    };
    format!("{}{}", scrub_pieces(&word[..start]), tag)
}

fn scrub_pieces(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut piece = String::new();
    for c in s.chars() {
        if SEPARATORS.contains(c) {
            out.push_str(&scrub_piece(&piece));
            piece.clear();
            out.push(c);
        } else {
            piece.push(c);
        }
    }
    out.push_str(&scrub_piece(&piece));
    out
}

fn scrub_piece(piece: &str) -> String {
    let trimmed = piece.trim_matches(|c: char| ".!?".contains(c));
    if let Some(host) = bare_domain_with_path(trimmed) {
        return format!("[url:{}]", host);
    }
    if trimmed.contains('@') && trimmed.contains('.') {
        return "[email]".to_string();
    }
    if looks_like_secret(trimmed) {
        return "[redacted]".to_string();
    }
    piece.to_string()
}

/// `example.com/private?x=1` → `Some("example.com")`.
fn bare_domain_with_path(s: &str) -> Option<&str> {
    let (host, path) = s.split_once('/')?;
    let tld = host.rsplit('.').next()?;
    let valid = host.contains('.')
        && !path.is_empty()
        && host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        && tld.len() >= 2
        && tld.chars().all(|c| c.is_ascii_alphabetic());
    valid.then_some(host)
}

/// Replace any run of digits and separators (space, `-`, `(`, `)`, `.`,
/// `+`) that holds 7 or more digits with `[number]`.
fn mask_digit_runs(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let starts_run = chars[i].is_ascii_digit()
            || (chars[i] == '+' && chars.get(i + 1).is_some_and(|c| c.is_ascii_digit()));
        if !starts_run {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let (mut j, mut digits, mut last_digit) = (i, 0, i);
        while j < chars.len() && (chars[j].is_ascii_digit() || " -().+".contains(chars[j])) {
            if chars[j].is_ascii_digit() {
                digits += 1;
                last_digit = j;
            }
            j += 1;
        }
        if digits >= 7 {
            out.push_str("[number]");
        } else {
            out.extend(&chars[i..=last_digit]);
        }
        i = last_digit + 1;
    }
    out
}

fn looks_like_secret(s: &str) -> bool {
    if s.len() < 24 {
        return false;
    }
    let charset = s.chars().all(|c| c.is_ascii_alphanumeric() || "-_+/.%".contains(c));
    let has_digit = s.chars().any(|c| c.is_ascii_digit());
    let has_alpha = s.chars().any(|c| c.is_ascii_alphabetic());
    charset && has_digit && has_alpha
}

/// Report a fetch failure — legacy endpoint. Still useful for aggregate
/// error counts on the KV-backed dashboard. `report_fetch` supersedes it
/// for per-host/per-strategy analysis.
pub fn report_failure(domain: &str, status: u16, error_type: &str) {
    if !should_report() {
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

    #[derive(serde::Deserialize)]
    struct ScrubCase {
        #[serde(rename = "in")]
        input: String,
        contains: Vec<String>,
        absent: Vec<String>,
    }

    #[test]
    fn scrub_matches_shared_cases() {
        let cases: Vec<ScrubCase> =
            serde_json::from_str(include_str!("../../worker/test/scrub-cases.json")).unwrap();
        for c in cases {
            let got = scrub(&c.input);
            for want in &c.contains {
                assert!(got.contains(want.as_str()), "{:?} -> {:?} lacks {:?}", c.input, got, want);
            }
            for bad in &c.absent {
                assert!(!got.contains(bad.as_str()), "{:?} -> {:?} keeps {:?}", c.input, got, bad);
            }
        }
    }

    #[test]
    fn scrub_caps_length() {
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
    async fn payload_carries_tool_and_retry() {
        let ev = FetchEvent {
            host: "h.example",
            strategy: "cronet",
            escalated_from: None,
            ok: true,
            status: 200,
            timing_ms: 5,
        };
        let p = with_tool("wick_crawl", async { fetch_payload(&ev, true) }).await;
        assert_eq!(p["tool"], "wick_crawl");
        assert_eq!(p["retry"], true);
        assert_eq!(fetch_payload(&ev, false)["retry"], false);
    }

    #[tokio::test]
    async fn suggestion_lookups_are_not_reported() {
        let in_suggest = with_tool(SUGGEST_TOOL, async { should_report() }).await;
        assert!(!in_suggest);
        let in_fetch = with_tool("wick_fetch", async { should_report() }).await;
        assert_eq!(in_fetch, !is_opted_out());
    }
}
