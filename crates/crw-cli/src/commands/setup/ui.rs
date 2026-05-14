//! UI helpers for styled terminal output.

use console::{Style, Term, style};
use std::sync::atomic::{AtomicBool, Ordering};

/// Global flag for color output (can be disabled via --no-color or NO_COLOR env).
static NO_COLOR: AtomicBool = AtomicBool::new(false);

/// Initialize color settings based on environment and flags.
pub fn init_color(no_color_flag: bool) {
    let no_color = no_color_flag
        || std::env::var("NO_COLOR").is_ok()
        || std::env::var("TERM").map(|t| t == "dumb").unwrap_or(false);
    NO_COLOR.store(no_color, Ordering::SeqCst);
}

/// Check if colors are enabled.
pub fn colors_enabled() -> bool {
    !NO_COLOR.load(Ordering::SeqCst)
}

/// Custom error for setup operations.
#[derive(Debug)]
pub enum SetupError {
    /// User cancelled with Ctrl+C.
    Cancelled,
    /// Other error with message.
    Other(String),
}

impl std::fmt::Display for SetupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SetupError::Cancelled => write!(f, "Setup cancelled"),
            SetupError::Other(msg) => write!(f, "{}", msg),
        }
    }
}

impl From<String> for SetupError {
    fn from(s: String) -> Self {
        SetupError::Other(s)
    }
}

impl From<&str> for SetupError {
    fn from(s: &str) -> Self {
        SetupError::Other(s.to_string())
    }
}

/// Convert dialoguer error to SetupError, detecting Ctrl+C.
pub fn handle_dialoguer_error(e: dialoguer::Error) -> SetupError {
    // dialoguer::Error wraps std::io::Error
    let dialoguer::Error::IO(io_err) = e;

    // Check if it's an interrupt (Ctrl+C)
    if io_err.kind() == std::io::ErrorKind::Interrupted {
        return SetupError::Cancelled;
    }

    // For other errors, check the error message
    let msg = io_err.to_string();
    if msg.contains("interrupted")
        || msg.contains("Ctrl+C")
        || msg.contains("operation was canceled")
    {
        SetupError::Cancelled
    } else {
        SetupError::Other(format!("Input error: {}", msg))
    }
}

/// Print the setup cancelled message.
pub fn print_cancelled() {
    println!();
    println!(
        "  {} Setup cancelled. Run 'crw setup' to try again.",
        if colors_enabled() {
            style("→").blue().to_string()
        } else {
            "->".to_string()
        }
    );
    println!();
}

/// Print the setup wizard header.
pub fn print_header() {
    let term = Term::stdout();
    let _ = term.clear_screen();

    let version = env!("CARGO_PKG_VERSION");

    println!();
    if colors_enabled() {
        println!(
            "  {}",
            style("╭─────────────────────────────────────────────────────────╮").cyan()
        );
        println!(
            "  {}  {}{}",
            style("│").cyan(),
            style(format!("🦀 CRW Setup Wizard v{}", version)).bold(),
            style(format!("{}│", " ".repeat(35 - version.len()))).cyan()
        );
        println!(
            "  {}  {}{}",
            style("│").cyan(),
            style("The fastest web scraper for AI agents").dim(),
            style("                  │").cyan()
        );
        println!(
            "  {}",
            style("╰─────────────────────────────────────────────────────────╯").cyan()
        );
    } else {
        println!("  +-----------------------------------------------------------+");
        println!(
            "  |  CRW Setup Wizard v{}{}|",
            version,
            " ".repeat(35 - version.len())
        );
        println!("  |  The fastest web scraper for AI agents                   |");
        println!("  +-----------------------------------------------------------+");
    }
    println!();
}

/// Print a section header (e.g., CLOUD SETUP or LOCAL SETUP).
pub fn print_section_header(icon: &str, title: &str) {
    println!();
    if colors_enabled() {
        println!(
            "  {}",
            style("═══════════════════════════════════════════════════════════").cyan()
        );
        println!("  {}  {}", icon, style(title).bold().cyan());
        println!(
            "  {}",
            style("═══════════════════════════════════════════════════════════").cyan()
        );
    } else {
        println!("  ===========================================================");
        println!("  {}  {}", icon, title);
        println!("  ===========================================================");
    }
    println!();
}

/// Print a step header (e.g., "Step 1 of 4: Check Requirements").
pub fn print_step(num: u8, total: u8, title: &str) {
    let dashes = "─".repeat(40_usize.saturating_sub(title.len()));
    println!("  ─── Step {} of {}: {} {}", num, total, title, dashes);
    println!();
}

/// Print a success message with green checkmark.
pub fn print_success(msg: &str) {
    if colors_enabled() {
        println!("  {} {}", style("✓").green().bold(), msg);
    } else {
        println!("  [OK] {}", msg);
    }
}

/// Print an error message with red X.
pub fn print_error(msg: &str) {
    if colors_enabled() {
        println!("  {} {}", style("✗").red().bold(), msg);
    } else {
        println!("  [ERROR] {}", msg);
    }
}

