//! SearXNG Docker setup for web search.

use crate::commands::setup::docker;
use crate::commands::setup::ui;
use indicatif::{ProgressBar, ProgressStyle};
use std::time::Duration;

pub const SEARXNG_IMAGE: &str = "searxng/searxng:latest";
pub const SEARXNG_CONTAINER_NAME: &str = "searxng";
pub const SEARXNG_DEFAULT_PORT: u16 = 8080;

/// SearXNG installation status.
#[derive(Debug)]
pub enum SearxngStatus {
    /// Container running and healthy.
    Running { url: String },
    /// Container exists but stopped.
    Stopped,
    /// Container doesn't exist.
    NotInstalled,
}

/// Check SearXNG container status.
pub fn check_status() -> SearxngStatus {
    if docker::container_running(SEARXNG_CONTAINER_NAME) {
        SearxngStatus::Running {
            url: format!("http://localhost:{}", SEARXNG_DEFAULT_PORT),
        }
    } else if docker::container_exists(SEARXNG_CONTAINER_NAME) {
        SearxngStatus::Stopped
    } else {
        SearxngStatus::NotInstalled
    }
}

/// Pull SearXNG Docker image with progress indicator.
pub async fn pull_image() -> Result<(), String> {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .template("    {spinner:.cyan} {msg}")
            .unwrap(),
    );
    pb.set_message("Pulling SearXNG image...");
    pb.enable_steady_tick(Duration::from_millis(100));

    // Run docker pull in a blocking task
    let result = tokio::task::spawn_blocking(|| docker::pull_image(SEARXNG_IMAGE)).await;

    pb.finish_and_clear();

    match result {
        Ok(Ok(())) => {
            ui::print_success("SearXNG image pulled");
            Ok(())
        }
        Ok(Err(e)) => Err(format!("Failed to pull image: {}", e)),
        Err(e) => Err(format!("Task error: {}", e)),
    }
}

/// Start or create SearXNG container.
pub async fn start_container() -> Result<String, String> {
    let status = check_status();

    match status {
        SearxngStatus::Running { url } => {
            ui::print_success(&format!("SearXNG already running at {}", url));
            return Ok(url);
        }
        SearxngStatus::Stopped => {
            ui::print_info("Starting existing SearXNG container...");
            docker::start_container(SEARXNG_CONTAINER_NAME)
                .map_err(|e| format!("Failed to start container: {}", e))?;
        }
        SearxngStatus::NotInstalled => {
            ui::print_info("Creating SearXNG container...");
            create_container()?;
        }
    }

    // Wait for container to be healthy
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .template("    {spinner:.cyan} {msg}")
            .unwrap(),
    );
    pb.set_message("Waiting for SearXNG to be ready...");
    pb.enable_steady_tick(Duration::from_millis(100));

    let result = wait_for_ready(30).await;
    pb.finish_and_clear();

    result?;

    let url = format!("http://localhost:{}", SEARXNG_DEFAULT_PORT);
    ui::print_success(&format!("SearXNG running at {}", url));
    Ok(url)
}

/// Create a new SearXNG container.
fn create_container() -> Result<String, String> {
    // Check if container already exists
    if docker::container_exists(SEARXNG_CONTAINER_NAME) {
        if docker::container_running(SEARXNG_CONTAINER_NAME) {
            return Err(format!(
                "Container '{}' is already running. Stop it first with: docker stop {}",
                SEARXNG_CONTAINER_NAME, SEARXNG_CONTAINER_NAME
            ));
        }
        // Container exists but stopped - remove it
        docker::remove_container(SEARXNG_CONTAINER_NAME)?;
    }

    let container_id = docker::run_container(
        SEARXNG_CONTAINER_NAME,
        SEARXNG_IMAGE,
        Some((&SEARXNG_DEFAULT_PORT.to_string(), "8080")),
        &[
            // SearXNG environment settings
            (
                "SEARXNG_BASE_URL",
                &format!("http://localhost:{}/", SEARXNG_DEFAULT_PORT),
            ),
        ],
        &[
            "--restart",
            "unless-stopped",
            // Resource limits for security
            "--memory",
            "512m",
            "--cpus",
            "1.0",
        ],
    )?;

    Ok(container_id)
}

/// Wait for SearXNG to be ready (HTTP health check).
async fn wait_for_ready(timeout_secs: u64) -> Result<(), String> {
    use std::time::Instant;
    use tokio::time::sleep;

    let start = Instant::now();
    let timeout = Duration::from_secs(timeout_secs);
    let url = format!("http://localhost:{}/", SEARXNG_DEFAULT_PORT);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    while start.elapsed() < timeout {
        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() || resp.status().is_redirection() => {
                return Ok(());
            }
            _ => {
                sleep(Duration::from_millis(500)).await;
            }
        }
    }

    Err(format!(
        "SearXNG did not become ready within {} seconds. You can check logs with: docker logs {}",
        timeout_secs, SEARXNG_CONTAINER_NAME
    ))
}

/// Stop SearXNG container.
#[allow(dead_code)]
pub fn stop() -> Result<(), String> {
    docker::stop_container(SEARXNG_CONTAINER_NAME)
}

/// Remove SearXNG container completely.
#[allow(dead_code)]
pub fn remove() -> Result<(), String> {
    docker::remove_container(SEARXNG_CONTAINER_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_status() {
        // This test depends on Docker being available
        let status = check_status();
        // Just make sure it doesn't panic
        match status {
            SearxngStatus::Running { .. } => {}
            SearxngStatus::Stopped => {}
            SearxngStatus::NotInstalled => {}
        }
    }
}
