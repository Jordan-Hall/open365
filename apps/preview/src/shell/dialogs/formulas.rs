//! The Formulas dialogs: Insert Function, Function Arguments, Name Manager (read side) and
//! Evaluate Formula.

use std::cell::RefCell;

use open365_ui_core::Ui;
use open365_ui_core::dialog::{ButtonRole, Dialog, FooterButton, Outcome, Spec, Values};

use super::{View, pick_selection, wrap};
use crate::dialogs::formulas_evaluate::EvalSession;
use crate::dialogs::formulas_function::{Category, formula_for, functions_in, search};
use crate::dialogs::formulas_names::NameRow;
use crate::dialogs::function_args::{Signature, signature};
use crate::dialogs::request::DialogRequest;
use crate::shell::Shell;

thread_local! {
    static NEXT: RefCell<Option<DialogRequest>> = const { RefCell::new(None) };
}

/// Asks the host to open `req` as soon as the current dialog has closed (Insert Function leads
/// to Function Arguments).
pub(super) fn queue(req: DialogRequest) {
    NEXT.with(|n| *n.borrow_mut() = Some(req));
}

pub(super) fn take_queued() -> Option<DialogRequest> {
    NEXT.with(|n| n.borrow_mut().take())
}

pub(super) fn make(s: &mut Shell, req: &DialogRequest) -> Option<(Spec, Box<dyn View>)> {
    match req {
        DialogRequest::InsertFunction { category } => Some(insert_function(s, *category)),
        DialogRequest::FunctionArguments { name } => Some(function_arguments(name)),
        DialogRequest::NameManager => Some(name_manager(s)),
        DialogRequest::EvaluateFormula => evaluate(s),
        _ => None,
    }
}

// ---- Insert Function -----------------------------------------------------------------------

#[derive(Debug)]
struct InsertFunction {
    /// The functions in the list now.
    names: Vec<String>,
    categories: Vec<Option<Category>>,
}

fn category_items() -> (Vec<String>, Vec<Option<Category>>) {
    let mut labels = vec!["Most Recently Used".to_owned(), "All".to_owned()];
    let mut cats = vec![None, None];
    for c in [
        Category::Financial,
        Category::DateTime,
        Category::MathTrig,
        Category::More,
        Category::LookupReference,
        Category::Text,
        Category::Logical,
    ] {
        labels.push(c.label().to_owned());
        cats.push(Some(c));
    }
    (labels, cats)
}

fn insert_function(s: &mut Shell, category: Option<Category>) -> (Spec, Box<dyn View>) {
    let has_recent = !s.sheet.recent_functions().is_empty();
    let (labels, cats) = category_items();
    let selected = match category {
        Some(c) => cats.iter().position(|x| *x == Some(c)).unwrap_or(1),
        None if has_recent => 0,
        None => 1,
    };
    let mut spec = Spec::new("Insert Function", 424.0, 318.0);
    spec.label(12.0, 8.0, 300.0, "Search for a &function:");
    spec.text("search", 12.0, 28.0, 300.0, "");
    spec.button("go", 322.0, 28.0, 90.0, "&Go");
    spec.label(12.0, 62.0, 300.0, "Or select a &category:");
    let items: Vec<&str> = labels.iter().map(String::as_str).collect();
    spec.dropdown("category", 12.0, 82.0, 300.0, &items, selected);
    spec.label(12.0, 114.0, 300.0, "Select a f&unction:");
    let mut view = InsertFunction {
        names: Vec::new(),
        categories: cats,
    };
    view.names = view.list_for(s, selected, "");
    spec.list(
        "funcs",
        open365_ui_core::Rect::new(12.0, 134.0, 400.0, 100.0),
        view.names.clone(),
        (!view.names.is_empty()).then_some(0),
    );
    let lines = view.description(if view.names.is_empty() { None } else { Some(0) });
    for i in 0..4 {
        spec.label(
            12.0,
            242.0 + 15.0 * i as f32,
            400.0,
            lines.get(i).map_or("", String::as_str),
        )
        .id = format!("d{i}");
    }
    spec.focus = Some("search".to_owned());
    (spec, Box::new(view))
}

impl InsertFunction {
    fn list_for(&self, s: &Shell, category: usize, query: &str) -> Vec<String> {
        if !query.trim().is_empty() {
            return search(query, None).into_iter().map(str::to_owned).collect();
        }
        match category {
            0 => s.sheet.recent_functions().to_vec(),
            1 => search("", None).into_iter().map(str::to_owned).collect(),
            i => self
                .categories
                .get(i)
                .copied()
                .flatten()
                .map(|c| functions_in(c).into_iter().map(str::to_owned).collect())
                .unwrap_or_default(),
        }
    }

