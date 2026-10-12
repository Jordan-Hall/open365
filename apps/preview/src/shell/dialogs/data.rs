//! The Data tab dialogs: Sort, Advanced Filter, Text to Columns, Remove Duplicates, Data
//! Validation, Consolidate, Goal Seek, Subtotal and Group.

use open365_commands::data_ops::{
    AllowKind, Cmp, ColumnFilter, DataCmp, ErrorAlert, ValidationRule,
};
use open365_ui_core::dialog::{ButtonRole, Dialog, FooterButton, Outcome, Spec, Values};
use open365_ui_core::{Rect, Ui};
use open365_xlsx::{ErrorStyle, ValidationOperator, ValidationType};

use super::{
    View, abs_range_text, col_letters, parse_ref, pick_selection, range_text, request_close,
    selection_text,
};
use crate::dialogs::data_sort::SortDialog;
use crate::dialogs::request::DialogRequest;
use crate::shell::Shell;

pub(super) fn make(s: &mut Shell, req: &DialogRequest) -> Option<(Spec, Box<dyn View>)> {
    Some(match req {
        DialogRequest::Sort => sort(s),
        DialogRequest::AdvancedFilter => advanced(s),
        DialogRequest::TextToColumns => text_to_columns(s),
        DialogRequest::RemoveDuplicates => remove_duplicates(s),
        DialogRequest::DataValidation => validation(s),
        DialogRequest::Consolidate => consolidate(s),
        DialogRequest::GoalSeek => goal_seek(s),
        DialogRequest::Subtotal => subtotal(s),
        DialogRequest::CustomFilter { column, operand } => custom_filter(s, *column, operand),
        DialogRequest::Group => group(s),
        _ => return None,
    })
}

fn strs(v: &[String]) -> Vec<&str> {
    v.iter().map(String::as_str).collect()
}

// ---- Sort ---------------------------------------------------------------------------------------

const SORT_ROWS: usize = 8;

#[derive(Debug)]
struct Sort {
    model: SortDialog,
    /// The level the Delete, Copy and Move buttons act on.
    current: usize,
}

fn sort(s: &mut Shell) -> (Spec, Box<dyn View>) {
    let model = SortDialog::open(&s.sheet);
    let names = model.column_names(&s.sheet);
    let titles: Vec<String> = names.iter().map(|(_, n)| n.clone()).collect();
    let mut spec = Spec::new("Sort", 560.0, 346.0);
    spec.button("add", 12.0, 8.0, 96.0, "&Add Level");
    spec.button("delete", 114.0, 8.0, 96.0, "&Delete Level");
    spec.button("copy", 216.0, 8.0, 96.0, "&Copy Level");
    spec.button("up", 318.0, 8.0, 84.0, "Move &Up");
    spec.button("down", 408.0, 8.0, 100.0, "Move Do&wn");
    spec.check(
        "header",
        12.0,
        40.0,
        200.0,
        "My data has &headers",
        model.has_header,
    );
    spec.label(12.0, 66.0, 200.0, "Column");
    spec.label(256.0, 66.0, 100.0, "Sort On");
    spec.label(376.0, 66.0, 100.0, "Order");
    for i in 0..SORT_ROWS {
        let y = 86.0 + 28.0 * i as f32;
        let sel = model
            .levels
            .get(i)
            .and_then(|l| names.iter().position(|(c, _)| *c == l.index))
            .unwrap_or(0);
        let on = i < model.levels.len();
        spec.dropdown(&format!("col{i}"), 12.0, y, 236.0, &strs(&titles), sel)
            .enabled = on;
        spec.dropdown(&format!("on{i}"), 256.0, y, 112.0, &["Cell Values"], 0)
            .enabled = on;
        let order = usize::from(model.levels.get(i).is_some_and(|l| l.descending));
        spec.dropdown(
            &format!("ord{i}"),
            376.0,
            y,
            172.0,
            &["A to Z", "Z to A"],
            order,
        )
        .enabled = on;
    }
    spec.check(
        "case",
        12.0,
        316.0,
        150.0,
        "Case sensiti&ve",
        model.case_sensitive,
    );
    spec.check(
        "ltr",
        170.0,
        316.0,
        200.0,
        "Sort left to &right",
        model.left_to_right,
    );
    spec.focus = Some("col0".to_owned());
    (spec, Box::new(Sort { model, current: 0 }))
}

impl Sort {
    /// Hides the rows beyond the levels.
    fn show_levels(&self, ui: &mut Ui, d: &mut Dialog) {
        for i in 0..SORT_ROWS {
            let on = i < self.model.levels.len();
            for k in ["col", "on", "ord"] {
                d.set_enabled(ui, &format!("{k}{i}"), on);
            }
        }
        d.set_enabled(ui, "delete", self.model.levels.len() > 1);
        d.set_enabled(ui, "add", self.model.levels.len() < SORT_ROWS);
        d.set_enabled(ui, "copy", self.model.levels.len() < SORT_ROWS);
    }

    /// Pulls the controls into the model.
    fn read(&mut self, s: &Shell, v: &Values) {
        self.model.has_header = v.flag("header");
        self.model.case_sensitive = v.flag("case");
        self.model.left_to_right = v.flag("ltr");
        let names = self.model.column_names(&s.sheet);
        for (i, l) in self.model.levels.iter_mut().enumerate() {
            if let Some(c) = v.index(&format!("col{i}")).and_then(|k| names.get(k)) {
                l.index = c.0;
            }
            l.descending = v.index(&format!("ord{i}")) == Some(1);
        }
    }

