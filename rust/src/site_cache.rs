use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

/// Per-domain strategy learned from past fetches.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteStrategy {
    /// What worked: "cef", "cronet", or "cef_timeout" (CEF failed, use Cronet)
    pub strategy: String,
    /// Whether residential IP was needed (datacenter IP got blocked)
    #[serde(default)]
    pub needs_residential: bool,
    /// Average successful response time in ms
    pub avg_time_ms: u64,
    /// Number of successful fetches
    pub successes: u32,
    /// Last successful fetch (epoch seconds)
    pub last_success: String,
}

static CACHE: std::sync::LazyLock<Mutex<HashMap<String, SiteStrategy>>> =
    std::sync::LazyLock::new(|| Mutex::new(load_cache()));

/// Given a host, return its one-level parent domain when that parent still
/// contains a dot (e.g. `www.reuters.com` → `reuters.com`, but `reuters.com` →
/// `None` since `com` isn't a usable key). Shared by the `site_cache` and
/// `site_rules` lookups so the two layers scope hosts identically — note this
/// is a single label strip, not PSL/eTLD+1 normalization.
pub(crate) fn parent_domain(host: &str) -> Option<&str> {
    let dot = host.find('.')?;
    let parent = &host[dot + 1..];
    parent.contains('.').then_some(parent)
}

/// Get the best strategy for a domain based on past fetches.
pub fn get(host: &str) -> Option<SiteStrategy> {
    let cache = CACHE.lock().ok()?;
    // Exact match, then one-level parent (e.g. sub.example.com → example.com).
    if let Some(s) = cache.get(host) {
        return Some(s.clone());
    }
    parent_domain(host).and_then(|p| cache.get(p).cloned())
}

/// Record the result of a fetch for future use.
pub fn record(host: &str, strategy: &str, needs_residential: bool, time_ms: u64) {
    let mut cache = match CACHE.lock() {
        Ok(c) => c,
        Err(_) => return,
    };

    let now = chrono_now();

    let entry = cache.entry(host.to_string()).or_insert(SiteStrategy {
        strategy: strategy.to_string(),
        needs_residential,
        avg_time_ms: time_ms,
        successes: 0,
        last_success: now.clone(),
    });

    // Update with exponential moving average
    entry.successes += 1;
    entry.avg_time_ms = (entry.avg_time_ms * 3 + time_ms) / 4;
    entry.last_success = now;
    entry.strategy = strategy.to_string();
    // Once a site needed residential IP, keep that flag
    if needs_residential {
        entry.needs_residential = true;
    }

    // Persist (best-effort, don't block on errors)
    let _ = save_cache(&cache);
}

fn cache_path() -> PathBuf {
    crate::analytics::wick_home().join("site-cache.json")
}

fn load_cache() -> HashMap<String, SiteStrategy> {
    let path = cache_path();
    match std::fs::read_to_string(&path) {
        Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
        Err(_) => HashMap::new(),
    }
}

fn save_cache(cache: &HashMap<String, SiteStrategy>) -> std::io::Result<()> {
    let path = cache_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let data = serde_json::to_string_pretty(cache)?;
    std::fs::write(&path, data)
}

/// Returns current time as UNIX epoch seconds (as a string).
/// Chosen over ISO 8601 to avoid the `chrono` dependency.
fn chrono_now() -> String {
    let dur = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}", dur.as_secs())
}
