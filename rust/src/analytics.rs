//! Lightweight usage analytics. No PII — just event counts.
//! Pings releases.getwick.dev/ping with event type, version, and OS.
//! Runs async, never blocks the main operation, fails silently.

const PING_URL: &str = "https://releases.getwick.dev/ping";

/// Send a usage ping (fire-and-forget, never fails the caller).
pub fn ping(event: &str) {
    let event = event.to_string();
    let version = env!("CARGO_PKG_VERSION").to_string();
    let os = std::env::consts::OS.to_string();

    // Don't ping more than once per event per day
    let marker = ping_marker(&event);
    if marker.exists() {
        return;
    }

    // Fire and forget — spawn a thread so we don't need async
    std::thread::spawn(move || {
        let _ = send_ping(&event, &version, &os);
        // Write marker file
        if let Some(dir) = marker.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&marker, "");
    });
}

fn ping_marker(event: &str) -> std::path::PathBuf {
    let home = std::env::var_os("HOME").unwrap_or_else(|| "/tmp".into());
    let date = chrono_today();
    std::path::PathBuf::from(home)
        .join(".wick")
        .join("pings")
        .join(format!("{}-{}", date, event))
}

fn chrono_today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = secs / 86400;
    // Simple date from epoch days (good enough for daily dedup)
    format!("{}", days)
}

fn send_ping(event: &str, version: &str, os: &str) -> Result<(), Box<dyn std::error::Error>> {
    let body = format!(
        r#"{{"event":"{}","version":"{}","os":"{}"}}"#,
        event, version, os
    );

    // Use a short timeout so this never delays anything
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()?;

    client.post(PING_URL)
        .header("Content-Type", "application/json")
        .body(body)
        .send()?;

    Ok(())
}