    /// Pushes the model into the controls.
    fn write(&self, s: &Shell, ui: &mut Ui, d: &mut Dialog) {
        let names = self.model.column_names(&s.sheet);
        let titles: Vec<String> = names.iter().map(|(_, n)| n.clone()).collect();
        for (i, l) in self.model.levels.iter().enumerate() {
            let sel = names.iter().position(|(c, _)| *c == l.index).unwrap_or(0);
            d.set_items(ui, &format!("col{i}"), titles.clone(), Some(sel));
            d.set_choice(ui, &format!("ord{i}"), usize::from(l.descending));
        }
        self.show_levels(ui, d);
    }
}

impl View for Sort {
    fn event(&mut self, s: &mut Shell, ui: &mut Ui, d: &mut Dialog, o: &Outcome) {
        let v = d.values(ui);
        self.read(s, &v);
        match o {
            Outcome::Changed(id) => {
                if let Some(n) = id.strip_prefix("col").or_else(|| id.strip_prefix("ord")) {
                    self.current = n.parse().unwrap_or(0);
                }
                if id == "header" || id == "ltr" {
                    self.write(s, ui, d);
                }
            }
            Outcome::Pressed(id) => {
                let at = self.current.min(self.model.levels.len().saturating_sub(1));
                match id.as_str() {
                    "add" => {
                        if let Err(e) = self.model.add_level() {
                            d.set_error(ui, &e);
                        }
                        self.current = self.model.levels.len() - 1;
                    }
                    "delete" => {
                        self.model.delete_level(at);
                        self.current = at.saturating_sub(1);
                    }
                    "copy" => {
                        if let Err(e) = self.model.copy_level(at) {
                            d.set_error(ui, &e);
                        }
                    }
                    "up" => {
                        self.model.move_level(at, true);
                        self.current = at.saturating_sub(1);
                    }
                    "down" => {
                        self.model.move_level(at, false);
                        self.current = (at + 1).min(self.model.levels.len() - 1);
                    }
                    _ => {}
                }
                self.write(s, ui, d);
            }
            _ => {}
        }
    }

    fn commit(
        &mut self,
        s: &mut Shell,
        ui: &mut Ui,
        d: &mut Dialog,
        v: &Values,
    ) -> Result<(), String> {
        let _ = (ui, d);
        self.read(s, v);
        self.model.apply(&mut s.sheet)
    }
}

// ---- Advanced Filter -------------------------------------------------------------------------------

#[derive(Debug)]
struct Advanced;

fn advanced(s: &mut Shell) -> (Spec, Box<dyn View>) {
    let mut spec = Spec::new("Advanced Filter", 380.0, 224.0);
    spec.label(12.0, 8.0, 200.0, "Action");
    spec.radio(
        "inplace",
        "action",
        24.0,
        28.0,
        340.0,
        "&Filter the list, in-place",
        true,
    );
    spec.radio(
        "copy",
        "action",
        24.0,
        50.0,
        340.0,
        "Copy to &another location",
        false,
    );
    spec.label(12.0, 80.0, 120.0, "&List range:");
    spec.reference(
        "list",
        140.0,
        76.0,
        228.0,
        &abs_range_text(s.sheet.data_target()),
    );
    spec.label(12.0, 108.0, 120.0, "&Criteria range:");
    spec.reference("criteria", 140.0, 104.0, 228.0, "");
    spec.label(12.0, 136.0, 120.0, "Copy &to:");
    spec.reference("dest", 140.0, 132.0, 228.0, "").disabled();
    spec.check("unique", 12.0, 166.0, 300.0, "&Unique records only", false);
    spec.focus = Some("list".to_owned());
    (spec, Box::new(Advanced))
}

impl View for Advanced {
    fn event(&mut self, s: &mut Shell, ui: &mut Ui, d: &mut Dialog, o: &Outcome) {
        pick_selection(s, ui, d, o);
        if let Outcome::Changed(id) = o
            && (id == "inplace" || id == "copy")
        {
            let copy = d.values(ui).radio("action") == "copy";
            d.set_enabled(ui, "dest", copy);
        }
    }

    fn commit(
        &mut self,
        s: &mut Shell,
        ui: &mut Ui,
        _d: &mut Dialog,
        v: &Values,
    ) -> Result<(), String> {
        let list = parse_ref(v.text("list")).ok_or("Enter a list range.")?;
        let criteria = parse_ref(v.text("criteria")).ok_or("Enter a criteria range.")?;
        let mut cmd = format!(
            "data.sort-filter.advanced=list={};criteria={}",
            range_text(list),
            range_text(criteria)
        );
        if v.radio("action") == "copy" {
            let dest = parse_ref(v.text("dest")).ok_or("Enter a cell to copy to.")?;
            cmd.push_str(&format!(
                ";copy={}",
                range_text((dest.0, dest.1, dest.0, dest.1))
            ));
        }
        if v.flag("unique") {
            cmd.push_str(";unique");
        }
        s.run_id(ui, &cmd);
        Ok(())
    }
}

// ---- Text to Columns --------------------------------------------------------------------------------

#[derive(Debug)]
struct TextToColumns {
    /// Step 1 chose fixed width.
    fixed: bool,
}

