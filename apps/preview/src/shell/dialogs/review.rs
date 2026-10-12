//! Protect Sheet, Protect Workbook, their Confirm Password step and the Unprotect password
//! prompts.
//!
//! As in Excel the dialog opens before anything is protected: a bare click on the button only
//! leaves the request (see `sheet_ext::review`), OK protects and Cancel leaves the sheet alone.
//! A password is typed twice: OK opens Confirm Password over the dialog, and only a matching
//! second entry protects. Passwords stay in masked fields and `Secret`s; they are never put in a
//! message, a label or a command name.

use std::cell::Cell;

use open365_commands::review_ops::SheetProtection;
use open365_ui_core::Ui;
use open365_ui_core::dialog::{Dialog, Secret, Spec, Values};

use super::{View, request_child, request_close};
use crate::dialogs::request::DialogRequest;
use crate::sheet_ext::review::PASSWORD_WRONG_MESSAGE;
use crate::shell::Shell;

/// Excel's text when the two entries differ.
const NOT_IDENTICAL: &str = "Confirmation password is not identical.";

thread_local! {
    /// What the last Confirm Password dialog decided: `Some(true)` the entries matched.
    static CONFIRMED: Cell<Option<bool>> = const { Cell::new(None) };
}

pub(super) fn make(s: &mut Shell, req: &DialogRequest) -> Option<(Spec, Box<dyn View>)> {
    Some(match req {
        DialogRequest::ProtectSheet => sheet(s),
        DialogRequest::ProtectWorkbook => workbook(),
        DialogRequest::UnprotectSheet => unprotect(false),
        DialogRequest::UnprotectWorkbook => unprotect(true),
        DialogRequest::ConfirmPassword { expected } => confirm(expected),
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

fn options_of(v: &Values) -> SheetProtection {
    SheetProtection {
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
    }
}

/// What a protect dialog keeps while Confirm Password is open over it.
#[derive(Debug, Default)]
struct Pending {
    options: Option<SheetProtection>,
    password: Option<Secret>,
}

#[derive(Debug)]
struct ProtectSheet {
    pending: Pending,
}

fn sheet(s: &Shell) -> (Spec, Box<dyn View>) {
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
    spec.password("password", 12.0, 56.0, 376.0);
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
            pending: Pending::default(),
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
        if !v.flag("protect") {
            return Ok(());
        }
        let options = options_of(v);
        let password = v.secret("password");
        if password.is_empty() {
            return s.sheet.review_protect_sheet(options, None);
        }
        self.pending = Pending {
            options: Some(options),
            password: Some(Secret::new(password)),
        };
        request_child(DialogRequest::ConfirmPassword {
            expected: Secret::new(password),
        });
        // Stay open, with no message, until the second entry is in.
        Err(String::new())
    }

    fn child_done(
        &mut self,
        s: &mut Shell,
        ui: &mut Ui,
        d: &mut Dialog,
        applied: bool,
    ) -> Result<(), String> {
        let confirmed = CONFIRMED.with(Cell::take);
        if !applied {
            return Ok(());
        }
        let (options, password) = (self.pending.options.take(), self.pending.password.take());
        match (confirmed, options, password) {
            (Some(true), Some(options), Some(password)) => {
                s.sheet
                    .review_protect_sheet(options, Some(password.expose()))?;
                request_close();
                Ok(())
            }
            _ => {
                d.set_text(ui, "password", "");
                d.focus(ui, "password");
                Err(NOT_IDENTICAL.to_owned())
            }
        }
    }
}

#[derive(Debug)]
struct ProtectWorkbook {
    pending: Pending,
}

fn workbook() -> (Spec, Box<dyn View>) {
    let mut spec = Spec::new("Protect Structure and Windows", 340.0, 176.0);
    spec.label(12.0, 8.0, 316.0, "Protect workbook for");
    spec.check("structure", 24.0, 28.0, 300.0, "&Structure", true);
    spec.check("windows", 24.0, 50.0, 300.0, "&Windows", false)
        .disabled();
    spec.label(12.0, 84.0, 316.0, "&Password (optional):");
    spec.password("password", 12.0, 104.0, 316.0);
    spec.focus = Some("password".to_owned());
    (
        spec,
        Box::new(ProtectWorkbook {
            pending: Pending::default(),
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
        if !v.flag("structure") {
            return Ok(());
        }
        let password = v.secret("password");
        if password.is_empty() {
            return s.sheet.review_protect_workbook(None);
        }
        self.pending.password = Some(Secret::new(password));
        request_child(DialogRequest::ConfirmPassword {
            expected: Secret::new(password),
        });
        Err(String::new())
    }

    fn child_done(
        &mut self,
        s: &mut Shell,
        ui: &mut Ui,
        d: &mut Dialog,
        applied: bool,
    ) -> Result<(), String> {
        let confirmed = CONFIRMED.with(Cell::take);
        if !applied {
            return Ok(());
        }
        match (confirmed, self.pending.password.take()) {
            (Some(true), Some(password)) => {
                s.sheet.review_protect_workbook(Some(password.expose()))?;
                request_close();
                Ok(())
            }
            _ => {
                d.set_text(ui, "password", "");
                d.focus(ui, "password");
                Err(NOT_IDENTICAL.to_owned())
            }
        }
    }
}

// ---- Confirm Password ----------------------------------------------------------------------------------

#[derive(Debug)]
struct Confirm {
    expected: Secret,
}

fn confirm(expected: &Secret) -> (Spec, Box<dyn View>) {
    let mut spec = Spec::new("Confirm Password", 340.0, 150.0);
    spec.label(12.0, 8.0, 316.0, "&Reenter password to proceed.");
    spec.password("password", 12.0, 30.0, 316.0);
    spec.note(
        12.0,
        64.0,
        316.0,
        75.0,
        "Caution: If you lose or forget the password, it cannot be recovered.\nIt is advisable to keep a list of passwords and their\ncorresponding workbook and sheet names in a safe place.\n(Remember that passwords are case-sensitive.)",
    );
    spec.focus = Some("password".to_owned());
    (
        spec,
        Box::new(Confirm {
            expected: expected.clone(),
        }),
    )
}

impl View for Confirm {
    fn commit(
        &mut self,
        _s: &mut Shell,
        _ui: &mut Ui,
        _d: &mut Dialog,
        v: &Values,
    ) -> Result<(), String> {
        CONFIRMED.with(|c| c.set(Some(v.secret("password") == self.expected.expose())));
        Ok(())
    }

    fn cancel(&mut self, _s: &mut Shell, _ui: &mut Ui) {
        CONFIRMED.with(|c| c.set(None));
    }
}

// ---- Unprotect --------------------------------------------------------------------------------------------

#[derive(Debug)]
struct Unprotect {
    workbook: bool,
}

fn unprotect(workbook: bool) -> (Spec, Box<dyn View>) {
    let title = if workbook {
        "Unprotect Workbook"
    } else {
        "Unprotect Sheet"
    };
    let mut spec = Spec::new(title, 300.0, 70.0);
    spec.label(12.0, 10.0, 276.0, "&Password:");
    spec.password("password", 12.0, 32.0, 276.0);
    spec.focus = Some("password".to_owned());
    (spec, Box::new(Unprotect { workbook }))
}

impl View for Unprotect {
    fn commit(
        &mut self,
        s: &mut Shell,
        _ui: &mut Ui,
        _d: &mut Dialog,
        v: &Values,
    ) -> Result<(), String> {
        let password = v.secret("password");
        let done = if self.workbook {
            s.sheet.review_unprotect_workbook(Some(password))
        } else {
            s.sheet.review_unprotect_sheet(Some(password))
        };
        match done {
            Err(e) if e == PASSWORD_WRONG_MESSAGE => {
                // Excel answers a wrong password with a message box, and the prompt stays.
                super::notice("Open365", &e);
                Err(String::new())
            }
            other => other,
        }
    }
}
