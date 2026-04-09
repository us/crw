//! Auto-detect and spawn a headless browser for JS rendering in embedded mode.
//!
//! Priority order:
//! 1. LightPanda binary (PATH or `~/.crw/lightpanda`, auto-downloaded if missing)
//! 2. Chrome/Chromium binary (heavier but widely available)
//! 3. LightPanda Docker container (last resort, requires Docker daemon)
//!
//! The spawned process/container is automatically cleaned up on drop.

use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

/// A managed browser process or Docker container.
/// Automatically cleaned up when dropped.
pub struct ManagedBrowser {
    kind: BrowserKind,
}

enum BrowserKind {
    /// A native process (LightPanda binary or Chrome).
    Process(Child),
    /// A Docker container, identified by its container ID.
    Docker(String),
}

impl Drop for ManagedBrowser {
    fn drop(&mut self) {
        match &mut self.kind {
            BrowserKind::Process(child) => {
                let _ = child.start_kill();
            }
            BrowserKind::Docker(container_id) => {
                // Best-effort stop + remove. Fire-and-forget.
                let _ = std::process::Command::new("docker")
                    .args(["rm", "-f", container_id])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn();
            }
        }
    }
}

/// Which renderer engine was spawned.
#[derive(Debug, Clone, Copy)]
pub enum RendererKind {
    LightPanda,
    Chrome,
}

/// Try to spawn a browser. Returns the managed handle + WS URL for CDP.
///
/// Tries in order: LightPanda native → Chrome native → LightPanda Docker.
pub async fn spawn_headless() -> Option<(ManagedBrowser, String)> {
    // 1. Try LightPanda native binary (PATH, ~/.crw/lightpanda, or auto-download).
    if let Some(result) = try_lightpanda_native().await {
        return Some(result);
    }

    // 2. Fallback to Chrome/Chromium native binary (widely available).
    if let Some(result) = try_chrome_native().await {
        return Some(result);
    }

    // 3. Last resort: LightPanda via Docker (requires Docker daemon).
    try_lightpanda_docker().await
}

/// Spawn all available browsers for a multi-renderer fallback chain.
///
/// Unlike `spawn_headless()` which returns the first browser found, this
/// function spawns every available browser so that `FallbackRenderer` can
/// try LightPanda first (fast, lightweight) and fall back to Chrome
/// (heavier but handles complex SPAs).
///
/// Docker is only tried if no native browser was found at all.
pub async fn spawn_all_headless() -> Vec<(ManagedBrowser, String, RendererKind)> {
    let mut browsers = Vec::new();

    // 1. Try LightPanda native (fast, lightweight).
    if let Some((guard, ws_url)) = try_lightpanda_native().await {
        browsers.push((guard, ws_url, RendererKind::LightPanda));
    }

    // 2. Also try Chrome/Chromium native (robust for complex SPAs).
    if let Some((guard, ws_url)) = try_chrome_native().await {
        browsers.push((guard, ws_url, RendererKind::Chrome));
    }

    // 3. Docker only if nothing native was found (last resort).
    if browsers.is_empty()
        && let Some((guard, ws_url)) = try_lightpanda_docker().await
    {
        browsers.push((guard, ws_url, RendererKind::LightPanda));
    }

    browsers
}

// --- LightPanda native ---

/// Find LightPanda binary: PATH → ~/.crw/lightpanda → auto-download.
async fn find_or_download_lightpanda() -> Option<String> {
    // 1. Check PATH.
    if let Some(bin) = find_in_path("lightpanda") {
        tracing::info!("Found LightPanda in PATH: {bin}");
        return Some(bin);
    }

    // 2. Check ~/.crw/lightpanda (our managed install location).
    let managed_path = lightpanda_managed_path()?;
    if managed_path.exists() {
        let path_str = managed_path.to_string_lossy().to_string();
        tracing::info!("Found managed LightPanda: {path_str}");
        return Some(path_str);
    }

    // 3. Auto-download from GitHub releases.
    let download_url = lightpanda_download_url()?;
    tracing::info!("Downloading LightPanda from {download_url}...");

    if let Some(parent) = managed_path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        tracing::warn!("Failed to create ~/.crw directory: {e}");
        return None;
    }

    let output = Command::new("curl")
        .args(["-fsSL", "-o"])
        .arg(managed_path.as_os_str())
        .arg(&download_url)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?
        .wait_with_output()
        .await
        .ok()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::warn!("Failed to download LightPanda: {stderr}");
        // Clean up partial download.
        let _ = std::fs::remove_file(&managed_path);
        return None;
    }

    // Make executable.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) =
            std::fs::set_permissions(&managed_path, std::fs::Permissions::from_mode(0o755))
        {
            tracing::warn!("Failed to chmod LightPanda binary: {e}");
            let _ = std::fs::remove_file(&managed_path);
            return None;
        }
    }

    let path_str = managed_path.to_string_lossy().to_string();
    tracing::info!("LightPanda downloaded to {path_str}");
    Some(path_str)
}

/// Get the managed install path: ~/.crw/lightpanda
fn lightpanda_managed_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".crw").join("lightpanda"))
}

/// Get the correct download URL for the current platform.
fn lightpanda_download_url() -> Option<String> {
    let base = "https://github.com/lightpanda-io/browser/releases/download/nightly";

    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some(format!("{base}/lightpanda-aarch64-macos")),
        ("linux", "x86_64") => Some(format!("{base}/lightpanda-x86_64-linux")),
        ("linux", "aarch64") => Some(format!("{base}/lightpanda-aarch64-linux")),
        (os, arch) => {
            tracing::debug!("No LightPanda binary available for {os}/{arch}");
            None
        }
    }
}

