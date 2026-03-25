//! Media download via yt-dlp. Supports Reddit, YouTube, Twitter, and 1000+ sites.

use anyhow::{bail, Result};
use std::path::PathBuf;

/// Download media from a URL. Returns the path to the downloaded file.
pub async fn download(url: &str, output_dir: Option<&str>) -> Result<DownloadResult> {
    // Check yt-dlp is installed
    if !is_available() {
        bail!(
            "yt-dlp is required for media downloads.\n\
             Install: brew install yt-dlp (macOS) or pip install yt-dlp"
        );
    }

    let dir = output_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    std::fs::create_dir_all(&dir)?;

    // Use yt-dlp to download
    let output = tokio::process::Command::new("yt-dlp")
        .args([
            "--no-playlist",
            "--merge-output-format", "mp4",
            "-o", &format!("{}/%(title).100s.%(ext)s", dir.display()),
            "--print", "after_move:filepath",
            url,
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if !output.status.success() {
        bail!("yt-dlp failed: {}", stderr);
    }

    // The --print after_move:filepath gives us the final file path
    let filepath = stdout.lines().last().unwrap_or("").to_string();

    if filepath.is_empty() {
        bail!("yt-dlp succeeded but no file path returned");
    }

    // Get file info
    let metadata = std::fs::metadata(&filepath)?;
    let size_mb = metadata.len() as f64 / 1_048_576.0;

    Ok(DownloadResult {
        path: filepath,
        size_mb,
    })
}

/// Get video info without downloading.
pub async fn info(url: &str) -> Result<VideoInfo> {
    if !is_available() {
        bail!("yt-dlp is required. Install: brew install yt-dlp");
    }

    let output = tokio::process::Command::new("yt-dlp")
        .args([
            "--no-download",
            "--print", "%(title)s\n%(duration)s\n%(filesize_approx)s\n%(ext)s\n%(webpage_url)s",
            url,
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let lines: Vec<&str> = stdout.lines().collect();

    Ok(VideoInfo {
        title: lines.first().unwrap_or(&"Unknown").to_string(),
        duration_secs: lines.get(1).and_then(|s| s.parse().ok()),
        size_approx: lines.get(2).unwrap_or(&"Unknown").to_string(),
        format: lines.get(3).unwrap_or(&"mp4").to_string(),
        url: lines.get(4).unwrap_or(&url).to_string(),
    })
}

pub struct DownloadResult {
    pub path: String,
    pub size_mb: f64,
}

pub struct VideoInfo {
    pub title: String,
    pub duration_secs: Option<f64>,
    pub size_approx: String,
    pub format: String,
    pub url: String,
}

fn is_available() -> bool {
    std::process::Command::new("yt-dlp")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
