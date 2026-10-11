//! Local MCP server (stdio). Agents drive documents through the command layer only
//! (ADR-002, ADR-007).
//!
//! Security model: paths are fenced to the `--root` folders; the server is read-only unless
//! edit is allowed; destructive and egress effects need an explicit flag (there is no native
//! confirmation UI yet); hidden content is left out of reads; every tool call writes one
//! JSON line to the audit log; request, document and response sizes are limited.

mod audit;
mod fence;
mod server;
mod tools;
mod tools_ext;
mod views;

pub use fence::{Denied, Fence};
pub use server::{
    Config, MAX_COMMANDS, MAX_DOCUMENT_BYTES, MAX_OPEN_DOCUMENTS, MAX_REQUEST_BYTES,
    MAX_RESPONSE_BYTES, Server,
};
pub use tools::definitions as tool_definitions;
