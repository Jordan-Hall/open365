//! Data tab on the sheet model: Sort, AutoFilter, Advanced Filter, Text to Columns, Remove
//! Duplicates, Data Validation, Consolidate, Goal Seek and the outline (Group, Subtotal). Every
//! edit is a `DataOp` command (one undo step); the ribbon module `ribbon::sheets::data` maps
//! control ids to these methods. Dialogs the shell cannot draw yet are requested through
//! [`DataState::take_request`] and reachable by value forms (`data.sort-filter.sort=B:desc`).

use std::sync::Arc;

use open365_calc::{SystemClock, Value};
use open365_commands::data_ops::{
    Aggregate, ColumnFilter, FilterAction, SortHeader, SortLevel, SortRequest, SourceRange,
    SplitMode, SubtotalRequest, ValidationRule,
};
use open365_commands::{Command, DataOp, RangeRef};
use open365_xlsx::{CellValue, ValidationContext, Validity, WorkbookCells};

use super::super::{Rect4, SheetModel};

/// A dialog the shell should open for a Data command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    Sort,
    AdvancedFilter,
    TextToColumns,
    RemoveDuplicates,
    DataValidation,
    Consolidate,
    GoalSeek,
    Group,
    Subtotal,
}

/// What the Data tab keeps on the sheet model (none of it is in the file or the undo history).
#[derive(Debug, Default)]
pub struct DataState {
    request: Option<Request>,
}

impl DataState {
    /// The dialog the last command asked for, once.
    pub fn take_request(&mut self) -> Option<Request> {
        self.request.take()
    }

    pub fn request(&mut self, r: Request) {
        self.request = Some(r);
    }
}

fn rr(r: Rect4) -> RangeRef {
    RangeRef {
        first_row: r.0,
        first_col: r.1,
        last_row: r.2,
        last_col: r.3,
    }
}

/// A rejected entry: the Error Alert of a data validation rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejected {
    /// `stop`, `warning` or `information`.
    pub style: &'static str,
    pub title: String,
    pub message: String,
}

impl SheetModel {
    fn data_cmd(&mut self, op: DataOp) -> Result<Vec<String>, String> {
        self.data_cmd_on(self.sheet, op)
    }

    fn data_cmd_on(&mut self, sheet: usize, op: DataOp) -> Result<Vec<String>, String> {
        match self.target.apply(Command::DataOp { sheet, op }) {
            Ok(o) => {
                self.status = o.notes.join(" ");
                Ok(o.notes)
            }
            Err(e) => {
                self.status = e.to_string();
                Err(self.status.clone())
            }
        }
    }

    /// The range a sort or filter works on: the selection, or the current region of a lone cell.
    #[must_use]
    pub fn data_target(&self) -> Rect4 {
        if self.is_range() {
            self.range()
        } else {
            self.home_current_region(self.active)
        }
    }

    /// The index of the sheet called `name`.
    #[must_use]
    pub fn data_sheet_index(&self, name: &str) -> Option<usize> {
        self.target.xlsx()?.sheet_index(name)
    }

    // ---- sort ----------------------------------------------------------------------------

    /// Sort A to Z (`descending` false) or Z to A: by the column of the active cell.
    pub fn data_sort_quick(&mut self, descending: bool) -> Result<(), String> {
        let range = self.data_target();
        let column = self.active.1.clamp(range.1, range.3);
        self.data_sort(SortRequest {
            range: rr(range),
            header: SortHeader::Guess,
            left_to_right: false,
            case_sensitive: false,
            levels: vec![SortLevel {
                index: column,
                descending,
                custom_list: None,
            }],
        })
    }

    /// The Sort dialog's OK.
    pub fn data_sort(&mut self, request: SortRequest) -> Result<(), String> {
        self.data_cmd(DataOp::Sort(request)).map(|_| ())
    }

    // ---- filter --------------------------------------------------------------------------