fn text_to_columns(s: &mut Shell) -> (Spec, Box<dyn View>) {
    let r = s.sheet.range();
    let mut spec = Spec::new("Convert Text to Columns Wizard", 480.0, 270.0).footer(vec![
        FooterButton::new("back", "< &Back", ButtonRole::Normal),
        FooterButton::new("next", "&Next >", ButtonRole::Normal),
        FooterButton::new("ok", "&Finish", ButtonRole::Default),
        FooterButton::new("cancel", "Cancel", ButtonRole::Cancel),
    ]);
    // Step 1.
    spec.on_page(Some(0));
    spec.note(12.0, 8.0, 456.0, 32.0, "The Text Wizard has determined that your data is Delimited.\nIf this is correct, choose Next, or choose the data type that best describes your data.");
    spec.label(12.0, 50.0, 400.0, "Original data type");
    spec.radio(
        "delimited",
        "kind",
        24.0,
        72.0,
        420.0,
        "&Delimited - Characters such as commas or tabs separate each field.",
        true,
    );
    spec.radio(
        "fixed",
        "kind",
        24.0,
        96.0,
        420.0,
        "Fixed &width - Fields are aligned in columns with spaces between each field.",
        false,
    );
    // Step 2, delimited.
    spec.on_page(Some(1));
    spec.label(12.0, 8.0, 456.0, "Set the delimiters your data contains.");
    spec.group(12.0, 30.0, 456.0, 100.0, "Delimiters");
    spec.check("tab", 24.0, 50.0, 100.0, "&Tab", true);
    spec.check("semicolon", 24.0, 74.0, 100.0, "&Semicolon", false);
    spec.check("comma", 130.0, 50.0, 100.0, "&Comma", false);
    spec.check("space", 130.0, 74.0, 100.0, "S&pace", false);
    spec.check("other", 24.0, 98.0, 70.0, "&Other:", false);
    spec.text("other_text", 96.0, 96.0, 40.0, "");
    spec.check(
        "consecutive",
        250.0,
        50.0,
        210.0,
        "T&reat consecutive delimiters as one",
        false,
    );
    spec.label(250.0, 78.0, 90.0, "Text &qualifier:");
    spec.dropdown("qualifier", 344.0, 74.0, 100.0, &["\"", "'", "{none}"], 0);
    // Step 2, fixed width.
    spec.on_page(Some(3));
    spec.label(
        12.0,
        8.0,
        456.0,
        "Type the character positions where the columns break, separated by commas.",
    );
    spec.label(12.0, 40.0, 120.0, "Break &positions:");
    spec.text("breaks", 140.0, 36.0, 200.0, "");
    // Step 3.
    spec.on_page(Some(2));
    spec.label(
        12.0,
        8.0,
        456.0,
        "Select each column and set the Data Format. General converts numeric values to numbers.",
    );
    spec.group(12.0, 34.0, 456.0, 56.0, "Column data format");
    spec.radio("general", "format", 24.0, 54.0, 120.0, "&General", true);
    spec.label(12.0, 104.0, 100.0, "&Destination:");
    spec.reference(
        "dest",
        96.0,
        100.0,
        300.0,
        &abs_range_text((r.0, r.1, r.0, r.1)),
    );
    spec.on_page(None);
    spec.focus = Some("delimited".to_owned());
    (spec, Box::new(TextToColumns { fixed: false }))
}

impl TextToColumns {
    fn step_buttons(ui: &mut Ui, d: &mut Dialog, page: usize) {
        d.set_enabled(ui, "back", page != 0);
        d.set_enabled(ui, "next", page == 0 || page == 1 || page == 3);
    }
}

impl View for TextToColumns {
    fn event(&mut self, s: &mut Shell, ui: &mut Ui, d: &mut Dialog, o: &Outcome) {
        pick_selection(s, ui, d, o);
        match o {
            Outcome::Changed(id) if id == "delimited" || id == "fixed" => {
                self.fixed = d.values(ui).radio("kind") == "fixed";
            }
            Outcome::Pressed(id) if id == "next" => {
                let next = match d.page() {
                    0 => {
                        if self.fixed {
                            3
                        } else {
                            1
                        }
                    }
                    _ => 2,
                };
                d.set_page(ui, next);
                Self::step_buttons(ui, d, next);
            }
            Outcome::Pressed(id) if id == "back" => {
                let prev = match d.page() {
                    2 => {
                        if self.fixed {
                            3
                        } else {
                            1
                        }
                    }
                    _ => 0,
                };
                d.set_page(ui, prev);
                Self::step_buttons(ui, d, prev);
            }
            _ => {}
        }
    }

    fn commit(
        &mut self,
        s: &mut Shell,
        ui: &mut Ui,
        _d: &mut Dialog,
        v: &Values,
    ) -> Result<(), String> {
        let dest = parse_ref(v.text("dest")).ok_or("Enter a destination cell.")?;
        let dest = range_text((dest.0, dest.1, dest.0, dest.1));
        let mut cmd = String::from("data.data-tools.text-to-columns=");
        if v.radio("kind") == "fixed" {
            let breaks = v.text("breaks").trim();
            if breaks.is_empty() {
                return Err("Type at least one break position.".to_owned());
            }
            cmd.push_str(&format!("fixed={breaks}"));
        } else {
            let mut delims = Vec::new();
            for (id, name) in [
                ("tab", "tab"),
                ("semicolon", "semicolon"),
                ("comma", "comma"),
                ("space", "space"),
            ] {
                if v.flag(id) {
                    delims.push(name.to_owned());
                }
            }
            if v.flag("other") {
                let t = v.text("other_text");
                if t.chars().count() != 1 || t.trim().is_empty() || t.contains([',', ';', '=']) {
                    return Err("Type one delimiter character in the Other box (not a comma, semicolon or equals sign).".to_owned());
                }
                delims.push(t.to_owned());
            }
            if delims.is_empty() {
                return Err("Choose at least one delimiter.".to_owned());
            }
            cmd.push_str(&format!("delim={}", delims.join(",")));
            if v.flag("consecutive") {
                cmd.push_str(";consecutive");
            }
            match v.index("qualifier") {
                Some(1) => cmd.push_str(";qualifier='"),
                Some(2) => cmd.push_str(";qualifier=none"),
                _ => {}
            }
        }
        cmd.push_str(&format!(";dest={dest}"));
        s.run_id(ui, &cmd);
        Ok(())
    }
}

