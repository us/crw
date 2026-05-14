//! LLM provider setup for AI-powered features (summary, search answers, structured extraction).

use crate::commands::setup::shell::ShellConfig;
use crate::commands::setup::ui::{self, SetupError};
use console::style;
use dialoguer::{Input, Select};

/// Supported LLM providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmProvider {
    Anthropic,
    OpenAI,
    DeepSeek,
    Azure,
    OpenRouter,
    Custom,
    Skip,
}

impl LlmProvider {
    pub fn name(&self) -> &'static str {
        match self {
            LlmProvider::Anthropic => "Anthropic (Claude)",
            LlmProvider::OpenAI => "OpenAI (GPT)",
            LlmProvider::DeepSeek => "DeepSeek",
            LlmProvider::Azure => "Azure OpenAI",
            LlmProvider::OpenRouter => "OpenRouter",
            LlmProvider::Custom => "Custom (OpenAI-compatible)",
            LlmProvider::Skip => "Skip",
        }
    }

    pub fn config_value(&self) -> &'static str {
        match self {
            LlmProvider::Anthropic => "anthropic",
            LlmProvider::OpenAI => "openai",
            LlmProvider::DeepSeek => "deepseek",
            LlmProvider::Azure => "azure",
            LlmProvider::OpenRouter => "openai", // OpenRouter uses OpenAI protocol
            LlmProvider::Custom => "openai",     // Custom uses OpenAI protocol
            LlmProvider::Skip => "",
        }
    }

    pub fn default_base_url(&self) -> Option<&'static str> {
        match self {
            LlmProvider::Anthropic => None, // Uses default
            LlmProvider::OpenAI => None,    // Uses default
            LlmProvider::DeepSeek => Some("https://api.deepseek.com/v1"),
            LlmProvider::Azure => None, // User must provide
            LlmProvider::OpenRouter => Some("https://openrouter.ai/api/v1"),
            LlmProvider::Custom => None, // User must provide
            LlmProvider::Skip => None,
        }
    }

    pub fn default_models(&self) -> Vec<(&'static str, &'static str)> {
        match self {
            LlmProvider::Anthropic => vec![
                (
                    "claude-sonnet-4-20250514",
                    "Claude Sonnet 4 (Recommended, $3/$15 per M tokens)",
                ),
                (
                    "claude-haiku-4-5-20250514",
                    "Claude Haiku 4.5 (Fast & cheap, $1/$5 per M tokens)",
                ),
                (
                    "claude-opus-4-20250514",
                    "Claude Opus 4 (Most capable, $15/$75 per M tokens)",
                ),
            ],
            LlmProvider::OpenAI => vec![
                (
                    "gpt-4o-mini",
                    "GPT-4o Mini (Recommended, $0.15/$0.6 per M tokens)",
                ),
                ("gpt-4o", "GPT-4o (More capable, $2.5/$10 per M tokens)"),
                ("gpt-4-turbo", "GPT-4 Turbo"),
            ],
            LlmProvider::DeepSeek => vec![
                (
                    "deepseek-chat",
                    "DeepSeek Chat (Recommended, $0.27/$1.1 per M tokens)",
                ),
                (
                    "deepseek-reasoner",
                    "DeepSeek Reasoner ($0.55/$2.19 per M tokens)",
                ),
            ],
            LlmProvider::Azure => vec![
                ("gpt-4o-mini", "GPT-4o Mini deployment"),
                ("gpt-4o", "GPT-4o deployment"),
            ],
            LlmProvider::OpenRouter => vec![
                ("anthropic/claude-sonnet-4", "Claude Sonnet 4"),
                ("openai/gpt-4o-mini", "GPT-4o Mini"),
                ("deepseek/deepseek-chat", "DeepSeek Chat"),
                ("google/gemini-pro-1.5", "Gemini Pro 1.5"),
            ],
            LlmProvider::Custom => vec![],
            LlmProvider::Skip => vec![],
        }
    }

    #[allow(dead_code)]
    pub fn api_key_hint(&self) -> &'static str {
        match self {
            LlmProvider::Anthropic => "sk-ant-... (from console.anthropic.com)",
            LlmProvider::OpenAI => "sk-... (from platform.openai.com)",
            LlmProvider::DeepSeek => "sk-... (from platform.deepseek.com)",
            LlmProvider::Azure => "Azure API key (from Azure Portal)",
            LlmProvider::OpenRouter => "sk-or-... (from openrouter.ai)",
            LlmProvider::Custom => "Your API key",
            LlmProvider::Skip => "",
        }
    }

    pub fn dashboard_url(&self) -> Option<&'static str> {
        match self {
            LlmProvider::Anthropic => Some("https://console.anthropic.com/settings/keys"),
            LlmProvider::OpenAI => Some("https://platform.openai.com/api-keys"),
            LlmProvider::DeepSeek => Some("https://platform.deepseek.com/api_keys"),
            LlmProvider::Azure => Some("https://portal.azure.com"),
            LlmProvider::OpenRouter => Some("https://openrouter.ai/keys"),
            LlmProvider::Custom => None,
            LlmProvider::Skip => None,
        }
    }
}