    fn data_auto_filter(&self) -> Option<open365_xlsx::AutoFilter> {
        self.target
            .xlsx()
            .and_then(|wb| wb.sheet(self.sheet))
            .and_then(open365_xlsx::Worksheet::auto_filter)
    }

    /// The AutoFilter range of the sheet (header row first).
    #[must_use]
    pub fn data_filter_range(&self) -> Option<Rect4> {
        self.data_auto_filter().map(|f| f.range)
    }

    /// The columns (absolute, 1-based) that have a criterion.
    #[must_use]
    pub fn data_filtered_columns(&self) -> Vec<u32> {
        self.data_auto_filter().map_or_else(Vec::new, |f| {
            f.columns
                .iter()
                .filter(|c| c.criteria.is_some())
                .map(|c| f.range.1 + c.col_id)
                .collect()
        })
    }

    /// Data > Filter: AutoFilter arrows on over the selection or current region, or off.
    pub fn data_toggle_filter(&mut self) -> Result<(), String> {
        let range = if self.data_auto_filter().is_some() {
            None
        } else {
            Some(rr(self.data_target()))
        };
        self.data_cmd(DataOp::AutoFilter { range }).map(|_| ())
    }

    /// One AutoFilter drop-down's OK (`None` clears the column).
    pub fn data_filter_column(
        &mut self,
        column: u32,
        filter: Option<ColumnFilter>,
    ) -> Result<(), String> {
        self.data_cmd(DataOp::FilterColumn { column, filter })
            .map(|_| ())
    }

    /// Data > Clear.
    pub fn data_clear_filter(&mut self) -> Result<(), String> {
        if self.data_auto_filter().is_none() {
            return Ok(());
        }
        self.data_cmd(DataOp::ClearFilter).map(|_| ())
    }

    /// Data > Reapply.
    pub fn data_reapply_filter(&mut self) -> Result<(), String> {
        if self.data_auto_filter().is_none() {
            return Ok(());
        }
        self.data_cmd(DataOp::ReapplyFilter).map(|_| ())
    }

    /// The check boxes of an AutoFilter drop-down: every distinct value of the column below the
    /// header as shown (numbers ascending, then text, case-insensitively), with `(Blanks)` last
    /// when the column has blanks, and whether the item is ticked.
    #[must_use]
    pub fn data_filter_items(&self, column: u32) -> Vec<(String, bool)> {
        let (Some(wb), Some(f)) = (self.target.xlsx(), self.data_auto_filter()) else {
            return Vec::new();
        };
        let (r1, c1, r2, c2) = f.range;
        if column < c1 || column > c2 {
            return Vec::new();
        }
        let mut numbers: Vec<(f64, String)> = Vec::new();
        let mut texts: Vec<String> = Vec::new();
        let mut blank = false;
        for r in r1 + 1..=r2 {
            let shown = open365_commands::cell_display_text(wb, self.sheet, r, column);
            match wb.cell(self.sheet, r, column).map(|c| &c.value) {
                None | Some(CellValue::Empty) => blank = true,
                Some(CellValue::Number(n)) => {
                    if !numbers.iter().any(|(_, s)| *s == shown) {
                        numbers.push((*n, shown));
                    }
                }
                Some(_) => {
                    if !texts.iter().any(|t| t.eq_ignore_ascii_case(&shown)) {
                        texts.push(shown);
                    }
                }
            }
        }
        numbers.sort_by(|a, b| a.0.total_cmp(&b.0));
        texts.sort_by_key(|t| t.to_lowercase());
        let criteria = f
            .columns
            .iter()
            .find(|c| c.col_id == column - c1)
            .and_then(|c| c.criteria.clone());
        let ticked = |item: &str, is_blank: bool| match &criteria {
            Some(open365_xlsx::FilterCriteria::Values(v)) => {
                if is_blank {
                    v.blank
                } else {
                    v.items.iter().any(|i| i.eq_ignore_ascii_case(item))
                }
            }
            _ => true,
        };
        let mut out: Vec<(String, bool)> = numbers
            .into_iter()
            .map(|(_, s)| (s.clone(), ticked(&s, false)))
            .collect();
        out.extend(texts.into_iter().map(|s| (s.clone(), ticked(&s, false))));
        if blank {
            out.push(("(Blanks)".to_owned(), ticked("", true)));
        }
        out
    }