// ---- Remove Duplicates --------------------------------------------------------------------------------

#[derive(Debug)]
struct RemoveDuplicates {
    cols: Vec<u32>,
    names: Vec<String>,
}

fn remove_duplicates(s: &mut Shell) -> (Spec, Box<dyn View>) {
    let r = s.sheet.data_target();
    let header = SortDialog::open(&s.sheet).has_header;
    let cols: Vec<u32> = (r.1..=r.3).take(16).collect();
    let names: Vec<String> = cols
        .iter()
        .map(|c| {
            let h = s.sheet.display(r.0, *c);
            if header && !h.is_empty() {
                h
            } else {
                format!("Column {}", col_letters(*c))
            }
        })
        .collect();
    let rows = cols.len().div_ceil(2).max(1);
    let mut spec = Spec::new("Remove Duplicates", 400.0, 96.0 + 24.0 * rows as f32);
    spec.note(
        12.0,
        8.0,
        376.0,
        30.0,
        "To delete duplicate values, select one or more columns that contain duplicates.",
    );
    spec.button("all", 12.0, 44.0, 96.0, "Select &All");
    spec.button("none", 114.0, 44.0, 108.0, "&Unselect All");
    spec.check("header", 250.0, 46.0, 140.0, "My data has &headers", header);
    for (i, n) in names.iter().enumerate() {
        let x = 12.0 + 190.0 * (i % 2) as f32;
        let y = 76.0 + 24.0 * (i / 2) as f32;
        spec.check(&format!("c{i}"), x, y, 184.0, n, true);
    }
    spec.focus = Some("c0".to_owned());
    (spec, Box::new(RemoveDuplicates { cols, names }))
}

impl View for RemoveDuplicates {
    fn event(&mut self, _s: &mut Shell, ui: &mut Ui, d: &mut Dialog, o: &Outcome) {
        if let Outcome::Pressed(id) = o
            && (id == "all" || id == "none")
        {
            for i in 0..self.cols.len() {
                d.set_checked(ui, &format!("c{i}"), id == "all");
            }
        }
        let _ = &self.names;
    }

    fn commit(
        &mut self,
        s: &mut Shell,
        ui: &mut Ui,
        _d: &mut Dialog,
        v: &Values,
    ) -> Result<(), String> {
        let chosen: Vec<String> = self
            .cols
            .iter()
            .enumerate()
            .filter(|(i, _)| v.flag(&format!("c{i}")))
            .map(|(_, c)| col_letters(*c))
            .collect();
        if chosen.is_empty() {
            return Err("You must select at least one column.".to_owned());
        }
        let header = if v.flag("header") { "yes" } else { "no" };
        s.run_id(
            ui,
            &format!(
                "data.data-tools.remove-duplicates=headers={header};cols={}",
                chosen.join(",")
            ),
        );
        Ok(())
    }
}

// ---- Data Validation ---------------------------------------------------------------------------------------

const ALLOW: [&str; 8] = [
    "Any value",
    "Whole number",
    "Decimal",
    "List",
    "Date",
    "Time",
    "Text length",
    "Custom",
];
const DATA_OPS: [&str; 8] = [
    "between",
    "not between",
    "equal to",
    "not equal to",
    "greater than",
    "less than",
    "greater than or equal to",
    "less than or equal to",
];
const STYLES: [&str; 3] = ["Stop", "Warning", "Information"];

#[derive(Debug)]
struct Validation;

