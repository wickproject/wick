//! Curated, evolving per-site behavior rules — the shared "known behaviors"
//! list the self-improvement loop maintains.
//!
//! This is **distinct from `site_cache`** (which is per-machine and learned
//! reactively from this machine's own fetches). `site_rules` is a *shared,
//! curated* list of known site behaviors — which transport works, whether
//! datacenter IPs are blocked, what selector to wait for. It lets a
//! brand-new client route correctly on its FIRST visit instead of re-paying
//! the Cronet-fail-then-escalate cost that `site_cache` only avoids after a
//! local miss.
//!
//! Precedence in `fetch.rs` (highest first):
//!   forced `RenderMode` → **curated rule (here)** → local `site_cache` →
//!   default (Cronet-first with on-block escalation).
//!
//! Sources, in increasing authority when both are present for a host:
//!   1. Bundled seed (`data/site-rules.json`, compiled in via `include_str!`)
//!      — always available, works offline / on first run.
//!   2. On-disk overlay (`<wick-home>/site-rules.json`), refreshed from
//!      `releases.getwick.dev/v1/site-rules` so the list "constantly evolves"
//!      without a reinstall. The overlay wins per-host where present.
//!
//! Only `render`, `needs_residential`, and `wait_for_selector` change fetch
//! behavior; `vendor` / `confidence` / `source` are advisory metadata for the
//! harness and `wick rules` introspection.

use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Bundled seed, embedded at compile time. The overlay (if any) is layered
/// on top at load.
const SEED: &str = include_str!("../data/site-rules.json");

