use anyhow::Result;
use std::io::Cursor;

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

/// Full extraction pipeline: raw HTML → readability → format conversion.
pub fn extract(html: &str, url: &url::Url, format: Format) -> Result<Extracted> {
    match format {
        Format::Html => Ok(Extracted {
            content: html.to_string(),
            title: None,
        }),
        Format::Text => {
            let readable = extract_readable(html, url)?;
            Ok(Extracted {
                content: readable.text,
                title: readable.title,
            })
        }
        Format::Markdown => {
            let readable = extract_readable(html, url)?;
            let md = to_markdown(&readable.html)?;
            let content = match &readable.title {
                Some(t) => format!("# {}\n\n{}", t, md.trim()),
                None => md.trim().to_string(),
            };
            Ok(Extracted {
                content,
                title: readable.title,
            })
        }
    }
}

struct Readable {
    title: Option<String>,
    html: String,
    text: String,
}

fn extract_readable(html: &str, url: &url::Url) -> Result<Readable> {
    let mut cursor = Cursor::new(html.as_bytes());
    let product = readability::extractor::extract(&mut cursor, url)
        .map_err(|e| anyhow::anyhow!("readability extraction failed: {}", e))?;
    Ok(Readable {
        title: if product.title.is_empty() {
            None
        } else {
            Some(product.title)
        },
        html: product.content,
        text: product.text,
    })
}

fn to_markdown(html: &str) -> Result<String> {
    let converter = htmd::HtmlToMarkdown::builder()
        .skip_tags(vec!["script", "style"])
        .build();
    Ok(converter.convert(html)?)
}
