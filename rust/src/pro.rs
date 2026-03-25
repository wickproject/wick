//! Pro subscription management.
//! `wick pro activate` → opens Stripe checkout → saves key → installs renderer.
//! `wick pro status` → shows subscription status.

use anyhow::Result;
use std::path::PathBuf;

const CHECKOUT_URL: &str = "https://releases.getwick.dev/pro/checkout";
const STATUS_URL: &str = "https://releases.getwick.dev/pro/status";
const VALIDATE_URL: &str = "https://releases.getwick.dev/pro/validate";

/// Activate Pro — either with an existing key or by creating a new subscription.
pub async fn activate(existing_key: Option<String>) -> Result<()> {
    if let Some(key) = existing_key {
        // Validate the key
        let client = reqwest::Client::new();
        let resp = client
            .get(format!("{}/{}", VALIDATE_URL, key))
            .send()
            .await?;
        let body: serde_json::Value = resp.json().await?;

        if body.get("valid").and_then(|v| v.as_bool()) != Some(true) {
            anyhow::bail!("Invalid API key. Contact hello@getwick.dev for help.");
        }

        save_key(&key)?;
        println!("Pro activated with key {}...{}", &key[..6], &key[key.len()-4..]);
        install_renderer().await?;
        return Ok(());
    }

    // Create a new checkout session
    println!("Opening Wick Pro checkout ($20/month)...\n");

    let client = reqwest::Client::new();
    let resp = client
        .post(CHECKOUT_URL)
        .json(&serde_json::json!({}))
        .send()
        .await?;

    let body: serde_json::Value = resp.json().await?;

    if let Some(error) = body.get("error") {
        anyhow::bail!("Checkout failed: {}", error);
    }

    let checkout_url = body["checkoutUrl"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("No checkout URL returned. Stripe may not be configured yet.\n\nFor early access, contact hello@getwick.dev"))?;
    let session_id = body["sessionId"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("No session ID"))?;

    // Open browser
    #[cfg(target_os = "macos")]
    { let _ = std::process::Command::new("open").arg(checkout_url).spawn(); }
    #[cfg(target_os = "linux")]
    { let _ = std::process::Command::new("xdg-open").arg(checkout_url).spawn(); }

    println!("If browser didn't open, go to:\n  {}\n", checkout_url);
    println!("Waiting for payment...");

    // Poll for key
    for i in 0..90 {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        if i % 5 == 4 { print!("."); }

        let resp = client
            .get(format!("{}/{}", STATUS_URL, session_id))
            .send()
            .await?;
        let status: serde_json::Value = resp.json().await?;

        if status["status"].as_str() == Some("active") {
            if let Some(key) = status["key"].as_str() {
                println!("\n\nPro activated!");
                save_key(key)?;
                install_renderer().await?;
                println!("\nYour API key: {}", key);
                println!("Saved to {}", key_path().display());
                println!("\nRun: wick fetch https://www.reddit.com/r/Epstein/ --no-robots");
                return Ok(());
            }
        }
    }

    println!("\n\nPayment not yet confirmed. If you completed checkout, run:");
    println!("  wick pro activate --key YOUR_KEY");
    println!("(Your key is shown on the checkout success page)");
    Ok(())
}

/// Show Pro status.
pub async fn status() -> Result<()> {
    let key = match load_key() {
        Some(k) => k,
        None => {
            println!("Wick Pro: not activated");
            println!("\nTo activate: wick pro activate ($20/month)");
            if crate::cef::is_available() {
                println!("\nNote: Pro renderer IS installed at:");
                println!("  (CEF detected but no API key saved)");
            }
            return Ok(());
        }
    };

    // Validate
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/{}", VALIDATE_URL, key))
        .send()
        .await;

    match resp {
        Ok(r) => {
            let body: serde_json::Value = r.json().await.unwrap_or_default();
            if body.get("valid").and_then(|v| v.as_bool()) == Some(true) {
                println!("Wick Pro: active");
                println!("Key: {}...{}", &key[..6], &key[key.len().saturating_sub(4)..]);
                if let Some(email) = body["email"].as_str() {
                    println!("Email: {}", email);
                }
            } else {
                println!("Wick Pro: key invalid or expired");
            }
        }
        Err(_) => {
            println!("Wick Pro: key saved (offline, can't verify)");
            println!("Key: {}...{}", &key[..6], &key[key.len().saturating_sub(4)..]);
        }
    }

    if crate::cef::is_available() {
        println!("Renderer: installed");
    } else {
        println!("Renderer: not installed");
        println!("  Run: wick pro activate");
    }

    Ok(())
}

/// Install the Pro renderer if not already present.
async fn install_renderer() -> Result<()> {
    if crate::cef::is_available() {
        println!("Pro renderer already installed.");
        return Ok(());
    }

    println!("Installing Pro renderer...");

    #[cfg(target_os = "macos")]
    {
        let script_url = "https://releases.getwick.dev/install-pro-mac.sh";
        let key = load_key().unwrap_or_default();
        let status = std::process::Command::new("bash")
            .arg("-c")
            .arg(format!("WICK_KEY={} curl -fsSL {} | bash", key, script_url))
            .status()?;
        if !status.success() {
            println!("Auto-install failed. Run manually:");
            println!("  WICK_KEY={} curl -fsSL {} | bash", key, script_url);
        }
    }

    #[cfg(target_os = "linux")]
    {
        let key = load_key().unwrap_or_default();
        println!("Run this to install the Pro renderer:");
        println!("  WICK_KEY={} curl -fsSL https://releases.getwick.dev/install-pro.sh | sudo -E bash", key);
    }

    Ok(())
}

fn key_path() -> PathBuf {
    let home = std::env::var_os("HOME").unwrap_or_else(|| "/tmp".into());
    PathBuf::from(home).join(".wick").join("pro-key")
}

fn save_key(key: &str) -> Result<()> {
    let path = key_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, key)?;
    // Restrict permissions
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    // Also set WICK_KEY env for current session
    std::env::set_var("WICK_KEY", key);
    Ok(())
}

pub fn load_key() -> Option<String> {
    // Check env first
    if let Ok(key) = std::env::var("WICK_KEY") {
        if !key.is_empty() { return Some(key); }
    }
    // Check file
    let path = key_path();
    std::fs::read_to_string(&path).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}
