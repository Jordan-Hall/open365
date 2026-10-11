//! Evaluating one field: the value it shows and how `\*`, `\#` and `\@` format it.

#![allow(dead_code)] // groundwork for the REF-family and TOC evaluators, which are not wired yet

use std::cell::OnceCell;

use crate::document::Document;
use crate::fields::{FieldContext, FieldKind, GeneralFormat, Instruction, SkipReason};
use crate::fields_date::{CivilDateTime, format_date_picture};
use crate::fields_formula::{self, FormulaEnv, Num};
use crate::fields_locale::LocaleNames;
use crate::fields_number as number;
use crate::fields_props::{DocProps, PropValue};
use crate::fields_scan::RawField;
use crate::fields_snap::{Snapshot, StyleTable};
use crate::fields_toc::TocPlan;
use crate::ranges::DocPos;

/// What evaluating a field produced.
pub(crate) enum Outcome {
    /// New result text; `merge` keeps the old formatting (`\* MERGEFORMAT`).
    Text {
        text: String,
        merge: bool,
    },
    Toc(TocPlan),
    /// Word leaves the result as it is (HYPERLINK).
    Keep,
    Skip(SkipReason),
}

/// A value before formatting.
pub(crate) enum Value {
    Text(String),
    Num(Num),
    /// Local time.
    Date(CivilDateTime),
    /// A document property's date: `time` says whether the default format shows the time.
    DocDate {
        at: CivilDateTime,
        time: bool,
    },
    /// Shown as is: error messages.
    Plain(String),
}

pub(crate) struct Evaluator<'c> {
    pub ctx: &'c FieldContext<'c>,
    pub props: DocProps,
    pub styles: StyleTable,
}

/// Lazily built per-field view of the document.
pub(crate) struct View<'a> {
    pub doc: &'a Document,
    pub raws: &'a [RawField],
    snap: &'a OnceCell<Snapshot>,
    styles: &'a StyleTable,
    shifts: &'a [(usize, isize)],
}

impl<'a> View<'a> {
    pub(crate) fn new(
        doc: &'a Document,
        raws: &'a [RawField],
        snap: &'a OnceCell<Snapshot>,
        styles: &'a StyleTable,
        shifts: &'a [(usize, isize)],
    ) -> Self {
        Self {
            doc,
            raws,
            snap,
            styles,
            shifts,
        }
    }

    /// The position in the numbering the page provider was built for: undoes the paragraphs
    /// this update added or removed.
    pub(crate) fn original(&self, pos: DocPos) -> DocPos {
        let mut para = pos.para;
        for &(start, delta) in self.shifts.iter().rev() {
            if para < start {
                continue;
            }
            para = match usize::try_from(delta) {
                Ok(d) if para < start + d => start - 1,
                _ => ((para as isize) - delta).max(0) as usize,
            };
        }
        DocPos {
            para,
            offset: pos.offset,
        }
    }

    /// Page holding `pos` (a position in the story as it is now).
    pub(crate) fn page_at(
        &self,
        ctx: &FieldContext<'_>,
        pos: DocPos,
    ) -> Option<crate::fields::PageInfo> {
        ctx.pages.page_at(self.original(pos))
    }

    pub(crate) fn snap(&self) -> &Snapshot {
        self.snap
            .get_or_init(|| Snapshot::build(self.doc, self.styles))
    }
}

impl<'c> Evaluator<'c> {
    pub(crate) fn new(doc: &Document, ctx: &'c FieldContext<'c>) -> Self {
        Self {
            ctx,
            props: DocProps::load(doc.package()),
            styles: StyleTable::load(doc),
        }
    }

