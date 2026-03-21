use anyhow::Result;
use robotstxt::DefaultMatcher;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const TTL: Duration = Duration::from_secs(3600);
const MAX_HOSTS: usize = 500;

struct CacheEntry {
    body: String,
    fetched_at: Instant,
}

static CACHE: std::sync::LazyLock<Mutex<HashMap<String, CacheEntry>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// Returns true if the URL is allowed by robots.txt.
/// Checks both "Wick" and "*" user agents.
pub async fn check(client: &crate::engine::Client, url: &str) -> bool {
    let parsed = match url::Url::parse(url) {
        Ok(u) => u,
        Err(_) => return true,
    };
    let host = format!("{}://{}", parsed.scheme(), parsed.host_str().unwrap_or(""));

    let robots_body = match get_robots(client, &host).await {
        Some(body) => body,
        None => return true, // can't fetch → allow
    };

    let mut matcher = DefaultMatcher::default();

    // Check Wick agent first
    if !matcher.one_agent_allowed_by_robots(&robots_body, "Wick", url) {
        return false;
    }
    // Then wildcard
    matcher.one_agent_allowed_by_robots(&robots_body, "*", url)
}

async fn get_robots(client: &crate::engine::Client, host: &str) -> Option<String> {
    // Check cache
    {
        let cache = CACHE.lock().ok()?;
        if let Some(entry) = cache.get(host) {
            if entry.fetched_at.elapsed() < TTL {
                return Some(entry.body.clone());
            }
        }
    }

    let url = format!("{}/robots.txt", host);
    let resp = client.get(&url).await.ok()?;
    let status = resp.status;
    let body = resp.body;

    let robots_body = match status {
        200 => body,
        404 | 410 => String::new(), // no robots.txt → allow all
        _ => return None,           // transient error → don't cache
    };

    // Cache it
    let mut cache = CACHE.lock().ok()?;
    if cache.len() >= MAX_HOSTS {
        cache.retain(|_, e| e.fetched_at.elapsed() < TTL);
    }
    cache.insert(
        host.to_string(),
        CacheEntry {
            body: robots_body.clone(),
            fetched_at: Instant::now(),
        },
    );

    Some(robots_body)
}