fn validation(s: &mut Shell) -> (Spec, Box<dyn View>) {
    let (r, c) = s.sheet.active;
    let cur = s.sheet.data_validation_at(r, c);
    let allow = cur.as_ref().map_or(0, |v| match v.kind {
        ValidationType::None => 0,
        ValidationType::Whole => 1,
        ValidationType::Decimal => 2,
        ValidationType::List => 3,
        ValidationType::Date => 4,
        ValidationType::Time => 5,
        ValidationType::TextLength => 6,
        ValidationType::Custom => 7,
    });
    let op = cur.as_ref().map_or(0, |v| match v.operator {
        ValidationOperator::Between => 0,
        ValidationOperator::NotBetween => 1,
        ValidationOperator::Equal => 2,
        ValidationOperator::NotEqual => 3,
        ValidationOperator::GreaterThan => 4,
        ValidationOperator::LessThan => 5,
        ValidationOperator::GreaterThanOrEqual => 6,
        ValidationOperator::LessThanOrEqual => 7,
    });
    let f1 = cur
        .as_ref()
        .and_then(|v| v.formula1.clone())
        .unwrap_or_default();
    let f2 = cur
        .as_ref()
        .and_then(|v| v.formula2.clone())
        .unwrap_or_default();
    let mut spec = Spec::new("Data Validation", 400.0, 270.0);
    spec.tabs = vec![
        "&Settings".into(),
        "Input &Message".into(),
        "&Error Alert".into(),
    ];
    spec.on_page(Some(0));
    spec.label(12.0, 44.0, 200.0, "Validation criteria");
    spec.label(12.0, 66.0, 60.0, "&Allow:");
    spec.dropdown("allow", 84.0, 62.0, 200.0, &ALLOW, allow);
    spec.check(
        "blank",
        296.0,
        64.0,
        100.0,
        "Ignore &blank",
        cur.as_ref().is_none_or(|v| v.allow_blank),
    );
    spec.label(12.0, 98.0, 60.0, "&Data:");
    spec.dropdown("data", 84.0, 94.0, 200.0, &DATA_OPS, op);
    spec.check(
        "dropdown",
        296.0,
        96.0,
        100.0,
        "In-cell dropdo&wn",
        cur.as_ref().is_none_or(|v| !v.hide_drop_down),
    );
    spec.label(12.0, 130.0, 70.0, "Mi&nimum:").id = "l1".to_owned();
    spec.text("v1", 84.0, 126.0, 300.0, &f1);
    spec.label(12.0, 162.0, 70.0, "Ma&ximum:").id = "l2".to_owned();
    spec.text("v2", 84.0, 158.0, 300.0, &f2);
    spec.on_page(Some(1));
    spec.check(
        "show_input",
        12.0,
        44.0,
        380.0,
        "Show input &message when cell is selected",
        cur.as_ref().is_some_and(|v| v.show_input_message),
    );
    spec.label(
        12.0,
        76.0,
        120.0,
        "When cell is selected, show this input message:",
    );
    spec.label(12.0, 100.0, 60.0, "&Title:");
    spec.text(
        "in_title",
        84.0,
        96.0,
        300.0,
        cur.as_ref()
            .and_then(|v| v.prompt_title.as_deref())
            .unwrap_or(""),
    );
    spec.label(12.0, 132.0, 70.0, "Input m&essage:");
    spec.text(
        "in_msg",
        84.0,
        128.0,
        300.0,
        cur.as_ref().and_then(|v| v.prompt.as_deref()).unwrap_or(""),
    );
    spec.on_page(Some(2));
    spec.check(
        "show_error",
        12.0,
        44.0,
        380.0,
        "Show error &alert after invalid data is entered",
        cur.as_ref().is_none_or(|v| v.show_error_message),
    );
    spec.label(
        12.0,
        76.0,
        300.0,
        "When user enters invalid data, show this error alert:",
    );
    spec.label(12.0, 102.0, 60.0, "St&yle:");
    let style = cur.as_ref().map_or(0, |v| match v.error_style {
        ErrorStyle::Stop => 0,
        ErrorStyle::Warning => 1,
        ErrorStyle::Information => 2,
    });
    spec.dropdown("style", 84.0, 98.0, 140.0, &STYLES, style);
    spec.label(12.0, 134.0, 60.0, "Tit&le:");
    spec.text(
        "er_title",
        84.0,
        130.0,
        300.0,
        cur.as_ref()
            .and_then(|v| v.error_title.as_deref())
            .unwrap_or(""),
    );
    spec.label(12.0, 166.0, 70.0, "Error mes&sage:");
    spec.text(
        "er_msg",
        84.0,
        162.0,
        300.0,
        cur.as_ref().and_then(|v| v.error.as_deref()).unwrap_or(""),
    );
    spec.on_page(None);
    spec.button("clear", 12.0, 232.0, 100.0, "&Clear All");
    (spec, Box::new(Validation))
}

/// The labels of the two value boxes and which controls are enabled, for the Allow and Data
/// boxes: `(label 1, label 2, [data, v1, v2, dropdown, blank])`.
fn validation_state(allow: usize, data: usize) -> (&'static str, &'static str, [bool; 5]) {
    let between = data <= 1;
    let (l1, l2) = match allow {
        3 => ("Source:", ""),
        7 => ("Formula:", ""),
        4 | 5 => (if between { "Start:" } else { "Value:" }, "End:"),
        _ => (if between { "Minimum:" } else { "Value:" }, "Maximum:"),
    };
    let compare = matches!(allow, 1 | 2 | 4 | 5 | 6);
    (
        l1,
        l2,
        [
            compare,
            allow != 0,
            compare && between,
            allow == 3,
            allow != 0,
        ],
    )
}

impl Validation {
    /// Labels and enabled state follow the Allow and Data boxes.
    fn sync(ui: &mut Ui, d: &mut Dialog) {
        let v = d.values(ui);
        let (l1, l2, on) =
            validation_state(v.index("allow").unwrap_or(0), v.index("data").unwrap_or(0));
        d.set_text(ui, "l1", l1);
        d.set_text(ui, "l2", l2);
        for (id, e) in ["data", "v1", "v2", "dropdown", "blank"]
            .into_iter()
            .zip(on)
        {
            d.set_enabled(ui, id, e);
        }
    }
}

impl View for Validation {
    fn opened(&mut self, _s: &mut Shell, ui: &mut Ui, d: &mut Dialog) {
        Self::sync(ui, d);
    }

    fn event(&mut self, _s: &mut Shell, ui: &mut Ui, d: &mut Dialog, o: &Outcome) {
        match o {
            Outcome::Changed(id) if id == "allow" || id == "data" => Self::sync(ui, d),
            Outcome::Pressed(id) if id == "clear" => {
                d.set_choice(ui, "allow", 0);
                d.set_text(ui, "v1", "");
                d.set_text(ui, "v2", "");
                d.set_text(ui, "in_title", "");
                d.set_text(ui, "in_msg", "");
                d.set_text(ui, "er_title", "");
                d.set_text(ui, "er_msg", "");
                d.set_checked(ui, "show_input", false);
                Self::sync(ui, d);
            }
            _ => {}
        }
    }

