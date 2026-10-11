//! Data validation: the `dataValidations` element and the check of a candidate value against a rule.
//!
//! The rules follow Excel's answers to `Validation.Value`, measured through COM
//! (`tools/office/make-sortfilter-refs.ps1`, `fixtures/reference/xlsx/sortfilter-validation/scenarios.json`):
//!
//! - A blank candidate passes when `allowBlank` is set. Otherwise it counts as 0 (number rules), as empty text
//!   (text length) and is not in any list; a custom formula sees the blank cell.
//! - Whole number: the candidate must be a number without a fractional part (compared exactly), then the operator
//!   applies. Decimal, date and time take any number. Text never passes; an error never passes; a logical value is
//!   greater than every number, as in a comparison. Numbers compare equal when they agree to 15 significant digits.
//! - Text length counts characters of the text a number shows (TRUE for a logical value) and compares with whole
//!   numbers.
//! - A list is either a quoted literal (`"a,b,c"`) or a range or formula. A literal list compares case-sensitively
//!   after trimming spaces from the candidate, and a number matches an item that reads as the same number. A list
//!   from cells compares ignoring case; a number never matches text and text never matches a number.
//! - Custom: the formula must give TRUE or a non-zero number; text and errors fail.
//! - The bounds are formulas. Relative references are relative to the top-left cell of the first range of `sqref`.

use std::sync::Arc;

use open365_calc::eval::evaluate_array_root;
use open365_calc::value::{ErrorKind, Value, compare_numbers, number_to_text};
use open365_calc::{
    Ast, CellSource, Clock, EvalContext, FormatLocale, SystemClock, evaluate, parse_formula,
};

use super::{CellValue, Rect, Worksheet};
use crate::refs::{parse_range, shift_formula};
use crate::structure::range_a1;
use crate::workbook::Workbook;
use crate::xml::{Attr, ElementHead, RawXml, XmlElement};

/// What a rule accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ValidationType {
    /// Any value (only the input message is used).
    #[default]
    None,
    Whole,
    Decimal,
    List,
    Date,
    Time,
    TextLength,
    Custom,
}

impl ValidationType {
    fn parse(s: Option<&str>) -> Self {
        match s {
            Some("whole") => Self::Whole,
            Some("decimal") => Self::Decimal,
            Some("list") => Self::List,
            Some("date") => Self::Date,
            Some("time") => Self::Time,
            Some("textLength") => Self::TextLength,
            Some("custom") => Self::Custom,
            _ => Self::None,
        }
    }

    fn name(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Whole => Some("whole"),
            Self::Decimal => Some("decimal"),
            Self::List => Some("list"),
            Self::Date => Some("date"),
            Self::Time => Some("time"),
            Self::TextLength => Some("textLength"),
            Self::Custom => Some("custom"),
        }
    }
}

/// How an invalid entry is treated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ErrorStyle {
    /// The entry is refused.
    #[default]
    Stop,
    /// The user may keep the entry after a warning.
    Warning,
    /// The user is told and the entry stays.
    Information,
}

/// The comparison of a number rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ValidationOperator {
    #[default]
    Between,
    NotBetween,
    Equal,
    NotEqual,
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,
}

impl ValidationOperator {
    fn parse(s: Option<&str>) -> Self {
        match s {
            Some("notBetween") => Self::NotBetween,
            Some("equal") => Self::Equal,
            Some("notEqual") => Self::NotEqual,
            Some("lessThan") => Self::LessThan,
            Some("lessThanOrEqual") => Self::LessThanOrEqual,
            Some("greaterThan") => Self::GreaterThan,
            Some("greaterThanOrEqual") => Self::GreaterThanOrEqual,
            _ => Self::Between,
        }
    }

