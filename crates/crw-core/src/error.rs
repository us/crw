use thiserror::Error;

#[derive(Debug, Error)]
pub enum CrwError {
    #[error("HTTP request failed: {0}")]
    HttpError(String),

    #[error("Target unreachable: {0}")]
    TargetUnreachable(String),

    #[error("URL parse error: {0}")]
    UrlParseError(#[from] url::ParseError),

    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    #[error("Renderer error: {0}")]
    RendererError(String),

    #[error("Extraction error: {0}")]
    ExtractionError(String),

    #[error("Crawl error: {0}")]
    CrawlError(String),

    #[error("Timeout after {0}ms")]
    Timeout(u64),

    #[error("Config error: {0}")]
    ConfigError(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Rate limited")]
    RateLimited,

    #[error("{0}")]
    Internal(String),
}

impl CrwError {
    /// Machine-readable error code for API consumers.
    pub fn error_code(&self) -> &'static str {
        match self {
            CrwError::HttpError(_) => "http_error",
            CrwError::TargetUnreachable(_) => "target_unreachable",
            CrwError::UrlParseError(_) => "invalid_url",
            CrwError::InvalidRequest(_) => "invalid_request",
            CrwError::RendererError(_) => "renderer_error",
            CrwError::ExtractionError(_) => "extraction_error",
            CrwError::CrawlError(_) => "crawl_error",
            CrwError::Timeout(_) => "timeout",
            CrwError::ConfigError(_) => "config_error",
            CrwError::NotFound(_) => "not_found",
            CrwError::RateLimited => "rate_limited",
            CrwError::Internal(_) => "internal_error",
        }
    }
}

pub type CrwResult<T> = Result<T, CrwError>;
