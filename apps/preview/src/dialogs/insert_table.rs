//! Create Table (Insert > Table, Ctrl+T): the range and the "My table has headers" box.

use crate::sheet::sheet_ext::insert::{Kind, a1_abs};

/// The Create Table dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateTableDialog {
    /// First row, first column, last row, last column.
    pub range: (u32, u32, u32, u32),
    /// "My table has headers", pre-set from Excel's guess.
    pub has_headers: bool,
}

impl CreateTableDialog {
    /// The dialog for `range`; `kinds` are the cell kinds of the range, row by row.
    #[must_use]
    pub fn new(range: (u32, u32, u32, u32), kinds: &[Vec<Kind>]) -> Self {
        Self {
            range,
            has_headers: guess_headers(kinds),
        }
    }

    /// The range as the dialog's text box shows it (`$A$1:$C$5`).
    #[must_use]
    pub fn range_text(&self) -> String {
        let (r1, c1, r2, c2) = self.range;
        format!("{}:{}", a1_abs(r1, c1), a1_abs(r2, c2))
    }
}

/// Excel's header guess: the first row is all text and the row below is not all text.
#[must_use]
pub fn guess_headers(kinds: &[Vec<Kind>]) -> bool {
    let Some(first) = kinds.first() else {
        return false;
    };
    if first.is_empty() || !first.iter().all(|k| *k == Kind::Text) {
        return false;
    }
    match kinds.get(1) {
        None => false,
        Some(second) => second.iter().any(|k| *k != Kind::Text),
    }
}
