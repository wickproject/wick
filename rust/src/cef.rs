use anyhow::{bail, Result};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

const RENDER_TIMEOUT: Duration = Duration::from_secs(60);
const BINDWG_PATH: &str = "/usr/local/lib/bindwg.so";

/// Persistent CEF daemon. Stays alive across requests for cookie
/// persistence (required for AWS WAF challenge → reload cycles).
static DAEMON: std::sync::LazyLock<Mutex<Option<DaemonProcess>>> =
    std::sync::LazyLock::new(|| Mutex::new(None));

struct DaemonProcess {
    child: Child,
}

/// Render with default options (no residential tunnel).
pub async fn render(url: &str) -> Result<String> {
    render_with_residential(url, false).await
}

pub async fn render_with_residential(url: &str, use_residential: bool) -> Result<String> {
    render_with_options(url, use_residential, None).await
}

pub async fn render_with_options(
    url: &str,
    use_residential: bool,
    _cookie: Option<&str>,
) -> Result<String> {
    let url = url.to_string();
    let result = tokio::task::spawn_blocking(move || {
        render_blocking(&url, use_residential)
    }).await
    .map_err(|e| anyhow::anyhow!("spawn_blocking: {}", e))?;

    result
}

/// Blocking render via daemon's stdin/stdout protocol.
fn render_blocking(url: &str, use_residential: bool) -> Result<String> {
    ensure_daemon(use_residential)?;

    let mut daemon = DAEMON.lock().map_err(|e| anyhow::anyhow!("lock: {}", e))?;
    let proc = daemon.as_mut().ok_or_else(|| anyhow::anyhow!("daemon not started"))?;

    let stdin = proc.child.stdin.as_mut()
        .ok_or_else(|| anyhow::anyhow!("daemon stdin closed"))?;
    let stdout = proc.child.stdout.as_mut()
        .ok_or_else(|| anyhow::anyhow!("daemon stdout closed"))?;

    // Send URL
    writeln!(stdin, "{}", url)?;
    stdin.flush()?;

    // Read length-prefixed response
    let mut reader = BufReader::new(stdout);
    let mut len_line = String::new();
    reader.read_line(&mut len_line)?;
    let byte_count: usize = len_line.trim().parse()
        .map_err(|e| anyhow::anyhow!("bad response '{}': {}", len_line.trim(), e))?;

    if byte_count == 0 {
        bail!("wick-renderer returned 0 bytes for {}", url);
    }

    let mut html_buf = vec![0u8; byte_count];
    reader.read_exact(&mut html_buf)?;

    Ok(String::from_utf8_lossy(&html_buf).into_owned())
}

fn ensure_daemon(use_residential: bool) -> Result<()> {
    let mut daemon = DAEMON.lock().map_err(|e| anyhow::anyhow!("lock: {}", e))?;

    // Check if existing daemon is still alive
    if let Some(ref mut d) = *daemon {
        match d.child.try_wait() {
            Ok(Some(_)) => { *daemon = None; }
            Ok(None) => return Ok(()),
            Err(_) => { *daemon = None; }
        }
    }

    let renderer_path = find_renderer()?;
    let renderer_dir = renderer_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("no parent dir"))?
        .to_path_buf();

    // No URL arg = daemon mode
    let mut cmd = Command::new(&renderer_path);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());

    // Xvfb for Linux WebGL
    #[cfg(target_os = "linux")]
    if std::env::var("DISPLAY").is_err() {
        if !std::path::Path::new("/tmp/.X99-lock").exists() {
            let _ = std::process::Command::new("Xvfb")
                .args([":99", "-screen", "0", "1920x1080x24"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
            std::thread::sleep(Duration::from_secs(1));
        }
        cmd.env("DISPLAY", ":99");
    }

    // LD_LIBRARY_PATH
    let lib_path = match std::env::var("LD_LIBRARY_PATH") {
        Ok(existing) => {
            #[cfg(target_os = "linux")]
            if std::env::var("DISPLAY").is_ok() || std::path::Path::new("/tmp/.X99-lock").exists() {
                format!("/usr/lib/x86_64-linux-gnu:{}:{}", renderer_dir.display(), existing)
            } else {
                format!("{}:{}", renderer_dir.display(), existing)
            }
            #[cfg(not(target_os = "linux"))]
            format!("{}:{}", renderer_dir.display(), existing)
        }
        Err(_) => renderer_dir.display().to_string(),
    };
    cmd.env("LD_LIBRARY_PATH", &lib_path);

    if use_residential && wireguard_active() && std::path::Path::new(BINDWG_PATH).exists() {
        cmd.env("LD_PRELOAD", BINDWG_PATH);
    }

    let child = cmd.spawn().map_err(|e| {
        anyhow::anyhow!("failed to start daemon {:?}: {}", renderer_path, e)
    })?;

    // Wait for CEF to initialize
    std::thread::sleep(Duration::from_secs(2));

    *daemon = Some(DaemonProcess { child });
    Ok(())
}

fn kill_daemon() {
    if let Ok(mut d) = DAEMON.lock() {
        if let Some(ref mut proc) = *d {
            let _ = proc.child.kill();
        }
        *d = None;
    }
}

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
        Some(PathBuf::from("/opt/wick/wick-renderer")),
        std::env::var_os("HOME").map(|h| {
            PathBuf::from(h).join(".wick").join("cef").join("wick-renderer")
        }),
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

    bail!("wick-renderer not found")
}

fn wireguard_active() -> bool {
    #[cfg(target_os = "linux")]
    { std::path::Path::new("/sys/class/net/wg-wick").exists() }
    #[cfg(not(target_os = "linux"))]
    { false }
}
