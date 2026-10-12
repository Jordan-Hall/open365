//! What the shell can be asked to open as a dialog (`Shell::open_dialog`). The Sheets models ask
//! through `take_request()` ([`formulas::Request`](crate::sheet_ext::formulas::Request),
//! [`data::Request`](crate::sheet_ext::data::Request),
//! [`review::Request`](crate::sheet_ext::review::Request)); the rest comes from ribbon commands
//! sent without a value (`home.cells.format.row-height`) or from keyboard shortcuts.

use open365_ui_core::dialog::Secret;

use crate::dialogs::formulas_function::Category;
use crate::sheet_ext::{data, formulas, review};

/// A tab of Format Cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FormatTab {
    #[default]
    Number,
    Alignment,
    Font,
    Border,
    Fill,
    Protection,
}

impl FormatTab {
    pub const ALL: [FormatTab; 6] = [
        FormatTab::Number,
        FormatTab::Alignment,
        FormatTab::Font,
        FormatTab::Border,
        FormatTab::Fill,
        FormatTab::Protection,
    ];

    #[must_use]
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }
}

/// A tab of Page Setup (Sheets).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PageTab {
    #[default]
    Page,
    Margins,
    HeaderFooter,
    Sheet,
}

impl PageTab {
    #[must_use]
    pub fn index(self) -> usize {
        match self {
            PageTab::Page => 0,
            PageTab::Margins => 1,
            PageTab::HeaderFooter => 2,
            PageTab::Sheet => 3,
        }
    }
}

/// The Writer dialogs that had only typed `=value` forms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriterDialog {
    /// Page Setup: Margins and Paper tabs.
    PageSetup,
    /// Columns (More Columns...).
    Columns,
    /// Line Numbers (Line Numbering Options...).
    LineNumbers,
    Hyphenation,
    Watermark,
    /// Borders and Shading, Page Border tab.
    PageBorders,
    Zoom,
    Paragraph,
}

/// A dialog to open.
#[derive(Debug, Clone, PartialEq)]
pub enum DialogRequest {
    // Sheets: Formulas
    InsertFunction {
        category: Option<Category>,
    },
    FunctionArguments {
        name: String,
    },
    NameManager,
    EvaluateFormula,
    // Sheets: Data
    Sort,
    AdvancedFilter,
    TextToColumns,
    RemoveDuplicates,
    DataValidation,
    Consolidate,
    GoalSeek,
    Group,
    Subtotal,
    /// Custom AutoFilter on a column (from the filter menu's Text/Number Filters > Custom
    /// Filter...); `operand` fills the first condition's value.
    CustomFilter {
        column: u32,
        operand: String,
    },
    // Sheets: Review
    ProtectSheet,
    ProtectWorkbook,
    /// The password prompt of Unprotect Sheet and Unprotect Workbook.
    UnprotectSheet,
    UnprotectWorkbook,
    /// Confirm Password, opened over Protect Sheet or Protect Workbook; `expected` is what was
    /// typed there.
    ConfirmPassword {
        expected: Secret,
    },
    /// New Name (`editing` false) and Edit Name, opened over Name Manager. `scope` is a sheet
    /// index, the workbook when absent.
    NameEdit {
        name: String,
        scope: Option<usize>,
        refers_to: String,
        comment: String,
        editing: bool,
    },
    /// The question Name Manager asks before it deletes a name.
    DeleteName {
        name: String,
        scope: Option<usize>,
    },
    /// A message box with an OK button.
    Notice {
        title: String,
        text: String,
    },
    /// Custom Header and Custom Footer of Page Setup, over it.
    HeaderFooterCustom {
        footer: bool,
        sections: [String; 3],
    },
    // Sheets: Home and the grid
    FormatCells {
        tab: FormatTab,
    },
    RowHeight,
    ColumnWidth,
    InsertDelete {
        insert: bool,
    },
    PasteSpecial,
    /// Excel's "Merging cells only keeps the upper-left value" question; OK runs `command`.
    MergeConfirm {
        command: String,
    },
    /// The data validation alert for an entry the rule does not accept. `style` is `warning` or
    /// `information` (a `stop` alert only has Retry and Cancel).
    ValidationAlert {
        row: u32,
        col: u32,
        text: String,
        style: String,
        title: String,
        message: String,
    },
    // Sheets: Insert
    /// Insert > Table (Ctrl+T).
    CreateTable,
    /// Insert > PivotTable.
    CreatePivot,
    /// Insert > Link (Ctrl+K).
    InsertLink,
    /// Insert > Symbol.
    Symbol,
    // Sheets: Page Layout and View
    PageSetup {
        tab: PageTab,
    },
    Zoom,
    // Writer
    Writer(WriterDialog),
}

