//! Cloud setup flow for CRW.

use crate::commands::setup::llm::{self, LlmSetupResult};
use crate::commands::setup::shell::{self, Shell, ShellConfig};
use crate::commands::setup::ui::{self, SetupError, SummaryItem};
use console::style;
use dialoguer::{Input, Select};
use serde::Deserialize;

const API_BASE_URL: &str = "https://api.fastcrw.com";
const DASHBOARD_URL: &str = "https://fastcrw.com/dashboard";

/// Account info returned from API key validation.
#[derive(Debug, Deserialize)]
struct AccountInfo {
    credits_remaining: Option<i64>,
    #[allow(dead_code)]
    email: Option<String>,
}

/// API key validation result.
#[derive(Debug)]
pub enum ApiKeyStatus {
    Valid { credits: i64 },
    Invalid,
    NetworkError(String),
}

/// Run the cloud setup flow.
pub async fn run() -> Result<(), SetupError> {
    ui::print_section_header("☁️", "CLOUD SETUP");

    // Step 1: Get CRW API key
    ui::print_step(1, 3, "Get your CRW API key");

    println!("  Visit: {}", style(DASHBOARD_URL).cyan().underlined());
    println!();
    println!("  1. Sign up (GitHub/Google, takes 10 seconds)");
    println!("  2. Copy your API key from the dashboard");
    println!();

    let api_key = get_api_key().await?;

    // Step 2: Configure LLM (optional)
    ui::print_step(2, 3, "Configure LLM (optional)");

    let llm_result = llm::run().await?;

    // Step 3: Save configuration
    ui::print_step(3, 3, "Save configuration");

    let shell = shell::detect_shell();
    ui::print_info(&format!("Detected shell: {}", shell));
    println!();

    let save_location = prompt_save_location(shell)?;

    match save_location {
        SaveLocation::ShellRc => {
            save_to_shell_rc(shell, &api_key, llm_result.as_ref())?;
        }
        SaveLocation::ConfigFile => {
            save_to_config_file(&api_key, llm_result.as_ref())?;
        }
        SaveLocation::Manual => {
            show_manual_instructions(&api_key, llm_result.as_ref());
        }
    }

    // Print configuration summary
    let summary_items = vec![
        SummaryItem::new("Cloud API", "Connected (fastcrw.com)", true),
        SummaryItem::new(
            "LLM Provider",
            llm_result
                .as_ref()
                .map(|l| l.provider.name())
                .unwrap_or("Not configured"),
            llm_result.is_some(),
        ),
        SummaryItem::new(
            "Config saved",
            match save_location {
                SaveLocation::ShellRc => "~/.zshrc",
                SaveLocation::ConfigFile => "~/.config/crw/config.toml",
                SaveLocation::Manual => "Manual (env vars)",
            },
            true,
        ),
    ];
    ui::print_summary("Configuration Summary", &summary_items);

    // Build quick start based on what was configured
    let mut quick_start = vec![
        "crw example.com              # Scrape a page",
        "crw search \"rust tutorials\"  # Web search",
    ];

    if llm_result.is_some() {
        quick_start.push("crw example.com -f summary   # AI-powered summary");
    }

    quick_start.push("crw --help                   # See all commands");

    // Print completion banner
    let source_cmd = shell::source_command(shell);
    ui::print_completion_banner(source_cmd.as_deref(), &quick_start, &[]);

    Ok(())
}

/// Get and validate API key from user.
async fn get_api_key() -> Result<String, SetupError> {
    loop {
        let api_key: String = Input::with_theme(&ui::select_style())
            .with_prompt("  Paste your API key")
            .validate_with(|input: &String| {
                if input.trim().is_empty() {
                    Err("API key cannot be empty")
                } else if !input.starts_with("fc-")
                    && !input.starts_with("sk-")
                    && !input.starts_with("crw_")
                {
                    Err("API key should start with 'fc-', 'sk-', or 'crw_'")
                } else {
                    Ok(())
                }
            })
            .interact_text()
            .map_err(ui::handle_dialoguer_error)?;

        let api_key = api_key.trim().to_string();

        // Validate API key
        print!("  ");
        match validate_api_key(&api_key).await {
            ApiKeyStatus::Valid { credits } => {
                ui::print_success(&format!(
                    "API key validated ({} credits remaining)",
                    credits
                ));
                println!();
                return Ok(api_key);
            }
            ApiKeyStatus::Invalid => {
                ui::print_error("Invalid API key");
                println!();
                println!("  The API key couldn't be verified. Try these steps:");
                println!("  1. Check for extra spaces (copy exactly from dashboard)");
                println!("  2. Ensure key hasn't been revoked");
                println!("  3. Check network: curl -I {}/health", API_BASE_URL);
                println!();

                let choice = Select::with_theme(&ui::select_style())
                    .with_prompt("  What would you like to do?")
                    .items(&[
                        "Try again",
                        "Get a new key (opens browser)",
                        "Continue anyway (key not verified)",
                    ])
                    .default(0)
                    .interact()
                    .map_err(ui::handle_dialoguer_error)?;

                match choice {
                    0 => continue,
                    1 => {
                        open_browser(DASHBOARD_URL);
                        continue;
                    }
                    2 => {
                        ui::print_warning("Continuing with unverified API key");
                        return Ok(api_key);
                    }
                    _ => unreachable!(),
                }
            }
            ApiKeyStatus::NetworkError(err) => {
                ui::print_warning(&format!("Could not verify API key: {}", err));
                println!();

                let choice = Select::with_theme(&ui::select_style())
                    .with_prompt("  What would you like to do?")
                    .items(&["Retry verification", "Continue anyway (key not verified)"])
                    .default(0)
                    .interact()
                    .map_err(ui::handle_dialoguer_error)?;

                match choice {
                    0 => continue,
                    1 => {
                        ui::print_warning("Continuing with unverified API key");
                        return Ok(api_key);
                    }
                    _ => unreachable!(),
                }
            }
        }
    }
}

