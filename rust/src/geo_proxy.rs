use anyhow::Result;

const PROXY_URL: &str = "https://releases.getwick.dev/proxy";

/// Fetch a URL through the Cloudflare Worker geo-proxy.
/// Bypasses geo-restrictions by originating from Cloudflare's edge PoPs
/// (Tokyo, Taipei, etc.) instead of the server's datacenter location.
pub async fn fetch(url: &str) -> Result<ProxyResponse> {
    let wick_key = std::env::var("WICK_KEY")
        .map_err(|_| anyhow::anyhow!("WICK_KEY not set — geo-proxy requires a Worker API key"))?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let resp = client
        .post(format!("{}/{}", PROXY_URL, wick_key))
        .json(&serde_json::json!({ "url": url }))
        .send()
        .await?;

    let status = resp.status().as_u16();
    let body = resp.text().await?;

    Ok(ProxyResponse { status, body })
}

pub struct ProxyResponse {
    pub status: u16,
    pub body: String,
}
