//! Auto-detect and spawn a headless browser for JS rendering in embedded mode.
//!
//! Priority order:
//! 1. LightPanda binary (native, ~64MB, fastest startup)
//! 2. LightPanda Docker container (`lightpanda/browser:latest`)
//! 3. Chrome/Chromium binary (heavier but widely available)
//!
//! The spawned process/container is automatically cleaned up on drop.

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

/// Try to spawn a browser. Returns the managed handle + WS URL for CDP.
///
/// Tries in order: LightPanda native → LightPanda Docker → Chrome native.
pub async fn spawn_headless() -> Option<(ManagedBrowser, String)> {
    // 1. Try LightPanda native binary.
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

// --- LightPanda native ---

fn find_lightpanda() -> Option<String> {
    find_in_path("lightpanda")
}

async fn try_lightpanda_native() -> Option<(ManagedBrowser, String)> {
    let bin = find_lightpanda()?;
    tracing::info!("Auto-detected LightPanda binary: {bin}");

    let mut child = Command::new(&bin)
        .args(["--host", "127.0.0.1", "--port", "0"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| tracing::warn!("Failed to spawn LightPanda: {e}"))
        .ok()?;

    let ws_url = read_ws_url_from_stderr(&mut child).await?;
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
        if candidate.starts_with('/') {
            if std::path::Path::new(candidate).exists() {
                return Some(candidate.to_string());
            }
        } else if find_in_path(candidate).is_some() {
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