async fn try_lightpanda_native() -> Option<(ManagedBrowser, String)> {
    let bin = find_or_download_lightpanda().await?;

    // Find an available port for LightPanda.
    let port = find_available_port()?;
    let port_str = port.to_string();

    let child = Command::new(&bin)
        .args(["serve", "--host", "127.0.0.1", "--port", &port_str])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| tracing::warn!("Failed to spawn LightPanda: {e}"))
        .ok()?;

    // LightPanda doesn't print a WS URL to stderr like Chrome does.
    // Poll /json/version until it's ready (up to 5 seconds).
    let ws_url = poll_cdp_endpoint(port, 5).await?;
    tracing::info!("LightPanda CDP endpoint: {ws_url}");

    Some((
        ManagedBrowser {
            kind: BrowserKind::Process(child),
        },
        ws_url,
    ))
}

// --- LightPanda Docker ---

async fn try_lightpanda_docker() -> Option<(ManagedBrowser, String)> {
    // Check if Docker is available.
    if !command_exists("docker") {
        return None;
    }

    tracing::info!("Trying LightPanda via Docker...");

    // `docker run --rm -d -p 0:9222` → random host port mapped to 9222.
    let output = Command::new("docker")
        .args([
            "run",
            "--rm",
            "-d",
            "-p",
            "0:9222",
            "lightpanda/browser:latest",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?
        .wait_with_output()
        .await
        .ok()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::debug!("LightPanda Docker failed: {stderr}");
        return None;
    }

    let container_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if container_id.is_empty() {
        return None;
    }

    tracing::info!("LightPanda container started: {}", &container_id[..12]);

    // Get the mapped host port via `docker port`.
    let port = get_docker_mapped_port(&container_id, 9222).await?;

    // LightPanda needs a moment to start listening.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let ws_url = format!("ws://127.0.0.1:{port}/");
    tracing::info!("LightPanda Docker CDP endpoint: {ws_url}");

    Some((
        ManagedBrowser {
            kind: BrowserKind::Docker(container_id),
        },
        ws_url,
    ))
}

async fn get_docker_mapped_port(container_id: &str, container_port: u16) -> Option<u16> {
    let output = Command::new("docker")
        .args(["port", container_id, &container_port.to_string()])
        .output()
        .await
        .ok()?;

    if !output.status.success() {
        return None;
    }

    // Output format: "0.0.0.0:55000\n" or "0.0.0.0:55000\n:::55000\n"
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .next()?
        .rsplit(':')
        .next()?
        .trim()
        .parse()
        .ok()
}

// --- Chrome/Chromium native ---

const CHROME_CANDIDATES: &[&str] = &[
    // macOS
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
    "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
    // Linux
    "google-chrome",
    "google-chrome-stable",
    "chromium",
    "chromium-browser",
];

fn find_chrome() -> Option<String> {
    for candidate in CHROME_CANDIDATES {
        let found = if candidate.starts_with('/') {
            std::path::Path::new(candidate).exists()
        } else {
            find_in_path(candidate).is_some()
        };
        if found {
            return Some(candidate.to_string());
        }
    }
    None
}

async fn try_chrome_native() -> Option<(ManagedBrowser, String)> {
    let bin = find_chrome()?;
    tracing::info!("Auto-detected Chrome: {bin}");

    let mut child = Command::new(&bin)
        .args([
            "--headless",
            "--disable-gpu",
            "--no-sandbox",
            "--disable-dev-shm-usage",
            "--remote-debugging-port=0",
            "--remote-allow-origins=*",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| tracing::warn!("Failed to spawn Chrome: {e}"))
        .ok()?;

    let ws_url = read_ws_url_from_stderr(&mut child).await?;
    tracing::info!("Chrome CDP endpoint: {ws_url}");
    Some((
        ManagedBrowser {
            kind: BrowserKind::Process(child),
        },
        ws_url,
    ))
}

// --- Shared helpers ---

/// Find an available TCP port by binding to port 0.
fn find_available_port() -> Option<u16> {
    std::net::TcpListener::bind("127.0.0.1:0")
        .ok()
        .and_then(|l| l.local_addr().ok())
        .map(|a| a.port())
}

/// Poll a CDP endpoint's /json/version until it responds with a webSocketDebuggerUrl.
async fn poll_cdp_endpoint(port: u16, timeout_secs: u64) -> Option<String> {
    let url = format!("http://127.0.0.1:{port}/json/version");
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);

    while tokio::time::Instant::now() < deadline {
        if let Ok(resp) = reqwest::get(&url).await
            && let Ok(json) = resp.json::<serde_json::Value>().await
            && let Some(ws_url) = json.get("webSocketDebuggerUrl").and_then(|v| v.as_str())
        {
            return Some(ws_url.to_string());
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    None
}

/// Read the WebSocket URL from a browser's stderr output.
/// Chrome prints "DevTools listening on ws://...", LightPanda prints "Listening on ws://...".
async fn read_ws_url_from_stderr(child: &mut Child) -> Option<String> {
    let stderr = child.stderr.take()?;
    let mut reader = BufReader::new(stderr).lines();

    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while let Ok(Some(line)) = reader.next_line().await {
            // Chrome: "DevTools listening on ws://127.0.0.1:PORT/devtools/browser/UUID"
            if let Some(url) = line.strip_prefix("DevTools listening on ") {
                return Some(url.trim().to_string());
            }
            // LightPanda or other: "Listening on ws://..."
            if let Some(start) = line.find("ws://") {
                return Some(line[start..].trim().to_string());
            }
        }
        None
    })
    .await
    .ok()
    .flatten()
}

fn find_in_path(name: &str) -> Option<String> {
    std::process::Command::new("which")
        .arg(name)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|_| name.to_string())
}

fn command_exists(name: &str) -> bool {
    find_in_path(name).is_some()
}
