//! Review tab, second part: Notes > Convert to Comments and Allow Edit Ranges. Both are undo
//! steps of the workbook; the ranges are written as the worksheet's `protectedRanges`.

use open365_commands::sheet_page_ops::{self, EditRange};
use open365_commands::{Command, SheetPageOp};

use super::super::SheetModel;

impl SheetModel {
    /// Notes > Convert to Comments: every note of the workbook becomes a threaded comment.
    ///
    /// # Errors
    /// The workbook has no notes.
    pub fn review_convert_notes_to_comments(&mut self) -> Result<(), String> {
        let sheet = self.sheet;
        self.apply(Command::SheetPage {
            sheet,
            op: SheetPageOp::ConvertNotesToComments,
        })
    }

    /// The Allow Edit Ranges of the active sheet.
    #[must_use]
    pub fn review_edit_ranges(&self) -> Vec<EditRange> {
        self.target
            .xlsx()
            .map(|wb| sheet_page_ops::edit_ranges(wb, self.sheet))
            .unwrap_or_default()
    }

    /// The title the New Range dialog suggests: `Range1`, `Range2`, ...
    #[must_use]
    pub fn review_next_range_title(&self) -> String {
        let have = self.review_edit_ranges();
        (1..)
            .map(|n| format!("Range{n}"))
            .find(|t| !have.iter().any(|r| r.title.eq_ignore_ascii_case(t)))
            .unwrap_or_default()
    }

    fn review_set_edit_ranges(&mut self, list: Vec<EditRange>) -> Result<(), String> {
        let sheet = self.sheet;
        self.apply(Command::SheetPage {
            sheet,
            op: SheetPageOp::SetEditRanges(list),
        })
    }

    fn check_refers(refers_to: &str) -> Result<(), String> {
        let text = refers_to.trim().trim_start_matches('=').replace('$', "");
        if text.is_empty() {
            return Err("The reference is not valid.".to_owned());
        }
        for part in text.split([' ', ',']).filter(|p| !p.is_empty()) {
            if open365_xlsx::parse_range(part).is_none() {
                return Err("The reference is not valid.".to_owned());
            }
        }
        Ok(())
    }

    /// New Range: `refers_to` is `A1:B3` (`$` and a leading `=` are accepted). A password locks
    /// the range against everybody but those who know it.
    ///
    /// # Errors
    /// The sheet is protected, the title is taken or empty, or the reference is invalid.
    pub fn review_add_edit_range(
        &mut self,
        title: &str,
        refers_to: &str,
        password: Option<&str>,
    ) -> Result<(), String> {
        Self::check_refers(refers_to)?;
        let mut list = self.review_edit_ranges();
        list.push(EditRange {
            title: title.trim().to_owned(),
            refers_to: refers_to.trim().trim_start_matches('=').replace('$', ""),
            password_hash: password
                .filter(|p| !p.is_empty())
                .map(sheet_page_ops::range_password_hash),
        });
        self.review_set_edit_ranges(list)
    }

    /// Modify: changes the range called `old`; an empty `password` keeps the one it has.
    ///
    /// # Errors
    /// As for [`SheetModel::review_add_edit_range`], or there is no range `old`.
    pub fn review_modify_edit_range(
        &mut self,
        old: &str,
        title: &str,
        refers_to: &str,
        password: Option<&str>,
    ) -> Result<(), String> {
        Self::check_refers(refers_to)?;
        let mut list = self.review_edit_ranges();
        let slot = list
            .iter_mut()
            .find(|r| r.title.eq_ignore_ascii_case(old.trim()))
            .ok_or_else(|| "There is no range with that title.".to_owned())?;
        slot.title = title.trim().to_owned();
        slot.refers_to = refers_to.trim().trim_start_matches('=').replace('$', "");
        if let Some(p) = password.filter(|p| !p.is_empty()) {
            slot.password_hash = Some(sheet_page_ops::range_password_hash(p));
        }
        self.review_set_edit_ranges(list)
    }

    /// Delete.
    ///
    /// # Errors
    /// The sheet is protected or there is no range of that title.
    pub fn review_delete_edit_range(&mut self, title: &str) -> Result<(), String> {
        let mut list = self.review_edit_ranges();
        let before = list.len();
        list.retain(|r| !r.title.eq_ignore_ascii_case(title.trim()));
        if list.len() == before {
            return Err("There is no range with that title.".to_owned());
        }
        self.review_set_edit_ranges(list)
    }

    /// Whether a protected sheet lets the cell be edited because it is in a range without a
    /// password (a range with one asks for it first, which needs the unlock dialog).
    #[must_use]
    pub fn review_in_open_edit_range(&self, row: u32, col: u32) -> bool {
        self.review_edit_ranges().iter().any(|r| {
            r.password_hash.is_none()
                && r.refers_to
                    .split([' ', ','])
                    .filter_map(open365_xlsx::parse_range)
                    .any(|(r1, c1, r2, c2)| (r1..=r2).contains(&row) && (c1..=c2).contains(&col))
        })
    }
}
