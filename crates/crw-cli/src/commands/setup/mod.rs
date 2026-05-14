//! Interactive setup wizard for CRW.
//!
//! Guides users through Cloud or Local installation with clear
//! explanations at each step.

mod browser;
mod cloud;
mod docker;
mod llm;
mod local;
mod searxng;
mod shell;
pub mod ui;
mod wizard;

use clap::Args;

/// Setup command arguments.
#[derive(Args)]
pub struct SetupArgs {
    /// Skip interactive prompts and use defaults (for scripting).
    #[arg(long)]
    pub non_interactive: bool,

    /// Force cloud setup mode.
    #[arg(long, conflicts_with = "local")]
    pub cloud: bool,

    /// Force local setup mode.
    #[arg(long, conflicts_with = "cloud")]
    pub local: bool,

    /// Disable colored output.
    #[arg(long)]
    pub no_color: bool,
}

/// Run the setup command.
pub async fn run(args: SetupArgs) {
    // Initialize color settings
    ui::init_color(args.no_color);

    // If specific mode is requested, run that directly
    let result = if args.cloud {
        cloud::run().await
    } else if args.local {
        local::run().await
    } else {
        // Interactive wizard
        wizard::run_wizard().await
    };

    match result {
        Ok(()) => {}
        Err(e) => {
            // Check if it was a cancellation
            if let ui::SetupError::Cancelled = e {
                ui::print_cancelled();
                std::process::exit(130); // Standard exit code for Ctrl+C
            } else {
                eprintln!();
                eprintln!("  Setup failed: {}", e);
                eprintln!();
                std::process::exit(1);
            }
        }
    }
}
