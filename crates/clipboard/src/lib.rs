//! Rich clipboard formats for Open365, pure Rust and free of platform code.
//!
//! * [`cfhtml`]: the Windows "HTML Format" container.
//! * [`writer_html`] and [`rtf`]: document fragments ([`RichText`]) to and from HTML and RTF.
//! * [`sheet`]: a copied cell range ([`SheetClip`]) to and from TSV and the Excel HTML table
//!   (values, number formats, fonts, fills, borders, alignment, merges).
//! * [`private`]: the lossless format used between Open365 windows.
//! * [`payload`]: the set of formats of one copy and the paste-side choice among them.
//!
//! Everything read from the clipboard is untrusted. Parsers are bounded by [`limits`], return
//! [`ParseError`] instead of panicking, never fetch a URL and never run script. See
//! `docs/security/threat-model.md` for the trust boundary.

pub mod cfhtml;
mod css;
mod html;
pub mod limits;
mod model;
pub mod payload;
pub mod private;
pub mod rtf;
pub mod sheet;
pub mod writer_html;

pub use model::{Align, CharFmt, ListKind, Para, ParseError, Rgb, RichText, Run, safe_link};
pub use payload::{
    ClipPayload, HTML_FORMAT_NAME, MemoryPayload, PayloadIo, RTF_FORMAT_NAME, Source, read_sheet,
    read_writer, sheet_payload, writer_payload,
};
pub use private::{FORMAT_NAME as PRIVATE_FORMAT_NAME, PrivateClip};
pub use sheet::{
    Borders, CellFont, CellKind, ClipCell, Edge, HAlign, Line, SheetClip, Underline, VAlign,
    parse_tsv,
};