    fn commit(
        &mut self,
        s: &mut Shell,
        ui: &mut Ui,
        _d: &mut Dialog,
        v: &Values,
    ) -> Result<(), String> {
        let allow = match v.index("allow").unwrap_or(0) {
            1 => AllowKind::WholeNumber,
            2 => AllowKind::Decimal,
            3 => AllowKind::List,
            4 => AllowKind::Date,
            5 => AllowKind::Time,
            6 => AllowKind::TextLength,
            7 => AllowKind::Custom,
            _ => {
                // Any value: no rule (an input message needs a rule in the model).
                s.sheet.data_set_validation(None)?;
                let _ = ui;
                return Ok(());
            }
        };
        let compare = match (allow, v.index("data").unwrap_or(0)) {
            (AllowKind::List | AllowKind::Custom, _) | (_, 0) => DataCmp::Between,
            (_, 1) => DataCmp::NotBetween,
            (_, 2) => DataCmp::Cmp(Cmp::Equal),
            (_, 3) => DataCmp::Cmp(Cmp::NotEqual),
            (_, 4) => DataCmp::Cmp(Cmp::Greater),
            (_, 5) => DataCmp::Cmp(Cmp::Less),
            (_, 6) => DataCmp::Cmp(Cmp::GreaterOrEqual),
            _ => DataCmp::Cmp(Cmp::LessOrEqual),
        };
        let v1 = v.text("v1").trim().to_owned();
        if v1.is_empty() {
            return Err(match allow {
                AllowKind::List => {
                    "The Source box is currently empty. Are you sure you want to continue?"
                        .to_owned()
                }
                AllowKind::Custom => "The Formula box is empty.".to_owned(),
                _ => "The Minimum value is empty.".to_owned(),
            });
        }
        let v2 = matches!(compare, DataCmp::Between | DataCmp::NotBetween)
            .then(|| v.text("v2").trim().to_owned())
            .filter(|t| !t.is_empty() && !matches!(allow, AllowKind::List | AllowKind::Custom));
        if matches!(compare, DataCmp::Between | DataCmp::NotBetween)
            && !matches!(allow, AllowKind::List | AllowKind::Custom)
            && v2.is_none()
        {
            return Err("The Maximum value is empty.".to_owned());
        }
        let rule = ValidationRule {
            allow,
            compare,
            value1: v1,
            value2: v2,
            ignore_blank: v.flag("blank"),
            in_cell_dropdown: v.flag("dropdown") || allow != AllowKind::List,
            input_message: v
                .flag("show_input")
                .then(|| (v.text("in_title").to_owned(), v.text("in_msg").to_owned())),
            error_alert: v.flag("show_error").then(|| ErrorAlert {
                style: ["stop", "warning", "information"][v.index("style").unwrap_or(0).min(2)]
                    .to_owned(),
                title: v.text("er_title").to_owned(),
                message: v.text("er_msg").to_owned(),
            }),
        };
        s.sheet.data_set_validation(Some(rule))
    }
}

// ---- Consolidate -----------------------------------------------------------------------------------------

const FUNCTIONS: [(&str, &str); 11] = [
    ("Sum", "sum"),
    ("Count", "count"),
    ("Average", "average"),
    ("Max", "max"),
    ("Min", "min"),
    ("Product", "product"),
    ("Count Numbers", "countnums"),
    ("StdDev", "stdev"),
    ("StdDevp", "stdevp"),
    ("Var", "var"),
    ("Varp", "varp"),
];

#[derive(Debug)]
struct Consolidate {
    refs: Vec<String>,
}

fn consolidate(s: &mut Shell) -> (Spec, Box<dyn View>) {
    let names: Vec<&str> = FUNCTIONS.iter().map(|f| f.0).collect();
    let mut spec = Spec::new("Consolidate", 400.0, 330.0);
    spec.label(12.0, 8.0, 120.0, "&Function:");
    spec.dropdown("fn", 12.0, 28.0, 220.0, &names, 0);
    spec.label(12.0, 60.0, 120.0, "&Reference:");
    spec.reference("ref", 12.0, 80.0, 290.0, &selection_text(s));
    spec.button("add", 310.0, 80.0, 78.0, "&Add");
    spec.label(12.0, 112.0, 200.0, "All r&eferences:");
    spec.list(
        "refs",
        Rect::new(12.0, 132.0, 290.0, 100.0),
        Vec::new(),
        None,
    );
    spec.button("delete", 310.0, 132.0, 78.0, "&Delete");
    spec.label(12.0, 242.0, 200.0, "Use labels in");
    spec.check("top", 24.0, 262.0, 120.0, "&Top row", false);
    spec.check("left", 150.0, 262.0, 140.0, "&Left column", false);
    spec.focus = Some("ref".to_owned());
    (spec, Box::new(Consolidate { refs: Vec::new() }))
}

impl View for Consolidate {
    fn event(&mut self, s: &mut Shell, ui: &mut Ui, d: &mut Dialog, o: &Outcome) {
        pick_selection(s, ui, d, o);
        let Outcome::Pressed(id) = o else { return };
        match id.as_str() {
            "add" => {
                let text = d.values(ui).text("ref").trim().to_owned();
                let (sheet_part, range) = match text.rsplit_once('!') {
                    Some((sh, r)) => (Some(sh.to_owned()), r.to_owned()),
                    None => (None, text.clone()),
                };
                match parse_ref(&range) {
                    Some(r) => {
                        let shown = match sheet_part {
                            Some(sh) => format!("{sh}!{}", abs_range_text(r)),
                            None => abs_range_text(r),
                        };
                        if !self.refs.contains(&shown) {
                            self.refs.push(shown);
                        }
                        d.set_items(ui, "refs", self.refs.clone(), Some(self.refs.len() - 1));
                        d.set_error(ui, "");
                    }
                    None => d.set_error(ui, "Reference isn't valid."),
                }
            }
            "delete" => {
                if let Some(i) = d.values(ui).index("refs")
                    && i < self.refs.len()
                {
                    self.refs.remove(i);
                    let sel = (!self.refs.is_empty()).then(|| i.min(self.refs.len() - 1));
                    d.set_items(ui, "refs", self.refs.clone(), sel);
                }
            }
            _ => {}
        }
    }