    /// Advanced Filter's OK.
    pub fn data_advanced_filter(
        &mut self,
        list: Rect4,
        criteria: Rect4,
        action: FilterAction,
        unique_only: bool,
    ) -> Result<(), String> {
        self.data_cmd(DataOp::AdvancedFilter {
            list: rr(list),
            criteria: rr(criteria),
            action,
            unique_only,
        })
        .map(|_| ())
    }

    // ---- data tools ----------------------------------------------------------------------

    /// Text to Columns' Finish over the selected column.
    pub fn data_text_to_columns(
        &mut self,
        split: SplitMode,
        destination: Option<(u32, u32)>,
    ) -> Result<(), String> {
        let sel = self.range();
        if sel.1 != sel.3 {
            return Err("Select a single column of data to convert.".to_owned());
        }
        self.data_cmd(DataOp::TextToColumns {
            range: rr(sel),
            split,
            columns: Vec::new(),
            destination,
        })
        .map(|_| ())
    }

    /// Remove Duplicates' OK over the selection or current region. `columns` are absolute; empty
    /// means every column of the range. Returns Excel's message.
    pub fn data_remove_duplicates(
        &mut self,
        has_headers: bool,
        columns: &[u32],
    ) -> Result<String, String> {
        let range = self.data_target();
        let columns: Vec<u32> = if columns.is_empty() {
            (range.1..=range.3).collect()
        } else {
            columns.to_vec()
        };
        let notes = self.data_cmd(DataOp::RemoveDuplicates {
            range: rr(range),
            has_headers,
            columns,
        })?;
        Ok(notes.join(" "))
    }

    /// The Data Validation dialog's OK on the selection (`None` is Clear All).
    pub fn data_set_validation(&mut self, rule: Option<ValidationRule>) -> Result<(), String> {
        let range = rr(self.range());
        self.data_cmd(DataOp::SetValidation { range, rule })
            .map(|_| ())
    }

    /// The validation rule that covers a cell.
    #[must_use]
    pub fn data_validation_at(&self, row: u32, col: u32) -> Option<open365_xlsx::DataValidation> {
        let ws = self.target.xlsx()?.sheet(self.sheet)?;
        ws.data_validations()?.at(row, col).cloned()
    }

    /// The items of a list rule's in-cell drop-down for a cell (empty when the cell has none).
    #[must_use]
    pub fn data_validation_list(&self, row: u32, col: u32) -> Vec<String> {
        let Some(wb) = self.target.xlsx() else {
            return Vec::new();
        };
        let Some(rule) = self.data_validation_at(row, col) else {
            return Vec::new();
        };
        if rule.hide_drop_down {
            return Vec::new();
        }
        let Some(info) = wb.sheets().get(self.sheet) else {
            return Vec::new();
        };
        let cells = WorkbookCells(wb);
        let locale = self.locale();
        let ctx = ValidationContext::new(&cells, &info.name, &locale);
        rule.list_values(row, col, &ctx)
            .into_iter()
            .map(|v| match v {
                Value::Text(t) => t,
                Value::Number(n) => open365_calc::value::number_to_text(n),
                other => format!("{other:?}"),
            })
            .collect()
    }