    fn name(self) -> Option<&'static str> {
        match self {
            Self::Between => None,
            Self::NotBetween => Some("notBetween"),
            Self::Equal => Some("equal"),
            Self::NotEqual => Some("notEqual"),
            Self::LessThan => Some("lessThan"),
            Self::LessThanOrEqual => Some("lessThanOrEqual"),
            Self::GreaterThan => Some("greaterThan"),
            Self::GreaterThanOrEqual => Some("greaterThanOrEqual"),
        }
    }
}

/// One `dataValidation`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DataValidation {
    pub kind: ValidationType,
    pub error_style: ErrorStyle,
    pub operator: ValidationOperator,
    /// Blank cells pass ("Ignore blank").
    pub allow_blank: bool,
    /// `showDropDown`: true HIDES the in-cell arrow of a list.
    pub hide_drop_down: bool,
    pub show_input_message: bool,
    pub show_error_message: bool,
    pub ime_mode: Option<String>,
    pub error_title: Option<String>,
    pub error: Option<String>,
    pub prompt_title: Option<String>,
    pub prompt: Option<String>,
    /// The cells the rule covers.
    pub sqref: Vec<Rect>,
    pub formula1: Option<String>,
    pub formula2: Option<String>,
    /// Read from Excel's `x14` extension (cross-sheet references); not written back by the model.
    pub from_ext: bool,
    other_attrs: Vec<Attr>,
}

/// The validations of a sheet.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DataValidations {
    pub items: Vec<DataValidation>,
    pub disable_prompts: bool,
    pub x_window: Option<u32>,
    pub y_window: Option<u32>,
    /// Rules found in the `x14` extension list, read only.
    pub ext_items: Vec<DataValidation>,
}

fn flag(head: &ElementHead, name: &str) -> bool {
    matches!(head.attr(name).map(str::trim), Some("1" | "true"))
}

fn parse_sqref(text: &str) -> Vec<Rect> {
    text.split_whitespace().filter_map(parse_range).collect()
}

fn child_text(el: &XmlElement, local: &str) -> Option<String> {
    el.elements()
        .find(|c| c.head.local_name() == local)
        .map(|c| c.own_text())
}

impl DataValidation {
    fn from_element(el: &XmlElement) -> Self {
        let h = &el.head;
        let known = [
            "type",
            "errorStyle",
            "imeMode",
            "operator",
            "allowBlank",
            "showDropDown",
            "showInputMessage",
            "showErrorMessage",
            "errorTitle",
            "error",
            "promptTitle",
            "prompt",
            "sqref",
        ];
        Self {
            kind: ValidationType::parse(h.attr("type")),
            error_style: match h.attr("errorStyle") {
                Some("warning") => ErrorStyle::Warning,
                Some("information") => ErrorStyle::Information,
                _ => ErrorStyle::Stop,
            },
            operator: ValidationOperator::parse(h.attr("operator")),
            allow_blank: flag(h, "allowBlank"),
            hide_drop_down: flag(h, "showDropDown"),
            show_input_message: flag(h, "showInputMessage"),
            show_error_message: flag(h, "showErrorMessage"),
            ime_mode: h.attr("imeMode").map(str::to_owned),
            error_title: h.attr("errorTitle").map(str::to_owned),
            error: h.attr("error").map(str::to_owned),
            prompt_title: h.attr("promptTitle").map(str::to_owned),
            prompt: h.attr("prompt").map(str::to_owned),
            sqref: h.attr("sqref").map(parse_sqref).unwrap_or_default(),
            formula1: child_text(el, "formula1"),
            formula2: child_text(el, "formula2"),
            from_ext: false,
            other_attrs: h
                .attrs
                .iter()
                .filter(|a| a.ns.is_some() || !known.contains(&a.name.as_str()))
                .cloned()
                .collect(),
        }
    }

    /// The x14 form: `x14:dataValidation` with `x14:formula1/xm:f` and `xm:sqref`.
    fn from_ext_element(el: &XmlElement) -> Self {
        let mut v = Self::from_element(el);
        let f = |name: &str| {
            el.elements()
                .find(|c| c.head.local_name() == name)
                .and_then(|c| child_text(c, "f"))
        };
        v.formula1 = f("formula1");
        v.formula2 = f("formula2");
        v.sqref = child_text(el, "sqref")
            .map(|t| parse_sqref(&t))
            .unwrap_or_default();
        v.from_ext = true;
        v
    }