    /// The syntax and help text of the selected function, as lines.
    fn description(&self, sel: Option<usize>) -> Vec<String> {
        let sig = sel.and_then(|i| self.names.get(i)).map(|n| signature(n));
        let mut lines = Vec::new();
        if let Some(sig) = &sig {
            lines.push(sig.syntax());
            lines.extend(wrap(&sig.help, 64, 3));
        }
        lines
    }

    fn describe(&self, ui: &mut Ui, d: &mut Dialog, sel: Option<usize>) {
        let lines = self.description(sel);
        for i in 0..4 {
            d.set_text(
                ui,
                &format!("d{i}"),
                lines.get(i).map_or("", String::as_str),
            );
        }
    }
}

impl View for InsertFunction {
    fn event(&mut self, s: &mut Shell, ui: &mut Ui, d: &mut Dialog, o: &Outcome) {
        let v = d.values(ui);
        match o {
            Outcome::Changed(id) if id == "search" || id == "category" => {
                let cat = v.index("category").unwrap_or(1);
                self.names = self.list_for(s, cat, v.text("search"));
                d.set_items(
                    ui,
                    "funcs",
                    self.names.clone(),
                    (!self.names.is_empty()).then_some(0),
                );
                self.describe(ui, d, (!self.names.is_empty()).then_some(0));
            }
            Outcome::Changed(id) if id == "funcs" => self.describe(ui, d, v.index("funcs")),
            Outcome::Pressed(id) if id == "go" => {
                let cat = v.index("category").unwrap_or(1);
                self.names = self.list_for(s, cat, v.text("search"));
                d.set_items(
                    ui,
                    "funcs",
                    self.names.clone(),
                    (!self.names.is_empty()).then_some(0),
                );
                self.describe(ui, d, (!self.names.is_empty()).then_some(0));
            }
            _ => {}
        }
    }

    fn commit(
        &mut self,
        _s: &mut Shell,
        ui: &mut Ui,
        d: &mut Dialog,
        v: &Values,
    ) -> Result<(), String> {
        let _ = (ui, d);
        let name = v
            .index("funcs")
            .and_then(|i| self.names.get(i))
            .ok_or_else(|| "Select a function.".to_owned())?;
        queue(DialogRequest::FunctionArguments { name: name.clone() });
        Ok(())
    }
}

// ---- Function Arguments ----------------------------------------------------------------------

#[derive(Debug)]
struct FunctionArguments {
    sig: Signature,
}

const FA_W: f32 = 520.0;

fn function_arguments(name: &str) -> (Spec, Box<dyn View>) {
    let sig = signature(name);
    let n = sig.boxes();
    let y = 32.0 + 30.0 * n as f32 + 4.0;
    let mut spec = Spec::new("Function Arguments", FA_W, y + 104.0);
    spec.label(12.0, 8.0, FA_W - 24.0, &sig.name).id = "fname".to_owned();
    for i in 0..n {
        let row = 32.0 + 30.0 * i as f32;
        spec.label(12.0, row + 3.0, 104.0, &sig.box_name(i));
        spec.reference(&format!("a{i}"), 120.0, row, 236.0, "");
        spec.label(364.0, row + 3.0, FA_W - 376.0, "").id = format!("p{i}");
    }
    for (i, line) in wrap(&sig.help, 82, 2).iter().enumerate() {
        spec.label(12.0, y + 15.0 * i as f32, FA_W - 24.0, line);
    }
    for i in 0..2 {
        spec.label(12.0, y + 36.0 + 15.0 * i as f32, FA_W - 24.0, "")
            .id = format!("adesc{i}");
    }
    spec.label(12.0, y + 78.0, FA_W - 24.0, "Formula result =")
        .id = "result".to_owned();
    spec.focus = Some("a0".to_owned());
    (spec, Box::new(FunctionArguments { sig }))
}

impl FunctionArguments {
    fn args<'a>(&self, v: &'a Values) -> Vec<&'a str> {
        (0..self.sig.boxes())
            .map(|i| v.text(&format!("a{i}")))
            .collect()
    }

    /// Argument `i`'s description under the boxes.
    fn describe(&self, ui: &mut Ui, d: &mut Dialog, i: usize) {
        let lines = wrap(&self.sig.arg_help(i), 82, 2);
        for k in 0..2 {
            d.set_text(
                ui,
                &format!("adesc{k}"),
                lines.get(k).map_or("", String::as_str),
            );
        }
    }

    /// The value of every argument and of the whole call, beside the boxes and under them.
    fn preview(&self, s: &Shell, ui: &mut Ui, d: &mut Dialog) {
        let v = d.values(ui);
        let args = self.args(&v);
        let texts: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
        for (i, shown) in s.sheet.evaluate_previews(&texts, true).iter().enumerate() {
            let text = shown
                .as_ref()
                .map_or_else(String::new, |t| format!("= {t}"));
            d.set_text(ui, &format!("p{i}"), &text);
        }
        let whole = vec![formula_for(&self.sig.name, &args)];
        let result = match s.sheet.evaluate_previews(&whole, false).pop().flatten() {
            Some(t) if args.iter().any(|a| !a.trim().is_empty()) || self.sig.args.is_empty() => {
                format!("Formula result = {t}")
            }
            _ => "Formula result =".to_owned(),
        };
        d.set_text(ui, "result", &result);
    }
}

