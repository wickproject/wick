use anyhow::Result;
use scraper::{Html, Selector};

use crate::engine::Client;

/// A single search result.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Search the web via DuckDuckGo's HTML interface.
/// Returns up to `num_results` results (default 5).
pub async fn search(client: &Client, query: &str, num_results: usize) -> Result<Vec<SearchResult>> {
    let encoded = urlencoding::encode(query);
    let search_url = format!("https://html.duckduckgo.com/html/?q={}", encoded);

    let resp = client.get(&search_url).await?;

    if resp.status >= 400 {
        anyhow::bail!("Search failed: HTTP {}", resp.status);
    }

    parse_results(&resp.body, num_results)
}

/// Parse DuckDuckGo HTML results page.
fn parse_results(html: &str, max: usize) -> Result<Vec<SearchResult>> {
    let doc = Html::parse_document(html);

    // DuckDuckGo HTML results are in <div class="result"> or <div class="web-result">
    let result_sel = Selector::parse(".result__body, .web-result__body").unwrap();
    let title_sel = Selector::parse(".result__a, .result__title a").unwrap();
    let snippet_sel = Selector::parse(".result__snippet").unwrap();

    let mut results = Vec::new();

    for element in doc.select(&result_sel) {
        if results.len() >= max {
            break;
        }

        let title = element
            .select(&title_sel)
            .next()
            .map(|el| el.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let url = element
            .select(&title_sel)
            .next()
            .and_then(|el| el.value().attr("href"))
            .map(|href| clean_ddg_url(href))
            .unwrap_or_default();

        let snippet = element
            .select(&snippet_sel)
            .next()
            .map(|el| el.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        if !title.is_empty() && !url.is_empty() {
            results.push(SearchResult {
                title,
                url,
                snippet,
            });
        }
    }

    Ok(results)
}

/// DuckDuckGo wraps URLs in a redirect. Extract the actual URL.
fn clean_ddg_url(href: &str) -> String {
    // DDG HTML links look like: //duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com&rut=...
    if let Some(pos) = href.find("uddg=") {
        let encoded = &href[pos + 5..];
        let end = encoded.find('&').unwrap_or(encoded.len());
        if let Ok(decoded) = urlencoding::decode(&encoded[..end]) {
            return decoded.into_owned();
        }
    }
    // Fallback: return as-is
    href.to_string()
}

/// Format search results as markdown for the LLM.
pub fn format_results(results: &[SearchResult]) -> String {
    if results.is_empty() {
        return "No results found.".to_string();
    }

    let mut out = String::new();
    for (i, r) in results.iter().enumerate() {
        out.push_str(&format!(
            "{}. [{}]({})\n   {}\n\n",
            i + 1,
            r.title,
            r.url,
            r.snippet
        ));
    }
    out.trim_end().to_string()
}