impl From<formulas::Request> for DialogRequest {
    fn from(r: formulas::Request) -> Self {
        match r {
            formulas::Request::InsertFunction { category } => Self::InsertFunction { category },
            formulas::Request::FunctionArguments { name } => Self::FunctionArguments { name },
            formulas::Request::NameManager => Self::NameManager,
            formulas::Request::EvaluateFormula => Self::EvaluateFormula,
        }
    }
}

impl From<data::Request> for DialogRequest {
    fn from(r: data::Request) -> Self {
        match r {
            data::Request::Sort => Self::Sort,
            data::Request::AdvancedFilter => Self::AdvancedFilter,
            data::Request::TextToColumns => Self::TextToColumns,
            data::Request::RemoveDuplicates => Self::RemoveDuplicates,
            data::Request::DataValidation => Self::DataValidation,
            data::Request::Consolidate => Self::Consolidate,
            data::Request::GoalSeek => Self::GoalSeek,
            data::Request::Group => Self::Group,
            data::Request::Subtotal => Self::Subtotal,
        }
    }
}

impl DialogRequest {
    /// The dialog for a Review request; `None` for those that are not dialogs of this host
    /// (spelling, accessibility and statistics have their own panes).
    #[must_use]
    pub fn from_review(r: review::Request) -> Option<Self> {
        match r {
            review::Request::ProtectSheet => Some(Self::ProtectSheet),
            review::Request::ProtectWorkbook => Some(Self::ProtectWorkbook),
            review::Request::UnprotectSheet => Some(Self::UnprotectSheet),
            review::Request::UnprotectWorkbook => Some(Self::UnprotectWorkbook),
            review::Request::Spelling
            | review::Request::Accessibility
            | review::Request::Statistics => None,
        }
    }

    /// The dialog a ribbon command opens when it is sent without a value (a bare click on
    /// "Row Height...", a group's dialog launcher). `writer` tells the two apps apart.
    #[must_use]
    pub fn for_command(id: &str, writer: bool) -> Option<Self> {
        if id.contains('=') || id.contains('#') {
            return None;
        }
        let id = id.strip_suffix(".run").unwrap_or(id);
        if writer {
            return Some(Self::Writer(match id {
                "layout.page-set-up.launcher"
                | "layout.page-set-up.margins.custom"
                | "layout.page-set-up.size.custom" => WriterDialog::PageSetup,
                "layout.page-set-up.columns.more" => WriterDialog::Columns,
                "layout.page-set-up.line-numbers.options" => WriterDialog::LineNumbers,
                "layout.page-set-up.hyphenation.options" => WriterDialog::Hyphenation,
                "design.page-background.watermark.custom" => WriterDialog::Watermark,
                "design.page-background.page-borders" => WriterDialog::PageBorders,
                "view.zoom.zoom" => WriterDialog::Zoom,
                "home.paragraph.launcher" | "layout.paragraph.launcher" => WriterDialog::Paragraph,
                _ => return None,
            }));
        }
        Some(match id {
            "home.cells.format.row-height" => Self::RowHeight,
            "home.cells.format.column-width" => Self::ColumnWidth,
            "home.cells.format.format-cells"
            | "home.number.launcher"
            | "home.number.number-format.more" => Self::FormatCells {
                tab: FormatTab::Number,
            },
            "home.alignment.launcher" => Self::FormatCells {
                tab: FormatTab::Alignment,
            },
            "home.font.launcher" => Self::FormatCells {
                tab: FormatTab::Font,
            },
            "home.editing.sort-filter.custom" => Self::Sort,
            "page-layout.page-set-up.launcher" | "page-layout.scale-to-fit.launcher" => {
                Self::PageSetup { tab: PageTab::Page }
            }
            "page-layout.page-set-up.margins.custom" => Self::PageSetup {
                tab: PageTab::Margins,
            },
            "page-layout.page-set-up.print-titles" | "page-layout.sheet-options.launcher" => {
                Self::PageSetup {
                    tab: PageTab::Sheet,
                }
            }
            "view.zoom.zoom" => Self::Zoom,
            "data.outline.launcher" => Self::Group,
            "insert.tables.table" => Self::CreateTable,
            "insert.tables.pivot-table" | "insert.tables.pivot-table.from-table-range" => {
                Self::CreatePivot
            }
            "insert.links.link" => Self::InsertLink,
            "insert.symbols.symbol" => Self::Symbol,
            "formulas.defined-names.define-name" => Self::NameManager,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_commands_map_to_dialogs_and_valued_ones_do_not() {
        assert_eq!(
            DialogRequest::for_command("home.cells.format.row-height", false),
            Some(DialogRequest::RowHeight)
        );
        assert_eq!(
            DialogRequest::for_command("home.cells.format.row-height=30", false),
            None
        );
        assert_eq!(
            DialogRequest::for_command("layout.page-set-up.columns.more", true),
            Some(DialogRequest::Writer(WriterDialog::Columns))
        );
        assert_eq!(DialogRequest::for_command("home.font.bold", false), None);
        assert_eq!(FormatTab::Fill.index(), 4);
    }
}
