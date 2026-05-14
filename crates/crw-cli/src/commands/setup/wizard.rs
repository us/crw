//! Main setup wizard flow.

use crate::commands::setup::ui::SetupError;
use crate::commands::setup::{cloud, local, ui};
use dialoguer::Select;

/// Setup mode choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupMode {
    Cloud,
    Local,
}

/// Run the interactive setup wizard.
pub async fn run_wizard() -> Result<(), SetupError> {
    ui::print_header();

    let mode = select_setup_mode()?;

    match mode {
        SetupMode::Cloud => cloud::run().await,
        SetupMode::Local => local::run().await,
    }
}

/// Prompt user to select Cloud or Local setup.
fn select_setup_mode() -> Result<SetupMode, SetupError> {
    println!("  How would you like to use CRW?");
    println!();

    let items = vec![
        "☁️  Cloud (Recommended for getting started)\n        • 500 free credits, no payment needed\n        • Zero setup, works instantly\n        • Managed infrastructure, always up-to-date",
        "🏠 Local (Self-hosted)\n        • Unlimited usage, completely free\n        • Full control over your data\n        • Requires: Docker (~1.5GB for images)",
    ];

    let selection = Select::with_theme(&ui::select_style())
        .items(&items)
        .default(0)
        .interact_opt()
        .map_err(ui::handle_dialoguer_error)?
        .ok_or(SetupError::Cancelled)?;

    Ok(match selection {
        0 => SetupMode::Cloud,
        1 => SetupMode::Local,
        _ => unreachable!(),
    })
}
