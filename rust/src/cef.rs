use anyhow::{bail, Result};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

const RENDER_TIMEOUT: Duration = Duration::from_secs(60);
const BINDWG_PATH: &str = "/usr/local/lib/bindwg.so";

/// Persistent CEF daemon. Stays alive across requests for cookie
/// persistence (required for AWS WAF challenge → reload cycles).
static DAEMON: std::sync::LazyLock<Mutex<Option<DaemonProcess>>> =
    std::sync::LazyLock::new(|| Mutex::new(None));

/// Takes ownership of the daemon's stdin/stdout so a single persistent
/// BufReader is reused across requests — avoids losing bytes buffered
/// ahead of the current length-prefixed response.
struct DaemonProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    /// Whether this daemon was spawned with the residential tunnel
    /// (LD_PRELOAD bindwg.so). The daemon is a process-wide singleton, so a
    /// later request in the other mode must respawn it — see `ensure_daemon`.
    use_residential: bool,
}

/// Render with default options (no residential tunnel, no selector wait).
pub async fn render(url: &str) -> Result<String> {
    render_with(url, RenderOptions::default()).await
}

/// Per-request knobs for the CEF renderer.
///
/// `wait_for_selector` tells the renderer to delay the DOM dump until the
/// given CSS selector exists in the document. Used to read SPA content
/// that loads via XHR after initial hydration (X timelines, etc).
#[derive(Debug, Clone, Default)]
pub struct RenderOptions {
    pub use_residential: bool,
    pub wait_for_selector: Option<String>,
}

pub async fn render_with_residential(url: &str, use_residential: bool) -> Result<String> {
    render_with(
        url,
        RenderOptions {
            use_residential,
            ..Default::default()
        },
    )
    .await
}

pub async fn render_with(url: &str, opts: RenderOptions) -> Result<String> {
    let url = url.to_string();
    tokio::task::spawn_blocking(move || render_blocking(&url, &opts))
        .await
        .map_err(|e| anyhow::anyhow!("spawn_blocking: {}", e))?
}

/// Blocking render via daemon's stdin/stdout protocol.
fn render_blocking(url: &str, opts: &RenderOptions) -> Result<String> {
    ensure_daemon(opts.use_residential)?;

    let mut daemon = DAEMON.lock().map_err(|e| anyhow::anyhow!("lock: {}", e))?;
    let proc = daemon.as_mut().ok_or_else(|| anyhow::anyhow!("daemon not started"))?;

    // Protocol: "URL\n" or "URL\tSELECTOR\n". The daemon strips
    // single-quote / backslash from the selector to keep it safe to splice
    // into a JS string literal, so callers don't have to JS-escape.
    match opts.wait_for_selector.as_deref() {
        Some(sel) if !sel.is_empty() => {
            writeln!(proc.stdin, "{}\t{}", url, sel)?;
        }
        _ => {
            writeln!(proc.stdin, "{}", url)?;
        }
    }
    proc.stdin.flush()?;

    // Read length-prefixed response using the persistent BufReader.
    // (Don't wrap a temporary BufReader here — read-ahead bytes would
    // be lost when it drops, desynchronizing subsequent renders.)
    let mut len_line = String::new();
    proc.stdout.read_line(&mut len_line)?;
    let byte_count: usize = len_line.trim().parse()
        .map_err(|e| anyhow::anyhow!("bad response '{}': {}", len_line.trim(), e))?;

    if byte_count == 0 {
        bail!("wick-renderer returned 0 bytes for {}", url);
    }

    let mut html_buf = vec![0u8; byte_count];
    proc.stdout.read_exact(&mut html_buf)?;

    Ok(String::from_utf8_lossy(&html_buf).into_owned())
}

