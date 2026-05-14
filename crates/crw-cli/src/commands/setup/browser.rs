//! Browser engine setup (LightPanda, Chrome).

use crate::commands::setup::shell::local_bin_dir;
use crate::commands::setup::ui;
use indicatif::{ProgressBar, ProgressStyle};
use sha2::{Digest, Sha256};
use std::env::consts::{ARCH, OS};
use std::path::PathBuf;

/// Find an executable in PATH using platform-specific command.
/// Returns the path if found and validated, None otherwise.
fn find_in_path(name: &str) -> Option<PathBuf> {
    // Use platform-specific command
    #[cfg(windows)]
    let cmd = "where";
    #[cfg(not(windows))]
    let cmd = "which";

    let output = std::process::Command::new(cmd).arg(name).output().ok()?;

    if !output.status.success() {
        return None;
    }

    let path_str = String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()? // Take first line (Windows `where` can return multiple)
        .trim()
        .to_string();

    if path_str.is_empty() {
        return None;
    }

    let path = PathBuf::from(&path_str);

    // Validate the path exists and is a file
    if path.exists() && path.is_file() {
        Some(path)
    } else {
        None
    }
}

const LIGHTPANDA_BASE_URL: &str =
    "https://github.com/lightpanda-io/browser/releases/download/nightly";

/// Browser engine choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserEngine {
    LightPanda,
    Chrome,
    None,
}

impl BrowserEngine {
    /// Display name for the browser engine.
    #[allow(dead_code)]
    pub fn name(&self) -> &'static str {
        match self {
            BrowserEngine::LightPanda => "LightPanda",
            BrowserEngine::Chrome => "Chrome/Chromium",
            BrowserEngine::None => "None (HTTP only)",
        }
    }
}

/// Platform support for LightPanda.
pub struct PlatformInfo {
    pub os_label: &'static str,
    pub arch_label: &'static str,
    pub binary_name: &'static str,
}

/// Get platform info for LightPanda download.
pub fn get_platform_info() -> Option<PlatformInfo> {
    match (OS, ARCH) {
        ("linux", "x86_64") => Some(PlatformInfo {
            os_label: "Linux",
            arch_label: "x86_64",
            binary_name: "lightpanda-x86_64-linux",
        }),
        ("macos", "aarch64") => Some(PlatformInfo {
            os_label: "macOS",
            arch_label: "aarch64 (Apple Silicon)",
            binary_name: "lightpanda-aarch64-macos",
        }),
        _ => None,
    }
}

/// Check if Chrome/Chromium is installed.
pub fn detect_chrome() -> Option<PathBuf> {
    let candidates = match OS {
        "macos" => vec![
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
        ],
        "linux" => vec![
            "/usr/bin/google-chrome",
            "/usr/bin/google-chrome-stable",
            "/usr/bin/chromium",
            "/usr/bin/chromium-browser",
        ],
        "windows" => vec![
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
        ],
        _ => vec![],
    };

    for path in candidates {
        let p = PathBuf::from(path);
        if p.exists() {
            return Some(p);
        }
    }

    // Also check PATH using platform-specific lookup
    if let Some(path) = find_in_path("google-chrome") {
        return Some(path);
    }
    if let Some(path) = find_in_path("chromium") {
        return Some(path);
    }
    if let Some(path) = find_in_path("chrome") {
        return Some(path);
    }

    None
}

/// Check if LightPanda is already installed.
pub fn detect_lightpanda() -> Option<PathBuf> {
    let local_path = local_bin_dir().join("lightpanda");
    if local_path.exists() && local_path.is_file() {
        return Some(local_path);
    }

    // Check PATH using platform-specific lookup
    find_in_path("lightpanda")
}

/// Download LightPanda with progress bar.
pub async fn download_lightpanda() -> Result<PathBuf, String> {
    let platform = get_platform_info().ok_or_else(|| {
        format!(
            "Unsupported platform: {} {}. LightPanda provides binaries for Linux x86_64 and macOS aarch64.",
            OS, ARCH
        )
    })?;

    ui::print_info(&format!(
        "Detected: {} {}",
        platform.os_label, platform.arch_label
    ));

    let install_dir = local_bin_dir();
    let install_path = install_dir.join("lightpanda");

    // Create install directory
    std::fs::create_dir_all(&install_dir)
        .map_err(|e| format!("Failed to create {}: {}", install_dir.display(), e))?;

    // Download binary
    let url = format!("{}/{}", LIGHTPANDA_BASE_URL, platform.binary_name);
    let bytes = download_with_progress(&url, "LightPanda").await?;

    // Verify checksum
    let actual_hash = sha256_hex(&bytes);
    let checksum_url = format!("{}.sha256", url);

    match download_checksum(&checksum_url).await {
        Ok(expected_hash) => {
            if actual_hash != expected_hash {
                return Err(format!(
                    "SHA256 checksum mismatch!\nExpected: {}\nActual: {}\nThe downloaded binary may be corrupted or tampered with.",
                    expected_hash, actual_hash
                ));
            }
            ui::print_success(&format!("SHA256 verified: {}...", &actual_hash[..12]));
        }
        Err(_) => {
            ui::print_warning(&format!(
                "No checksum file available, SHA256: {}...",
                &actual_hash[..12]
            ));
        }
    }

    // Write binary
    std::fs::write(&install_path, &bytes)
        .map_err(|e| format!("Failed to write {}: {}", install_path.display(), e))?;

    // Make executable on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o755);
        std::fs::set_permissions(&install_path, perms)
            .map_err(|e| format!("Failed to chmod +x: {}", e))?;
    }

    ui::print_success(&format!("Installed to {}", install_path.display()));

    Ok(install_path)
}

/// Download a file with progress bar.
async fn download_with_progress(url: &str, name: &str) -> Result<Vec<u8>, String> {
    use futures::StreamExt;

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch {}: {}", url, e))?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {} for {}", resp.status(), url));
    }

    let total_size = resp.content_length().unwrap_or(0);

    let pb = ProgressBar::new(total_size);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("    [{bar:40.cyan/blue}] {percent}% ({bytes}/{total_bytes})")
            .unwrap()
            .progress_chars("█▓░"),
    );
    pb.set_message(format!("Downloading {}", name));

    let mut bytes = Vec::new();
    let mut stream = resp.bytes_stream();

    while let Some(chunk_result) = stream.next().await {
        match chunk_result {
            Ok(chunk) => {
                bytes.extend_from_slice(&chunk);
                pb.set_position(bytes.len() as u64);
            }
            Err(e) => {
                pb.finish_and_clear();
                return Err(format!("Download error: {}", e));
            }
        }
    }

    pb.finish_and_clear();
    Ok(bytes)
}

/// Download and parse a checksum file.
async fn download_checksum(url: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|e| format!("Client build error: {}", e))?;

    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Fetch error: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let text = resp
        .text()
        .await
        .map_err(|e| format!("Read error: {}", e))?;

    // Parse checksum file: first token is the hex hash
    let hash = text
        .split_whitespace()
        .next()
        .ok_or_else(|| "Empty checksum file".to_string())?
        .to_lowercase();

    // Sanity check: should be 64 hex chars (SHA256)
    if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("Invalid checksum format: {}", hash));
    }

    Ok(hash)
}

/// Compute SHA256 hex digest.
fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_platform_info() {
        // This test is platform-dependent
        let info = get_platform_info();
        // On supported platforms, should return Some
        // On unsupported platforms, returns None
        let _ = info;
    }

    #[test]
    fn test_sha256_hex() {
        let hash = sha256_hex(b"hello world");
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