impl View for FunctionArguments {
    fn opened(&mut self, s: &mut Shell, ui: &mut Ui, d: &mut Dialog) {
        self.describe(ui, d, 0);
        self.preview(s, ui, d);
    }

    fn event(&mut self, s: &mut Shell, ui: &mut Ui, d: &mut Dialog, o: &Outcome) {
        pick_selection(s, ui, d, o);
        let touched = match o {
            Outcome::Changed(id) => id.strip_prefix('a').and_then(|n| n.parse::<usize>().ok()),
            Outcome::Ref { id, .. } => id.strip_prefix('a').and_then(|n| n.parse::<usize>().ok()),
            _ => None,
        };
        if let Some(i) = touched {
            self.describe(ui, d, i);
        }
        self.preview(s, ui, d);
    }

    fn commit(
        &mut self,
        s: &mut Shell,
        ui: &mut Ui,
        d: &mut Dialog,
        v: &Values,
    ) -> Result<(), String> {
        let _ = (ui, d);
        s.sheet.apply_function(&self.sig.name, &self.args(v))
    }
}

// ---- Name Manager -----------------------------------------------------------------------------

#[derive(Debug)]
struct NameManager {
    rows: Vec<NameRow>,
}

fn row_text(r: &NameRow) -> String {
    format!("{}    {}    ({})", r.name, r.refers_to, r.scope)
}

fn name_manager(s: &mut Shell) -> (Spec, Box<dyn View>) {
    let rows = s.sheet.name_rows();
    let items: Vec<String> = rows.iter().map(row_text).collect();
    let any = !rows.is_empty();
    let mut spec = Spec::new("Name Manager", 520.0, 270.0).close_only();
    spec.button("new", 12.0, 8.0, 80.0, "&New...");
    spec.button("edit", 98.0, 8.0, 80.0, "&Edit...");
    if !any && let Some(c) = spec.controls.last_mut() {
        c.enabled = false;
    }
    spec.button("delete", 184.0, 8.0, 80.0, "&Delete");
    if !any && let Some(c) = spec.controls.last_mut() {
        c.enabled = false;
    }
    spec.list(
        "names",
        open365_ui_core::Rect::new(12.0, 40.0, 496.0, 160.0),
        items,
        any.then_some(0),
    );
    spec.label(12.0, 208.0, 200.0, "Refers to:");
    spec.text(
        "refers",
        12.0,
        228.0,
        496.0,
        rows.first().map_or("", |r| r.refers_to.as_str()),
    )
    .enabled = false;
    spec.focus = Some("names".to_owned());
    (spec, Box::new(NameManager { rows }))
}

impl NameManager {
    fn selected(&self, ui: &Ui, d: &Dialog) -> Option<&NameRow> {
        d.values(ui).index("names").and_then(|i| self.rows.get(i))
    }

    /// Shows the refers-to text of the selected name and enables Edit and Delete for it.
    fn sync(&self, ui: &mut Ui, d: &mut Dialog) {
        let text = self
            .selected(ui, d)
            .map(|r| r.refers_to.clone())
            .unwrap_or_default();
        let has = self.selected(ui, d).is_some();
        d.set_text(ui, "refers", &text);
        d.set_enabled(ui, "edit", has);
        d.set_enabled(ui, "delete", has);
    }
}

impl View for NameManager {
    fn event(&mut self, s: &mut Shell, ui: &mut Ui, d: &mut Dialog, o: &Outcome) {
        match o {
            Outcome::Changed(id) if id == "names" => self.sync(ui, d),
            Outcome::Pressed(id) if id == "new" => {
                super::request_child(DialogRequest::NameEdit {
                    name: String::new(),
                    scope: None,
                    refers_to: super::names::selection_ref(s),
                    comment: String::new(),
                    editing: false,
                });
            }
            Outcome::Pressed(id) if id == "edit" => {
                if let Some(r) = self.selected(ui, d) {
                    super::request_child(DialogRequest::NameEdit {
                        name: r.name.clone(),
                        scope: r.scope_index,
                        refers_to: r.refers_to.clone(),
                        comment: r.comment.clone(),
                        editing: true,
                    });
                }
            }
            Outcome::Pressed(id) if id == "delete" => {
                if let Some(r) = self.selected(ui, d) {
                    super::request_child(DialogRequest::DeleteName {
                        name: r.name.clone(),
                        scope: r.scope_index,
                    });
                }
            }
            _ => {}
        }
    }