fn ensure_daemon(use_residential: bool) -> Result<()> {
    let mut daemon = DAEMON.lock().map_err(|e| anyhow::anyhow!("lock: {}", e))?;

    // Check if existing daemon is still alive AND in the requested
    // residential mode. The daemon is a process-wide singleton whose
    // residential tunnel is fixed at spawn (LD_PRELOAD), so reusing one
    // started in the other mode would silently route through the wrong exit
    // — e.g. a needs_residential site served over the datacenter IP it was
    // flagged as blocking. On a mode mismatch, kill and respawn.
    if let Some(ref mut d) = *daemon {
        match d.child.try_wait() {
            Ok(Some(_)) => { *daemon = None; }
            Ok(None) if d.use_residential == use_residential => return Ok(()),
            Ok(None) => {
                let _ = d.child.kill();
                *daemon = None;
            }
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

    // LD_LIBRARY_PATH. On Linux with a display, also include the multiarch
    // system lib dir (e.g. x86_64-linux-gnu, aarch64-linux-gnu) so Xvfb-dep
    // libraries resolve correctly regardless of architecture.
    let lib_path = match std::env::var("LD_LIBRARY_PATH") {
        Ok(existing) => {
            #[cfg(target_os = "linux")]
            {
                let mut parts = Vec::new();
                if std::env::var("DISPLAY").is_ok()
                    || std::path::Path::new("/tmp/.X99-lock").exists()
                {
                    if let Some(multiarch) = linux_multiarch_lib_dir() {
                        parts.push(multiarch.display().to_string());
                    }
                }
                parts.push(renderer_dir.display().to_string());
                parts.push(existing);
                parts.join(":")
            }
            #[cfg(not(target_os = "linux"))]
            format!("{}:{}", renderer_dir.display(), existing)
        }
        Err(_) => renderer_dir.display().to_string(),
    };
    cmd.env("LD_LIBRARY_PATH", &lib_path);

    if use_residential && wireguard_active() && std::path::Path::new(BINDWG_PATH).exists() {
        // Append rather than clobber any existing LD_PRELOAD.
        let preload = match std::env::var("LD_PRELOAD") {
            Ok(existing) if !existing.trim().is_empty() => {
                format!("{} {}", BINDWG_PATH, existing)
            }
            _ => BINDWG_PATH.to_string(),
        };
        cmd.env("LD_PRELOAD", preload);
    }

    let mut child = cmd.spawn().map_err(|e| {
        anyhow::anyhow!("failed to start daemon {:?}: {}", renderer_path, e)
    })?;

    let stdin = child.stdin.take()
        .ok_or_else(|| anyhow::anyhow!("daemon stdin unavailable"))?;
    let stdout = BufReader::new(
        child.stdout.take()
            .ok_or_else(|| anyhow::anyhow!("daemon stdout unavailable"))?
    );

    // Wait for CEF to initialize
    std::thread::sleep(Duration::from_secs(2));

    *daemon = Some(DaemonProcess { child, stdin, stdout, use_residential });
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

#[cfg(target_os = "linux")]
fn linux_multiarch_lib_dir() -> Option<PathBuf> {
    // Prefer dpkg-architecture when available (Debian/Ubuntu).
    if let Ok(output) = Command::new("dpkg-architecture")
        .arg("-qDEB_HOST_MULTIARCH")
        .output()
    {
        if output.status.success() {
            let triplet = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !triplet.is_empty() {
                let path = PathBuf::from(format!("/usr/lib/{}", triplet));
                if path.is_dir() {
                    return Some(path);
                }
            }
        }
    }

    // Fallback: scan /usr/lib for any *-linux-gnu dir.
    let entries = std::fs::read_dir("/usr/lib").ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.ends_with("-linux-gnu") {
                    return Some(path);
                }
            }
        }
    }
    None
}

fn wireguard_active() -> bool {
    #[cfg(target_os = "linux")]
    { std::path::Path::new("/sys/class/net/wg-wick").exists() }
    #[cfg(not(target_os = "linux"))]
    { false }
}
