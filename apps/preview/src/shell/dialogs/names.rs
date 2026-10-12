//! New Name, Edit Name and the delete question of Name Manager. They open over Name Manager,
//! which refreshes its list when one of them closes with OK.

use open365_commands::objects_ops::NameRequest;
use open365_ui_core::Ui;
use open365_ui_core::dialog::{Dialog, Outcome, Spec, Values};

use super::{View, notice};
use crate::dialogs::request::DialogRequest;
use crate::shell::Shell;

/// Excel's answer to a name it cannot use (the product name is left out).
pub(super) const NAME_INVALID: &str = "The name that you entered is not valid.\n\nReasons for this can include:\n\n- The name begins with a space or a number\n- The name contains a space or other invalid characters\n- The name conflicts with a built-in name or the name of another object in the workbook";
/// ... and for one that exists in the same scope.
pub(super) const NAME_TAKEN: &str = "That name is already taken. Try a different one.";
const FORMULA_PROBLEM: &str = "There's a problem with this formula.";

pub(super) fn make(s: &mut Shell, req: &DialogRequest) -> Option<(Spec, Box<dyn View>)> {
    Some(match req {
        DialogRequest::NameEdit {
            name,
            scope,
            refers_to,
            comment,
            editing,
        } => edit(s, name, *scope, refers_to, comment, *editing),
        DialogRequest::DeleteName { name, scope } => delete(name, *scope),
        _ => return None,
    })
}

/// `'My Sheet'!` for a sheet name that needs quotes, `Sheet1!` for one that does not.
pub(super) fn sheet_prefix(name: &str) -> String {
    let plain = name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
        && !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit());
    if plain {
        format!("{name}!")
    } else {
        format!("'{}'!", name.replace('\'', "''"))
    }
}

/// `=Sheet1!$A$1` for the selection, the text New Name starts with.
pub(super) fn selection_ref(s: &Shell) -> String {
    let names = s.sheet.sheet_names();
    let sheet = names
        .get(s.sheet.active_sheet_index())
        .map_or_else(String::new, |n| sheet_prefix(n));
    format!("={sheet}{}", super::selection_text(s))
}

#[derive(Debug)]
struct Edit {
    editing: bool,
    original: (String, Option<usize>),
    sheets: Vec<String>,
}

fn edit(
    s: &Shell,
    name: &str,
    scope: Option<usize>,
    refers_to: &str,
    comment: &str,
    editing: bool,
) -> (Spec, Box<dyn View>) {
    let sheets = s.sheet.sheet_names();
    let mut spec = Spec::new(if editing { "Edit Name" } else { "New Name" }, 420.0, 150.0);
    spec.label(12.0, 14.0, 70.0, "&Name:");
    spec.text("name", 90.0, 10.0, 318.0, name);
    spec.label(12.0, 46.0, 70.0, "&Scope:");
    let mut scopes: Vec<&str> = vec!["Workbook"];
    scopes.extend(sheets.iter().map(String::as_str));
    spec.dropdown(
        "scope",
        90.0,
        42.0,
        318.0,
        &scopes,
        scope.map_or(0, |i| i + 1),
    );
    if editing {
        // Excel cannot move a name to another scope.
        if let Some(c) = spec.controls.last_mut() {
            c.enabled = false;
        }
    }
    spec.label(12.0, 78.0, 70.0, "Co&mment:");
    spec.text("comment", 90.0, 74.0, 318.0, comment);
    spec.label(12.0, 110.0, 70.0, "&Refers to:");
    spec.reference("refers", 90.0, 106.0, 318.0, refers_to);
    spec.focus = Some("name".to_owned());
    (
        spec,
        Box::new(Edit {
            editing,
            original: (name.to_owned(), scope),
            sheets,
        }),
    )
}

impl View for Edit {
    fn event(&mut self, s: &mut Shell, ui: &mut Ui, d: &mut Dialog, o: &Outcome) {
        if let Outcome::Ref {
            id,
            collapsed: false,
        } = o
        {
            d.set_text(ui, id, &selection_ref(s));
        }
    }

    fn commit(
        &mut self,
        s: &mut Shell,
        _ui: &mut Ui,
        _d: &mut Dialog,
        v: &Values,
    ) -> Result<(), String> {
        let name = v.text("name");
        let slot = v.index("scope").unwrap_or(0);
        let scope = slot.checked_sub(1).filter(|i| *i < self.sheets.len());
        let stay = |text: &str| {
            notice("Open365", text);
            Err(String::new())
        };
        if open365_xlsx::validate_defined_name(name).is_err() {
            return stay(NAME_INVALID);
        }
        let same_as_before =
            self.editing && self.original.0.eq_ignore_ascii_case(name) && self.original.1 == scope;
        let scope_label = scope.map_or("Workbook", |i| self.sheets[i].as_str());
        if !same_as_before
            && s.sheet
                .name_rows()
                .iter()
                .any(|r| r.name.eq_ignore_ascii_case(name) && r.scope == scope_label)
        {
            return stay(NAME_TAKEN);
        }
        let refers = v.text("refers").trim().trim_start_matches('=').to_owned();
        if refers.is_empty() || open365_calc::parse_formula(&refers).is_err() {
            return stay(FORMULA_PROBLEM);
        }
        let comment = v.text("comment");
        let req = NameRequest {
            name: name.to_owned(),
            scope,
            refers_to: refers,
            comment: (!comment.is_empty()).then(|| comment.to_owned()),
            hidden: false,
        };
        let done = if self.editing {
            s.sheet.name_edit(&self.original.0, self.original.1, req)
        } else {
            s.sheet.name_define(req)
        };
        match done {
            Ok(()) => Ok(()),
            Err(e) => stay(&e),
        }
    }
}

// ---- Delete ----------------------------------------------------------------------------------------------

#[derive(Debug)]
struct Delete {
    name: String,
    scope: Option<usize>,
}

fn delete(name: &str, scope: Option<usize>) -> (Spec, Box<dyn View>) {
    let mut spec = Spec::new("Open365", 380.0, 56.0);
    spec.label(
        14.0,
        14.0,
        352.0,
        &format!("Are you sure you want to delete the name '{name}'?"),
    );
    (
        spec,
        Box::new(Delete {
            name: name.to_owned(),
            scope,
        }),
    )
}

impl View for Delete {
    fn commit(
        &mut self,
        s: &mut Shell,
        _ui: &mut Ui,
        _d: &mut Dialog,
        _v: &Values,
    ) -> Result<(), String> {
        s.sheet.name_delete(&self.name, self.scope)
    }
}
