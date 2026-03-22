// Test challenge detection logic (extracted from fetch.rs)

fn is_challenge(body: &str) -> bool {
    let lower = body.to_lowercase();
    [
        "challenges.cloudflare.com",
        "cf-browser-verification",
        "just a moment...",
        "checking your browser",
        "google.com/recaptcha",
        "hcaptcha.com",
    ]
    .iter()
    .any(|sig| lower.contains(sig))
}

#[test]
fn test_detect_cloudflare_challenge() {
    let body = r#"<html><head><title>Just a moment...</title></head>
    <body><h1>Checking your browser before accessing example.com.</h1>
    <script src="https://challenges.cloudflare.com/turnstile/v0/api.js"></script>
    </body></html>"#;
    assert!(is_challenge(body), "should detect Cloudflare challenge");
}

#[test]
fn test_detect_recaptcha() {
    let body = r#"<html><body>
    <script src="https://www.google.com/recaptcha/api.js"></script>
    <div class="g-recaptcha"></div>
    </body></html>"#;
    assert!(is_challenge(body), "should detect reCAPTCHA");
}

#[test]
fn test_detect_hcaptcha() {
    let body = r#"<html><body>
    <script src="https://hcaptcha.com/1/api.js"></script>
    </body></html>"#;
    assert!(is_challenge(body), "should detect hCaptcha");
}

#[test]
fn test_detect_cf_browser_verification() {
    let body = r#"<html><body>
    <div id="cf-browser-verification">Verifying...</div>
    </body></html>"#;
    assert!(
        is_challenge(body),
        "should detect cf-browser-verification div"
    );
}

#[test]
fn test_normal_page_not_challenge() {
    let body = r#"<html><head><title>Example</title></head>
    <body><h1>Welcome</h1><p>Normal content.</p></body></html>"#;
    assert!(!is_challenge(body), "normal page should not be a challenge");
}

#[test]
fn test_case_insensitive_detection() {
    let body = "JUST A MOMENT... Checking Your Browser";
    assert!(
        is_challenge(body),
        "should detect challenge case-insensitively"
    );
}

#[test]
fn test_empty_body_not_challenge() {
    assert!(!is_challenge(""), "empty body is not a challenge");
}

// Test robots.txt matching using the robotstxt crate directly

#[test]
fn test_robots_allow() {
    let mut matcher = robotstxt::DefaultMatcher::default();
    let robots = "User-agent: *\nDisallow: /private/\n";
    assert!(matcher.one_agent_allowed_by_robots(robots, "Wick", "https://example.com/public"));
    assert!(matcher.one_agent_allowed_by_robots(robots, "*", "https://example.com/public"));
}

#[test]
fn test_robots_disallow() {
    let mut matcher = robotstxt::DefaultMatcher::default();
    let robots = "User-agent: *\nDisallow: /private/\n";
    assert!(!matcher.one_agent_allowed_by_robots(
        robots,
        "*",
        "https://example.com/private/secret"
    ));
}

#[test]
fn test_robots_wick_specific() {
    let mut matcher = robotstxt::DefaultMatcher::default();
    let robots = "User-agent: Wick\nDisallow: /\n\nUser-agent: *\nAllow: /\n";
    assert!(!matcher.one_agent_allowed_by_robots(robots, "Wick", "https://example.com/page"));
    assert!(matcher.one_agent_allowed_by_robots(robots, "*", "https://example.com/page"));
}

#[test]
fn test_robots_empty_allows_all() {
    let mut matcher = robotstxt::DefaultMatcher::default();
    assert!(matcher.one_agent_allowed_by_robots("", "Wick", "https://example.com/anything"));
}
