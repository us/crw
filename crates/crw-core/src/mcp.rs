//! Re-export shim. MCP JSON-RPC types now live in the `crw-mcp-proto` crate so
//! `crw-browse` can depend on them without pulling in `crw-core`'s HTTP stack.
//!
//! Existing `use crw_core::mcp::{...}` imports continue to work unchanged.

pub use crw_mcp_proto::*;