    pub(crate) fn evaluate(&self, view: &View<'_>, index: usize, inst: &Instruction) -> Outcome {
        let doc = view.doc;
        let raw = &view.raws[index];
        let lang = self.field_lang(doc, raw);
        let merge = inst.general_formats().contains(&GeneralFormat::MergeFormat);
        let value = match inst.kind {
            FieldKind::Quote => Some(self.quote(inst)),
            FieldKind::Formula => Some(self.formula(inst, view, raw)),
            FieldKind::Date | FieldKind::Time => Some(Value::Date(self.ctx.now)),
            FieldKind::CreateDate => self.props.created().map(|d| self.local_minutes(d)),
            FieldKind::SaveDate => self.props.modified().map(|d| self.local_minutes(d)),
            FieldKind::PrintDate => self.props.last_printed().map(|d| self.local_minutes(d)),
            FieldKind::Author => self.text_prop(inst, self.props.author()),
            FieldKind::LastSavedBy => self.text_prop(inst, self.props.last_saved_by()),
            FieldKind::Title => self.text_prop(inst, self.props.title()),
            FieldKind::Subject => self.text_prop(inst, self.props.subject()),
            FieldKind::Keywords => self.text_prop(inst, self.props.keywords()),
            FieldKind::Comments => self.text_prop(inst, self.props.comments()),
            FieldKind::FileName => self.file_name(inst),
            FieldKind::DocProperty => self.doc_property(inst),
            FieldKind::Info => self.info(inst),
            FieldKind::Page
            | FieldKind::NumPages
            | FieldKind::SectionPages
            | FieldKind::Section => match self.page_value(view, raw, inst) {
                Ok(v) => Some(v),
                Err(reason) => return Outcome::Skip(reason),
            },
            _ => return crate::fields_refs::evaluate(self, view, index, inst, &lang),
        };
        let Some(value) = value else {
            return Outcome::Skip(SkipReason::MissingInput);
        };
        Outcome::Text {
            text: self.render(
                inst,
                value,
                &lang,
                matches!(inst.kind, FieldKind::Date | FieldKind::Time),
            ),
            merge,
        }
    }

    /// Document dates: Word shows them to the minute.
    fn local_minutes(&self, utc: CivilDateTime) -> Value {
        let mut local = utc.add_minutes(self.ctx.utc_offset_minutes);
        local.second = 0;
        Value::Date(local)
    }

    fn local(&self, utc: CivilDateTime) -> Value {
        Value::Date(utc.add_minutes(self.ctx.utc_offset_minutes))
    }

    /// The language Word uses for names and formats of the field.
    pub(crate) fn field_lang(&self, doc: &Document, raw: &RawField) -> String {
        if let Some(props) = &raw.code_props {
            if let Some(lang) = props
                .any_w_element("lang")
                .and_then(|e| e.head.attr_w("val"))
            {
                return lang.to_owned();
            }
            if let Some(lang) = props.style_id().and_then(|s| self.styles.lang_of(s)) {
                return lang;
            }
        }
        let para_style = doc
            .paragraph(raw.begin.para)
            .and_then(|p| p.style_id())
            .map(str::to_owned)
            .or_else(|| self.styles.default_paragraph.clone());
        if let Some(lang) = para_style.and_then(|s| self.styles.lang_of(&s)) {
            return lang;
        }
        self.styles
            .default_lang
            .clone()
            .unwrap_or_else(|| self.ctx.default_language.clone())
    }

    pub(crate) fn names_for(&self, lang: &str) -> LocaleNames {
        self.ctx
            .date_names
            .and_then(|n| n.names(lang))
            .or_else(|| LocaleNames::for_tag(lang))
            .unwrap_or_else(LocaleNames::english)
    }

    fn quote(&self, inst: &Instruction) -> Value {
        let text: String = inst.args.iter().map(|a| a.text.as_str()).collect();
        Value::Text(text)
    }

    fn formula(&self, inst: &Instruction, view: &View<'_>, raw: &RawField) -> Value {
        let env = DocEnv { view, raw };
        match fields_formula::evaluate(&inst.body, &env) {
            Ok(n) => Value::Num(n),
            Err(msg) => Value::Plain(msg),
        }
    }

    fn text_prop(&self, inst: &Instruction, value: Option<&str>) -> Option<Value> {
        if let Some(new) = inst.first_arg() {
            return Some(Value::Text(new.to_owned()));
        }
        Some(Value::Text(value.unwrap_or_default().to_owned()))
    }

    fn file_name(&self, inst: &Instruction) -> Option<Value> {
        let name = self.ctx.file_name.clone()?;
        if inst.has_switch("p") {
            Some(Value::Text(self.ctx.file_path.clone().unwrap_or(name)))
        } else {
            Some(Value::Text(name))
        }
    }

    /// `INFO`: the file name, else the document properties under their `INFO` keywords.
    fn info(&self, inst: &Instruction) -> Option<Value> {
        if inst
            .first_arg()
            .is_some_and(|k| k.eq_ignore_ascii_case("FileName"))
        {
            return self.file_name(inst);
        }
        match self.doc_property(inst) {
            Some(Value::Plain(m)) if m.contains("Unknown document property") => {
                Some(Value::Plain("Error! Unknown info keyword.".to_owned()))
            }
            other => other,
        }
    }

