//! Typed command layer: every mutation is a scoped, audited command. See ADR-002.
//!
//! The UI, the CLI and any future agent interface build a [`Command`] and hand it to
//! [`apply`]. The command declares its [`EffectClass`], so a permission layer can decide
//! before anything runs. Applied commands are appended to a JSON-serialisable log and
//! recorded on an undo/redo stack.
//!
//! Undo strategy: spreadsheet edits capture the previous cell (cheap) and recalculate on
//! undo. The DOCX model has no delete or paragraph-restore API yet, so a DOCX undo reopens
//! the original bytes and replays the remaining commands (always correct, O(edits)).

mod command;
mod error;
mod target;
mod xlsx_state;

pub use command::{CellEntry, CellRef, Command, CommandLog, EffectClass, Outcome};
pub use error::CommandError;
pub use target::{Target, apply};