#[derive(Debug, Clone, Deserialize)]
pub struct SiteRule {
    /// What transport is known to work: `"cef"` or `"cronet"`. Defaulted so a
    /// single overlay entry missing `render` (manual edit, a residential-only
    /// rule, a partial doc) can't fail the whole-file parse and silently
    /// disable every overlay rule — an empty/unknown value is "no opinion" to
    /// the consumer (`fetch.rs::should_use_cef_first`), the same as no rule.
    #[serde(default)]
    pub render: String,
    /// Datacenter IPs are blocked here — a residential exit is needed for a
    /// reliable fetch. Advisory for clients without a residential transport
    /// (most local users); acted on by server/Pro deployments and the CEF
    /// residential tunnel when it's available (see `cef::ensure_daemon`,
    /// which no-ops the request if no tunnel is present).
    #[serde(default)]
    pub needs_residential: bool,
    /// Optional CSS selector to wait for before dumping the DOM (SPAs that
    /// hydrate content after first paint). Only meaningful for CEF renders.
    #[serde(default)]
    pub wait_for_selector: Option<String>,
    /// Anti-bot vendor, advisory only (e.g. `"datadome"`, `"cloudflare"`).
    #[serde(default)]
    pub vendor: Option<String>,
    /// 0..1 confidence. Hand-vetted seeds sit ~0.5–0.9; the harness writes
    /// measured confidence. Advisory in PR1 (every present rule is applied);
    /// a future threshold can gate on it.
    #[serde(default)]
    pub confidence: f32,
    /// Provenance: `"seed"` | `"measured"` | `"curated"`. Lets the harness
    /// tell its own measurements apart from hand-seeds it may overwrite.
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RuleFile {
    #[serde(default)]
    rules: HashMap<String, SiteRule>,
}

static RULES: std::sync::LazyLock<HashMap<String, SiteRule>> =
    std::sync::LazyLock::new(load_rules);

fn load_rules() -> HashMap<String, SiteRule> {
    // 1. Bundled seed (always present; a parse failure here is a build-time
    //    bug in the embedded JSON, so fall back to empty rather than panic
    //    a user's fetch).
    let mut map = parse(SEED);
    // 2. On-disk overlay wins per-host where present. Missing/corrupt
    //    overlay just leaves the seed in place.
    if let Some(overlay) = read_overlay() {
        for (host, rule) in parse(&overlay) {
            map.insert(host, rule);
        }
    }
    map
}

fn parse(data: &str) -> HashMap<String, SiteRule> {
    serde_json::from_str::<RuleFile>(data)
        .map(|f| f.rules)
        .unwrap_or_default()
}

fn overlay_path() -> PathBuf {
    crate::analytics::wick_home().join("site-rules.json")
}

fn read_overlay() -> Option<String> {
    std::fs::read_to_string(overlay_path()).ok()
}

/// Where clients pull the evolving curated rules (served by the Worker; see
/// `worker/src/index.js` `GET /v1/site-rules`).
const RULES_URL: &str = "https://releases.getwick.dev/v1/site-rules";
/// Refresh the on-disk overlay at most once a day.
const REFRESH_INTERVAL_SECS: u64 = 24 * 3600;

static REFRESH_ONCE: std::sync::Once = std::sync::Once::new();

/// Best-effort daily refresh of the on-disk overlay from the Worker — this is
/// what lets the curated list "constantly evolve" without a reinstall.
///
/// Fire-and-forget: spawns a background thread (at most once per process) and
/// never blocks or fails the caller. The refreshed file is consumed on the
/// NEXT process start — `RULES` is a process-lifetime snapshot — which is the
/// right cadence for a CLI (rules change slowly; a long-lived `wick serve`
/// picks them up on restart). Honors the telemetry opt-out, since it's a
/// network callback home.
pub fn refresh_if_stale() {
    REFRESH_ONCE.call_once(|| {
        if crate::analytics::is_opted_out() {
            return;
        }
        let path = overlay_path();
        if !needs_refresh(&path) {
            return;
        }
        std::thread::Builder::new()
            .name("wick-rules-refresh".into())
            .spawn(move || {
                let _ = refresh_now(&path);
            })
            .ok();
    });
}

fn needs_refresh(path: &Path) -> bool {
    match std::fs::metadata(path).and_then(|m| m.modified()) {
        // Refresh if the overlay is older than the interval, or if the clock
        // is unreadable (treat as stale rather than never-refresh).
        Ok(mtime) => mtime
            .elapsed()
            .map(|e| e.as_secs() > REFRESH_INTERVAL_SECS)
            .unwrap_or(true),
        // Missing / unreadable → first refresh.
        Err(_) => true,
    }
}

/// Fetch the published rules and atomically replace the overlay. Only writes
/// when the body is a valid rules doc, so a transient 5xx / truncated
/// response can't clobber a good overlay with garbage. Writing even an empty
/// `{"rules":{}}` is fine — it just refreshes mtime (suppressing refetch for a
/// day) and overlays nothing onto the bundled seed.
fn refresh_now(path: &Path) -> std::io::Result<()> {
    let io_err = |e| std::io::Error::new(std::io::ErrorKind::Other, e);
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(io_err)?;
    let resp = client.get(RULES_URL).send().map_err(io_err)?;
    if !resp.status().is_success() {
        return Ok(()); // leave the existing overlay / seed in place
    }
    let body = resp.text().map_err(io_err)?;
    // Validate before overwriting — must parse as our schema.
    if serde_json::from_str::<RuleFile>(&body).is_err() {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Atomic replace: write a temp file then rename, so a reader never sees a
    // half-written overlay.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, body.as_bytes())?;
    // Unix `rename` atomically replaces the destination; Windows `rename`
    // errors if it already exists, which would silently stop refresh once the
    // overlay was written. Fall back to a direct overwrite there.
    if std::fs::rename(&tmp, path).is_err() {
        std::fs::write(path, body.as_bytes())?;
        let _ = std::fs::remove_file(&tmp);
    }
    Ok(())
}

/// Look up the curated rule for a host: exact match, then a one-level parent
/// (so `www.reuters.com` resolves a `reuters.com` rule). Uses the same
/// `site_cache::parent_domain` walk as the local cache so the two layers
/// scope hosts identically.
pub fn get(host: &str) -> Option<SiteRule> {
    if let Some(r) = RULES.get(host) {
        return Some(r.clone());
    }
    crate::site_cache::parent_domain(host).and_then(|p| RULES.get(p).cloned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_parses_and_is_nonempty() {
        // The bundled seed must always parse — a malformed edit to
        // data/site-rules.json should fail CI here, not silently ship an
        // empty rule set.
        let seed = parse(SEED);
        assert!(!seed.is_empty(), "bundled seed should contain rules");
        assert!(seed.contains_key("reuters.com"));
    }

    #[test]
    fn known_cef_seed_resolves_via_www_parent() {
        // Telemetry reports `www.reuters.com`; the rule is keyed on the bare
        // domain and must resolve through the one-level parent walk.
        let r = get("www.reuters.com").expect("www.reuters.com → reuters.com");
        assert_eq!(r.render, "cef");
    }

    #[test]
    fn exact_subdomain_rule_does_not_overreach() {
        // `finance.yahoo.com` is keyed precisely; a bare `yahoo.com` (or a
        // sibling like `mail.yahoo.com`) must NOT inherit its cef rule.
        assert!(get("finance.yahoo.com").is_some());
        assert!(get("mail.yahoo.com").is_none());
        assert!(get("yahoo.com").is_none());
    }

    #[test]
    fn residential_flag_seeded_for_datacenter_blockers() {
        assert!(get("apkmirror.com").map(|r| r.needs_residential).unwrap_or(false));
        // www. parent walk carries the flag too.
        assert!(get("www.apkmirror.com").map(|r| r.needs_residential).unwrap_or(false));
    }

    #[test]
    fn unknown_host_has_no_rule() {
        assert!(get("example.com").is_none());
        assert!(get("news.ycombinator.com").is_none());
    }

    #[test]
    fn rule_missing_render_still_parses() {
        // A partial / residential-only entry must NOT fail the whole-file
        // parse and silently disable every overlay rule. render defaults to
        // "" (no opinion), the rest of the entry still applies.
        let doc = r#"{"version":1,"rules":{
            "a.com":{"needs_residential":true},
            "b.com":{"render":"cef"}
        }}"#;
        let m = parse(doc);
        assert_eq!(m.len(), 2, "a malformed entry must not drop the whole file");
        assert_eq!(m["a.com"].render, "");
        assert!(m["a.com"].needs_residential);
        assert_eq!(m["b.com"].render, "cef");
    }
}