/// LLM configuration result.
#[derive(Debug, Clone)]
pub struct LlmSetupResult {
    pub provider: LlmProvider,
    pub api_key: String,
    pub model: String,
    pub base_url: Option<String>,
    pub azure_api_version: Option<String>,
}

/// Run the LLM setup flow.
pub async fn run() -> Result<Option<LlmSetupResult>, SetupError> {
    println!();
    println!("  CRW supports AI-powered features for smarter scraping:");
    println!();
    println!(
        "  • {} - Generate concise summaries of pages",
        style("Summary").cyan()
    );
    println!(
        "  • {} - Answer questions using search results",
        style("Search Answers").cyan()
    );
    println!(
        "  • {} - Extract data into JSON schema",
        style("Structured Extraction").cyan()
    );
    println!();
    println!("  These features require an LLM API key (BYOK - Bring Your Own Key).");
    println!();

    // Provider selection
    let provider = prompt_provider()?;

    if provider == LlmProvider::Skip {
        ui::print_info("Skipping LLM setup (you can configure later)");
        return Ok(None);
    }

    // Show dashboard URL
    if let Some(url) = provider.dashboard_url() {
        println!();
        println!("  Get your API key: {}", style(url).cyan().underlined());
        println!();
    }

    // Get API key
    let api_key = prompt_api_key(provider)?;

    // Get model
    let model = prompt_model(provider)?;

    // Get base URL if needed
    let base_url = prompt_base_url(provider)?;

    // Get Azure API version if needed
    let azure_api_version = if provider == LlmProvider::Azure {
        Some(prompt_azure_version()?)
    } else {
        None
    };

    println!();
    ui::print_success(&format!("LLM configured: {} / {}", provider.name(), model));

    Ok(Some(LlmSetupResult {
        provider,
        api_key,
        model,
        base_url,
        azure_api_version,
    }))
}

/// Prompt for LLM provider selection.
fn prompt_provider() -> Result<LlmProvider, SetupError> {
    let items = vec![
        format!(
            "{}\n      • Best reasoning, great for summaries\n      • Models: Claude Sonnet 4, Haiku, Opus",
            style("Anthropic (Claude) - Recommended").bold()
        ),
        format!(
            "{}\n      • Wide compatibility, good pricing\n      • Models: GPT-4o, GPT-4o-mini",
            "OpenAI (GPT)"
        ),
        format!(
            "{}\n      • Best value, excellent performance\n      • Models: DeepSeek Chat, Reasoner",
            "DeepSeek"
        ),
        format!(
            "{}\n      • Enterprise deployments\n      • Requires Azure subscription",
            "Azure OpenAI"
        ),
        format!(
            "{}\n      • Access multiple providers with one key\n      • Models from OpenAI, Anthropic, Google, etc.",
            "OpenRouter"
        ),
        format!(
            "{}\n      • Any OpenAI-compatible API\n      • Ollama, vLLM, LocalAI, etc.",
            "Custom endpoint"
        ),
        format!(
            "{}\n      • Configure later or use without LLM features\n      • Basic scraping still works",
            style("Skip for now").dim()
        ),
    ];

    let providers = [
        LlmProvider::Anthropic,
        LlmProvider::OpenAI,
        LlmProvider::DeepSeek,
        LlmProvider::Azure,
        LlmProvider::OpenRouter,
        LlmProvider::Custom,
        LlmProvider::Skip,
    ];

    let choice = Select::with_theme(&ui::select_style())
        .with_prompt("  Which LLM provider would you like to use?")
        .items(&items)
        .default(0)
        .interact_opt()
        .map_err(ui::handle_dialoguer_error)?
        .ok_or(SetupError::Cancelled)?;

    Ok(providers[choice])
}

/// Prompt for API key.
fn prompt_api_key(provider: LlmProvider) -> Result<String, SetupError> {
    let api_key: String = Input::with_theme(&ui::select_style())
        .with_prompt(format!("  Enter your {} API key", provider.name()))
        .validate_with(|input: &String| {
            if input.trim().is_empty() {
                Err("API key cannot be empty")
            } else if input.trim().len() < 10 {
                Err("API key seems too short")
            } else {
                Ok(())
            }
        })
        .interact_text()
        .map_err(ui::handle_dialoguer_error)?;

    Ok(api_key.trim().to_string())
}

