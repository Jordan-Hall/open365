//! The Formulas dialogs: Insert Function, Function Arguments, Name Manager (read side) and
//! Evaluate Formula.

use std::cell::RefCell;

use open365_ui_core::Ui;
use open365_ui_core::dialog::{ButtonRole, Dialog, FooterButton, Outcome, Spec, Values};

use super::{View, pick_selection, wrap};
use crate::dialogs::formulas_evaluate::EvalSession;
use crate::dialogs::formulas_function::{Category, formula_for, functions_in, search};
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

fn function_arguments(name: &str) -> (Spec, Box<dyn View>) {
    let sig = signature(name);
    let n = sig.boxes();
    let h = 34.0 + 30.0 * n as f32 + 92.0;
    let mut spec = Spec::new("Function Arguments", 440.0, h);
    spec.label(12.0, 8.0, 416.0, &sig.name).id = "fname".to_owned();
    for i in 0..n {
        let y = 32.0 + 30.0 * i as f32;
        spec.label(12.0, y + 3.0, 120.0, &sig.box_name(i));
        spec.reference(&format!("a{i}"), 134.0, y, 294.0, "");
    }
    let y = 32.0 + 30.0 * n as f32 + 4.0;
    spec.label(12.0, y, 416.0, &format!("Formula: ={}()", sig.name))
        .id = "preview".to_owned();
    for (i, line) in wrap(&sig.help, 66, 3).iter().enumerate() {
        spec.label(12.0, y + 24.0 + 15.0 * i as f32, 416.0, line);
    }
    spec.focus = Some("a0".to_owned());
    (spec, Box::new(FunctionArguments { sig }))
}

impl FunctionArguments {
    fn args<'a>(&self, v: &'a Values) -> Vec<&'a str> {
        (0..self.sig.boxes())
            .map(|i| v.text(&format!("a{i}")))
            .collect()
    }
}

impl View for FunctionArguments {
    fn event(&mut self, s: &mut Shell, ui: &mut Ui, d: &mut Dialog, o: &Outcome) {
        pick_selection(s, ui, d, o);
        let v = d.values(ui);
        d.set_text(
            ui,
            "preview",
            &format!("Formula: {}", formula_for(&self.sig.name, &self.args(&v))),
        );
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
    refers: Vec<String>,
}

fn name_manager(s: &mut Shell) -> (Spec, Box<dyn View>) {
    let rows = s.sheet.name_rows();
    let items: Vec<String> = rows
        .iter()
        .map(|r| format!("{}    {}    ({})", r.name, r.refers_to, r.scope))
        .collect();
    let mut spec = Spec::new("Name Manager", 520.0, 270.0).close_only();
    spec.button("new", 12.0, 8.0, 80.0, "&New...").disabled();
    spec.button("edit", 98.0, 8.0, 80.0, "&Edit...").disabled();
    spec.button("delete", 184.0, 8.0, 80.0, "&Delete")
        .disabled();
    spec.list(
        "names",
        open365_ui_core::Rect::new(12.0, 40.0, 496.0, 160.0),
        items,
        (!rows.is_empty()).then_some(0),
    );
    spec.label(12.0, 208.0, 200.0, "&Refers to:");
    spec.text(
        "refers",
        12.0,
        228.0,
        496.0,
        rows.first().map_or("", |r| r.refers_to.as_str()),
    );
    spec.focus = Some("names".to_owned());
    (
        spec,
        Box::new(NameManager {
            refers: rows.into_iter().map(|r| r.refers_to).collect(),
        }),
    )
}

impl View for NameManager {
    fn event(&mut self, _s: &mut Shell, ui: &mut Ui, d: &mut Dialog, o: &Outcome) {
        if let Outcome::Changed(id) = o
            && id == "names"
            && let Some(i) = d.values(ui).index("names")
        {
            let text = self.refers.get(i).cloned().unwrap_or_default();
            d.set_text(ui, "refers", &text);
        }
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
    let (r, c) = s.sheet.active;
    let mut spec = Spec::new("Evaluate Formula", 460.0, 110.0).footer(vec![
        FooterButton::new("step", "&Evaluate", ButtonRole::Default),
        FooterButton::new("restart", "&Restart", ButtonRole::Normal),
        FooterButton::new("cancel", "&Close", ButtonRole::Cancel),
    ]);
    spec.label(12.0, 8.0, 150.0, "Reference:");
    spec.label(12.0, 30.0, 150.0, &crate::sheet::a1(r, c)).id = "ref".to_owned();
    spec.label(180.0, 8.0, 270.0, "E&valuation:");
    spec.label(180.0, 30.0, 270.0, &session.text()).id = "formula".to_owned();
    spec.label(
        12.0,
        70.0,
        436.0,
        "To show the result of the underlined expression, click Evaluate.",
    )
    .id = "hint".to_owned();
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

impl View for Evaluate {
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
            "restart" => {
                self.session = self.start.clone();
                d.set_text(
                    ui,
                    "hint",
                    "To show the result of the underlined expression, click Evaluate.",
                );
            }
            _ => {}
        }
        d.set_text(ui, "formula", &self.session.text());
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
