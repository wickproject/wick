use scraper::{Html, Selector};

// Test DuckDuckGo HTML parsing with a fixture

const DDG_FIXTURE: &str = r#"
<html>
<body>
<div class="results">
  <div class="result results_links results_links_deep web-result">
    <div class="result__body">
      <h2 class="result__title">
        <a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Frust-lang.org%2F&amp;rut=abc123">
          Rust Programming Language
        </a>
      </h2>
      <div class="result__snippet">
        A language empowering everyone to build reliable software.
      </div>
    </div>
  </div>
  <div class="result results_links results_links_deep web-result">
    <div class="result__body">
      <h2 class="result__title">
        <a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fen.wikipedia.org%2Fwiki%2FRust_(programming_language)&amp;rut=def456">
          Rust (programming language) - Wikipedia
        </a>
      </h2>
      <div class="result__snippet">
        Rust is a multi-paradigm, general-purpose programming language.
      </div>
    </div>
  </div>
  <div class="result results_links results_links_deep web-result">
    <div class="result__body">
      <h2 class="result__title">
        <a class="result__a" href="https://doc.rust-lang.org/book/">
          The Rust Programming Language - Book
        </a>
      </h2>
      <div class="result__snippet">
        The official book on the Rust programming language.
      </div>
    </div>
  </div>
</div>
</body>
</html>
"#;

fn parse_results(html: &str, max: usize) -> Vec<(String, String, String)> {
    let doc = Html::parse_document(html);
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
            results.push((title, url, snippet));
        }
    }

    results
}

fn clean_ddg_url(href: &str) -> String {
    if let Some(pos) = href.find("uddg=") {
        let encoded = &href[pos + 5..];
        let end = encoded.find('&').unwrap_or(encoded.len());
        if let Ok(decoded) = urlencoding::decode(&encoded[..end]) {
            return decoded.into_owned();
        }
    }
    href.to_string()
}

#[test]
fn test_parse_ddg_results() {
    let results = parse_results(DDG_FIXTURE, 10);
    assert_eq!(results.len(), 3, "should parse 3 results");

    assert_eq!(results[0].0, "Rust Programming Language");
    assert_eq!(results[0].1, "https://rust-lang.org/");
    assert!(results[0].2.contains("reliable software"));

    assert_eq!(
        results[1].1,
        "https://en.wikipedia.org/wiki/Rust_(programming_language)"
    );
}

#[test]
fn test_parse_ddg_results_limit() {
    let results = parse_results(DDG_FIXTURE, 2);
    assert_eq!(results.len(), 2, "should respect max limit");
}

#[test]
fn test_clean_ddg_url_with_redirect() {
    let url = "//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fpage&rut=abc123";
    assert_eq!(clean_ddg_url(url), "https://example.com/page");
}

#[test]
fn test_clean_ddg_url_direct() {
    let url = "https://doc.rust-lang.org/book/";
    assert_eq!(clean_ddg_url(url), "https://doc.rust-lang.org/book/");
}

#[test]
fn test_clean_ddg_url_encoded_special_chars() {
    let url = "//duckduckgo.com/l/?uddg=https%3A%2F%2Fen.wikipedia.org%2Fwiki%2FRust_%28programming_language%29&rut=xyz";
    assert_eq!(
        clean_ddg_url(url),
        "https://en.wikipedia.org/wiki/Rust_(programming_language)"
    );
}

#[test]
fn test_empty_html_returns_no_results() {
    let results = parse_results("<html><body></body></html>", 10);
    assert!(results.is_empty());
}
