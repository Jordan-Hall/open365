//! The Sort dialog (Data > Sort): levels with Add, Delete, Copy and Move Up/Down, the header
//! check box, and the Options (case sensitive, left to right). `to_request` is the dialog's OK.

use open365_commands::RangeRef;
use open365_commands::data_ops::{SortHeader, SortLevel, SortRequest};

use crate::sheet::{SheetModel, a1};

/// Most levels Excel's dialog accepts.
pub const MAX_LEVELS: usize = 64;

/// One row of the level list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Level {
    /// Absolute 1-based column (row when sorting left to right).
    pub index: u32,
    pub descending: bool,
    /// "Custom List...": the items in order.
    pub custom_list: Option<Vec<String>>,
}

/// The dialog's state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortDialog {
    pub range: (u32, u32, u32, u32),
    /// "My data has headers".
    pub has_header: bool,
    pub case_sensitive: bool,
    pub left_to_right: bool,
    pub levels: Vec<Level>,
}

impl SortDialog {
    /// Opens the dialog on the selection or the current region, one level on the active cell's
    /// column, with the header box ticked as Excel guesses it: text above numbers.
    #[must_use]
    pub fn open(m: &SheetModel) -> Self {
        let range = m.data_target();
        let header = Self::guess_header(m, range);
        Self {
            range,
            has_header: header,
            case_sensitive: false,
            left_to_right: false,
            levels: vec![Level {
                index: m.active.1.clamp(range.1, range.3),
                descending: false,
                custom_list: None,
            }],
        }
    }

    fn guess_header(m: &SheetModel, (r1, c1, r2, c2): (u32, u32, u32, u32)) -> bool {
        if r2 <= r1 {
            return false;
        }
        (c1..=c2).any(|c| {
            let first = m.shown(r1, c);
            let second = m.shown(r1 + 1, c);
            matches!((first, second), (Some((_, false)), Some((_, true))))
        })
    }

    /// The names the "Sort by" list offers: the header texts, or `Column B`.
    #[must_use]
    pub fn column_names(&self, m: &SheetModel) -> Vec<(u32, String)> {
        let (r1, c1, _, c2) = self.range;
        (c1..=c2)
            .map(|c| {
                let header = m.display(r1, c);
                let name = if self.has_header && !header.is_empty() {
                    header
                } else {
                    format!("Column {}", a1(1, c).trim_end_matches('1'))
                };
                (c, name)
            })
            .collect()
    }

    /// Add Level: a new level on the first column not used yet.
    pub fn add_level(&mut self) -> Result<(), String> {
        if self.levels.len() >= MAX_LEVELS {
            return Err("You can sort by at most 64 columns.".to_owned());
        }
        let used: Vec<u32> = self.levels.iter().map(|l| l.index).collect();
        let next = (self.range.1..=self.range.3)
            .find(|c| !used.contains(c))
            .unwrap_or(self.range.1);
        self.levels.push(Level {
            index: next,
            descending: false,
            custom_list: None,
        });
        Ok(())
    }

    /// Delete Level; the last level cannot be deleted.
    pub fn delete_level(&mut self, at: usize) {
        if self.levels.len() > 1 && at < self.levels.len() {
            self.levels.remove(at);
        }
    }

    /// Copy Level: a duplicate right below.
    pub fn copy_level(&mut self, at: usize) -> Result<(), String> {
        if self.levels.len() >= MAX_LEVELS {
            return Err("You can sort by at most 64 columns.".to_owned());
        }
        if let Some(l) = self.levels.get(at).cloned() {
            self.levels.insert(at + 1, l);
        }
        Ok(())
    }

    /// Move Up (`up`) or Move Down.
    pub fn move_level(&mut self, at: usize, up: bool) {
        let to = if up { at.checked_sub(1) } else { Some(at + 1) };
        if let Some(to) = to.filter(|t| *t < self.levels.len() && at < self.levels.len()) {
            self.levels.swap(at, to);
        }
    }

    /// OK: the sort request, or Excel's complaint (a column used twice, a column outside the
    /// range).
    pub fn to_request(&self) -> Result<SortRequest, String> {
        let (lo, hi) = if self.left_to_right {
            (self.range.0, self.range.2)
        } else {
            (self.range.1, self.range.3)
        };
        for (i, l) in self.levels.iter().enumerate() {
            if l.index < lo || l.index > hi {
                return Err("The sort reference is not valid. Make sure that it's within the data you want to sort.".to_owned());
            }
            if self.levels[..i].iter().any(|o| o.index == l.index) {
                return Err("This sort criterion is being used more than once. Delete the duplicate sort criteria and try again.".to_owned());
            }
        }
        Ok(SortRequest {
            range: RangeRef {
                first_row: self.range.0,
                first_col: self.range.1,
                last_row: self.range.2,
                last_col: self.range.3,
            },
            header: if self.has_header {
                SortHeader::Yes
            } else {
                SortHeader::No
            },
            left_to_right: self.left_to_right,
            case_sensitive: self.case_sensitive,
            levels: self
                .levels
                .iter()
                .map(|l| SortLevel {
                    index: l.index,
                    descending: l.descending,
                    custom_list: l.custom_list.clone(),
                })
                .collect(),
        })
    }

    /// Applies the dialog to the sheet.
    pub fn apply(&self, m: &mut SheetModel) -> Result<(), String> {
        m.data_sort(self.to_request()?)
    }
}
