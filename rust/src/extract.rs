use anyhow::Result;

#[derive(Debug, Clone, Copy)]
pub enum Format {
    Markdown,
    Html,
    Text,
}

impl Format {
    pub fn from_str(s: &str) -> Self {
        match s {
            "html" => Self::Html,
            "text" => Self::Text,
            _ => Self::Markdown,
        }
    }
}

pub struct Extracted {
    pub content: String,
    pub title: Option<String>,
}

/// Convert raw HTML to the requested format.
/// Converts the entire page — agents want all content, not article extraction.
pub fn extract(html: &str, _url: &url::Url, format: Format) -> Result<Extracted> {
    let title = extract_title(html);

    match format {
        Format::Html => Ok(Extracted {
            content: html.to_string(),
            title,
        }),
        Format::Text => Ok(Extracted {
            content: strip_tags(html),
            title,
        }),
        Format::Markdown => {
            let md = to_markdown(html)?;
            let content = match &title {
                Some(t) => format!("# {}\n\n{}", t, md.trim()),
                None => md.trim().to_string(),
            };
            Ok(Extracted { content, title })
        }
    }
}

/// Quick tag stripping for text output.
fn strip_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len() / 4);
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(c),
            _ => {}
        }
    }
    result
}

/// Extract title from <title> tag.
fn extract_title(html: &str) -> Option<String> {
    let lower = html.to_lowercase();
    let start = lower.find("<title")?.checked_add(lower[lower.find("<title")?..].find('>')? + 1)?;
    let end = lower[start..].find("</title>").map(|i| i + start)?;
    let title = html[start..end].trim().to_string();
    if title.is_empty() { None } else { Some(title) }
}

fn to_markdown(html: &str) -> Result<String> {
    let converter = htmd::HtmlToMarkdown::builder()
        .skip_tags(vec!["script", "style", "nav", "header", "footer", "aside"])
        .build();
    Ok(converter.convert(html)?)
}