    fn to_element(&self, like: &ElementHead) -> XmlElement {
        let mut el = XmlElement {
            head: like.sibling("dataValidation"),
            children: Vec::new(),
        };
        let h = &mut el.head;
        if let Some(t) = self.kind.name() {
            h.set_attr("type", t);
        }
        match self.error_style {
            ErrorStyle::Stop => {}
            ErrorStyle::Warning => h.set_attr("errorStyle", "warning"),
            ErrorStyle::Information => h.set_attr("errorStyle", "information"),
        }
        if let Some(m) = &self.ime_mode {
            h.set_attr("imeMode", m);
        }
        if let Some(o) = self.operator.name() {
            h.set_attr("operator", o);
        }
        for (name, on) in [
            ("allowBlank", self.allow_blank),
            ("showDropDown", self.hide_drop_down),
            ("showInputMessage", self.show_input_message),
            ("showErrorMessage", self.show_error_message),
        ] {
            if on {
                h.set_attr(name, "1");
            }
        }
        for (name, text) in [
            ("errorTitle", &self.error_title),
            ("error", &self.error),
            ("promptTitle", &self.prompt_title),
            ("prompt", &self.prompt),
        ] {
            if let Some(t) = text {
                h.set_attr(name, t);
            }
        }
        let sqref: Vec<String> = self.sqref.iter().map(|r| range_a1(*r)).collect();
        h.set_attr("sqref", &sqref.join(" "));
        h.attrs.extend(self.other_attrs.iter().cloned());
        for (name, f) in [("formula1", &self.formula1), ("formula2", &self.formula2)] {
            if let Some(f) = f {
                let mut c = el.sibling(name);
                c.children = vec![RawXml::Text(f.clone())];
                el.children.push(RawXml::Element(c));
            }
        }
        el
    }

    /// True when the rule covers `(row, col)`.
    pub fn contains(&self, row: u32, col: u32) -> bool {
        self.sqref
            .iter()
            .any(|&(r1, c1, r2, c2)| (r1..=r2).contains(&row) && (c1..=c2).contains(&col))
    }

    /// Whether a list shows its in-cell arrow.
    pub fn in_cell_dropdown(&self) -> bool {
        !self.hide_drop_down
    }

    /// The top-left cell of the first range: the cell relative references in the formulas are relative to.
    fn anchor(&self) -> (u32, u32) {
        self.sqref.first().map_or((1, 1), |r| (r.0, r.1))
    }
}

impl DataValidations {
    /// Reads a `dataValidations` element.
    pub fn from_element(el: &XmlElement) -> Self {
        Self {
            items: el
                .elements()
                .filter(|e| e.head.is_main("dataValidation"))
                .map(DataValidation::from_element)
                .collect(),
            disable_prompts: flag(&el.head, "disablePrompts"),
            x_window: el.head.attr("xWindow").and_then(|v| v.trim().parse().ok()),
            y_window: el.head.attr("yWindow").and_then(|v| v.trim().parse().ok()),
            ext_items: Vec::new(),
        }
    }

    /// The rule that covers `(row, col)`; the main list wins over the extension list.
    pub fn at(&self, row: u32, col: u32) -> Option<&DataValidation> {
        self.items
            .iter()
            .chain(&self.ext_items)
            .find(|v| v.contains(row, col))
    }

    fn to_element(&self, like: &ElementHead) -> XmlElement {
        let mut el = XmlElement {
            head: like.sibling("dataValidations"),
            children: Vec::new(),
        };
        el.head.set_attr("count", &self.items.len().to_string());
        if self.disable_prompts {
            el.head.set_attr("disablePrompts", "1");
        }
        if let Some(x) = self.x_window {
            el.head.set_attr("xWindow", &x.to_string());
        }
        if let Some(y) = self.y_window {
            el.head.set_attr("yWindow", &y.to_string());
        }
        el.children = self
            .items
            .iter()
            .map(|v| RawXml::Element(v.to_element(like)))
            .collect();
        el
    }
}

