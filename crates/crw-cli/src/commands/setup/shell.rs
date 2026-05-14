//! Shell detection and RC file configuration.

use std::env;
use std::fs;
use std::path::PathBuf;

/// Supported shells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
    Unknown,
}

impl Shell {
    /// Human-readable name for the shell.
    pub fn name(&self) -> &'static str {
        match self {
            Shell::Bash => "bash",
            Shell::Zsh => "zsh",
            Shell::Fish => "fish",
            Shell::Unknown => "unknown",
        }
    }
}

impl std::fmt::Display for Shell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// Detect the current user's shell.
pub fn detect_shell() -> Shell {
    // First, check $SHELL env var
    if let Ok(shell_path) = env::var("SHELL") {
        let basename = std::path::Path::new(&shell_path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");

        return match basename {
            "bash" => Shell::Bash,
            "zsh" => Shell::Zsh,
            "fish" => Shell::Fish,
            _ => Shell::Unknown,
        };
    }

    // Fallback: check common shell paths
    if std::path::Path::new("/bin/zsh").exists() || std::path::Path::new("/usr/bin/zsh").exists() {
        return Shell::Zsh;
    }

    if std::path::Path::new("/bin/bash").exists() || std::path::Path::new("/usr/bin/bash").exists()
    {
        return Shell::Bash;
    }

    Shell::Unknown
}

/// Get the RC file path for a given shell.
pub fn get_rc_file(shell: Shell) -> Option<PathBuf> {
    let home = home_dir()?;

    match shell {
        Shell::Zsh => Some(home.join(".zshrc")),
        Shell::Bash => {
            // Prefer .bashrc, but use .bash_profile on macOS if .bashrc doesn't exist
            let bashrc = home.join(".bashrc");
            let bash_profile = home.join(".bash_profile");

            if bashrc.exists() {
                Some(bashrc)
            } else if bash_profile.exists() {
                Some(bash_profile)
            } else {
                // Default to .bashrc
                Some(bashrc)
            }
        }
        Shell::Fish => Some(home.join(".config").join("fish").join("config.fish")),
        Shell::Unknown => None,
    }
}

/// Get the home directory.
pub fn home_dir() -> Option<PathBuf> {
    env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .ok()
        .map(PathBuf::from)
}

/// Get ~/.local/bin directory (creating if needed).
pub fn local_bin_dir() -> PathBuf {
    let home = home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".local").join("bin")
}

/// Configuration entry to add to shell RC file.
#[derive(Clone)]
pub struct ShellConfig {
    /// Comment header for the config block.
    pub header: String,
    /// Lines to add (each line is a full shell command/export).
    pub lines: Vec<String>,
}

impl ShellConfig {
    /// Create a new shell config for CRW.
    pub fn new() -> Self {
        Self {
            header: "CRW Configuration (added by crw setup)".to_string(),
            lines: Vec::new(),
        }
    }

    /// Add an export line.
    pub fn export(&mut self, key: &str, value: &str) -> &mut Self {
        self.lines.push(format!("export {}=\"{}\"", key, value));
        self
    }

    /// Add a PATH modification.
    pub fn add_to_path(&mut self, path: &str) -> &mut Self {
        self.lines.push(format!("export PATH=\"{}:$PATH\"", path));
        self
    }

    /// Generate the shell config block as a string.
    pub fn generate(&self, shell: Shell) -> String {
        let comment_prefix = match shell {
            Shell::Fish => "#",
            _ => "#",
        };

        let mut output = String::new();
        output.push('\n');
        output.push_str(&format!("{} {}\n", comment_prefix, self.header));

        for line in &self.lines {
            // Convert to fish syntax if needed
            let converted = if shell == Shell::Fish {
                convert_to_fish(line)
            } else {
                line.clone()
            };
            output.push_str(&converted);
            output.push('\n');
        }

        output
    }

    /// Check if the config is already present in a file.
    #[allow(dead_code)]
    pub fn is_present_in(&self, content: &str) -> bool {
        // Check if ALL lines are already present (line-based matching)
        let content_lines: Vec<&str> = content.lines().map(|l| l.trim()).collect();
        self.lines.iter().all(|line| {
            let trimmed = line.trim();
            content_lines.contains(&trimmed)
        })
    }

