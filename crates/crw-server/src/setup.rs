//! Interactive setup command that downloads LightPanda and creates a local config.

use std::env::consts::{ARCH, OS};
use std::path::PathBuf;

const LIGHTPANDA_BASE_URL: &str =
    "https://github.com/lightpanda-io/browser/releases/download/nightly";

/// Run the interactive setup: download LightPanda binary and create config.
pub async fn run_setup() {
    println!();
    let (os_label, arch_label, binary_name) = match (OS, ARCH) {
        ("linux", "x86_64") => ("Linux", "x86_64", "lightpanda-x86_64-linux"),
        ("macos", "aarch64") => ("macOS", "aarch64", "lightpanda-aarch64-macos"),
        _ => {
            eprintln!("  ✗ Unsupported platform: {OS} {ARCH}");
            eprintln!("    LightPanda provides binaries for Linux x86_64 and macOS aarch64.");
            std::process::exit(1);
        }
    };

    println!("  → Detected: {os_label} {arch_label}");

    // Download LightPanda binary.
    let install_dir = home_local_bin();
    let install_path = install_dir.join("lightpanda");

    println!("  → Downloading LightPanda...");

    let url = format!("{LIGHTPANDA_BASE_URL}/{binary_name}");
    let bytes = match download_binary(&url).await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("  ✗ Download failed: {e}");
            std::process::exit(1);
        }
    };

    if let Err(e) = std::fs::create_dir_all(&install_dir) {
        eprintln!("  ✗ Failed to create {}: {e}", install_dir.display());
        std::process::exit(1);
    }

    if let Err(e) = std::fs::write(&install_path, &bytes) {
        eprintln!("  ✗ Failed to write {}: {e}", install_path.display());
        std::process::exit(1);
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o755);
        if let Err(e) = std::fs::set_permissions(&install_path, perms) {
            eprintln!("  ✗ Failed to chmod +x: {e}");
            std::process::exit(1);
        }
    }

    println!("  ✓ Installed to {}", install_path.display());

    // Write config.local.toml if it doesn't exist.
    let config_path = PathBuf::from("config.local.toml");
    if config_path.exists() {
        println!("  ✓ config.local.toml already exists (skipped)");
    } else {
        let config_content = r#"[renderer]
mode = "lightpanda"

[renderer.lightpanda]
ws_url = "ws://127.0.0.1:9222/"
"#;
        if let Err(e) = std::fs::write(&config_path, config_content) {
            eprintln!("  ✗ Failed to write config.local.toml: {e}");
            std::process::exit(1);
        }
        println!("  ✓ Created config.local.toml");
    }

    println!();
    println!("  Start the server with JS rendering:");
    println!("    lightpanda serve --host 127.0.0.1 --port 9222 &");
    println!("    crw-server");
    println!();
}

async fn download_binary(url: &str) -> Result<Vec<u8>, reqwest::Error> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()?;

    let resp = client.get(url).send().await?.error_for_status()?;
    let bytes = resp.bytes().await?;
    Ok(bytes.to_vec())
}

fn home_local_bin() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".local").join("bin")
}
