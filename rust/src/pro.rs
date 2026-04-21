//! CEF renderer installer (formerly "Pro activate").
//! Wick is now fully unified — CEF + stealth + auto-CAPTCHA are all free.
//! This module just installs the CEF runtime on demand since it's ~200MB.

use anyhow::Result;

/// Install the CEF renderer (JS rendering + stealth patches).
/// Previously called "pro activate" — kept under the pro subcommand
/// for backward compatibility, but Wick is fully open source now.
pub async fn activate(_legacy_key: Option<String>) -> Result<()> {
    if crate::cef::is_available() {
        println!("CEF renderer already installed.");
        return Ok(());
    }

    println!("Installing CEF renderer (~200 MB download)...");

    #[cfg(target_os = "macos")]
    {
        let script_url = "https://raw.githubusercontent.com/wickproject/wick/main/scripts/install-cef-mac.sh";
        let status = std::process::Command::new("bash")
            .arg("-c")
            .arg(format!("curl -fsSL {} | bash", script_url))
            .status()?;
        if !status.success() {
            println!("Auto-install failed. Run manually:");
            println!("  curl -fsSL {} | bash", script_url);
        }
    }

    #[cfg(target_os = "linux")]
    {
        let script_url = "https://raw.githubusercontent.com/wickproject/wick/main/scripts/install-cef-linux.sh";
        println!("Run this to install the CEF renderer:");
        println!("  curl -fsSL {} | sudo -E bash", script_url);
    }

    Ok(())
}

/// Show CEF renderer install status.
pub async fn status() -> Result<()> {
    if crate::cef::is_available() {
        println!("Wick: unified open source (MIT)");
        println!("CEF renderer: installed");
    } else {
        println!("Wick: unified open source (MIT)");
        println!("CEF renderer: not installed");
        println!("  Run: wick install cef");
    }
    Ok(())
}

/// Legacy key loader — kept for compatibility with code that still
/// references WICK_KEY (e.g. the tunnel/release endpoints). No-op
/// for typical users now.
pub fn load_key() -> Option<String> {
    std::env::var("WICK_KEY").ok().filter(|s| !s.is_empty())
}
