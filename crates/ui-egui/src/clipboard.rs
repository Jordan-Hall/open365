//! Copy, cut and paste for both document kinds.
//!
//! Word selections go out as plain text and HTML; plain text comes in. Excel ranges go out as TSV
//! and HTML; TSV comes in. A range this application copied itself is remembered, so pasting it
//! keeps formulas and shifts their relative references as Excel does; text from elsewhere is
//! entered as typed. The operating system's clipboard sits behind [`ClipboardBackend`] so tests
//! can read what was copied.

use std::cell::RefCell;
use std::rc::Rc;

use open365_commands::{Command, GridClip, Target, docx_clip, grid_copy, parse_tsv, paste_rows};

use crate::doc_view::DocView;
use crate::grid_view::GridView;

/// The most cells one copy carries.
const MAX_COPY_CELLS: usize = 1_000_000;

/// Where copied data goes.
pub trait ClipboardBackend: std::fmt::Debug {
    /// Puts `text` and `html` on the clipboard together.
    fn set(&mut self, text: &str, html: &str);
}

/// The operating system's clipboard (plain text and HTML formats together).
#[derive(Debug, Default)]
pub struct SystemClipboard;

impl ClipboardBackend for SystemClipboard {
    fn set(&mut self, text: &str, html: &str) {
        // Windows applications expect CR LF in plain text.
        let text = if cfg!(windows) {
            text.replace("\r\n", "\n").replace('\n', "\r\n")
        } else {
            text.to_owned()
        };
        if let Ok(mut board) = arboard::Clipboard::new() {
            let _ = board.set().html(html, Some(text.as_str()));
        }
    }
}

/// What a [`MemoryClipboard`] holds.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ClipboardContents {
    pub text: String,
    pub html: String,
}

/// A clipboard in memory, for tests.
#[derive(Debug, Default, Clone)]
pub struct MemoryClipboard(pub Rc<RefCell<ClipboardContents>>);

impl ClipboardBackend for MemoryClipboard {
    fn set(&mut self, text: &str, html: &str) {
        *self.0.borrow_mut() = ClipboardContents {
            text: text.to_owned(),
            html: html.to_owned(),
        };
    }
}

fn unix_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// The clipboard as the application uses it.
#[derive(Debug)]
pub struct ClipboardState {
    backend: Box<dyn ClipboardBackend>,
    /// The range this application copied last, to keep formulas on paste.
    grid: Option<GridClip>,
}

impl Default for ClipboardState {
    fn default() -> Self {
        Self::new(Box::<SystemClipboard>::default())
    }
}

impl ClipboardState {
    pub fn new(backend: Box<dyn ClipboardBackend>) -> Self {
        Self {
            backend,
            grid: None,
        }
    }

    /// Copies the Word selection; false when nothing is selected.
    pub fn copy_doc(&mut self, target: &Target, doc: &DocView) -> bool {
        let (Some(d), true) = (target.docx(), doc.has_selection()) else {
            return false;
        };
        let (start, end) = doc.selection();
        let clip = docx_clip(d, start, end);
        self.grid = None;
        self.backend.set(&clip.text, &clip.html);
        true
    }

    /// Copies the selected range; false when it is too large to copy.
    pub fn copy_grid(&mut self, target: &Target, grid: &GridView) -> bool {
        let Some(wb) = target.xlsx() else {
            return false;
        };
        let (r1, c1, r2, c2) = grid.selection();
        let Some(clip) = grid_copy(wb, grid.sheet(), (r1, c1), (r2, c2), MAX_COPY_CELLS) else {
            return false;
        };
        self.backend.set(&clip.tsv, &clip.html);
        self.grid = Some(clip);
        true
    }

    /// Cuts the selected range: copies it, then clears it in one undoable step.
    pub fn cut_grid(&mut self, target: &mut Target, grid: &mut GridView) -> String {
        if !self.copy_grid(target, grid) {
            return "That range is too large to cut".to_owned();
        }
        let (r1, c1, r2, c2) = grid.selection();
        let rows = vec![vec![String::new(); (c2 - c1 + 1) as usize]; (r2 - r1 + 1) as usize];
        let command = Command::XlsxPaste {
            sheet: grid.sheet(),
            row: r1,
            col: c1,
            rows,
        };
        match target.apply(command) {
            Ok(_) => {
                grid.invalidate_layout();
                "Cut".to_owned()
            }
            Err(e) => format!("Cut failed: {e}"),
        }
    }

    /// Pastes tab-separated text at the top-left cell of the selection.
    pub fn paste_grid(&mut self, target: &mut Target, grid: &mut GridView, text: &str) -> String {
        let (row, col, _, _) = grid.selection();
        let dest = open365_commands::CellRef {
            sheet: grid.sheet(),
            row,
            col,
        };
        let own = self
            .grid
            .as_ref()
            .filter(|c| unix_newlines(&c.tsv) == unix_newlines(text));
        let rows = match own {
            Some(clip) => paste_rows(&clip.rows, Some(clip.origin), dest),
            None => parse_tsv(&unix_newlines(text)),
        };
        if rows.is_empty() {
            return "Nothing to paste".to_owned();
        }
        let cells: usize = rows.iter().map(Vec::len).sum();
        let command = Command::XlsxPaste {
            sheet: dest.sheet,
            row: dest.row,
            col: dest.col,
            rows,
        };
        match target.apply(command) {
            Ok(_) => {
                grid.invalidate_layout();
                format!("Pasted {cells} cells")
            }
            Err(e) => format!("Paste failed: {e}"),
        }
    }
}