fn collect_ext(el: &XmlElement, out: &mut Vec<DataValidation>) {
    for c in el.elements() {
        match c.head.local_name() {
            "dataValidation" => out.push(DataValidation::from_ext_element(c)),
            _ => collect_ext(c, out),
        }
    }
}

impl Worksheet {
    /// The sheet's validations: the `dataValidations` element and the `x14` extension.
    pub fn data_validations(&self) -> Option<DataValidations> {
        let mut v = self
            .root_element("dataValidations")
            .map(DataValidations::from_element);
        if let Some(ext) = self.root_element("extLst") {
            let mut items = Vec::new();
            for e in ext.elements() {
                for dv in e
                    .elements()
                    .filter(|x| x.head.local_name() == "dataValidations")
                {
                    collect_ext(dv, &mut items);
                }
            }
            if !items.is_empty() {
                v.get_or_insert_with(DataValidations::default).ext_items = items;
            }
        }
        v
    }

    /// Replaces (or with `None` removes) the `dataValidations` element. The extension list is not touched.
    pub fn set_data_validations(&mut self, validations: Option<&DataValidations>) {
        let el = validations
            .filter(|v| !v.items.is_empty())
            .map(|v| v.to_element(&self.sheet_data_head));
        self.put_root_element("dataValidations", el);
    }
}

// ---- checking a candidate --------------------------------------------------------------------------------

/// What a validation check needs to evaluate its formulas.
#[derive(Clone, Copy)]
pub struct ValidationContext<'a> {
    pub source: &'a dyn CellSource,
    /// The sheet the validated cell is on.
    pub sheet: &'a str,
    pub date1904: bool,
    pub clock: &'a dyn Clock,
    pub locale: &'a Arc<FormatLocale>,
}

impl std::fmt::Debug for ValidationContext<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ValidationContext")
            .field("sheet", &self.sheet)
            .finish_non_exhaustive()
    }
}

/// The outcome of a check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Validity {
    Valid,
    Invalid {
        style: ErrorStyle,
        title: Option<String>,
        message: Option<String>,
    },
}

impl Validity {
    pub fn is_valid(&self) -> bool {
        matches!(self, Self::Valid)
    }
}

static SYSTEM_CLOCK: SystemClock = SystemClock;

impl<'a> ValidationContext<'a> {
    /// A context reading the system clock and using the en-US formats.
    pub fn new(source: &'a dyn CellSource, sheet: &'a str, locale: &'a Arc<FormatLocale>) -> Self {
        Self {
            source,
            sheet,
            date1904: false,
            clock: &SYSTEM_CLOCK,
            locale,
        }
    }

    fn eval_context(&self, row: u32, col: u32) -> EvalContext<'a> {
        EvalContext::new(self.source, self.sheet)
            .with_locale(self.locale)
            .with_date1904(self.date1904)
            .with_clock(self.clock)
            .at_cell(row, col)
    }
}

fn shifted(formula: &str, v: &DataValidation, row: u32, col: u32) -> Option<Ast> {
    let (ar, ac) = v.anchor();
    let text = shift_formula(
        formula.trim_start_matches('='),
        i64::from(row) - i64::from(ar),
        i64::from(col) - i64::from(ac),
    );
    parse_formula(&text).ok()
}

fn eval_scalar(
    formula: &str,
    v: &DataValidation,
    row: u32,
    col: u32,
    ctx: &ValidationContext<'_>,
) -> Value {
    match shifted(formula, v, row, col) {
        Some(ast) => evaluate(&ast, &ctx.eval_context(row, col)).top_left(),
        None => Value::Error(ErrorKind::Name),
    }
}

