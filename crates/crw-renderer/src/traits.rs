use async_trait::async_trait;
use crw_core::error::CrwResult;
use crw_core::types::FetchResult;

/// Trait for fetching page content, optionally with JS rendering.
#[async_trait]
pub trait PageFetcher: Send + Sync {
    /// Fetch a URL and return its HTML content.
    async fn fetch(
        &self,
        url: &str,
        headers: &std::collections::HashMap<String, String>,
        wait_for_ms: Option<u64>,
    ) -> CrwResult<FetchResult>;

    /// Human-readable name for this renderer.
    fn name(&self) -> &str;

    /// Whether this renderer supports JavaScript execution.
    fn supports_js(&self) -> bool;

    /// Check if the renderer is available / connected.
    async fn is_available(&self) -> bool;
}
