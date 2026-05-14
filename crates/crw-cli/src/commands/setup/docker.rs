//! Docker detection and container management.

use std::process::Command;

/// Docker installation status.
#[derive(Debug, Clone)]
pub enum DockerStatus {
    /// Docker is installed and running.
    Running { version: String },
    /// Docker is installed but not running.
    NotRunning { version: String },
    /// Docker is not installed.
    NotFound,
}

impl DockerStatus {
    /// Check if Docker is available and ready to use.
    pub fn is_ready(&self) -> bool {
        matches!(self, DockerStatus::Running { .. })
    }

    /// Check if Docker is installed (regardless of running state).
    #[allow(dead_code)]
    pub fn is_installed(&self) -> bool {
        !matches!(self, DockerStatus::NotFound)
    }

    /// Get the version string if available.
    #[allow(dead_code)]
    pub fn version(&self) -> Option<&str> {
        match self {
            DockerStatus::Running { version } | DockerStatus::NotRunning { version } => {
                Some(version)
            }
            DockerStatus::NotFound => None,
        }
    }
}

/// Check Docker installation and running status.
pub fn check_docker() -> DockerStatus {
    // First, check if docker command exists and get version
    let version_output = Command::new("docker").args(["--version"]).output();

    let version = match version_output {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        }
        _ => return DockerStatus::NotFound,
    };

    // Check if Docker daemon is running by running `docker info`
    let info_output = Command::new("docker").args(["info"]).output();

    match info_output {
        Ok(output) if output.status.success() => DockerStatus::Running { version },
        _ => DockerStatus::NotRunning { version },
    }
}

/// Get available disk space in GB (rough estimate).
pub fn get_available_disk_space() -> Option<u64> {
    // Use df command to get available disk space
    let output = Command::new("df").args(["-k", "/"]).output().ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Parse df output: Filesystem 1K-blocks Used Available Use% Mounted
    // Skip header line and get the 4th column (Available)
    let available_kb: u64 = stdout
        .lines()
        .nth(1)?
        .split_whitespace()
        .nth(3)?
        .parse()
        .ok()?;

    // Convert KB to GB
    Some(available_kb / (1024 * 1024))
}

/// Result of a container operation.
#[derive(Debug)]
#[allow(dead_code)]
pub enum ContainerResult {
    /// Container started successfully.
    Started { container_id: String },
    /// Container already exists and is running.
    AlreadyRunning { container_id: String },
    /// Container exists but was stopped, now started.
    Restarted { container_id: String },
    /// Failed to start container.
    Failed { error: String },
}

/// Check if a container with the given name exists.
pub fn container_exists(name: &str) -> bool {
    let output = Command::new("docker")
        .args([
            "ps",
            "-a",
            "--filter",
            &format!("name=^{}$", name),
            "--format",
            "{{.Names}}",
        ])
        .output();

    match output {
        Ok(out) if out.status.success() => !String::from_utf8_lossy(&out.stdout).trim().is_empty(),
        _ => false,
    }
}

/// Check if a container is running.
pub fn container_running(name: &str) -> bool {
    let output = Command::new("docker")
        .args([
            "ps",
            "--filter",
            &format!("name=^{}$", name),
            "--format",
            "{{.Names}}",
        ])
        .output();

    match output {
        Ok(out) if out.status.success() => !String::from_utf8_lossy(&out.stdout).trim().is_empty(),
        _ => false,
    }
}

/// Start an existing stopped container.
pub fn start_container(name: &str) -> Result<String, String> {
    let output = Command::new("docker")
        .args(["start", name])
        .output()
        .map_err(|e| format!("Failed to run docker start: {}", e))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// Stop a running container.
#[allow(dead_code)]
pub fn stop_container(name: &str) -> Result<(), String> {
    let output = Command::new("docker")
        .args(["stop", name])
        .output()
        .map_err(|e| format!("Failed to run docker stop: {}", e))?;

    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// Remove a container.
pub fn remove_container(name: &str) -> Result<(), String> {
    let output = Command::new("docker")
        .args(["rm", "-f", name])
        .output()
        .map_err(|e| format!("Failed to run docker rm: {}", e))?;

    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// Pull a Docker image.
pub fn pull_image(image: &str) -> Result<(), String> {
    let output = Command::new("docker")
        .args(["pull", image])
        .output()
        .map_err(|e| format!("Failed to run docker pull: {}", e))?;

    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// Run a new container.
pub fn run_container(
    name: &str,
    image: &str,
    port_mapping: Option<(&str, &str)>,
    env_vars: &[(&str, &str)],
    extra_args: &[&str],
) -> Result<String, String> {
    let mut cmd = Command::new("docker");
    cmd.args(["run", "-d", "--name", name]);

    // Add port mapping
    if let Some((host_port, container_port)) = port_mapping {
        cmd.arg("-p")
            .arg(format!("{}:{}", host_port, container_port));
    }

    // Add environment variables
    for (key, value) in env_vars {
        cmd.arg("-e").arg(format!("{}={}", key, value));
    }

    // Add extra args
    cmd.args(extra_args);

    // Add image name
    cmd.arg(image);

    let output = cmd
        .output()
        .map_err(|e| format!("Failed to run docker: {}", e))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// Wait for a container to be healthy (with timeout).
#[allow(dead_code)]
pub async fn wait_for_healthy(name: &str, timeout_secs: u64) -> Result<(), String> {
    use std::time::{Duration, Instant};
    use tokio::time::sleep;

    let start = Instant::now();
    let timeout = Duration::from_secs(timeout_secs);

    while start.elapsed() < timeout {
        if container_running(name) {
            // Check if the container's main process is responsive
            // For simplicity, just check if it's running
            return Ok(());
        }
        sleep(Duration::from_millis(500)).await;
    }

    Err(format!(
        "Container {} did not become healthy within {} seconds",
        name, timeout_secs
    ))
}

/// Get installation instructions for Docker based on OS.
pub fn docker_install_instructions() -> Vec<String> {
    let os = std::env::consts::OS;

    match os {
        "macos" => vec![
            "macOS:   brew install --cask docker".to_string(),
            "         or https://docker.com/get-started".to_string(),
        ],
        "linux" => vec!["Linux:   https://docs.docker.com/engine/install".to_string()],
        "windows" => vec!["Windows: https://docs.docker.com/desktop/windows".to_string()],
        _ => vec!["Visit https://docker.com/get-started".to_string()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_docker() {
        // This test depends on the environment
        let status = check_docker();
        // Just ensure it doesn't panic
        let _ = status.is_ready();
        let _ = status.is_installed();
    }
}
