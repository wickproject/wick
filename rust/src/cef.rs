use anyhow::{bail, Result};
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;

const RENDER_TIMEOUT: Duration = Duration::from_secs(30);

/// Persistent CEF renderer process. Spawned once, reused for all requests.
/// If the process dies or gets wedged, it's killed and respawned.
struct RendererProcess {
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    reader: BufReader<tokio::process::ChildStdout>,
}

impl Drop for RendererProcess {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

static RENDERER: Mutex<Option<RendererProcess>> = Mutex::const_new(None);

async fn get_or_spawn() -> Result<()> {
    let mut guard = RENDERER.lock().await;
    if guard.is_none() {
        *guard = Some(spawn_renderer().await?);
    }
    Ok(())
}

async fn kill_and_respawn() -> Result<()> {
    let mut guard = RENDERER.lock().await;
    // Drop the old process (kill_on_drop via Drop impl)
    *guard = None;
    *guard = Some(spawn_renderer().await?);
    Ok(())
}

async fn spawn_renderer() -> Result<RendererProcess> {
    cleanup_cef_caches();

    let renderer_path = find_renderer()?;

    let mut child = Command::new(&renderer_path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit()) // inherit stderr to avoid pipe buffer stall
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| {
            anyhow::anyhow!(
                "failed to start wick-renderer at {:?}: {}. \
                 Run 'wick setup --with-js' to install CEF.",
                renderer_path,
                e
            )
        })?;

    let stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();

    Ok(RendererProcess {
        child,
        stdin,
        reader: BufReader::new(stdout),
    })
}

/// Render a page using the persistent CEF renderer.
/// First call spawns the process; subsequent calls reuse it.
/// On timeout or protocol error, the renderer is killed and respawned.
pub async fn render(url: &str) -> Result<String> {
    get_or_spawn().await?;

    let result = tokio::time::timeout(RENDER_TIMEOUT, async {
        let mut guard = RENDERER.lock().await;
        let renderer = guard.as_mut().unwrap();

        // Send URL
        renderer
            .stdin
            .write_all(format!("{}\n", url).as_bytes())
            .await?;
        renderer.stdin.flush().await?;

        // Read length line
        let mut len_line = String::new();
        renderer.reader.read_line(&mut len_line).await?;
        let len: usize = len_line.trim().parse().map_err(|e| {
            anyhow::anyhow!("invalid length from renderer: {:?} ({})", len_line.trim(), e)
        })?;

        if len == 0 {
            bail!("renderer returned empty page");
        }

        // Read exactly `len` bytes of HTML
        let mut html_bytes = vec![0u8; len];
        tokio::io::AsyncReadExt::read_exact(&mut renderer.reader, &mut html_bytes).await?;

        Ok(String::from_utf8_lossy(&html_bytes).into_owned())
    })
    .await;

    match result {
        Ok(Ok(html)) => Ok(html),
        Ok(Err(e)) => {
            // Protocol or IO error — kill renderer and respawn for next request
            tracing::warn!("CEF renderer error, respawning: {}", e);
            let _ = kill_and_respawn().await;
            Err(e)
        }
        Err(_) => {
            // Timeout — kill wedged renderer and respawn
            tracing::warn!("CEF renderer timed out, respawning");
            let _ = kill_and_respawn().await;
            bail!("CEF rendering timed out after {}s", RENDER_TIMEOUT.as_secs())
        }
    }
}

/// Check if CEF renderer is available.
pub fn is_available() -> bool {
    find_renderer().is_ok()
}

fn find_renderer() -> Result<PathBuf> {
    let locations = [
        std::env::current_exe().ok().and_then(|p| {
            p.parent()
                .map(|d| d.join("wick-renderer.app/Contents/MacOS/wick-renderer"))
        }),
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("wick-renderer"))),
        std::env::var_os("HOME").map(|h| {
            PathBuf::from(h)
                .join(".wick")
                .join("cef")
                .join("wick-renderer.app/Contents/MacOS/wick-renderer")
        }),
    ];

    for loc in locations.iter().flatten() {
        if loc.exists() {
            return Ok(loc.clone());
        }
    }

    if let Ok(p) = which("wick-renderer") {
        return Ok(p);
    }

    bail!(
        "wick-renderer not found. \
         Run 'wick setup --with-js' to install CEF for JavaScript rendering."
    )
}

fn cleanup_cef_caches() {
    if let Some(home) = std::env::var_os("HOME") {
        let wick_dir = PathBuf::from(home).join(".wick");
        if let Ok(entries) = std::fs::read_dir(&wick_dir) {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    // Match both "cef-cache" and "cef-cache-*" variants
                    if name.starts_with("cef-cache") {
                        let _ = std::fs::remove_dir_all(entry.path());
                    }
                }
            }
        }
    }
}

fn which(name: &str) -> Result<PathBuf> {
    let output = std::process::Command::new("which").arg(name).output()?;
    if output.status.success() {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return Ok(PathBuf::from(path));
        }
    }
    bail!("{} not on PATH", name)
}
