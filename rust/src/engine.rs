use anyhow::Result;
use std::path::PathBuf;

#[cfg(feature = "cronet")]
use crate::cronet;

const CHROME_MAJOR: &str = "143";
const CHROME_FULL: &str = "143.0.7499.109";

/// HTTP client with Chrome-equivalent headers.
/// With the `cronet` feature, uses Chromium's actual network stack.
/// Without it, uses reqwest with Chrome-like headers (weaker fingerprint).
pub struct Client { // Debug not derived: inner types don't support it
    #[cfg(feature = "cronet")]
    engine: cronet::Engine,
    #[cfg(not(feature = "cronet"))]
    inner: reqwest::Client,
}

impl Client {
    pub fn new() -> Result<Self> {
        #[cfg(feature = "cronet")]
        {
            let storage = storage_path()?;
            let engine = cronet::Engine::new(&storage, &chrome_user_agent())?;
            Ok(Self { engine })
        }
        #[cfg(not(feature = "cronet"))]
        {
            let client = reqwest::Client::builder()
                .user_agent(chrome_user_agent())
                .default_headers(chrome_headers_reqwest())
                .gzip(true)
                .brotli(true)
                .deflate(true)
                .timeout(std::time::Duration::from_secs(30))
                .build()?;
            Ok(Self { inner: client })
        }
    }

    pub async fn get(&self, url: &str) -> Result<HttpResponse> {
        #[cfg(feature = "cronet")]
        {
            let headers = chrome_header_pairs();
            let resp = self.engine.get(url, &headers).await?;
            Ok(HttpResponse {
                status: resp.status_code,
                body: resp.text(),
            })
        }
        #[cfg(not(feature = "cronet"))]
        {
            let resp = self.inner.get(url).send().await?;
            let status = resp.status().as_u16();
            let body = resp.text().await?;
            Ok(HttpResponse { status, body })
        }
    }
}

pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

fn storage_path() -> Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?;
    Ok(PathBuf::from(home).join(".wick").join("data"))
}

pub fn chrome_user_agent() -> String {
    format!(
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
         AppleWebKit/537.36 (KHTML, like Gecko) \
         Chrome/{CHROME_FULL} Safari/537.36"
    )
}

/// Header pairs for Cronet (name, value).
#[cfg(feature = "cronet")]
fn chrome_header_pairs() -> Vec<(&'static str, &'static str)> {
    vec![
        ("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8"),
        ("Accept-Language", "en-US,en;q=0.9"),
        ("Accept-Encoding", "gzip, deflate, br, zstd"),
        ("Cache-Control", "max-age=0"),
        ("Sec-Fetch-Dest", "document"),
        ("Sec-Fetch-Mode", "navigate"),
        ("Sec-Fetch-Site", "none"),
        ("Sec-Fetch-User", "?1"),
        ("Upgrade-Insecure-Requests", "1"),
    ]
}

/// Headers for reqwest fallback.
#[cfg(not(feature = "cronet"))]
fn chrome_headers_reqwest() -> reqwest::header::HeaderMap {
    use reqwest::header::{HeaderMap, HeaderValue};
    let mut h = HeaderMap::new();
    h.insert("Accept", HeaderValue::from_static(
        "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8",
    ));
    h.insert("Accept-Language", HeaderValue::from_static("en-US,en;q=0.9"));
    h.insert("Accept-Encoding", HeaderValue::from_static("gzip, deflate, br"));
    h.insert("Cache-Control", HeaderValue::from_static("max-age=0"));
    h.insert(
        "Sec-Ch-Ua",
        HeaderValue::from_str(&format!(
            r#""Chromium";v="{CHROME_MAJOR}", "Google Chrome";v="{CHROME_MAJOR}", "Not:A-Brand";v="24""#
        )).unwrap(),
    );
    h.insert("Sec-Ch-Ua-Mobile", HeaderValue::from_static("?0"));
    h.insert("Sec-Ch-Ua-Platform", HeaderValue::from_static("\"macOS\""));
    h.insert("Sec-Fetch-Dest", HeaderValue::from_static("document"));
    h.insert("Sec-Fetch-Mode", HeaderValue::from_static("navigate"));
    h.insert("Sec-Fetch-Site", HeaderValue::from_static("none"));
    h.insert("Sec-Fetch-User", HeaderValue::from_static("?1"));
    h.insert("Upgrade-Insecure-Requests", HeaderValue::from_static("1"));
    h
}
