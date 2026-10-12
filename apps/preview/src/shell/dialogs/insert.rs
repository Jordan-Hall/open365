//! The Sheets Insert tab dialogs: Create Table, Create PivotTable, Insert Hyperlink and Symbol.
//! OK sends the dialog's answer as the value of the ribbon command the dialog stands for.

use open365_ui_core::Rect;
use open365_ui_core::Ui;
use open365_ui_core::dialog::{Dialog, Outcome, Spec, Values};

use super::{View, abs_range_text, parse_ref, pick_selection, range_text};
use crate::dialogs::insert_symbol::{SUBSETS, subset_chars};
use crate::dialogs::insert_table::CreateTableDialog;
use crate::dialogs::request::DialogRequest;
use crate::shell::Shell;

pub(super) fn make(s: &mut Shell, req: &DialogRequest) -> Option<(Spec, Box<dyn View>)> {
    Some(match req {
        DialogRequest::CreateTable => create_table(s),
        DialogRequest::CreatePivot => create_pivot(s),
        DialogRequest::InsertLink => insert_link(s),
        DialogRequest::Symbol => symbol(),
        _ => return None,
    })
}

/// Runs `id` and turns the status the model left into the dialog's error.
fn run_checked(s: &mut Shell, ui: &mut Ui, id: &str) -> Result<(), String> {
    s.sheet.status.clear();
    s.run_id(ui, id);
    if s.sheet.status.is_empty() {
        Ok(())
    } else {
        Err(s.sheet.status.clone())
    }
}

/// Selects the range typed in a reference box.
fn select_typed(s: &mut Shell, text: &str) -> Result<(), String> {
    let r = parse_ref(text).ok_or_else(|| "The reference isn't valid.".to_owned())?;
    s.sheet.select_cell((r.0, r.1));
    s.sheet.extend_to((r.2, r.3));
    Ok(())
}

// ---- Create Table ----------------------------------------------------------------------------

#[derive(Debug)]
struct CreateTable;

fn create_table(s: &mut Shell) -> (Spec, Box<dyn View>) {
    let range = s.sheet.insert_data_range();
    let kinds = s.sheet.insert_kinds(range);
    let model = CreateTableDialog::new(range, &kinds);
    let mut spec = Spec::new("Create Table", 300.0, 92.0);
    spec.label(12.0, 8.0, 270.0, "Where is the data for your table?");
    spec.reference("range", 12.0, 28.0, 270.0, &model.range_text());
    spec.check(
        "headers",
        12.0,
        58.0,
        270.0,
        "My table has &headers",
        model.has_headers,
    );
    spec.focus = Some("range".to_owned());
    (spec, Box::new(CreateTable))
}

impl View for CreateTable {
    fn event(&mut self, s: &mut Shell, ui: &mut Ui, d: &mut Dialog, o: &Outcome) {
        pick_selection(s, ui, d, o);
    }

    fn commit(
        &mut self,
        s: &mut Shell,
        ui: &mut Ui,
        _d: &mut Dialog,
        v: &Values,
    ) -> Result<(), String> {
        select_typed(s, v.text("range"))?;
        let arg = if v.flag("headers") {
            "headers"
        } else {
            "no-headers"
        };
        run_checked(s, ui, &format!("insert.tables.table={arg}"))
    }
}

// ---- Create PivotTable -----------------------------------------------------------------------

#[derive(Debug)]
struct CreatePivot;

fn create_pivot(s: &mut Shell) -> (Spec, Box<dyn View>) {
    let range = s.sheet.insert_data_range();
    let mut spec = Spec::new("Create PivotTable", 340.0, 160.0);
    spec.label(12.0, 8.0, 310.0, "Choose the data that you want to analyze");
    spec.label(12.0, 32.0, 310.0, "&Table/Range:");
    spec.reference("range", 12.0, 50.0, 310.0, &abs_range_text(range));
    spec.label(
        12.0,
        82.0,
        310.0,
        "Choose where you want the PivotTable to be placed",
    );
    spec.radio("new", "where", 12.0, 104.0, 310.0, "&New Worksheet", true);
    spec.radio(
        "existing",
        "where",
        12.0,
        126.0,
        310.0,
        "&Existing Worksheet",
        false,
    )
    .disabled();
    spec.focus = Some("range".to_owned());
    (spec, Box::new(CreatePivot))
}

