//! Axum-based HTTP API server for the CRW web scraper.
//!
//! Implements the Firecrawl-compatible REST API and built-in MCP transport:
//!
//! - [`app`] — Application builder and router setup
//! - [`routes`] — API endpoint handlers (`/v1/scrape`, `/v1/crawl`, `/v1/map`, `/mcp`)
//! - [`middleware`] — Auth middleware with constant-time Bearer token comparison
//! - [`error`] — HTTP error responses
//! - [`state`] — Shared application state (renderer, crawler, config)
//!
//! # Example
//!
//! ```rust,ignore
//! use crw_server::app::create_app;
//! use crw_server::state::AppState;
//!
//! let state = AppState::new(config)?;
//! let app = create_app(state);
//! let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
//! axum::serve(listener, app).await?;
//! ```

pub mod app;
pub mod error;
pub mod middleware;
pub mod routes;
pub mod setup;
pub mod state;
