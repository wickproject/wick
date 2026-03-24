//! Lightweight CEF renderer integration for the free binary.
//! If wick-renderer (Pro) is installed, use it for JS rendering.
//! The renderer has stealth patches, smart content polling, etc.
//! This module just finds it and communicates via stdin/stdout.

use anyhow::{bail, Result};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

static DAEMON: std::sync::LazyLock<Mutex<Option<Child>>> =
    std::sync::LazyLock::new(|| Mutex::new(None));

/// Check if the Pro renderer is installed.
pub fn is_available() -> bool {
    find_renderer().is_ok()
}

/// Render a URL via the Pro renderer daemon.
pub async fn render(url: &str) -> Result<String> {
    let url = url.to_string();
    tokio::task::spawn_blocking(move || render_blocking(&url))
        .await
        .map_err(|e| anyhow::anyhow!("spawn: {}", e))?
}

fn render_blocking(url: &str) -> Result<String> {
    ensure_daemon()?;

    let mut daemon = DAEMON.lock().map_err(|e| anyhow::anyhow!("lock: {}", e))?;
    let child = daemon.as_mut().ok_or_else(|| anyhow::anyhow!("daemon not running"))?;

    let stdin = child.stdin.as_mut()
        .ok_or_else(|| anyhow::anyhow!("stdin closed"))?;
    let stdout = child.stdout.as_mut()
        .ok_or_else(|| anyhow::anyhow!("stdout closed"))?;

    writeln!(stdin, "{}", url)?;
    stdin.flush()?;

    let mut reader = BufReader::new(stdout);
    let mut len_line = String::new();
    reader.read_line(&mut len_line)?;
    let byte_count: usize = len_line.trim().parse()
        .map_err(|e| anyhow::anyhow!("bad response '{}': {}", len_line.trim(), e))?;

    if byte_count == 0 {
        bail!("renderer returned 0 bytes");
    }

    let mut buf = vec![0u8; byte_count];
    reader.read_exact(&mut buf)?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

fn ensure_daemon() -> Result<()> {
    let mut daemon = DAEMON.lock().map_err(|e| anyhow::anyhow!("lock: {}", e))?;

    if let Some(ref mut child) = *daemon {
        match child.try_wait() {
            Ok(Some(_)) => { *daemon = None; }
            Ok(None) => return Ok(()),
            Err(_) => { *daemon = None; }
        }
    }

    let path = find_renderer()?;
    let child = Command::new(&path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to start {:?}: {}", path, e))?;

    std::thread::sleep(std::time::Duration::from_secs(2));
    *daemon = Some(child);
    Ok(())
}

fn find_renderer() -> Result<PathBuf> {
    let paths = [
        // Next to wick binary (macOS .app)
        std::env::current_exe().ok().and_then(|p| {
            p.parent().map(|d| d.join("wick-renderer.app/Contents/MacOS/wick-renderer"))
        }),
        // Next to wick binary (Linux)
        std::env::current_exe().ok().and_then(|p| {
            p.parent().map(|d| d.join("wick-renderer"))
        }),
        // System install (Linux Pro)
        Some(PathBuf::from("/opt/wick/wick-renderer")),
        // User install (macOS Pro)
        std::env::var_os("HOME").map(|h| {
            PathBuf::from(h).join(".wick").join("cef")
                .join("wick-renderer.app/Contents/MacOS/wick-renderer")
        }),
        // User install (Linux)
        std::env::var_os("HOME").map(|h| {
            PathBuf::from(h).join(".wick").join("cef").join("wick-renderer")
        }),
    ];

    for p in paths.iter().flatten() {
        if p.exists() {
            return Ok(p.clone());
        }
    }

    bail!("wick-renderer not found")
}
