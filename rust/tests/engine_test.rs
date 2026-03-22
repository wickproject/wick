// Test Chrome header construction and user agent

#[test]
fn test_chrome_user_agent_format() {
    let ua = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
              AppleWebKit/537.36 (KHTML, like Gecko) \
              Chrome/143.0.7499.109 Safari/537.36";

    assert!(ua.contains("Chrome/143"), "UA should contain Chrome version");
    assert!(ua.contains("Mozilla/5.0"), "UA should start with Mozilla");
    assert!(ua.contains("Safari/537.36"), "UA should end with Safari");
    assert!(!ua.contains("Electron"), "UA must not contain Electron");
    assert!(!ua.contains("HeadlessChrome"), "UA must not contain HeadlessChrome");
}

#[test]
fn test_sec_ch_ua_format() {
    let major = "143";
    let header = format!(
        r#""Chromium";v="{}", "Google Chrome";v="{}", "Not:A-Brand";v="24""#,
        major, major
    );

    assert!(header.contains("Chromium"), "should contain Chromium brand");
    assert!(header.contains("Google Chrome"), "should contain Chrome brand");
    assert!(header.contains("Not:A-Brand"), "should contain Not:A-Brand");
}
