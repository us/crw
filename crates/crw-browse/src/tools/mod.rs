//! Tool implementations live in this module. Each `#[tool]` method on
//! `CrwBrowse` (in `server.rs`) is a thin wrapper that delegates to the
//! `handle()` function in the corresponding submodule. This keeps `server.rs`
//! readable as a registry of available tools and lets each tool's logic +
//! tests sit alongside its input/output types.
//!
//! Why thin wrappers instead of pure delegation: rmcp's `#[tool_router]` macro
//! consumes a single `impl` block and generates the dispatch table from
//! `#[tool]`-decorated methods on it. Splitting tools into separate impls is
//! not supported by the macro — so the wrappers stay in `server.rs`.

pub mod click;
pub mod common;
pub mod console;
pub mod evaluate;
pub mod fill;
pub mod goto;
pub mod html;
pub mod network;
pub mod screenshot;
pub mod script;
pub mod storage;
pub mod text;
pub mod tree;
pub mod type_text;
pub mod wait;
