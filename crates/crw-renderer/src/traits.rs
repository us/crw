use async_trait::async_trait;
use crw_core::Deadline;
use crw_core::error::CrwResult;
use crw_core::types::FetchResult;

/// Trait for fetching page content, optionally with JS rendering.
#[async_trait]
pub trait PageFetcher: Send + Sync {
    /// Fetch a URL and return its HTML content.
    ///
    /// The `deadline` is the absolute end-of-budget for the request and is
    /// expected to be honored by every implementation: clamp internal timeouts
    /// against `deadline.remaining()` and bail out early if the deadline has
    /// already expired.
    async fn fetch(
        &self,
        url: &str,
        headers: &std::collections::HashMap<String, String>,
        wait_for_ms: Option<u64>,
        deadline: Deadline,
    ) -> CrwResult<FetchResult>;

    /// Human-readable name for this renderer.
    fn name(&self) -> &str;

    /// Whether this renderer supports JavaScript execution.
    fn supports_js(&self) -> bool;

    /// Check if the renderer is available / connected.
    async fn is_available(&self) -> bool;
}
