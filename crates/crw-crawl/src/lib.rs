//! Async BFS web crawler with rate limiting and robots.txt support.
//!
//! This crate powers CRW's crawling capabilities:
//!
//! - [`crawl`] — BFS crawler with configurable depth, concurrency, and rate limiting
//! - [`single`] — Single-URL scrape (fetch + extract in one call)
//! - [`robots`] — robots.txt parser with wildcard patterns and RFC 9309 specificity
//! - [`sitemap`] — Sitemap.xml parser for URL discovery
//!
//! # Example
//!
//! ```rust,no_run
//! use crw_crawl::single;
//!
//! # async fn example() {
//! // Single-page scrape is the simplest entry point
//! // For full BFS crawling, see crawl::BfsCrawler
//! # }
//! ```

pub mod crawl;
pub mod robots;
pub mod single;
pub mod sitemap;
