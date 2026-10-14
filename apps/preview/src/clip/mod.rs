//! Rich clipboard for the preview apps: the glue between the document models and
//! `open365-clipboard` (formats, parsers, serializers) and `open365-ui-host` (the system
//! clipboard).
//!
//! * [`writer`]: a Writer selection to a [`open365_clipboard::RichText`] for copy, and a pasted
//!   `RichText` to the text to insert plus the commands that restore its formatting.
//! * [`sheet`]: a Sheets range to a [`open365_clipboard::SheetClip`] for copy, and a pasted clip
//!   to the paste command plus the format commands, with Paste Special modes.
//!
//! Both are pure functions over the models and commands, so they run headless in tests. The
//! apps call them from their copy and paste handlers (see the hooks in the lane report).

pub mod sheet;
pub mod writer;

pub use open365_clipboard::{
    ClipPayload, MemoryPayload, PayloadIo, Source, read_sheet, read_writer, sheet_payload,
    writer_payload,
};