    /// Checks `text` against the cell's validation rule as Excel does on entry.
    pub fn data_check_entry(&self, row: u32, col: u32, text: &str) -> Result<(), Rejected> {
        let Some(wb) = self.target.xlsx() else {
            return Ok(());
        };
        let candidate = if text.trim().is_empty() {
            Value::Empty
        } else if text.trim().eq_ignore_ascii_case("true") {
            Value::Bool(true)
        } else if text.trim().eq_ignore_ascii_case("false") {
            Value::Bool(false)
        } else if let Some(n) = open365_calc::value::parse_value_text_in(text, wb.date1904()) {
            Value::Number(n)
        } else {
            Value::Text(text.to_owned())
        };
        let locale: Arc<open365_calc::FormatLocale> = self.locale();
        match wb.validate_cell(self.sheet, row, col, &candidate, &locale, &SystemClock) {
            None | Some(Validity::Valid) => Ok(()),
            Some(Validity::Invalid {
                style,
                title,
                message,
            }) => Err(Rejected {
                style: match style {
                    open365_xlsx::ErrorStyle::Stop => "stop",
                    open365_xlsx::ErrorStyle::Warning => "warning",
                    open365_xlsx::ErrorStyle::Information => "information",
                },
                title: title.unwrap_or_else(|| "Error".to_owned()),
                message: message.unwrap_or_else(|| {
                    "This value doesn't match the data validation restrictions defined for this cell."
                        .to_owned()
                }),
            }),
        }
    }

    /// Typing into a cell with validation: a `stop` alert refuses the entry; `warning` and
    /// `information` alerts let it through (the shell asks first with [`Self::data_check_entry`]).
    pub fn data_enter(&mut self, row: u32, col: u32, text: &str) -> Result<(), String> {
        if let Err(r) = self.data_check_entry(row, col, text)
            && r.style == "stop"
        {
            self.status = r.message.clone();
            return Err(r.message);
        }
        self.enter(row, col, text)
    }

    /// Consolidate's OK: the result goes to `destination` on the active sheet.
    pub fn data_consolidate(
        &mut self,
        function: Aggregate,
        sources: Vec<SourceRange>,
        use_top_row: bool,
        use_left_column: bool,
        destination: (u32, u32),
    ) -> Result<(), String> {
        self.data_cmd(DataOp::Consolidate {
            function,
            sources,
            use_top_row,
            use_left_column,
            row: destination.0,
            col: destination.1,
        })
        .map(|_| ())
    }

    /// What-If Analysis > Goal Seek: sets `set` to `to_value` by changing `change`.
    pub fn data_goal_seek(
        &mut self,
        set: (u32, u32),
        to_value: f64,
        change: (u32, u32),
    ) -> Result<String, String> {
        let notes = self.data_cmd(DataOp::GoalSeek {
            set_row: set.0,
            set_col: set.1,
            to_value,
            change_row: change.0,
            change_col: change.1,
        })?;
        Ok(notes.join(" "))
    }

    // ---- outline -------------------------------------------------------------------------

    /// The outline level of a row (0 when ungrouped).
    #[must_use]
    pub fn data_row_level(&self, row: u32) -> u32 {
        self.target
            .xlsx()
            .and_then(|wb| wb.sheet(self.sheet))
            .and_then(|ws| ws.rows().iter().find(|r| r.r == row))
            .and_then(|r| {
                r.attrs()
                    .iter()
                    .find(|a| a.name == "outlineLevel")
                    .and_then(|a| a.value.parse().ok())
            })
            .unwrap_or(0)
    }

    /// Whether the selection is whole columns (the Group command then groups columns).
    #[must_use]
    pub fn data_selection_is_columns(&self) -> bool {
        let (r1, _, r2, _) = self.range();
        r1 == 1 && r2 >= super::super::MAX_ROW
    }

    /// The outline level of a column (0 when ungrouped).
    #[must_use]
    pub fn data_col_level(&self, col: u32) -> u32 {
        let Some(xml) = self
            .target
            .xlsx()
            .and_then(|wb| wb.serialize_sheet(self.sheet))
        else {
            return 0;
        };
        let xml = String::from_utf8_lossy(&xml);
        let Some(start) = xml.find("<cols>") else {
            return 0;
        };
        let end = xml[start..]
            .find("</cols>")
            .map_or(xml.len(), |e| start + e);
        let attr = |tag: &str, name: &str| -> Option<u32> {
            let key = format!(" {name}=\"");
            let at = tag.find(&key)? + key.len();
            tag[at..].split('"').next()?.parse().ok()
        };
        xml[start..end]
            .split("<col ")
            .skip(1)
            .find_map(|tag| {
                let tag = format!(" {tag}");
                let tag = tag.as_str();
                let (min, max) = (attr(tag, "min")?, attr(tag, "max")?);
                (min <= col && col <= max).then(|| attr(tag, "outlineLevel").unwrap_or(0))
            })
            .unwrap_or(0)
    }