/// A literal list: `"a,b,c"` (quotes inside are doubled).
fn literal_list(formula: &str) -> Option<Vec<String>> {
    let f = formula.trim();
    let inner = f.strip_prefix('"')?.strip_suffix('"')?;
    Some(
        inner
            .replace("\"\"", "\"")
            .split(',')
            .map(str::to_owned)
            .collect(),
    )
}

fn plain_range(formula: &str) -> Option<(Option<String>, Rect)> {
    let f = formula.trim();
    let (sheet, cells) = match f.rsplit_once('!') {
        Some((s, c)) => {
            let s = s.trim();
            let s = s
                .strip_prefix('\'')
                .and_then(|x| x.strip_suffix('\''))
                .map_or_else(|| s.to_owned(), |x| x.replace("''", "'"));
            (Some(s), c)
        }
        None => (None, f),
    };
    let plain: String = cells.chars().filter(|c| *c != '$').collect();
    Some((sheet, parse_range(&plain)?))
}

/// The items a list rule offers for the cell `(row, col)`.
fn list_items(v: &DataValidation, row: u32, col: u32, ctx: &ValidationContext<'_>) -> Vec<Value> {
    let Some(f) = &v.formula1 else {
        return Vec::new();
    };
    if let Some((sheet, (r1, c1, r2, c2))) = plain_range(f) {
        let sheet = sheet.unwrap_or_else(|| ctx.sheet.to_owned());
        let mut out = Vec::new();
        for r in r1..=r2 {
            for c in c1..=c2 {
                out.push(ctx.source.cell(&sheet, r, c));
            }
        }
        return out;
    }
    let Some(ast) = shifted(f, v, row, col) else {
        return Vec::new();
    };
    match evaluate_array_root(&ast, &ctx.eval_context(row, col)) {
        Value::Array(rows) => rows.into_iter().flatten().collect(),
        Value::Error(_) => Vec::new(),
        other => vec![other],
    }
}

fn plain_number(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok().filter(|n| n.is_finite())
}

fn list_contains(
    v: &DataValidation,
    candidate: &Value,
    row: u32,
    col: u32,
    ctx: &ValidationContext<'_>,
) -> bool {
    if let Some(items) = v.formula1.as_deref().and_then(literal_list) {
        return match candidate {
            Value::Text(t) => {
                let t = t.trim_matches(' ');
                items.iter().any(|i| {
                    i == t
                        || matches!((plain_number(i), plain_number(t)),
                            (Some(a), Some(b)) if compare_numbers(a, b).is_eq())
                })
            }
            Value::Number(n) => items.iter().any(|i| {
                *i == number_to_text(*n)
                    || plain_number(i).is_some_and(|x| compare_numbers(x, *n).is_eq())
            }),
            Value::Bool(b) => items.iter().any(|i| i == if *b { "TRUE" } else { "FALSE" }),
            _ => false,
        };
    }
    list_items(v, row, col, ctx)
        .iter()
        .any(|item| match (item, candidate) {
            (Value::Text(a), Value::Text(b)) => a.to_lowercase() == b.to_lowercase(),
            (Value::Number(a), Value::Number(b)) => compare_numbers(*a, *b).is_eq(),
            (Value::Bool(a), Value::Bool(b)) => a == b,
            _ => false,
        })
}

fn compare_operand(candidate: &Value, bound: f64) -> Option<std::cmp::Ordering> {
    match candidate {
        Value::Number(n) => Some(compare_numbers(*n, bound)),
        Value::Empty => Some(compare_numbers(0.0, bound)),
        // A logical value is greater than any number.
        Value::Bool(_) => Some(std::cmp::Ordering::Greater),
        _ => None,
    }
}

fn bound_number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => Some(*n),
        Value::Bool(b) => Some(f64::from(u8::from(*b))),
        Value::Text(t) => plain_number(t),
        _ => None,
    }
}