    fn doc_property(&self, inst: &Instruction) -> Option<Value> {
        let Some(name) = inst.first_arg() else {
            return Some(Value::Plain("Error! No property name supplied.".to_owned()));
        };
        match self.props.lookup(name) {
            Some(PropValue::Text(t)) => Some(Value::Text(t)),
            Some(PropValue::Number(n)) => Some(Value::Num(Num {
                value: n,
                scale: decimals(n),
            })),
            Some(PropValue::Date(d)) => {
                // Word shows a custom date property as a date at midnight, a built-in one to
                // the minute.
                let mut dt = d;
                let custom = self.props.is_custom(name).unwrap_or(false);
                if custom {
                    dt.hour = 0;
                    dt.minute = 0;
                }
                dt.second = 0;
                Some(Value::DocDate {
                    at: dt,
                    time: !custom,
                })
            }
            None => Some(Value::Plain(
                "Error! Unknown document property name.".to_owned(),
            )),
        }
    }

    fn page_value(
        &self,
        view: &View<'_>,
        raw: &RawField,
        inst: &Instruction,
    ) -> Result<Value, SkipReason> {
        let doc = view.doc;
        let pos = crate::fields_refs::position_of(doc, &raw.begin);
        let pages = self.ctx.pages;
        let int = |n: u32| {
            Value::Num(Num {
                value: f64::from(n),
                scale: 0,
            })
        };
        match inst.kind {
            FieldKind::Page => {
                let info = view
                    .page_at(self.ctx, pos)
                    .ok_or(SkipReason::MissingInput)?;
                Ok(match info.label {
                    Some(label) if inst.general_formats().is_empty() => Value::Text(label),
                    _ => int(info.number),
                })
            }
            FieldKind::NumPages => pages.total_pages().map(int).ok_or(SkipReason::MissingInput),
            FieldKind::SectionPages => {
                let section = view
                    .page_at(self.ctx, pos)
                    .map(|i| i.section)
                    .or_else(|| section_of(doc, pos.para))
                    .ok_or(SkipReason::MissingInput)?;
                pages
                    .section_pages(section)
                    .map(int)
                    .ok_or(SkipReason::MissingInput)
            }
            _ => {
                let section = view
                    .page_at(self.ctx, pos)
                    .map(|i| i.section)
                    .or_else(|| section_of(doc, pos.para));
                section
                    .map(|s| int(u32::try_from(s + 1).unwrap_or(1)))
                    .ok_or(SkipReason::MissingInput)
            }
        }
    }

    /// Formats `value` with the instruction's `\*`, `\#` and `\@` switches.
    pub(crate) fn render(
        &self,
        inst: &Instruction,
        value: Value,
        lang: &str,
        default_is_short_date: bool,
    ) -> String {
        let _ = default_is_short_date;
        let formats = inst.general_formats();
        let case_formats: Vec<GeneralFormat> = formats
            .iter()
            .copied()
            .filter(|f| {
                matches!(
                    f,
                    GeneralFormat::Upper
                        | GeneralFormat::Lower
                        | GeneralFormat::FirstCap
                        | GeneralFormat::Caps
                )
            })
            .collect();
        let numbering = formats.iter().copied().find(|f| {
            matches!(
                f,
                GeneralFormat::CardText
                    | GeneralFormat::OrdText
                    | GeneralFormat::DollarText
                    | GeneralFormat::Ordinal
                    | GeneralFormat::Arabic
                    | GeneralFormat::Roman(_)
                    | GeneralFormat::Alphabetic(_)
                    | GeneralFormat::Hex
            )
        });
        let text = match value {
            Value::Plain(t) => return t,
            Value::Date(dt) => {
                let names = self.names_for(lang);
                match inst.date_picture() {
                    Some(pic) => format_date_picture(dt, pic, &names),
                    None => format_date_picture(dt, &names.default_date_picture(inst.kind), &names),
                }
            }
            Value::DocDate { at, time } => {
                let names = self.names_for(lang);
                match inst.date_picture() {
                    Some(pic) => format_date_picture(at, pic, &names),
                    None if time => {
                        let clock = names.time_picture.replace(":ss", "");
                        format_date_picture(at, &format!("{} {clock}", names.date_picture), &names)
                    }
                    None => format_date_picture(at, &names.date_picture, &names),
                }
            }
            Value::Num(n) => self.render_number(inst, n, numbering),
            Value::Text(t) => {
                if inst.number_picture().is_some()
                    && let Some(n) = parse_number(&t)
                {
                    self.render_number(inst, n, numbering)
                } else {
                    t
                }
            }
        };
        apply_case(text, &case_formats)
    }

