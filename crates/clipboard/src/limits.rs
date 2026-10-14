//! Size limits for clipboard data. Clipboard contents come from other programs and are
//! untrusted: every parser stops at these bounds and returns [`crate::ParseError`].

/// Largest HTML clipboard payload parsed, in bytes.
pub const MAX_HTML_BYTES: usize = 16 << 20;
/// Largest RTF clipboard payload parsed, in bytes.
pub const MAX_RTF_BYTES: usize = 16 << 20;
/// Largest private-format payload decoded, in bytes.
pub const MAX_PRIVATE_BYTES: usize = 32 << 20;
/// Largest plain-text payload parsed, in bytes.
pub const MAX_TEXT_BYTES: usize = 16 << 20;
/// Most HTML tokens (tags and text runs) read from one payload.
pub const MAX_TOKENS: usize = 1_000_000;
/// Deepest element or RTF group nesting.
pub const MAX_DEPTH: usize = 64;
/// Most paragraphs in a pasted document fragment.
pub const MAX_PARAS: usize = 200_000;
/// Most characters in a pasted document fragment.
pub const MAX_CHARS: usize = 16 << 20;
/// Most style rules read from a style block.
pub const MAX_CSS_RULES: usize = 5_000;
/// Most cells in a pasted range (matches the copy limit of the Sheets app).
pub const MAX_CELLS: usize = 2_000_000;
/// Worksheet limits.
pub const MAX_ROWS: usize = 1_048_576;
pub const MAX_COLS: usize = 16_384;
/// Longest cell text (the worksheet limit).
pub const MAX_CELL_CHARS: usize = 32_767;
/// Longest font name kept.
pub const MAX_FONT_NAME: usize = 64;
/// Longest number format code kept.
pub const MAX_FORMAT_CODE: usize = 255;