impl View for CreatePivot {
    fn event(&mut self, s: &mut Shell, ui: &mut Ui, d: &mut Dialog, o: &Outcome) {
        pick_selection(s, ui, d, o);
    }

    fn commit(
        &mut self,
        s: &mut Shell,
        ui: &mut Ui,
        _d: &mut Dialog,
        v: &Values,
    ) -> Result<(), String> {
        let r = parse_ref(v.text("range"))
            .ok_or_else(|| "The PivotTable field name or reference isn't valid.".to_owned())?;
        run_checked(
            s,
            ui,
            &format!("insert.tables.pivot-table={}", range_text(r)),
        )
    }
}

// ---- Insert Hyperlink ------------------------------------------------------------------------

#[derive(Debug)]
struct InsertLink;

fn insert_link(s: &mut Shell) -> (Spec, Box<dyn View>) {
    let (r, c) = s.sheet.active;
    let mut spec = Spec::new("Insert Hyperlink", 400.0, 130.0);
    spec.label(12.0, 12.0, 100.0, "Text to &display:");
    spec.text("text", 120.0, 8.0, 268.0, &s.sheet.display(r, c));
    spec.label(12.0, 42.0, 100.0, "Screen&Tip:");
    spec.text("tip", 120.0, 38.0, 268.0, "");
    spec.label(12.0, 72.0, 100.0, "&Address:");
    spec.text("addr", 120.0, 68.0, 268.0, "");
    spec.focus = Some("addr".to_owned());
    (spec, Box::new(InsertLink))
}

impl View for InsertLink {
    fn commit(
        &mut self,
        s: &mut Shell,
        ui: &mut Ui,
        _d: &mut Dialog,
        v: &Values,
    ) -> Result<(), String> {
        let addr = v.text("addr").trim();
        if addr.is_empty() {
            return Err("Type the address of the link.".to_owned());
        }
        let id = format!(
            "insert.links.link={}|{}|{}",
            addr,
            v.text("text").trim(),
            v.text("tip").trim()
        );
        run_checked(s, ui, &id)
    }
}

// ---- Symbol ----------------------------------------------------------------------------------

#[derive(Debug)]
struct Symbol;

fn symbol() -> (Spec, Box<dyn View>) {
    let names: Vec<&str> = SUBSETS.iter().map(|x| x.0).collect();
    let mut spec = Spec::new("Symbol", 360.0, 270.0);
    spec.label(12.0, 12.0, 60.0, "S&ubset:");
    spec.dropdown("subset", 80.0, 8.0, 268.0, &names, 0);
    let chars: Vec<String> = subset_chars(names[0]).iter().map(char::to_string).collect();
    spec.list("chars", Rect::new(12.0, 40.0, 336.0, 180.0), chars, Some(0));
    spec.focus = Some("chars".to_owned());
    (spec, Box::new(Symbol))
}

impl View for Symbol {
    fn event(&mut self, _s: &mut Shell, ui: &mut Ui, d: &mut Dialog, o: &Outcome) {
        if let Outcome::Changed(id) = o
            && id == "subset"
        {
            let i = d
                .values(ui)
                .index("subset")
                .unwrap_or(0)
                .min(SUBSETS.len() - 1);
            let chars = subset_chars(SUBSETS[i].0)
                .iter()
                .map(char::to_string)
                .collect();
            d.set_items(ui, "chars", chars, Some(0));
        }
    }

    fn commit(
        &mut self,
        s: &mut Shell,
        ui: &mut Ui,
        _d: &mut Dialog,
        v: &Values,
    ) -> Result<(), String> {
        let subset = v.index("subset").unwrap_or(0).min(SUBSETS.len() - 1);
        let ch = v
            .index("chars")
            .and_then(|i| subset_chars(SUBSETS[subset].0).get(i).copied())
            .ok_or_else(|| "Choose a symbol to insert.".to_owned())?;
        run_checked(s, ui, &format!("insert.symbols.symbol=U+{:04X}", ch as u32))
    }
}