impl DataValidation {
    fn invalid(&self) -> Validity {
        Validity::Invalid {
            style: self.error_style,
            title: self.error_title.clone(),
            message: self.error.clone(),
        }
    }

    /// Checks `candidate`, the value the cell at `(row, col)` would hold. `(row, col)` should be inside
    /// [`DataValidation::sqref`].
    pub fn check(
        &self,
        row: u32,
        col: u32,
        candidate: &Value,
        ctx: &ValidationContext<'_>,
    ) -> Validity {
        // Formulas see the candidate in the cell being validated.
        let overlay = Overlay {
            inner: ctx.source,
            sheet: ctx.sheet,
            at: (row, col),
            value: candidate.clone(),
        };
        let ctx = &ValidationContext {
            source: &overlay,
            ..*ctx
        };
        let blank = matches!(candidate, Value::Empty);
        if blank && self.allow_blank {
            return Validity::Valid;
        }
        let ok = match self.kind {
            ValidationType::None => true,
            ValidationType::Custom => {
                let result = self
                    .formula1
                    .as_deref()
                    .map_or(Value::Error(ErrorKind::Value), |f| {
                        eval_scalar(f, self, row, col, ctx)
                    });
                match result {
                    Value::Bool(b) => b,
                    Value::Number(n) => n != 0.0,
                    _ => false,
                }
            }
            ValidationType::List => !blank && list_contains(self, candidate, row, col, ctx),
            ValidationType::TextLength => {
                let text = match candidate {
                    Value::Empty => Some(String::new()),
                    Value::Number(n) => Some(number_to_text(*n)),
                    Value::Bool(b) => Some(if *b { "TRUE" } else { "FALSE" }.to_owned()),
                    Value::Text(t) => Some(t.clone()),
                    _ => None,
                };
                text.is_some_and(|t| {
                    self.compare_number(row, col, ctx, &Value::Number(t.chars().count() as f64))
                })
            }
            ValidationType::Whole
            | ValidationType::Decimal
            | ValidationType::Date
            | ValidationType::Time => {
                let whole_ok = !matches!(
                    (self.kind, candidate),
                    (ValidationType::Whole, Value::Number(n)) if n.fract() != 0.0
                );
                whole_ok && self.compare_number(row, col, ctx, candidate)
            }
        };
        if ok { Validity::Valid } else { self.invalid() }
    }

    /// The operator test of a number rule.
    fn compare_number(
        &self,
        row: u32,
        col: u32,
        ctx: &ValidationContext<'_>,
        candidate: &Value,
    ) -> bool {
        use std::cmp::Ordering::{Equal, Greater, Less};
        let bound = |f: &Option<String>| {
            f.as_deref()
                .and_then(|f| bound_number(&eval_scalar(f, self, row, col, ctx)))
        };
        let Some(b1) = bound(&self.formula1) else {
            return false;
        };
        let Some(o1) = compare_operand(candidate, b1) else {
            return false;
        };
        match self.operator {
            ValidationOperator::Between | ValidationOperator::NotBetween => {
                let Some(b2) = bound(&self.formula2) else {
                    return false;
                };
                let Some(o2) = compare_operand(candidate, b2) else {
                    return false;
                };
                let inside = o1 != Less && o2 != Greater;
                inside == (self.operator == ValidationOperator::Between)
            }
            ValidationOperator::Equal => o1 == Equal,
            ValidationOperator::NotEqual => o1 != Equal,
            ValidationOperator::LessThan => o1 == Less,
            ValidationOperator::LessThanOrEqual => o1 != Greater,
            ValidationOperator::GreaterThan => o1 == Greater,
            ValidationOperator::GreaterThanOrEqual => o1 != Less,
        }
    }

    /// The items a list rule offers in its drop-down for the cell `(row, col)`, as the values they hold.
    pub fn list_values(&self, row: u32, col: u32, ctx: &ValidationContext<'_>) -> Vec<Value> {
        if self.kind != ValidationType::List {
            return Vec::new();
        }
        match self.formula1.as_deref().and_then(literal_list) {
            Some(items) => items.into_iter().map(Value::Text).collect(),
            None => list_items(self, row, col, ctx),
        }
    }
}