    /// Filter out lines that are already present in content.
    /// Uses line-based matching to avoid substring false positives.
    pub fn filter_existing(&mut self, content: &str) {
        let content_lines: Vec<&str> = content.lines().map(|l| l.trim()).collect();
        self.lines.retain(|line| {
            let trimmed = line.trim();
            // Check if this exact line already exists
            !content_lines.contains(&trimmed)
        });
    }
}

/// Convert bash export syntax to fish set syntax.
fn convert_to_fish(line: &str) -> String {
    if let Some(rest) = line.strip_prefix("export ")
        && let Some((key, value)) = rest.split_once('=')
    {
        let value = value.trim_matches('"');
        // Handle PATH specially
        if key == "PATH" && value.contains("$PATH") {
            let new_path = value.replace(":$PATH", "").replace("$PATH:", "");
            return format!("fish_add_path {}", new_path);
        }
        return format!("set -gx {} {}", key, value);
    }
    line.to_string()
}

/// Write content to a file with secure permissions (0600 on Unix).
#[cfg(unix)]
fn write_secure(path: &PathBuf, content: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600) // Owner read/write only
        .open(path)?;

    let mut writer = std::io::BufWriter::new(file);
    writer.write_all(content.as_bytes())?;
    Ok(())
}

#[cfg(not(unix))]
fn write_secure(path: &PathBuf, content: &str) -> std::io::Result<()> {
    std::fs::write(path, content)
}

/// Append configuration to a shell RC file (idempotent).
pub fn append_to_rc(shell: Shell, config: &ShellConfig) -> Result<PathBuf, String> {
    let rc_path =
        get_rc_file(shell).ok_or_else(|| "Could not determine RC file path".to_string())?;

    // Read existing content
    let existing = if rc_path.exists() {
        fs::read_to_string(&rc_path)
            .map_err(|e| format!("Failed to read {}: {}", rc_path.display(), e))?
    } else {
        // Create parent directories if needed
        if let Some(parent) = rc_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create {}: {}", parent.display(), e))?;
        }
        String::new()
    };

    // Filter out lines that already exist (idempotent)
    let mut config = config.clone();
    config.filter_existing(&existing);

    // If all lines already exist, nothing to do
    if config.lines.is_empty() {
        return Ok(rc_path);
    }

    // Append only the new lines with secure permissions
    let new_content = format!("{}{}", existing, config.generate(shell));
    write_secure(&rc_path, &new_content)
        .map_err(|e| format!("Failed to write {}: {}", rc_path.display(), e))?;

    Ok(rc_path)
}

/// Get the source command for applying RC file changes.
pub fn source_command(shell: Shell) -> Option<String> {
    let rc_path = get_rc_file(shell)?;
    let rc_str = rc_path.to_str()?;

    match shell {
        Shell::Fish => Some(format!("source {}", rc_str)),
        _ => Some(format!("source {}", rc_str)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_shell() {
        // This will depend on the test environment
        let shell = detect_shell();
        assert!(matches!(
            shell,
            Shell::Bash | Shell::Zsh | Shell::Fish | Shell::Unknown
        ));
    }

    #[test]
    fn test_shell_config_generate() {
        let mut config = ShellConfig::new();
        config.export("CRW_API_KEY", "test-key");
        config.add_to_path("$HOME/.local/bin");

        let output = config.generate(Shell::Bash);
        assert!(output.contains("export CRW_API_KEY=\"test-key\""));
        assert!(output.contains("export PATH=\"$HOME/.local/bin:$PATH\""));
    }

    #[test]
    fn test_convert_to_fish() {
        assert_eq!(convert_to_fish("export FOO=\"bar\""), "set -gx FOO bar");
        assert_eq!(
            convert_to_fish("export PATH=\"$HOME/.local/bin:$PATH\""),
            "fish_add_path $HOME/.local/bin"
        );
    }
}