    fn child_done(
        &mut self,
        s: &mut Shell,
        ui: &mut Ui,
        d: &mut Dialog,
        applied: bool,
    ) -> Result<(), String> {
        if !applied {
            return Ok(());
        }
        let was = d.values(ui).index("names").unwrap_or(0);
        self.rows = s.sheet.name_rows();
        let items: Vec<String> = self.rows.iter().map(row_text).collect();
        let sel = (!items.is_empty()).then(|| was.min(items.len() - 1));
        d.set_items(ui, "names", items, sel);
        self.sync(ui, d);
        Ok(())
    }

    fn commit(
        &mut self,
        _s: &mut Shell,
        _ui: &mut Ui,
        _d: &mut Dialog,
        _v: &Values,
    ) -> Result<(), String> {
        Ok(())
    }
}

// ---- Evaluate Formula ---------------------------------------------------------------------------

#[derive(Debug)]
struct Evaluate {
    session: EvalSession,
    start: EvalSession,
}

fn evaluate(s: &mut Shell) -> Option<(Spec, Box<dyn View>)> {
    let session = s.sheet.evaluate_session()?;
    let mut spec = Spec::new("Evaluate Formula", 480.0, 110.0).footer(vec![
        FooterButton::new("step", "&Evaluate", ButtonRole::Default),
        FooterButton::new("stepin", "Step &In", ButtonRole::Normal),
        FooterButton::new("stepout", "Step &Out", ButtonRole::Normal),
        FooterButton::new("restart", "&Restart", ButtonRole::Normal),
        FooterButton::new("cancel", "&Close", ButtonRole::Cancel),
    ]);
    spec.label(12.0, 8.0, 150.0, "Reference:");
    spec.label(12.0, 30.0, 150.0, &s.sheet.evaluate_reference(&session))
        .id = "ref".to_owned();
    spec.label(180.0, 8.0, 290.0, "E&valuation:");
    spec.label(180.0, 30.0, 290.0, &session.text()).id = "formula".to_owned();
    spec.label(12.0, 70.0, 456.0, HINT).id = "hint".to_owned();
    spec.focus = Some("step".to_owned());
    // Evaluate is the footer's first control, so the label buttons are not tab stops.
    Some((
        spec,
        Box::new(Evaluate {
            start: session.clone(),
            session,
        }),
    ))
}

const HINT: &str = "To show the result of the underlined expression, click Evaluate.";

impl Evaluate {
    /// Shows the formula and the cell it is in, and enables Step In and Step Out as they apply.
    fn show(&self, s: &Shell, ui: &mut Ui, d: &mut Dialog) {
        d.set_text(ui, "formula", &self.session.text());
        d.set_text(ui, "ref", &s.sheet.evaluate_reference(&self.session));
        d.set_enabled(ui, "stepin", s.sheet.evaluate_can_step_in(&self.session));
        d.set_enabled(ui, "stepout", self.session.can_step_out());
    }
}

impl View for Evaluate {
    fn opened(&mut self, s: &mut Shell, ui: &mut Ui, d: &mut Dialog) {
        self.show(s, ui, d);
    }

    fn event(&mut self, s: &mut Shell, ui: &mut Ui, d: &mut Dialog, o: &Outcome) {
        let Outcome::Pressed(id) = o else { return };
        match id.as_str() {
            // Done: Excel turns Evaluate into Restart.
            "step" if !s.sheet.evaluate_step(&mut self.session) => {
                d.set_text(
                    ui,
                    "hint",
                    "The evaluation is complete. Click Restart to evaluate again.",
                );
            }
            "stepin" => {
                s.sheet.evaluate_step_in(&mut self.session);
            }
            "stepout" => {
                s.sheet.evaluate_step_out(&mut self.session);
            }
            "restart" => {
                self.session = self.start.clone();
                d.set_text(ui, "hint", HINT);
            }
            _ => {}
        }
        self.show(s, ui, d);
    }

    fn commit(
        &mut self,
        _s: &mut Shell,
        _ui: &mut Ui,
        _d: &mut Dialog,
        _v: &Values,
    ) -> Result<(), String> {
        Ok(())
    }
}
