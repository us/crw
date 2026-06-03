//! SearXNG-backed search client and result transforms.
//!
//! Public surface mirrors `crw-saas/src/lib/{searxng-client,search-params,search-transform}.ts`
//! so the SaaS layer can be reduced to a thin proxy over `/v1/search`.
//!
//! - [`client::SearxngClient`] — HTTP client wrapping `reqwest::Client`.
//! - [`params::map_to_searxng_params`] — translate a public [`SearchRequest`]
//!   into SearXNG query parameters.
//! - [`transform::transform_flat`] / [`transform::transform_grouped`] — turn
//!   a [`client::SearxngResponse`] into the user-facing result shape.
//!
//! [`SearchRequest`]: crw_core::types::SearchRequest

pub mod client;
pub mod params;
pub mod rerank;
pub mod structured;
pub mod transform;
pub mod wikidata;

pub use client::{SearchError, SearxngClient, SearxngResponse, SearxngResult};
pub use params::{SearxngParams, clean_query, map_to_searxng_params};
pub use rerank::{rerank, rerank_relevance};
pub use structured::{StructuredFact, structured_facts};
pub use transform::{transform_flat, transform_flat_reranked, transform_grouped};