    fn render_number(
        &self,
        inst: &Instruction,
        n: Num,
        numbering: Option<GeneralFormat>,
    ) -> String {
        // A numbering switch before the picture switch is an error in Word.
        if let Some(pic_at) = inst.switches.iter().position(|s| s.name == "#") {
            let numeric_before = inst.switches[..pic_at].iter().any(|s| {
                s.name == "*"
                    && s.arg.as_deref().is_some_and(|a| {
                        matches!(
                            a.to_ascii_lowercase().as_str(),
                            "cardtext"
                                | "ordtext"
                                | "dollartext"
                                | "ordinal"
                                | "arabic"
                                | "roman"
                                | "alphabetic"
                                | "hex"
                        )
                    })
            });
            if numeric_before {
                return "Error! Picture switch must be first formatting switch.".to_owned();
            }
        }
        if let Some(fmt) = numbering {
            let v = n.value;
            let out = match fmt {
                GeneralFormat::CardText => number::card_text(v),
                GeneralFormat::OrdText => number::ord_text(v),
                GeneralFormat::DollarText => number::dollar_text(v),
                GeneralFormat::Ordinal => number::ordinal_number(v),
                GeneralFormat::Arabic => number::arabic(v),
                GeneralFormat::Roman(lower) => number::roman(v, lower),
                GeneralFormat::Alphabetic(lower) => number::alphabetic(v, lower),
                GeneralFormat::Hex => number::hex(v),
                _ => None,
            };
            return out.unwrap_or_else(|| number::NOT_REPRESENTABLE.to_owned());
        }
        match inst.number_picture() {
            Some(pic) => number::format_picture(n.value, pic, &self.ctx.number_symbols),
            None => n.text(),
        }
    }
}

fn decimals(value: f64) -> u32 {
    let text = format!("{value}");
    text.split_once('.')
        .map_or(0, |(_, f)| u32::try_from(f.len()).unwrap_or(0))
}

/// A number written in text, with the scale of its decimals.
pub(crate) fn parse_number(text: &str) -> Option<Num> {
    let t = text.trim().replace(',', "");
    if t.is_empty()
        || !t
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+'))
    {
        return None;
    }
    let value: f64 = t.parse().ok()?;
    let scale = t
        .split_once('.')
        .map_or(0, |(_, f)| u32::try_from(f.len()).unwrap_or(0));
    Some(Num { value, scale })
}

/// Section (0-based) holding paragraph `para`.
pub(crate) fn section_of(doc: &Document, para: usize) -> Option<usize> {
    let sections = doc.sections();
    sections
        .iter()
        .position(|s| s.end_paragraph.is_none_or(|end| end >= para))
}

/// `\* Upper`, `Lower`, `FirstCap` and `Caps`, in order.
pub(crate) fn apply_case(text: String, formats: &[GeneralFormat]) -> String {
    let mut out = text;
    for f in formats {
        out = match f {
            GeneralFormat::Upper => out.to_uppercase(),
            GeneralFormat::Lower => out.to_lowercase(),
            GeneralFormat::FirstCap => first_cap(&out),
            GeneralFormat::Caps => caps(&out),
            _ => out,
        };
    }
    out
}

fn first_cap(text: &str) -> String {
    let mut done = false;
    text.chars()
        .map(|c| {
            if !done && !c.is_whitespace() {
                done = true;
                c.to_uppercase().collect::<String>()
            } else {
                c.to_string()
            }
        })
        .collect()
}

fn caps(text: &str) -> String {
    let mut boundary = true;
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if boundary {
            out.extend(c.to_uppercase());
        } else {
            out.extend(c.to_lowercase());
        }
        boundary = !(c.is_alphanumeric() || c == '\'' || c == '\u{2019}');
    }
    out
}

/// Bookmarks and table cells for the formula field.
struct DocEnv<'a, 'b> {
    view: &'a View<'b>,
    raw: &'a RawField,
}

impl FormulaEnv for DocEnv<'_, '_> {
    fn bookmark_value(&self, name: &str) -> Option<f64> {
        let b = self.view.snap().bookmark(name)?;
        parse_number(b.text.trim_end_matches('\r')).map(|n| n.value)
    }

    fn bookmark_exists(&self, name: &str) -> bool {
        self.view.snap().bookmark(name).is_some()
    }

    fn table_values(&self, reference: &str) -> Option<Vec<f64>> {
        crate::fields_refs::table_values(self.view, self.raw, reference)
    }
}

#[allow(dead_code)]
fn unused(_: DocPos) {}