/// Print a warning message with yellow triangle.
pub fn print_warning(msg: &str) {
    if colors_enabled() {
        println!("  {} {}", style("⚠").yellow().bold(), msg);
    } else {
        println!("  [WARN] {}", msg);
    }
}

/// Print an info message with blue arrow.
pub fn print_info(msg: &str) {
    if colors_enabled() {
        println!("  {} {}", style("→").blue(), msg);
    } else {
        println!("  -> {}", msg);
    }
}

/// Print a dimmed info line (indented sub-info).
pub fn print_detail(msg: &str) {
    if colors_enabled() {
        println!("    {} {}", style("└─").dim(), style(msg).dim());
    } else {
        println!("    |_ {}", msg);
    }
}

/// Print a box with information content.
pub fn print_info_box(lines: &[&str]) {
    let max_len = lines.iter().map(|l| l.len()).max().unwrap_or(50);
    let width = max_len + 4;

    println!();
    println!("  ┌{}┐", "─".repeat(width));
    for line in lines {
        println!("  │  {}{} │", line, " ".repeat(max_len - line.len()));
    }
    println!("  └{}┘", "─".repeat(width));
    println!();
}

/// Setup configuration summary item.
pub struct SummaryItem {
    pub label: String,
    pub value: String,
    pub enabled: bool,
}

impl SummaryItem {
    pub fn new(label: &str, value: &str, enabled: bool) -> Self {
        Self {
            label: label.to_string(),
            value: value.to_string(),
            enabled,
        }
    }
}

/// Print a setup summary before completion.
pub fn print_summary(title: &str, items: &[SummaryItem]) {
    println!();
    println!(
        "  {} {}",
        if colors_enabled() {
            style("📋").to_string()
        } else {
            "[i]".to_string()
        },
        if colors_enabled() {
            style(title).bold().to_string()
        } else {
            title.to_string()
        }
    );
    println!();

    for item in items {
        let status = if item.enabled {
            if colors_enabled() {
                style("✓").green().to_string()
            } else {
                "[+]".to_string()
            }
        } else {
            if colors_enabled() {
                style("○").dim().to_string()
            } else {
                "[-]".to_string()
            }
        };

        let value = if item.enabled {
            if colors_enabled() {
                style(&item.value).cyan().to_string()
            } else {
                item.value.clone()
            }
        } else {
            if colors_enabled() {
                style(&item.value).dim().to_string()
            } else {
                format!("({})", item.value)
            }
        };

        println!("    {} {}: {}", status, item.label, value);
    }
    println!();
}

/// Print the final success banner with next steps.
pub fn print_completion_banner(source_cmd: Option<&str>, quick_start: &[&str], extras: &[&str]) {
    println!();
    if colors_enabled() {
        println!(
            "  {}",
            style("════════════════════════════════════════════════════════════").green()
        );
        println!(
            "  {} {}",
            style("✓").green().bold(),
            style("Setup complete!").green().bold()
        );
    } else {
        println!("  ============================================================");
        println!("  [OK] Setup complete!");
    }
    println!();

    if let Some(cmd) = source_cmd {
        println!("  Run this to apply changes (or restart your terminal):");
        if colors_enabled() {
            println!("    {}", style(cmd).cyan());
        } else {
            println!("    {}", cmd);
        }
        println!();
    }

    if !quick_start.is_empty() {
        println!("  Quick start:");
        for line in quick_start {
            if colors_enabled() {
                println!("    {}", style(line).cyan());
            } else {
                println!("    {}", line);
            }
        }
        println!();
    }

    // Verification hint
    println!("  Verify your setup:");
    if colors_enabled() {
        println!("    {}", style("crw --version").cyan());
    } else {
        println!("    crw --version");
    }
    println!();

    if !extras.is_empty() {
        for line in extras {
            println!("  {}", line);
        }
        println!();
    }

    if colors_enabled() {
        println!(
            "  {}",
            style("════════════════════════════════════════════════════════════").green()
        );
    } else {
        println!("  ============================================================");
    }
    println!();
}

/// Style for select prompt items.
pub fn select_style() -> dialoguer::theme::ColorfulTheme {
    if colors_enabled() {
        dialoguer::theme::ColorfulTheme {
            active_item_style: Style::new().cyan().bold(),
            active_item_prefix: style("❯ ".to_string()).cyan().bold(),
            inactive_item_prefix: style("  ".to_string()),
            checked_item_prefix: style("  ✓ ".to_string()).green(),
            unchecked_item_prefix: style("    ".to_string()),
            ..Default::default()
        }
    } else {
        dialoguer::theme::ColorfulTheme {
            active_item_style: Style::new(),
            active_item_prefix: style("> ".to_string()),
            inactive_item_prefix: style("  ".to_string()),
            checked_item_prefix: style("  [x] ".to_string()),
            unchecked_item_prefix: style("  [ ] ".to_string()),
            ..Default::default()
        }
    }
}
