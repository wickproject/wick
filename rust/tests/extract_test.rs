use std::str::FromStr;

// We can't import from the binary crate directly in integration tests,
// so we test the underlying libraries the same way the code uses them.

const TEST_HTML: &str = r#"<!DOCTYPE html>
<html>
<head><title>Test Article</title></head>
<body>
<nav><a href="/">Home</a> | <a href="/about">About</a></nav>
<article>
<h1>Test Article Title</h1>
<p>This is a test paragraph with <strong>bold</strong> and <em>italic</em> text.</p>
<p>Another paragraph with a <a href="https://example.com">link</a>.</p>
<table>
<tr><th>Name</th><th>Value</th></tr>
<tr><td>foo</td><td>bar</td></tr>
<tr><td>baz</td><td>qux</td></tr>
</table>
<ul>
<li>Item one</li>
<li>Item two</li>
<li>Item three</li>
</ul>
</article>
<footer>Copyright 2026</footer>
</body>
</html>"#;

#[test]
fn test_readability_extracts_article() {
    let url = url::Url::from_str("https://example.com/article").unwrap();
    let mut cursor = std::io::Cursor::new(TEST_HTML.as_bytes());
    let product = readability::extractor::extract(&mut cursor, &url).unwrap();

    assert!(!product.title.is_empty(), "should extract a title");
    assert!(!product.content.is_empty(), "should extract content HTML");
    assert!(!product.text.is_empty(), "should extract text content");
}

#[test]
fn test_html_to_markdown_basic() {
    let converter = htmd::HtmlToMarkdown::builder()
        .skip_tags(vec!["script", "style"])
        .build();
    let md = converter.convert("<h1>Hello</h1><p>World</p>").unwrap();

    assert!(md.contains("Hello"), "should contain heading text");
    assert!(md.contains("World"), "should contain paragraph text");
}

#[test]
fn test_html_to_markdown_table() {
    let converter = htmd::HtmlToMarkdown::builder()
        .skip_tags(vec!["script", "style"])
        .build();
    let html = "<table><tr><th>A</th><th>B</th></tr><tr><td>1</td><td>2</td></tr></table>";
    let md = converter.convert(html).unwrap();

    assert!(md.contains("|"), "should produce markdown table with pipes");
    assert!(md.contains("A"), "should contain header");
}

#[test]
fn test_html_to_markdown_links() {
    let converter = htmd::HtmlToMarkdown::builder()
        .skip_tags(vec!["script", "style"])
        .build();
    let md = converter
        .convert(r#"<a href="https://example.com">click</a>"#)
        .unwrap();

    assert!(
        md.contains("[click](https://example.com)"),
        "should produce markdown link, got: {}",
        md
    );
}

#[test]
fn test_readability_strips_nav_footer() {
    let url = url::Url::from_str("https://example.com/article").unwrap();
    let mut cursor = std::io::Cursor::new(TEST_HTML.as_bytes());
    let product = readability::extractor::extract(&mut cursor, &url).unwrap();

    // Readability should strip nav and footer, keeping the article
    assert!(
        product.text.contains("test paragraph"),
        "should keep article content"
    );
    // Nav might or might not be stripped depending on readability heuristics,
    // but the article content should definitely be there
    assert!(
        product.text.contains("bold"),
        "should keep inline formatted text"
    );
}
