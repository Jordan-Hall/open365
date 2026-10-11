//! Local MCP server (stdio). Agents drive documents through the command layer only
//! (ADR-002, ADR-007).
//!
//! Security model: paths are fenced to the `--root` folders; the session holds only the scopes
//! its operator granted (`scope`); destructive effects need the operator's confirmation; hidden content is left out of reads; every tool call writes one
//! JSON line to the audit log; request, document and response sizes are limited.

mod audit;
mod authz;
mod bounded;
mod fence;
pub mod policy;
pub mod registry;
mod resources;
pub mod scope;
mod server;
mod tools;
mod tools_ext;
mod tools_read;
pub mod views;
pub mod views_ext;

pub use fence::{Denied, Fence};
pub use scope::{
    Confirm, ConfirmKind, ConfirmRequest, Decision, DenyAll, OperatorFlag, Scope, TerminalConfirm,
};
pub use server::{
    Config, DEFAULT_DEADLINE, LEGACY_VERSIONS, MAX_COMMANDS, MAX_DOCUMENT_BYTES,
    MAX_OPEN_DOCUMENTS, MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES, MODERN_VERSION, Server,
};
pub use tools::definitions as tool_definitions;
