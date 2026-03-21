use anyhow::Result;
use std::path::PathBuf;
use std::process::Command;

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