/// Validate API key against the API.
async fn validate_api_key(key: &str) -> ApiKeyStatus {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(e) => return ApiKeyStatus::NetworkError(e.to_string()),
    };

    let resp = match client
        .get(format!("{}/v1/account", API_BASE_URL))
        .header("Authorization", format!("Bearer {}", key))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return ApiKeyStatus::NetworkError(e.to_string()),
    };

    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return ApiKeyStatus::Invalid;
    }

    if !resp.status().is_success() {
        return ApiKeyStatus::NetworkError(format!("HTTP {}", resp.status()));
    }

    match resp.json::<AccountInfo>().await {
        Ok(info) => ApiKeyStatus::Valid {
            credits: info.credits_remaining.unwrap_or(0),
        },
        Err(_) => {
            // If we got a 200 but can't parse, assume it's valid
            ApiKeyStatus::Valid { credits: -1 }
        }
    }
}

/// Where to save the configuration.
#[derive(Debug, Clone, Copy)]
enum SaveLocation {
    ShellRc,
    ConfigFile,
    Manual,
}

/// Prompt user for save location.
fn prompt_save_location(shell: Shell) -> Result<SaveLocation, SetupError> {
    let rc_file = shell::get_rc_file(shell);
    let rc_name = rc_file
        .as_ref()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("shell rc");

    let items = vec![
        format!("~/{} (recommended)", rc_name),
        "~/.config/crw/config.toml".to_string(),
        "Environment variable only (I'll set it myself)".to_string(),
    ];

    let choice = Select::with_theme(&ui::select_style())
        .with_prompt("  Where should I save your API key?")
        .items(&items)
        .default(0)
        .interact_opt()
        .map_err(ui::handle_dialoguer_error)?
        .ok_or(SetupError::Cancelled)?;

    Ok(match choice {
        0 => SaveLocation::ShellRc,
        1 => SaveLocation::ConfigFile,
        2 => SaveLocation::Manual,
        _ => unreachable!(),
    })
}

/// Save configuration to shell RC file.
fn save_to_shell_rc(
    shell: Shell,
    api_key: &str,
    llm_result: Option<&LlmSetupResult>,
) -> Result<(), String> {
    let mut config = ShellConfig::new();
    config.export("CRW_API_URL", API_BASE_URL);
    config.export("CRW_API_KEY", api_key);

    // Add LLM config if provided
    if let Some(llm) = llm_result {
        llm::add_to_shell_config(&mut config, llm);
    }

    let rc_path = shell::append_to_rc(shell, &config)?;

    ui::print_success(&format!("Added to {}:", rc_path.display()));
    println!("    export CRW_API_URL=\"{}\"", API_BASE_URL);
    println!(
        "    export CRW_API_KEY=\"{}...\"",
        &api_key[..std::cmp::min(8, api_key.len())]
    );

    if let Some(llm) = llm_result {
        println!(
            "    export CRW_EXTRACTION__LLM__PROVIDER=\"{}\"",
            llm.provider.config_value()
        );
        println!("    export CRW_EXTRACTION__LLM__MODEL=\"{}\"", llm.model);
    }

    println!();
    Ok(())
}

/// Write content to a file with secure permissions (0600 on Unix).
#[cfg(unix)]
fn write_secure(path: &std::path::PathBuf, content: &str) -> std::io::Result<()> {
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
fn write_secure(path: &std::path::PathBuf, content: &str) -> std::io::Result<()> {
    std::fs::write(path, content)
}

/// Mask an API key for display (show first 4 and last 4 chars).
fn mask_api_key(key: &str) -> String {
    if key.len() <= 12 {
        return "*".repeat(key.len());
    }
    format!("{}...{}", &key[..4], &key[key.len() - 4..])
}

/// Save configuration to config file.
fn save_to_config_file(api_key: &str, llm_result: Option<&LlmSetupResult>) -> Result<(), String> {
    let config_dir = shell::home_dir()
        .ok_or_else(|| "Could not determine home directory".to_string())?
        .join(".config")
        .join("crw");

    std::fs::create_dir_all(&config_dir)
        .map_err(|e| format!("Failed to create config directory: {}", e))?;

    let config_path = config_dir.join("config.toml");

    let mut content = format!(
        r#"# CRW Configuration
# Generated by crw setup

[api]
url = "{}"
key = "{}"
"#,
        API_BASE_URL, api_key
    );

    // Add LLM config if provided
    if let Some(llm) = llm_result {
        content.push('\n');
        content.push_str(&llm::generate_toml_config(llm));
    }

    write_secure(&config_path, &content)
        .map_err(|e| format!("Failed to write config file: {}", e))?;

    ui::print_success(&format!("Created {}", config_path.display()));
    println!();

    Ok(())
}

/// Show manual configuration instructions.
fn show_manual_instructions(api_key: &str, llm_result: Option<&LlmSetupResult>) {
    println!();
    println!("  Add these environment variables to your shell:");
    println!();
    println!("    export CRW_API_URL=\"{}\"", API_BASE_URL);
    println!("    export CRW_API_KEY=\"{}\"", mask_api_key(api_key));

    if let Some(llm) = llm_result {
        llm::show_manual_config(llm);
    } else {
        println!();
    }
}

/// Open URL in default browser.
fn open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/c", "start", url])
            .spawn();
    }
}