    fn commit(
        &mut self,
        s: &mut Shell,
        ui: &mut Ui,
        d: &mut Dialog,
        v: &Values,
    ) -> Result<(), String> {
        let _ = d;
        if self.refs.is_empty() {
            return Err("Add at least one reference.".to_owned());
        }
        let f = FUNCTIONS[v.index("fn").unwrap_or(0).min(FUNCTIONS.len() - 1)].1;
        let mut cmd = format!("data.data-tools.consolidate=fn={f}");
        for r in &self.refs {
            cmd.push_str(&format!(";ref={}", r.replace('$', "")));
        }
        if v.flag("top") {
            cmd.push_str(";top");
        }
        if v.flag("left") {
            cmd.push_str(";left");
        }
        let (r, c) = s.sheet.active;
        cmd.push_str(&format!(";dest={}", crate::sheet::a1(r, c)));
        s.run_id(ui, &cmd);
        Ok(())
    }
}

// ---- Goal Seek ------------------------------------------------------------------------------------------

#[derive(Debug)]
struct GoalSeek;

fn goal_seek(s: &mut Shell) -> (Spec, Box<dyn View>) {
    let (r, c) = s.sheet.active;
    let mut spec = Spec::new("Goal Seek", 330.0, 130.0);
    spec.label(12.0, 12.0, 110.0, "S&et cell:");
    spec.reference("set", 130.0, 8.0, 190.0, &abs_range_text((r, c, r, c)));
    spec.label(12.0, 44.0, 110.0, "To &value:");
    spec.text("to", 130.0, 40.0, 190.0 - 26.0, "");
    spec.label(12.0, 76.0, 110.0, "&By changing cell:");
    spec.reference("by", 130.0, 72.0, 190.0, "");
    spec.focus = Some("to".to_owned());
    (spec, Box::new(GoalSeek))
}

impl View for GoalSeek {
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
        let set = parse_ref(v.text("set")).ok_or("Reference isn't valid.")?;
        let by = parse_ref(v.text("by")).ok_or("Reference isn't valid.")?;
        let to: f64 = v
            .text("to")
            .trim()
            .parse()
            .map_err(|_| "Enter a number for To value.".to_owned())?;
        let cell = |r: (u32, u32, u32, u32)| crate::sheet::a1(r.0, r.1);
        s.run_id(
            ui,
            &format!(
                "data.forecast.what-if-analysis=goalseek={},{to},{}",
                cell(set),
                cell(by)
            ),
        );
        Ok(())
    }
}

// ---- Subtotal ------------------------------------------------------------------------------------------

#[derive(Debug)]
struct Subtotal {
    cols: Vec<u32>,
}

fn subtotal(s: &mut Shell) -> (Spec, Box<dyn View>) {
    let r = s.sheet.data_target();
    let header = SortDialog::open(&s.sheet).has_header;
    let cols: Vec<u32> = (r.1..=r.3).take(12).collect();
    let names: Vec<String> = cols
        .iter()
        .map(|c| {
            let h = s.sheet.display(r.0, *c);
            if header && !h.is_empty() {
                h
            } else {
                format!("Column {}", col_letters(*c))
            }
        })
        .collect();
    let rows = cols.len().div_ceil(2).max(1);
    let mut spec = Spec::new("Subtotal", 400.0, 236.0 + 24.0 * rows as f32);
    spec.label(12.0, 8.0, 200.0, "&At each change in:");
    spec.dropdown("at", 12.0, 28.0, 376.0, &strs(&names), 0);
    spec.label(12.0, 60.0, 200.0, "&Use function:");
    let fnames: Vec<&str> = FUNCTIONS.iter().map(|f| f.0).collect();
    spec.dropdown("fn", 12.0, 80.0, 376.0, &fnames, 0);
    spec.label(12.0, 112.0, 200.0, "Add su&btotal to:");
    for (i, n) in names.iter().enumerate() {
        let x = 12.0 + 190.0 * (i % 2) as f32;
        let y = 132.0 + 24.0 * (i / 2) as f32;
        spec.check(&format!("c{i}"), x, y, 184.0, n, i + 1 == names.len());
    }
    let y = 142.0 + 24.0 * rows as f32;
    spec.check(
        "replace",
        12.0,
        y,
        300.0,
        "&Replace current subtotals",
        true,
    );
    spec.check(
        "pagebreak",
        12.0,
        y + 24.0,
        300.0,
        "&Page break between groups",
        false,
    )
    .disabled();
    spec.check("below", 12.0, y + 48.0, 300.0, "&Summary below data", true);
    spec.button("remove", 296.0, y + 24.0, 92.0, "Remo&ve All");
    spec.focus = Some("at".to_owned());
    (spec, Box::new(Subtotal { cols }))
}

impl View for Subtotal {
    fn event(&mut self, s: &mut Shell, ui: &mut Ui, _d: &mut Dialog, o: &Outcome) {
        if let Outcome::Pressed(id) = o
            && id == "remove"
        {
            s.run_id(ui, "data.outline.subtotal=remove");
            request_close();
        }
    }