    /// Data > Group on the columns of the selection.
    pub fn data_group_cols(&mut self) -> Result<(), String> {
        let (_, c1, _, c2) = self.range();
        self.data_cmd(DataOp::GroupCols {
            first: c1,
            last: c2,
        })
        .map(|_| ())
    }

    /// Data > Ungroup on the columns of the selection.
    pub fn data_ungroup_cols(&mut self) -> Result<(), String> {
        let (_, c1, _, c2) = self.range();
        self.data_cmd(DataOp::UngroupCols {
            first: c1,
            last: c2,
        })
        .map(|_| ())
    }

    /// Hide or Show Detail on the columns right before the selected summary column.
    pub fn data_detail_cols(&mut self, show: bool) -> Result<(), String> {
        let (_, c1, _, c2) = self.range();
        let level = self.data_col_level(c2);
        let mut first = c2;
        while first > 1 && self.data_col_level(first - 1) > level {
            first -= 1;
        }
        let (first, last) = if self.data_col_level(c1) > 0
            && self.data_col_level(c2 + 1) < self.data_col_level(c2)
        {
            (c1, c2)
        } else if first < c2 {
            (first, c2 - 1)
        } else {
            return Err("Select the summary column of a group.".to_owned());
        };
        self.data_cmd(DataOp::ShowDetailCols {
            first,
            last,
            show,
            summary_right: true,
        })
        .map(|_| ())
    }

    /// Data > Group on the rows of the selection.
    pub fn data_group_rows(&mut self) -> Result<(), String> {
        let (r1, _, r2, _) = self.range();
        self.data_cmd(DataOp::GroupRows {
            first: r1,
            last: r2,
        })
        .map(|_| ())
    }

    /// Data > Ungroup on the rows of the selection.
    pub fn data_ungroup_rows(&mut self) -> Result<(), String> {
        let (r1, _, r2, _) = self.range();
        self.data_cmd(DataOp::UngroupRows {
            first: r1,
            last: r2,
        })
        .map(|_| ())
    }

    /// The detail rows of the summary row `row`: the grouped rows right above it.
    fn data_detail_of(&self, row: u32) -> Option<(u32, u32)> {
        let level = self.data_row_level(row);
        let mut first = row;
        while first > 1 && self.data_row_level(first - 1) > level {
            first -= 1;
        }
        (first < row).then_some((first, row - 1))
    }

    /// Data > Hide Detail (`show` false) or Show Detail on the active cell's summary row; when
    /// the selection itself is grouped detail, that detail.
    pub fn data_detail(&mut self, show: bool) -> Result<(), String> {
        let (r1, _, r2, _) = self.range();
        let (first, last) = if self.data_row_level(r1) > 0
            && self.data_row_level(r2 + 1) < self.data_row_level(r2)
        {
            (r1, r2)
        } else {
            self.data_detail_of(r2)
                .ok_or_else(|| "Select the summary row of a group.".to_owned())?
        };
        self.data_cmd(DataOp::ShowDetailRows {
            first,
            last,
            show,
            summary_below: true,
        })
        .map(|_| ())
    }

    /// Subtotal's OK over the current region.
    pub fn data_subtotal(&mut self, mut request: SubtotalRequest) -> Result<(), String> {
        if request.range.first_row == request.range.last_row {
            request.range = rr(self.data_target());
        }
        self.data_cmd(DataOp::Subtotal(request)).map(|_| ())
    }

    /// Subtotal's Remove All over the current region.
    pub fn data_remove_subtotals(&mut self) -> Result<(), String> {
        let range = rr(self.data_target());
        self.data_cmd(DataOp::RemoveSubtotals { range }).map(|_| ())
    }
}
