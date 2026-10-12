//! Protect Sheet and Protect Workbook.
//!
//! The model protects at once when the ribbon button is clicked without a value (and asks for
//! this dialog on top), so the dialog starts on a protected sheet: OK lifts that protection and
//! applies the chosen options, Cancel lifts it again.

use open365_commands::review_ops::SheetProtection;
use open365_ui_core::Ui;
use open365_ui_core::dialog::{Dialog, Spec, Values};

use super::View;
use crate::dialogs::request::DialogRequest;
use crate::shell::Shell;

pub(super) fn make(s: &mut Shell, req: &DialogRequest) -> Option<(Spec, Box<dyn View>)> {
    Some(match req {
        DialogRequest::ProtectSheet => sheet(s),
        DialogRequest::ProtectWorkbook => workbook(s),
        _ => return None,
    })
}

/// (id, caption) of the "Allow all users of this worksheet to" boxes, in Excel's order.
const ALLOW: [(&str, &str); 15] = [
    ("select_locked", "Select &locked cells"),
    ("select_unlocked", "Select &unlocked cells"),
    ("format_cells", "&Format cells"),
    ("format_columns", "Format colu&mns"),
    ("format_rows", "Format r&ows"),
    ("insert_columns", "Insert &columns"),
    ("insert_rows", "Insert &rows"),
    ("insert_hyperlinks", "Insert &hyperlinks"),
    ("delete_columns", "&Delete columns"),
    ("delete_rows", "Delete rows"),
    ("sort", "&Sort"),
    ("auto_filter", "Use &AutoFilter"),
    ("pivot", "Use Pivo&tTable and PivotChart"),
    ("objects", "Edit o&bjects"),
    ("scenarios", "Edit scenarios"),
];

#[derive(Debug)]
struct ProtectSheet {
    /// The sheet was already protected when the dialog opened (the model's default protection).
    was_protected: bool,
}

fn sheet(s: &mut Shell) -> (Spec, Box<dyn View>) {
    let cur = s.sheet.review_sheet_protection().unwrap_or_default();
    let on = [
        cur.select_locked_cells,
        cur.select_unlocked_cells,
        cur.format_cells,
        cur.format_columns,
        cur.format_rows,
        cur.insert_columns,
        cur.insert_rows,
        cur.insert_hyperlinks,
        cur.delete_columns,
        cur.delete_rows,
        cur.sort,
        cur.use_auto_filter,
        cur.use_pivot_tables,
        cur.edit_objects,
        cur.edit_scenarios,
    ];
    let mut spec = Spec::new("Protect Sheet", 400.0, 296.0);
    spec.check(
        "protect",
        12.0,
        8.0,
        376.0,
        "&Protect worksheet and contents of locked cells",
        true,
    );
    spec.label(12.0, 36.0, 376.0, "Pass&word to unprotect sheet:");
    spec.text("password", 12.0, 56.0, 376.0, "");
    spec.label(12.0, 90.0, 376.0, "Allow all users of this worksheet to:");
    for (i, ((id, label), on)) in ALLOW.iter().zip(on).enumerate() {
        let x = 12.0 + 196.0 * (i % 2) as f32;
        let y = 112.0 + 22.0 * (i / 2) as f32;
        spec.check(id, x, y, 190.0, label, on);
    }
    spec.focus = Some("password".to_owned());
    (
        spec,
        Box::new(ProtectSheet {
            was_protected: s.sheet.review_sheet_protection().is_some(),
        }),
    )
}

impl View for ProtectSheet {
    fn commit(
        &mut self,
        s: &mut Shell,
        _ui: &mut Ui,
        _d: &mut Dialog,
        v: &Values,
    ) -> Result<(), String> {
        if self.was_protected && s.sheet.review_sheet_protection().is_some() {
            s.sheet.review_unprotect_sheet(None)?;
            self.was_protected = false;
        }
        if !v.flag("protect") {
            return Ok(());
        }
        let options = SheetProtection {
            select_locked_cells: v.flag("select_locked"),
            select_unlocked_cells: v.flag("select_unlocked"),
            format_cells: v.flag("format_cells"),
            format_columns: v.flag("format_columns"),
            format_rows: v.flag("format_rows"),
            insert_columns: v.flag("insert_columns"),
            insert_rows: v.flag("insert_rows"),
            insert_hyperlinks: v.flag("insert_hyperlinks"),
            delete_columns: v.flag("delete_columns"),
            delete_rows: v.flag("delete_rows"),
            sort: v.flag("sort"),
            use_auto_filter: v.flag("auto_filter"),
            use_pivot_tables: v.flag("pivot"),
            edit_objects: v.flag("objects"),
            edit_scenarios: v.flag("scenarios"),
        };
        let password = v.text("password");
        s.sheet
            .review_protect_sheet(options, (!password.is_empty()).then_some(password))
    }

    fn cancel(&mut self, s: &mut Shell, _ui: &mut Ui) {
        if self.was_protected && s.sheet.review_sheet_protection().is_some() {
            let _ = s.sheet.review_unprotect_sheet(None);
        }
    }
}

#[derive(Debug)]
struct ProtectWorkbook {
    was_protected: bool,
}

fn workbook(s: &Shell) -> (Spec, Box<dyn View>) {
    let mut spec = Spec::new("Protect Structure and Windows", 340.0, 176.0);
    spec.label(12.0, 8.0, 316.0, "Protect workbook for");
    spec.check("structure", 24.0, 28.0, 300.0, "&Structure", true);
    spec.check("windows", 24.0, 50.0, 300.0, "&Windows", false)
        .disabled();
    spec.label(12.0, 84.0, 316.0, "Password (optional):");
    spec.text("password", 12.0, 104.0, 316.0, "");
    spec.focus = Some("password".to_owned());
    (
        spec,
        Box::new(ProtectWorkbook {
            was_protected: s.sheet.review_workbook_protected(),
        }),
    )
}

impl View for ProtectWorkbook {
    fn commit(
        &mut self,
        s: &mut Shell,
        _ui: &mut Ui,
        _d: &mut Dialog,
        v: &Values,
    ) -> Result<(), String> {
        if self.was_protected && s.sheet.review_workbook_protected() {
            s.sheet.review_unprotect_workbook(None)?;
            self.was_protected = false;
        }
        if !v.flag("structure") {
            return Ok(());
        }
        let password = v.text("password");
        s.sheet
            .review_protect_workbook((!password.is_empty()).then_some(password))
    }

    fn cancel(&mut self, s: &mut Shell, _ui: &mut Ui) {
        if self.was_protected && s.sheet.review_workbook_protected() {
            let _ = s.sheet.review_unprotect_workbook(None);
        }
    }
}