    fn commit(
        &mut self,
        s: &mut Shell,
        ui: &mut Ui,
        _d: &mut Dialog,
        v: &Values,
    ) -> Result<(), String> {
        let at = self
            .cols
            .get(v.index("at").unwrap_or(0))
            .copied()
            .unwrap_or(1);
        let chosen: Vec<String> = self
            .cols
            .iter()
            .enumerate()
            .filter(|(i, _)| v.flag(&format!("c{i}")))
            .map(|(_, c)| col_letters(*c))
            .collect();
        if chosen.is_empty() {
            return Err("Select at least one column to subtotal.".to_owned());
        }
        let f = FUNCTIONS[v.index("fn").unwrap_or(0).min(FUNCTIONS.len() - 1)].1;
        let mut cmd = format!(
            "data.outline.subtotal=at={};fn={f};cols={}",
            col_letters(at),
            chosen.join(",")
        );
        if !v.flag("below") {
            cmd.push_str(";above");
        }
        if !v.flag("replace") {
            cmd.push_str(";keep");
        }
        s.run_id(ui, &cmd);
        Ok(())
    }
}

// ---- Group ------------------------------------------------------------------------------------------------

#[derive(Debug)]
struct Group;

fn group(s: &mut Shell) -> (Spec, Box<dyn View>) {
    let cols = s.sheet.data_selection_is_columns();
    let mut spec = Spec::new("Group", 220.0, 96.0);
    spec.radio("rows", "by", 24.0, 12.0, 180.0, "&Rows", !cols);
    spec.radio("columns", "by", 24.0, 36.0, 180.0, "&Columns", cols);
    spec.focus = Some(if cols { "columns" } else { "rows" }.to_owned());
    (spec, Box::new(Group))
}

impl View for Group {
    fn commit(
        &mut self,
        s: &mut Shell,
        ui: &mut Ui,
        _d: &mut Dialog,
        v: &Values,
    ) -> Result<(), String> {
        let by = if v.radio("by") == "columns" {
            "columns"
        } else {
            "rows"
        };
        s.run_id(ui, &format!("data.outline.group={by}"));
        Ok(())
    }
}

// ---- Custom AutoFilter -------------------------------------------------------------------------------

/// Excel's twelve comparisons: the label, the comparison and the wildcards put round the value.
const FILTER_OPS: [(&str, Cmp, &str, &str); 12] = [
    ("equals", Cmp::Equal, "", ""),
    ("does not equal", Cmp::NotEqual, "", ""),
    ("is greater than", Cmp::Greater, "", ""),
    ("is greater than or equal to", Cmp::GreaterOrEqual, "", ""),
    ("is less than", Cmp::Less, "", ""),
    ("is less than or equal to", Cmp::LessOrEqual, "", ""),
    ("begins with", Cmp::Equal, "", "*"),
    ("does not begin with", Cmp::NotEqual, "", "*"),
    ("ends with", Cmp::Equal, "*", ""),
    ("does not end with", Cmp::NotEqual, "*", ""),
    ("contains", Cmp::Equal, "*", "*"),
    ("does not contain", Cmp::NotEqual, "*", "*"),
];

#[derive(Debug)]
struct CustomFilter {
    column: u32,
}

fn custom_filter(s: &mut Shell, column: u32, operand: &str) -> (Spec, Box<dyn View>) {
    let items = s.sheet.data_filter_items(column);
    let values: Vec<String> = items.into_iter().map(|(v, _)| v).take(50).collect();
    let vals: Vec<&str> = values.iter().map(String::as_str).collect();
    let ops: Vec<&str> = FILTER_OPS.iter().map(|o| o.0).collect();
    let mut spec = Spec::new("Custom AutoFilter", 400.0, 150.0);
    spec.label(
        12.0,
        8.0,
        376.0,
        &format!("Show rows where column {}:", col_letters(column)),
    );
    let op1 = std::mem::take(&mut s.filter_op).min(FILTER_OPS.len() - 1);
    spec.dropdown("op1", 12.0, 32.0, 150.0, &ops, op1);
    spec.combo("v1", 172.0, 32.0, 216.0, &vals, operand);
    spec.radio("and", "join", 90.0, 64.0, 80.0, "&And", true);
    spec.radio("or", "join", 190.0, 64.0, 80.0, "&Or", false);
    spec.dropdown("op2", 12.0, 90.0, 150.0, &ops, 0);
    spec.combo("v2", 172.0, 90.0, 216.0, &vals, "");
    spec.focus = Some("v1".to_owned());
    (spec, Box::new(CustomFilter { column }))
}

impl View for CustomFilter {
    fn commit(
        &mut self,
        s: &mut Shell,
        _ui: &mut Ui,
        _d: &mut Dialog,
        v: &Values,
    ) -> Result<(), String> {
        let first = v.text("v1").trim().to_owned();
        if first.is_empty() {
            return Err("Enter a value for the first condition.".to_owned());
        }
        let cmp = |i: Option<usize>, text: String| {
            let (_, cmp, pre, post) = FILTER_OPS[i.unwrap_or(0).min(FILTER_OPS.len() - 1)];
            (cmp, format!("{pre}{text}{post}"))
        };
        let second = v.text("v2").trim().to_owned();
        let filter = ColumnFilter::Custom {
            first: cmp(v.index("op1"), first),
            second: (!second.is_empty()).then(|| cmp(v.index("op2"), second)),
            and: v.radio("join") != "or",
        };
        s.sheet.data_filter_column(self.column, Some(filter))
    }
}