/// Prompt for model selection.
fn prompt_model(provider: LlmProvider) -> Result<String, SetupError> {
    let models = provider.default_models();

    if models.is_empty() {
        // Custom provider - ask for model name
        let model: String = Input::with_theme(&ui::select_style())
            .with_prompt("  Enter the model name")
            .default("gpt-4o-mini".to_string())
            .interact_text()
            .map_err(ui::handle_dialoguer_error)?;

        return Ok(model.trim().to_string());
    }

    let items: Vec<String> = models.iter().map(|(_, desc)| desc.to_string()).collect();

    let choice = Select::with_theme(&ui::select_style())
        .with_prompt("  Which model would you like to use?")
        .items(&items)
        .default(0)
        .interact_opt()
        .map_err(ui::handle_dialoguer_error)?
        .ok_or(SetupError::Cancelled)?;

    Ok(models[choice].0.to_string())
}

/// Prompt for base URL (for custom/Azure providers).
fn prompt_base_url(provider: LlmProvider) -> Result<Option<String>, SetupError> {
    // Return default URL if provider has one
    if let Some(url) = provider.default_base_url() {
        return Ok(Some(url.to_string()));
    }

    // Only ask for custom and Azure
    if provider != LlmProvider::Custom && provider != LlmProvider::Azure {
        return Ok(None);
    }

    let prompt = if provider == LlmProvider::Azure {
        "  Enter your Azure OpenAI endpoint (e.g., https://myresource.openai.azure.com)"
    } else {
        "  Enter the API base URL (e.g., http://localhost:11434/v1)"
    };

    let url: String = Input::with_theme(&ui::select_style())
        .with_prompt(prompt)
        .validate_with(|input: &String| {
            if input.trim().is_empty() {
                Err("URL cannot be empty")
            } else if !input.starts_with("http://") && !input.starts_with("https://") {
                Err("URL must start with http:// or https://")
            } else {
                Ok(())
            }
        })
        .interact_text()
        .map_err(ui::handle_dialoguer_error)?;

    Ok(Some(url.trim().to_string()))
}

/// Prompt for Azure API version.
fn prompt_azure_version() -> Result<String, SetupError> {
    let version: String = Input::with_theme(&ui::select_style())
        .with_prompt("  Enter the Azure API version")
        .default("2024-05-01-preview".to_string())
        .interact_text()
        .map_err(ui::handle_dialoguer_error)?;

    Ok(version.trim().to_string())
}

/// Add LLM configuration to shell config.
pub fn add_to_shell_config(config: &mut ShellConfig, result: &LlmSetupResult) {
    config.export(
        "CRW_EXTRACTION__LLM__PROVIDER",
        result.provider.config_value(),
    );
    config.export("CRW_EXTRACTION__LLM__API_KEY", &result.api_key);
    config.export("CRW_EXTRACTION__LLM__MODEL", &result.model);

    if let Some(ref url) = result.base_url {
        config.export("CRW_EXTRACTION__LLM__BASE_URL", url);
    }

    if let Some(ref version) = result.azure_api_version {
        config.export("CRW_EXTRACTION__LLM__AZURE_API_VERSION", version);
    }
}

/// Generate TOML config content for LLM.
pub fn generate_toml_config(result: &LlmSetupResult) -> String {
    let mut config = String::new();

    config.push_str("[extraction.llm]\n");
    config.push_str(&format!(
        "provider = \"{}\"\n",
        result.provider.config_value()
    ));
    config.push_str(&format!("api_key = \"{}\"\n", result.api_key));
    config.push_str(&format!("model = \"{}\"\n", result.model));

    if let Some(ref url) = result.base_url {
        config.push_str(&format!("base_url = \"{}\"\n", url));
    }

    if let Some(ref version) = result.azure_api_version {
        config.push_str(&format!("azure_api_version = \"{}\"\n", version));
    }

    config.push_str("max_tokens = 4096\n");
    config.push_str("max_concurrency = 4\n");

    config
}

/// Mask an API key for display (show first 4 and last 4 chars).
fn mask_api_key(key: &str) -> String {
    if key.len() <= 12 {
        return "*".repeat(key.len());
    }
    format!("{}...{}", &key[..4], &key[key.len() - 4..])
}

/// Show LLM configuration for manual setup.
pub fn show_manual_config(result: &LlmSetupResult) {
    println!();
    println!("  Add these environment variables to your shell:");
    println!();
    println!(
        "    export CRW_EXTRACTION__LLM__PROVIDER=\"{}\"",
        result.provider.config_value()
    );
    println!(
        "    export CRW_EXTRACTION__LLM__API_KEY=\"{}\"",
        mask_api_key(&result.api_key)
    );
    println!("    export CRW_EXTRACTION__LLM__MODEL=\"{}\"", result.model);

    if let Some(ref url) = result.base_url {
        println!("    export CRW_EXTRACTION__LLM__BASE_URL=\"{}\"", url);
    }

    if let Some(ref version) = result.azure_api_version {
        println!(
            "    export CRW_EXTRACTION__LLM__AZURE_API_VERSION=\"{}\"",
            version
        );
    }

    println!();
}
