//! Core types, configuration, and error handling for the CRW web scraper.
//!
//! This crate provides the foundational building blocks shared across all CRW crates:
//!
//! - [`config`] — Layered TOML configuration with environment variable overrides
//! - [`error`] — Unified error types ([`CrwError`]) and result alias ([`CrwResult`])
//! - [`types`] — Shared data structures (`ScrapeData`, `FetchResult`, `OutputFormat`, etc.)
//! - [`url_safety`] — SSRF protection (blocks private IPs, cloud metadata, non-HTTP schemes)
//!
//! # Example
//!
//! ```rust
//! use crw_core::{AppConfig, CrwError, CrwResult};
//!
//! let config = AppConfig::load().unwrap();
//! assert!(config.server.port > 0);
//! ```

pub mod config;
pub mod error;
pub mod types;
pub mod url_safety;

pub use config::AppConfig;
pub use error::{CrwError, CrwResult};
