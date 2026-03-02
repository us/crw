use thiserror::Error;

#[derive(Debug, Error)]
pub enum CrwError {
    #[error("HTTP request failed: {0}")]
    HttpError(String),

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

    #[error("{0}")]
    Internal(String),
}

pub type CrwResult<T> = Result<T, CrwError>;
