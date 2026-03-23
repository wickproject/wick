use anyhow::Result;
use std::path::PathBuf;
use std::process::Command;

const CEF_VERSION: &str = "0.1.0";
const CEF_BUNDLE_SHA256: &str = "b2530de3015612797021b672712984aa1efff5d0e20ff2a3ef68fda801c87a38";

/// Download and install the CEF renderer bundle for JavaScript rendering.
pub fn install_cef() -> Result<()> {
    let wick_bin = std::env::current_exe()?;
    let install_dir = wick_bin.parent().ok_or_else(|| anyhow::anyhow!("no parent dir"))?;
    let app_dir = install_dir.join("wick-renderer.app");

    if app_dir.exists() {
        println!("CEF renderer already installed at {}", app_dir.display());
        return Ok(());
    }

    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    let platform = match (os, arch) {
        ("macos", "aarch64") => "darwin-arm64",
        _ => anyhow::bail!("CEF bundle not available for {}-{}. Only macOS arm64 is currently supported.", os, arch),
    };

    let url = format!(
        "https://github.com/wickproject/wick/releases/download/v{}/wick-cef-{}.tar.gz",
        CEF_VERSION, platform
    );

    let tmp_dir = install_dir.join(".wick-cef-download");
    std::fs::create_dir_all(&tmp_dir)?;
    let tarball = tmp_dir.join("wick-cef.tar.gz");

    println!("Downloading CEF renderer (~121MB)...");
    let status = Command::new("curl")
        .args(["-fL", "--progress-bar", "-o"])
        .arg(&tarball)
        .arg(&url)
        .status()?;

    if !status.success() {
        let _ = std::fs::remove_dir_all(&tmp_dir);
        anyhow::bail!("Download failed. Check your internet connection and try again.");
    }

    // Verify checksum
    println!("Verifying checksum...");
    let output = Command::new("shasum")
        .args(["-a", "256"])
        .arg(&tarball)
        .output()?;
    let hash = String::from_utf8_lossy(&output.stdout);
    let actual = hash.split_whitespace().next().unwrap_or("");
    if actual != CEF_BUNDLE_SHA256 {
        let _ = std::fs::remove_dir_all(&tmp_dir);
        anyhow::bail!(
            "Checksum mismatch!\n  Expected: {}\n  Got:      {}\nThe download may be corrupted.",
            CEF_BUNDLE_SHA256, actual
        );
    }
    println!("Checksum OK.");

    // Extract
    println!("Extracting...");
    let status = Command::new("tar")
        .args(["xzf"])
        .arg(&tarball)
        .arg("-C")
        .arg(install_dir)
        .status()?;

    if !status.success() {
        let _ = std::fs::remove_dir_all(&tmp_dir);
        anyhow::bail!("Extraction failed");
    }

    // Cleanup
    let _ = std::fs::remove_dir_all(&tmp_dir);

    println!("CEF renderer installed at {}", app_dir.display());
    println!("JavaScript rendering is now available for wick_fetch.");
    Ok(())
}

pub fn setup() -> Result<()> {
    let wick_path = std::env::current_exe()
        .map_err(|e| anyhow::anyhow!("find wick binary: {}", e))?;
    let wick_str = wick_path.to_string_lossy();

    // Try claude CLI first
    if setup_claude_cli(&wick_str).is_ok() {
        println!("Configured Wick for Claude Code (via claude mcp add)");
        return Ok(());
    }

    let mut configured = false;

    if setup_claude_json(&wick_str).is_ok() {
        println!("Configured Wick for Claude Code (via ~/.claude.json)");
        configured = true;
    }

    if setup_cursor(&wick_str).is_ok() {
        println!("Configured Wick for Cursor");
        configured = true;
    }

    if !configured {
        anyhow::bail!(
            "no MCP clients found — install Claude Code or Cursor, then run 'wick setup' again"
        );
    }
    Ok(())
}

fn setup_claude_cli(wick_path: &str) -> Result<()> {
    let status = Command::new("claude")
        .args(["mcp", "add", "--transport", "stdio", "--scope", "user", "wick", "--"])
        .arg(wick_path)
        .args(["serve", "--mcp"])
        .status()?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("claude mcp add failed")
    }
}

fn setup_claude_json(wick_path: &str) -> Result<()> {
    let home = home_dir()?;
    let config_path = home.join(".claude.json");

    let mut config: serde_json::Map<String, serde_json::Value> = if config_path.exists() {
        let data = std::fs::read_to_string(&config_path)?;
        serde_json::from_str(&data)
            .map_err(|e| anyhow::anyhow!("malformed JSON in {}: {}", config_path.display(), e))?
    } else {
        serde_json::Map::new()
    };

    let servers = config
        .entry("mcpServers")
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));

    if let Some(map) = servers.as_object_mut() {
        map.insert(
            "wick".to_string(),
            serde_json::json!({
                "command": wick_path,
                "args": ["serve", "--mcp"]
            }),
        );
    }

    let json = serde_json::to_string_pretty(&config)?;
    std::fs::write(&config_path, json)?;
    Ok(())
}

fn setup_cursor(wick_path: &str) -> Result<()> {
    let home = home_dir()?;
    let config_dir = home.join(".cursor");

    if !config_dir.exists() {
        anyhow::bail!("cursor config directory not found");
    }

    let config_path = config_dir.join("mcp.json");

    let mut config: serde_json::Map<String, serde_json::Value> = if config_path.exists() {
        let data = std::fs::read_to_string(&config_path)?;
        serde_json::from_str(&data)
            .map_err(|e| anyhow::anyhow!("malformed JSON in {}: {}", config_path.display(), e))?
    } else {
        serde_json::Map::new()
    };

    let servers = config
        .entry("mcpServers")
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));

    if let Some(map) = servers.as_object_mut() {
        map.insert(
            "wick".to_string(),
            serde_json::json!({
                "command": wick_path,
                "args": ["serve", "--mcp"]
            }),
        );
    }

    let json = serde_json::to_string_pretty(&config)?;
    std::fs::write(&config_path, json)?;
    Ok(())
}

fn home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))
}
