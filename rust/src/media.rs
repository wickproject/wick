//! Extract media URLs from page HTML. Used by fetch to detect
//! downloadable video/audio that agents or crawlers may want.

/// Media found on a page.
pub struct MediaLink {
    pub url: String,
    pub media_type: String, // "video", "audio", "embed"
    pub source: String,     // "reddit", "youtube", "twitter", etc.
}

/// Scan HTML for embedded media URLs.
pub fn extract_media(html: &str, page_url: &url::Url) -> Vec<MediaLink> {
    let mut links = Vec::new();
    let lower = html.to_lowercase();

    // Reddit video (v.redd.it)
    if let Some(url) = extract_pattern(html, "https://v.redd.it/", "\"") {
        links.push(MediaLink {
            url,
            media_type: "video".into(),
            source: "reddit".into(),
        });
    }

    // YouTube embeds
    for pattern in &["youtube.com/embed/", "youtube.com/watch?v=", "youtu.be/"] {
        if let Some(url) = extract_pattern(html, pattern, "\"") {
            let full = if url.starts_with("http") { url } else { format!("https://{}", url) };
            links.push(MediaLink {
                url: full,
                media_type: "video".into(),
                source: "youtube".into(),
            });
            break;
        }
    }

    // Twitter/X video
    if lower.contains("video.twimg.com") {
        if let Some(url) = extract_pattern(html, "https://video.twimg.com/", "\"") {
            links.push(MediaLink {
                url,
                media_type: "video".into(),
                source: "twitter".into(),
            });
        }
    }

    // HTML5 <video> src
    if let Some(url) = extract_pattern(&lower, "<video", "</video>") {
        if let Some(src) = extract_pattern(&html[..], "src=\"", "\"") {
            if src.starts_with("http") {
                links.push(MediaLink {
                    url: src,
                    media_type: "video".into(),
                    source: "html5".into(),
                });
            }
        }
    }

    // HTML5 <audio> src
    if let Some(url) = extract_pattern(&lower, "<audio", "</audio>") {
        if let Some(src) = extract_pattern(&html[..], "src=\"", "\"") {
            if src.starts_with("http") {
                links.push(MediaLink {
                    url: src,
                    media_type: "audio".into(),
                    source: "html5".into(),
                });
            }
        }
    }

    // Deduplicate
    let mut seen = std::collections::HashSet::new();
    links.retain(|l| seen.insert(l.url.clone()));

    links
}

fn extract_pattern(html: &str, start: &str, end: &str) -> Option<String> {
    let pos = html.find(start)?;
    let after = &html[pos..];
    let url_end = after.find(end).unwrap_or(after.len().min(500));
    let url = &after[..url_end];
    if url.len() > 10 && url.len() < 500 {
        Some(url.to_string())
    } else {
        None
    }
}