/// The stored (cached) values of an opened workbook as a formula source.
#[derive(Debug, Clone, Copy)]
pub struct WorkbookCells<'a>(pub &'a Workbook);

impl CellSource for WorkbookCells<'_> {
    fn cell(&self, sheet: &str, row: u32, col: u32) -> Value {
        let Some(index) = self.0.sheet_index(sheet) else {
            return Value::Empty;
        };
        match self.0.cell(index, row, col).map(|c| &c.value) {
            None | Some(CellValue::Empty) => Value::Empty,
            Some(CellValue::Number(n)) => Value::Number(*n),
            Some(CellValue::Bool(b)) => Value::Bool(*b),
            Some(CellValue::Error(e)) => {
                Value::Error(ErrorKind::parse(e.code()).unwrap_or(ErrorKind::Value))
            }
            Some(other) => Value::Text(other.as_text().unwrap_or_default().to_owned()),
        }
    }

    fn name(&self, name: &str) -> Option<Ast> {
        self.0
            .defined_names()
            .iter()
            .find(|n| !n.is_sheet_scoped() && n.name.eq_ignore_ascii_case(name))
            .and_then(|n| parse_formula(&n.formula).ok())
    }

    fn name_in(&self, sheet: &str, name: &str) -> Option<Ast> {
        let index = self.0.sheet_index(sheet)?;
        self.0
            .sheet_defined_names(index)
            .find(|n| n.name.eq_ignore_ascii_case(name))
            .and_then(|n| parse_formula(&n.formula).ok())
            .or_else(|| self.name(name))
    }

    fn sheet_exists(&self, sheet: &str) -> bool {
        self.0.sheet_index(sheet).is_some()
    }

    fn has_formula(&self, sheet: &str, row: u32, col: u32) -> bool {
        self.0
            .sheet_index(sheet)
            .and_then(|i| self.0.cell(i, row, col))
            .is_some_and(|c| c.formula.is_some())
    }

    fn sheet_position(&self, sheet: &str) -> Option<usize> {
        self.0.sheet_index(sheet)
    }

    fn sheet_count(&self) -> usize {
        self.0.sheets().len()
    }

    fn used_bounds(&self, sheet: &str) -> (u32, u32) {
        self.0
            .sheet_index(sheet)
            .and_then(|i| self.0.sheet(i))
            .and_then(Worksheet::used_range)
            .map_or((0, 0), |(_, _, r2, c2)| (r2, c2))
    }
}

struct Overlay<'a> {
    inner: &'a dyn CellSource,
    sheet: &'a str,
    at: (u32, u32),
    value: Value,
}

impl CellSource for Overlay<'_> {
    fn cell(&self, sheet: &str, row: u32, col: u32) -> Value {
        if sheet.eq_ignore_ascii_case(self.sheet) && (row, col) == self.at {
            return self.value.clone();
        }
        self.inner.cell(sheet, row, col)
    }
    fn name(&self, name: &str) -> Option<Ast> {
        self.inner.name(name)
    }
    fn name_in(&self, sheet: &str, name: &str) -> Option<Ast> {
        self.inner.name_in(sheet, name)
    }
    fn sheet_exists(&self, sheet: &str) -> bool {
        self.inner.sheet_exists(sheet)
    }
    fn has_formula(&self, sheet: &str, row: u32, col: u32) -> bool {
        self.inner.has_formula(sheet, row, col)
    }
    fn sheet_position(&self, sheet: &str) -> Option<usize> {
        self.inner.sheet_position(sheet)
    }
    fn sheet_count(&self) -> usize {
        self.inner.sheet_count()
    }
    fn used_bounds(&self, sheet: &str) -> (u32, u32) {
        self.inner.used_bounds(sheet)
    }
}
