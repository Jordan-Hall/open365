//! Stage 0 sharing (ADR-013): send a copy, send a PDF, find hidden content, write a
//! sanitised copy, and describe a request for the operating system's share sheet.
//!
//! Nothing here calls an operating system API and nothing modifies the open document. Every
//! file this module writes is a new file (or a temporary one); the document's own path is
//! refused as a destination.
//!
//! The inspector and the sanitiser read a package as untrusted input: parts are scanned as text
//! with a small tag scanner, never evaluated, and no URL is followed. Removal edits only the
//! parts it has to; all other parts stay byte-identical.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};

use open365_opc::{Limits, Package, PartName, rels_part_name, resolve_target};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::CommandError;
use crate::target::write_atomic_within;

/// Prefix of the folder a temporary share copy is made in. A folder that starts with it and
/// sits directly in the system temp folder is ours to delete.
pub const SHARE_TEMP_PREFIX: &str = "open365-share-";

/// A class of content a copy may carry that its author may not intend to share.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HiddenKind {
    /// Comments and replies (and, in a workbook, notes and threaded comments).
    Comments,
    /// Tracked changes (accepted on removal) and shared-workbook revision history.
    TrackedChanges,
    /// Hidden text in a document; hidden sheets, rows and columns in a workbook.
    HiddenText,
    /// Title, subject, keywords, description, category, status and custom properties.
    Properties,
    /// Author names: creator, last modified by, company, manager, and the author of each
    /// comment and tracked change.
    PersonalInfo,
}

impl HiddenKind {
    pub const ALL: [HiddenKind; 5] = [
        Self::Comments,
        Self::TrackedChanges,
        Self::HiddenText,
        Self::Properties,
        Self::PersonalInfo,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Comments => "comments",
            Self::TrackedChanges => "tracked_changes",
            Self::HiddenText => "hidden_text",
            Self::Properties => "properties",
            Self::PersonalInfo => "personal_info",
        }
    }
}

/// One finding of the inspector.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HiddenItem {
    pub kind: HiddenKind,
    pub part: String,
    pub count: usize,
    pub detail: String,
    /// Whether `share_sanitize_copy` can remove it. Hidden sheets, rows and columns of a
    /// workbook cannot be removed without changing formulas, so they are reported only.
    pub removable: bool,
}

/// What the inspector found in a DOCX or XLSX.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HiddenReport {
    pub schema: &'static str,
    pub format: &'static str,
    pub items: Vec<HiddenItem>,
    /// Distinct comment and tracked-change authors, sorted.
    pub authors: Vec<String>,
}

impl HiddenReport {
    pub fn has(&self, kind: HiddenKind) -> bool {
        self.items.iter().any(|i| i.kind == kind)
    }

    pub fn clean(&self) -> bool {
        self.items.is_empty()
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    fn lines(&self) -> Vec<String> {
        self.items
            .iter()
            .map(|i| format!("{}: {}: {}", i.kind.name(), i.part, i.detail))
            .collect()
    }
}

/// A sanitised package.
#[derive(Debug)]
pub struct Sanitized {
    pub bytes: Vec<u8>,
    /// How many items of each chosen kind were removed.
    pub removed: BTreeMap<HiddenKind, usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Docx,
    Xlsx,
}

fn package_error(e: impl std::fmt::Display) -> CommandError {
    CommandError::Share(format!("package: {e}"))
}

fn open_package(bytes: &[u8]) -> Result<(Package, Format), CommandError> {
    let pkg = Package::read(bytes, &Limits::default()).map_err(package_error)?;
    let format = if pkg.part("word/document.xml").is_some() {
        Format::Docx
    } else if pkg.part("xl/workbook.xml").is_some() {
        Format::Xlsx
    } else {
        return Err(CommandError::Share(
            "not a DOCX or XLSX package (no word/document.xml or xl/workbook.xml)".to_owned(),
        ));
    };
    Ok((pkg, format))
}

fn text_of<'a>(pkg: &'a Package, name: &str) -> Result<Option<&'a str>, CommandError> {
    let Some(part) = pkg.part(name) else {
        return Ok(None);
    };
    std::str::from_utf8(part.data())
        .map(Some)
        .map_err(|_| CommandError::Share(format!("{name} is not UTF-8 XML")))
}

fn part_names(pkg: &Package) -> Vec<String> {
    pkg.parts().map(|p| p.name().as_str().to_owned()).collect()
}

fn set_text(pkg: &mut Package, name: &str, text: String) -> Result<(), CommandError> {
    let part = PartName::new(name).map_err(package_error)?;
    pkg.set_part(part, text.into_bytes());
    Ok(())
}

// ---- a small XML tag scanner ----

#[derive(Debug, Clone, Copy)]
struct Tag<'a> {
    start: usize,
    end: usize,
    name: &'a str,
    close: bool,
    empty: bool,
}

impl Tag<'_> {
    fn local(&self) -> &str {
        local(self.name)
    }
}

fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

/// Every element tag of `xml`, in order. Comments, processing instructions, CDATA and
/// declarations are skipped. Quotes in attribute values are respected.
fn scan(xml: &str) -> Vec<Tag<'_>> {
    let b = xml.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &xml[i..];
        let skip_to = |end: &str| rest.find(end).map_or(b.len(), |p| i + p + end.len());
        if rest.starts_with("<!--") {
            i = skip_to("-->");
            continue;
        }
        if rest.starts_with("<![CDATA[") {
            i = skip_to("]]>");
            continue;
        }
        if rest.starts_with("<?") {
            i = skip_to("?>");
            continue;
        }
        if rest.starts_with("<!") {
            i = skip_to(">");
            continue;
        }
        let mut j = i + 1;
        let mut quote = 0u8;
        while j < b.len() {
            let c = b[j];
            if quote != 0 {
                if c == quote {
                    quote = 0;
                }
            } else if c == b'"' || c == b'\'' {
                quote = c;
            } else if c == b'>' {
                break;
            }
            j += 1;
        }
        if j >= b.len() {
            break;
        }
        let close = b.get(i + 1) == Some(&b'/');
        let name_start = if close { i + 2 } else { i + 1 };
        let mut k = name_start;
        while k < j && !b[k].is_ascii_whitespace() && b[k] != b'/' {
            k += 1;
        }
        out.push(Tag {
            start: i,
            end: j + 1,
            name: &xml[name_start..k],
            close,
            empty: !close && b[j - 1] == b'/',
        });
        i = j + 1;
    }
    out
}

/// The attributes of an opening tag as (name, value range in `xml`).
fn attrs(xml: &str, t: &Tag<'_>) -> Vec<(String, Range<usize>)> {
    let b = xml.as_bytes();
    let mut pos = t.start + 1 + t.name.len();
    let limit = if t.empty { t.end - 2 } else { t.end - 1 };
    let mut out = Vec::new();
    while pos < limit {
        while pos < limit && b[pos].is_ascii_whitespace() {
            pos += 1;
        }
        let name_start = pos;
        while pos < limit && b[pos] != b'=' && !b[pos].is_ascii_whitespace() {
            pos += 1;
        }
        let name = xml[name_start..pos].to_owned();
        while pos < limit && b[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos >= limit || b[pos] != b'=' {
            continue;
        }
        pos += 1;
        while pos < limit && b[pos].is_ascii_whitespace() {
            pos += 1;
        }
        let Some(&quote) = b.get(pos).filter(|q| **q == b'"' || **q == b'\'') else {
            break;
        };
        let value_start = pos + 1;
        let Some(len) = xml[value_start..].find(quote as char) else {
            break;
        };
        out.push((name, value_start..value_start + len));
        pos = value_start + len + 1;
    }
    out
}

fn attr<'a>(xml: &'a str, t: &Tag<'_>, wanted: &str) -> Option<&'a str> {
    attrs(xml, t)
        .into_iter()
        .find(|(n, _)| local(n) == wanted)
        .map(|(_, r)| &xml[r])
}

fn matching_close(tags: &[Tag<'_>], i: usize) -> Option<usize> {
    let t = tags[i];
    if t.empty {
        return Some(i);
    }
    let mut depth = 0i32;
    for (j, u) in tags.iter().enumerate().skip(i) {
        if u.name != t.name {
            continue;
        }
        if u.close {
            depth -= 1;
            if depth == 0 {
                return Some(j);
            }
        } else if !u.empty {
            depth += 1;
        }
    }
    None
}

#[derive(Debug, Clone, Copy)]
enum Act {
    Keep,
    /// Cut the element with its content. The kind, if any, is counted.
    Drop(Option<HiddenKind>),
    /// Cut the start and end tag only.
    Unwrap(Option<HiddenKind>),
}

type Rule<'r> = &'r mut dyn FnMut(&str, &[Tag<'_>], usize) -> Act;

/// Rewrites `xml` by the rule's decision for each start tag. Returns the new text and the
/// count per kind.
fn edit_xml(xml: &str, rule: Rule<'_>) -> (String, BTreeMap<HiddenKind, usize>) {
    let tags = scan(xml);
    let mut counts = BTreeMap::new();
    let mut out = String::with_capacity(xml.len());
    let mut pos = 0;
    let mut closes: HashSet<usize> = HashSet::new();
    let mut i = 0;
    while i < tags.len() {
        let t = tags[i];
        if t.close {
            if closes.contains(&i) {
                out.push_str(&xml[pos..t.start]);
                pos = t.end;
            }
            i += 1;
            continue;
        }
        match rule(xml, &tags, i) {
            Act::Keep => i += 1,
            Act::Drop(kind) => {
                let j = matching_close(&tags, i).unwrap_or(i);
                out.push_str(&xml[pos..t.start]);
                pos = tags[j].end;
                if let Some(k) = kind {
                    *counts.entry(k).or_insert(0) += 1;
                }
                i = j + 1;
            }
            Act::Unwrap(kind) => {
                out.push_str(&xml[pos..t.start]);
                pos = t.end;
                if let Some(j) = matching_close(&tags, i)
                    && j != i
                {
                    closes.insert(j);
                }
                if let Some(k) = kind {
                    *counts.entry(k).or_insert(0) += 1;
                }
                i += 1;
            }
        }
    }
    out.push_str(&xml[pos..]);
    (out, counts)
}

/// Replaces the text content of elements named `locals` (non-empty ones only) with `new`.
fn set_element_text(xml: &str, locals: &[&str], new: &str) -> (String, usize) {
    let tags = scan(xml);
    let mut out = String::with_capacity(xml.len());
    let mut pos = 0;
    let mut n = 0;
    let mut i = 0;
    while i < tags.len() {
        let t = tags[i];
        if !t.close
            && !t.empty
            && locals.contains(&t.local())
            && let Some(j) = matching_close(&tags, i)
            && j != i
        {
            let inner = &xml[t.end..tags[j].start];
            if !inner.trim().is_empty() && inner != new {
                out.push_str(&xml[pos..t.end]);
                out.push_str(new);
                pos = tags[j].start;
                n += 1;
            }
            i = j + 1;
            continue;
        }
        i += 1;
    }
    out.push_str(&xml[pos..]);
    (out, n)
}

/// Non-empty text of elements named `locals`, as (local name, text).
fn element_texts(xml: &str, locals: &[&str]) -> Vec<(String, String)> {
    let tags = scan(xml);
    let mut out = Vec::new();
    for (i, t) in tags.iter().enumerate() {
        if t.close || t.empty || !locals.contains(&t.local()) {
            continue;
        }
        if let Some(j) = matching_close(&tags, i)
            && j != i
        {
            let inner = xml[t.end..tags[j].start].trim();
            if !inner.is_empty() {
                out.push((t.local().to_owned(), inner.to_owned()));
            }
        }
    }
    out
}

/// Replaces the value of attributes by local name. Returns the new text and the count.
fn set_attr_values(xml: &str, replacements: &[(&str, &str)]) -> (String, usize) {
    let mut edits: Vec<(Range<usize>, &str)> = Vec::new();
    for t in scan(xml).iter().filter(|t| !t.close) {
        for (name, range) in attrs(xml, t) {
            if let Some((_, new)) = replacements.iter().find(|(l, _)| *l == local(&name))
                && &xml[range.clone()] != *new
            {
                edits.push((range, new));
            }
        }
    }
    let mut out = String::with_capacity(xml.len());
    let mut pos = 0;
    for (range, new) in &edits {
        out.push_str(&xml[pos..range.start]);
        out.push_str(new);
        pos = range.end;
    }
    out.push_str(&xml[pos..]);
    (out, edits.len())
}

fn clip(text: &str) -> String {
    let mut s: String = text.chars().take(80).collect();
    if text.chars().count() > 80 {
        s.push('…');
    }
    s
}

// ---- word-processing content ----

const WORD_TRACKED_DROP: &[&str] = &[
    "del",
    "moveFrom",
    "rPrChange",
    "pPrChange",
    "sectPrChange",
    "tblPrChange",
    "tblGridChange",
    "tcPrChange",
    "trPrChange",
    "numberingChange",
    "cellIns",
    "cellDel",
    "cellMerge",
    "moveFromRangeStart",
    "moveFromRangeEnd",
    "moveToRangeStart",
    "moveToRangeEnd",
];
const WORD_TRACKED_UNWRAP: &[&str] = &["ins", "moveTo"];
const WORD_COMMENT_MARKS: &[&str] = &["commentRangeStart", "commentRangeEnd", "commentReference"];
const WORD_COMMENT_PARTS: &[&str] = &[
    "word/comments.xml",
    "word/commentsextended.xml",
    "word/commentsids.xml",
    "word/commentsextensible.xml",
];

fn is_word_content(lower: &str) -> bool {
    lower.starts_with("word/")
        && lower.ends_with(".xml")
        && !lower.starts_with("word/_rels/")
        && !lower.starts_with("word/theme/")
        && !WORD_COMMENT_PARTS.contains(&lower)
        && lower != "word/people.xml"
}

fn vanish_on(xml: &str, t: &Tag<'_>) -> bool {
    !matches!(attr(xml, t, "val"), Some("0" | "false" | "off"))
}

/// Whether the run starting at tag `i` is hidden: its first child is a `rPr` with `vanish`
/// or `specVanish` that is not switched off. The old properties inside `rPrChange` do not
/// count.
fn is_hidden_run(xml: &str, tags: &[Tag<'_>], i: usize) -> bool {
    let t = tags[i];
    if t.close || t.empty || t.local() != "r" {
        return false;
    }
    let Some(first) = tags.get(i + 1) else {
        return false;
    };
    if first.close || first.empty || first.local() != "rPr" {
        return false;
    }
    let Some(end) = matching_close(tags, i + 1) else {
        return false;
    };
    let mut k = i + 2;
    while k < end {
        let u = tags[k];
        if !u.close && u.local() == "rPrChange" {
            k = matching_close(tags, k).unwrap_or(k) + 1;
            continue;
        }
        if !u.close && matches!(u.local(), "vanish" | "specVanish") && vanish_on(xml, &u) {
            return true;
        }
        k += 1;
    }
    false
}

fn tracked_action(t: &Tag<'_>) -> Option<Act> {
    let l = t.local();
    if WORD_TRACKED_DROP.contains(&l) {
        Some(Act::Drop(Some(HiddenKind::TrackedChanges)))
    } else if WORD_TRACKED_UNWRAP.contains(&l) {
        Some(Act::Unwrap(Some(HiddenKind::TrackedChanges)))
    } else {
        None
    }
}

// ---- inspection ----

fn push(
    items: &mut Vec<HiddenItem>,
    kind: HiddenKind,
    part: &str,
    count: usize,
    detail: String,
    removable: bool,
) {
    items.push(HiddenItem {
        kind,
        part: part.to_owned(),
        count,
        detail,
        removable,
    });
}

fn count_by_local(tags: &[Tag<'_>], locals: &[&str]) -> Vec<(String, usize)> {
    let mut out: Vec<(String, usize)> = Vec::new();
    for t in tags
        .iter()
        .filter(|t| !t.close && locals.contains(&t.local()))
    {
        match out.iter_mut().find(|(n, _)| n == t.local()) {
            Some(slot) => slot.1 += 1,
            None => out.push((t.local().to_owned(), 1)),
        }
    }
    out
}

fn inspect_word(
    pkg: &Package,
    items: &mut Vec<HiddenItem>,
    authors: &mut BTreeSet<String>,
) -> Result<(), CommandError> {
    for name in part_names(pkg) {
        let lower = name.to_ascii_lowercase();
        let Some(xml) = text_of(pkg, &name)? else {
            continue;
        };
        if lower == "word/comments.xml" {
            let tags = scan(xml);
            let mut n = 0;
            for t in tags.iter().filter(|t| !t.close && t.local() == "comment") {
                n += 1;
                if let Some(a) = attr(xml, t, "author").filter(|a| !a.is_empty()) {
                    authors.insert(a.to_owned());
                }
            }
            if n > 0 {
                push(
                    items,
                    HiddenKind::Comments,
                    &name,
                    n,
                    format!("{n} comment(s)"),
                    true,
                );
            }
        } else if lower == "word/people.xml" {
            for t in scan(xml)
                .iter()
                .filter(|t| !t.close && t.local() == "person")
            {
                if let Some(a) = attr(xml, t, "author").filter(|a| !a.is_empty()) {
                    authors.insert(a.to_owned());
                }
            }
        } else if is_word_content(&lower) {
            let tags = scan(xml);
            let mut hidden = 0;
            for i in 0..tags.len() {
                if is_hidden_run(xml, &tags, i) {
                    hidden += 1;
                }
            }
            let mut locals: Vec<&str> = WORD_TRACKED_DROP.to_vec();
            locals.extend_from_slice(WORD_TRACKED_UNWRAP);
            let found = count_by_local(&tags, &locals);
            let total: usize = found.iter().map(|(_, n)| n).sum();
            for t in tags
                .iter()
                .filter(|t| !t.close && tracked_action(t).is_some())
            {
                if let Some(a) = attr(xml, t, "author").filter(|a| !a.is_empty()) {
                    authors.insert(a.to_owned());
                }
            }
            if total > 0 {
                let detail = found
                    .iter()
                    .map(|(n, c)| format!("{c} w:{n}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                push(
                    items,
                    HiddenKind::TrackedChanges,
                    &name,
                    total,
                    detail,
                    true,
                );
            }
            if hidden > 0 {
                push(
                    items,
                    HiddenKind::HiddenText,
                    &name,
                    hidden,
                    format!("{hidden} hidden run(s)"),
                    true,
                );
            }
        }
    }
    Ok(())
}

fn inspect_excel(
    pkg: &Package,
    items: &mut Vec<HiddenItem>,
    authors: &mut BTreeSet<String>,
) -> Result<(), CommandError> {
    for name in part_names(pkg) {
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".rels") {
            continue;
        }
        if lower.starts_with("xl/comments") && lower.ends_with(".xml") {
            let Some(xml) = text_of(pkg, &name)? else {
                continue;
            };
            let n = scan(xml)
                .iter()
                .filter(|t| !t.close && t.local() == "comment")
                .count();
            for (_, a) in element_texts(xml, &["author"]) {
                authors.insert(a);
            }
            if n > 0 {
                push(
                    items,
                    HiddenKind::Comments,
                    &name,
                    n,
                    format!("{n} note(s)"),
                    true,
                );
            }
        } else if lower.starts_with("xl/threadedcomments/") && lower.ends_with(".xml") {
            let Some(xml) = text_of(pkg, &name)? else {
                continue;
            };
            let n = scan(xml)
                .iter()
                .filter(|t| !t.close && t.local() == "threadedComment")
                .count();
            if n > 0 {
                push(
                    items,
                    HiddenKind::Comments,
                    &name,
                    n,
                    format!("{n} threaded comment(s)"),
                    true,
                );
            }
        } else if lower.starts_with("xl/persons/") && lower.ends_with(".xml") {
            let Some(xml) = text_of(pkg, &name)? else {
                continue;
            };
            for t in scan(xml)
                .iter()
                .filter(|t| !t.close && t.local() == "person")
            {
                if let Some(a) = attr(xml, t, "displayName").filter(|a| !a.is_empty()) {
                    authors.insert(a.to_owned());
                }
            }
        } else if lower.starts_with("xl/revisions/") {
            push(
                items,
                HiddenKind::TrackedChanges,
                &name,
                1,
                "shared-workbook revision history".to_owned(),
                true,
            );
        } else if lower == "xl/workbook.xml" {
            let Some(xml) = text_of(pkg, &name)? else {
                continue;
            };
            let hidden: Vec<String> = scan(xml)
                .iter()
                .filter(|t| !t.close && t.local() == "sheet")
                .filter(|t| matches!(attr(xml, t, "state"), Some("hidden" | "veryHidden")))
                .map(|t| clip(attr(xml, t, "name").unwrap_or("?")))
                .collect();
            if !hidden.is_empty() {
                push(
                    items,
                    HiddenKind::HiddenText,
                    &name,
                    hidden.len(),
                    format!("hidden sheet(s): {}", hidden.join(", ")),
                    false,
                );
            }
        } else if lower.starts_with("xl/worksheets/") && lower.ends_with(".xml") {
            let Some(xml) = text_of(pkg, &name)? else {
                continue;
            };
            let tags = scan(xml);
            let on = |t: &Tag<'_>| matches!(attr(xml, t, "hidden"), Some("1" | "true"));
            let rows = tags
                .iter()
                .filter(|t| !t.close && t.local() == "row" && on(t))
                .count();
            let cols = tags
                .iter()
                .filter(|t| !t.close && t.local() == "col" && on(t))
                .count();
            if rows + cols > 0 {
                push(
                    items,
                    HiddenKind::HiddenText,
                    &name,
                    rows + cols,
                    format!("{rows} hidden row(s), {cols} hidden column range(s)"),
                    false,
                );
            }
        }
    }
    Ok(())
}

const CORE_PROPERTIES: &[&str] = &[
    "title",
    "subject",
    "description",
    "keywords",
    "category",
    "contentStatus",
];
const CORE_PERSONAL: &[&str] = &["creator", "lastModifiedBy"];
const APP_PERSONAL: &[&str] = &["Company", "Manager"];

fn inspect_properties(pkg: &Package, items: &mut Vec<HiddenItem>) -> Result<(), CommandError> {
    let list = |texts: Vec<(String, String)>| -> String {
        texts
            .iter()
            .map(|(n, v)| format!("{n}: {}", clip(v)))
            .collect::<Vec<_>>()
            .join("; ")
    };
    if let Some(xml) = text_of(pkg, "docProps/core.xml")? {
        let props = element_texts(xml, CORE_PROPERTIES);
        if !props.is_empty() {
            push(
                items,
                HiddenKind::Properties,
                "docProps/core.xml",
                props.len(),
                list(props),
                true,
            );
        }
        let people = element_texts(xml, CORE_PERSONAL);
        if !people.is_empty() {
            push(
                items,
                HiddenKind::PersonalInfo,
                "docProps/core.xml",
                people.len(),
                list(people),
                true,
            );
        }
    }
    if let Some(xml) = text_of(pkg, "docProps/app.xml")? {
        let people = element_texts(xml, APP_PERSONAL);
        if !people.is_empty() {
            push(
                items,
                HiddenKind::PersonalInfo,
                "docProps/app.xml",
                people.len(),
                list(people),
                true,
            );
        }
    }
    if let Some(xml) = text_of(pkg, "docProps/custom.xml")? {
        let n = scan(xml)
            .iter()
            .filter(|t| !t.close && t.local() == "property")
            .count();
        if n > 0 {
            push(
                items,
                HiddenKind::Properties,
                "docProps/custom.xml",
                n,
                format!("{n} custom propert(ies)"),
                true,
            );
        }
    }
    Ok(())
}

/// Lists the hidden content of a DOCX or XLSX package. Read-only.
///
/// # Errors
/// The bytes are not a readable DOCX or XLSX package, or a part is not UTF-8 XML.
pub fn inspect_bytes(bytes: &[u8]) -> Result<HiddenReport, CommandError> {
    let (pkg, format) = open_package(bytes)?;
    let mut items = Vec::new();
    let mut authors = BTreeSet::new();
    match format {
        Format::Docx => inspect_word(&pkg, &mut items, &mut authors)?,
        Format::Xlsx => inspect_excel(&pkg, &mut items, &mut authors)?,
    }
    inspect_properties(&pkg, &mut items)?;
    if !authors.is_empty() {
        let names: Vec<String> = authors.iter().map(|a| clip(a)).collect();
        push(
            &mut items,
            HiddenKind::PersonalInfo,
            "(comments and tracked changes)",
            authors.len(),
            format!("author(s): {}", names.join(", ")),
            true,
        );
    }
    Ok(HiddenReport {
        schema: "open365.hidden-content/1",
        format: match format {
            Format::Docx => "docx",
            Format::Xlsx => "xlsx",
        },
        items,
        authors: authors.into_iter().take(50).collect(),
    })
}

// ---- sanitising ----

fn rels_source(rels_name: &str) -> Option<Option<PartName>> {
    if rels_name.eq_ignore_ascii_case("_rels/.rels") {
        return Some(None);
    }
    let (dir, file) = rels_name.rsplit_once('/')?;
    let (parent, last) = match dir.rsplit_once('/') {
        Some((p, l)) => (Some(p), l),
        None => (None, dir),
    };
    if !last.eq_ignore_ascii_case("_rels") {
        return None;
    }
    let stem = file
        .strip_suffix(".rels")
        .or_else(|| file.strip_suffix(".RELS"))?;
    let source = match parent {
        Some(p) => format!("{p}/{stem}"),
        None => stem.to_owned(),
    };
    PartName::new(&source).ok().map(Some)
}

/// Removes parts, their relationships and content-type overrides. Returns the
/// (source part, relationship id) pairs that pointed at them, so the source can drop the
/// element that used the id.
fn drop_parts(pkg: &mut Package, names: &[String]) -> Result<Vec<(String, String)>, CommandError> {
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let doomed: HashSet<String> = names.iter().map(|n| n.to_ascii_lowercase()).collect();
    let mut refs = Vec::new();
    for rels in part_names(pkg)
        .into_iter()
        .filter(|n| n.to_ascii_lowercase().ends_with(".rels"))
    {
        let Some(source) = rels_source(&rels) else {
            continue;
        };
        let Some(xml) = text_of(pkg, &rels)? else {
            continue;
        };
        let source_name = source.as_ref().map(|s| s.as_str().to_owned());
        let mut found = Vec::new();
        let mut rule = |xml: &str, tags: &[Tag<'_>], i: usize| {
            let t = tags[i];
            if t.local() != "Relationship" || attr(xml, &t, "TargetMode") == Some("External") {
                return Act::Keep;
            }
            let hit = attr(xml, &t, "Target")
                .and_then(|target| resolve_target(source.as_ref(), target).ok())
                .is_some_and(|p| doomed.contains(&p.as_str().to_ascii_lowercase()));
            if hit {
                found.push(attr(xml, &t, "Id").unwrap_or_default().to_owned());
                Act::Drop(None)
            } else {
                Act::Keep
            }
        };
        let (new, _) = edit_xml(xml, &mut rule);
        if !found.is_empty() {
            set_text(pkg, &rels, new)?;
            if let Some(src) = &source_name {
                refs.extend(found.into_iter().map(|id| (src.clone(), id)));
            }
        }
    }
    for name in names {
        pkg.remove_part_and_override(name);
        if let Ok(part) = PartName::new(name) {
            pkg.remove_part(rels_part_name(Some(&part)).as_str());
        }
    }
    Ok(refs)
}

/// Drops `legacyDrawing` elements that used a removed relationship.
fn drop_legacy_refs(pkg: &mut Package, refs: &[(String, String)]) -> Result<(), CommandError> {
    let sources: BTreeSet<&String> = refs.iter().map(|(s, _)| s).collect();
    for source in sources {
        let ids: HashSet<&str> = refs
            .iter()
            .filter(|(s, _)| s == source)
            .map(|(_, id)| id.as_str())
            .collect();
        let Some(xml) = text_of(pkg, source)? else {
            continue;
        };
        let mut rule = |xml: &str, tags: &[Tag<'_>], i: usize| {
            let t = tags[i];
            if t.local() == "legacyDrawing"
                && attr(xml, &t, "id").is_some_and(|id| ids.contains(id))
            {
                Act::Drop(None)
            } else {
                Act::Keep
            }
        };
        let (new, _) = edit_xml(xml, &mut rule);
        if new != xml {
            set_text(pkg, source, new)?;
        }
    }
    Ok(())
}

/// Whether a VML drawing holds only comment boxes (every `ObjectType` is `Note`).
fn is_note_vml(text: &str) -> bool {
    let mut seen = 0;
    let mut rest = text;
    while let Some(p) = rest.find("ObjectType=") {
        rest = &rest[p + "ObjectType=".len()..];
        let value = rest.trim_start_matches(['"', '\'']);
        if !value.starts_with("Note") {
            return false;
        }
        seen += 1;
    }
    seen > 0
}

fn blank_properties(
    pkg: &mut Package,
    kinds: &BTreeSet<HiddenKind>,
) -> Result<BTreeMap<HiddenKind, usize>, CommandError> {
    let mut removed = BTreeMap::new();
    let mut count = |kind: HiddenKind, n: usize| {
        if n > 0 {
            *removed.entry(kind).or_insert(0) += n;
        }
    };
    if let Some(xml) = text_of(pkg, "docProps/core.xml")? {
        let mut text = xml.to_owned();
        if kinds.contains(&HiddenKind::Properties) {
            let (new, n) = set_element_text(&text, CORE_PROPERTIES, "");
            text = new;
            count(HiddenKind::Properties, n);
        }
        if kinds.contains(&HiddenKind::PersonalInfo) {
            let (new, n) = set_element_text(&text, CORE_PERSONAL, "");
            text = new;
            count(HiddenKind::PersonalInfo, n);
        }
        if text != xml {
            set_text(pkg, "docProps/core.xml", text)?;
        }
    }
    if kinds.contains(&HiddenKind::PersonalInfo)
        && let Some(xml) = text_of(pkg, "docProps/app.xml")?
    {
        let (new, n) = set_element_text(xml, APP_PERSONAL, "");
        if n > 0 {
            count(HiddenKind::PersonalInfo, n);
            set_text(pkg, "docProps/app.xml", new)?;
        }
    }
    if kinds.contains(&HiddenKind::Properties)
        && let Some(xml) = text_of(pkg, "docProps/custom.xml")?
    {
        let n = scan(xml)
            .iter()
            .filter(|t| !t.close && t.local() == "property")
            .count();
        drop_parts(pkg, &["docProps/custom.xml".to_owned()])?;
        count(HiddenKind::Properties, n);
    }
    Ok(removed)
}

fn sanitize_word(
    pkg: &mut Package,
    kinds: &BTreeSet<HiddenKind>,
    removed: &mut BTreeMap<HiddenKind, usize>,
) -> Result<(), CommandError> {
    let comments = kinds.contains(&HiddenKind::Comments);
    let tracked = kinds.contains(&HiddenKind::TrackedChanges);
    let hidden = kinds.contains(&HiddenKind::HiddenText);
    let personal = kinds.contains(&HiddenKind::PersonalInfo);
    if comments {
        let n = match text_of(pkg, "word/comments.xml")? {
            Some(xml) => scan(xml)
                .iter()
                .filter(|t| !t.close && t.local() == "comment")
                .count(),
            None => 0,
        };
        let doomed: Vec<String> = part_names(pkg)
            .into_iter()
            .filter(|p| WORD_COMMENT_PARTS.contains(&p.to_ascii_lowercase().as_str()))
            .collect();
        drop_parts(pkg, &doomed)?;
        if n > 0 {
            *removed.entry(HiddenKind::Comments).or_insert(0) += n;
        }
    }
    if personal {
        let people: Vec<String> = part_names(pkg)
            .into_iter()
            .filter(|p| p.eq_ignore_ascii_case("word/people.xml"))
            .collect();
        drop_parts(pkg, &people)?;
    }
    if personal && let Some(xml) = text_of(pkg, "word/comments.xml")? {
        let (new, n) = set_attr_values(xml, &[("author", "Author"), ("initials", "")]);
        if n > 0 {
            *removed.entry(HiddenKind::PersonalInfo).or_insert(0) += n;
            set_text(pkg, "word/comments.xml", new)?;
        }
    }
    if comments || tracked || hidden || personal {
        for name in part_names(pkg) {
            if !is_word_content(&name.to_ascii_lowercase()) {
                continue;
            }
            let Some(xml) = text_of(pkg, &name)? else {
                continue;
            };
            let mut rule = |xml: &str, tags: &[Tag<'_>], i: usize| {
                let t = tags[i];
                if comments && WORD_COMMENT_MARKS.contains(&t.local()) {
                    return Act::Drop(None);
                }
                if tracked && let Some(act) = tracked_action(&t) {
                    return act;
                }
                if hidden && is_hidden_run(xml, tags, i) {
                    return Act::Drop(Some(HiddenKind::HiddenText));
                }
                Act::Keep
            };
            let (mut text, counts) = edit_xml(xml, &mut rule);
            for (k, n) in counts {
                *removed.entry(k).or_insert(0) += n;
            }
            if personal {
                let (new, n) = set_attr_values(&text, &[("author", "Author"), ("initials", "")]);
                text = new;
                if n > 0 {
                    *removed.entry(HiddenKind::PersonalInfo).or_insert(0) += n;
                }
            }
            if text != xml {
                set_text(pkg, &name, text)?;
            }
        }
    }
    Ok(())
}

fn sanitize_excel(
    pkg: &mut Package,
    kinds: &BTreeSet<HiddenKind>,
    removed: &mut BTreeMap<HiddenKind, usize>,
) -> Result<(), CommandError> {
    let mut doomed: Vec<String> = Vec::new();
    if kinds.contains(&HiddenKind::Comments) {
        let mut n = 0;
        for name in part_names(pkg) {
            let lower = name.to_ascii_lowercase();
            if lower.ends_with(".rels") {
                continue;
            }
            let tag = if lower.starts_with("xl/comments") && lower.ends_with(".xml") {
                "comment"
            } else if lower.starts_with("xl/threadedcomments/") && lower.ends_with(".xml") {
                "threadedComment"
            } else if lower.starts_with("xl/drawings/") && lower.ends_with(".vml") {
                if text_of(pkg, &name)?.is_some_and(is_note_vml) {
                    doomed.push(name);
                }
                continue;
            } else {
                continue;
            };
            if let Some(xml) = text_of(pkg, &name)? {
                n += scan(xml)
                    .iter()
                    .filter(|t| !t.close && t.local() == tag)
                    .count();
            }
            doomed.push(name);
        }
        if n > 0 {
            *removed.entry(HiddenKind::Comments).or_insert(0) += n;
        }
    }
    if kinds.contains(&HiddenKind::TrackedChanges) {
        let revisions: Vec<String> = part_names(pkg)
            .into_iter()
            .filter(|n| {
                let l = n.to_ascii_lowercase();
                l.starts_with("xl/revisions/") && !l.ends_with(".rels")
            })
            .collect();
        if !revisions.is_empty() {
            *removed.entry(HiddenKind::TrackedChanges).or_insert(0) += revisions.len();
        }
        doomed.extend(revisions);
    }
    let refs = drop_parts(pkg, &doomed)?;
    drop_legacy_refs(pkg, &refs)?;
    if kinds.contains(&HiddenKind::PersonalInfo) {
        let mut n = 0;
        for name in part_names(pkg) {
            let lower = name.to_ascii_lowercase();
            if lower.ends_with(".rels") || !lower.ends_with(".xml") {
                continue;
            }
            let Some(xml) = text_of(pkg, &name)? else {
                continue;
            };
            let new = if lower.starts_with("xl/comments") {
                let (t, c) = set_element_text(xml, &["author"], "Author");
                n += c;
                t
            } else if lower.starts_with("xl/persons/") {
                let (t, c) = set_attr_values(xml, &[("displayName", "Author"), ("userId", "")]);
                n += c;
                t
            } else {
                continue;
            };
            if new != xml {
                set_text(pkg, &name, new)?;
            }
        }
        if n > 0 {
            *removed.entry(HiddenKind::PersonalInfo).or_insert(0) += n;
        }
    }
    Ok(())
}

/// Returns a copy of the package with the chosen kinds removed. The input is not changed.
/// Tracked changes are accepted (insertions kept, deletions and old formatting dropped).
///
/// # Errors
/// Nothing was chosen; the package is unreadable; or a workbook has hidden sheets, rows or
/// columns and `HiddenText` was chosen (they cannot be removed without changing formulas;
/// unhide or delete them first).
pub fn sanitize_bytes(bytes: &[u8], remove: &[HiddenKind]) -> Result<Sanitized, CommandError> {
    let kinds: BTreeSet<HiddenKind> = remove.iter().copied().collect();
    if kinds.is_empty() {
        return Err(CommandError::Share(
            "choose at least one kind of content to remove".to_owned(),
        ));
    }
    let (mut pkg, format) = open_package(bytes)?;
    let mut removed = BTreeMap::new();
    match format {
        Format::Docx => sanitize_word(&mut pkg, &kinds, &mut removed)?,
        Format::Xlsx => {
            if kinds.contains(&HiddenKind::HiddenText) {
                let mut items = Vec::new();
                inspect_excel(&pkg, &mut items, &mut BTreeSet::new())?;
                if items.iter().any(|i| i.kind == HiddenKind::HiddenText) {
                    return Err(CommandError::Share(
                        "hidden sheets, rows and columns cannot be removed from a workbook without changing it; unhide or delete them first"
                            .to_owned(),
                    ));
                }
            }
            sanitize_excel(&mut pkg, &kinds, &mut removed)?;
        }
    }
    for (kind, n) in blank_properties(&mut pkg, &kinds)? {
        *removed.entry(kind).or_insert(0) += n;
    }
    Ok(Sanitized {
        bytes: pkg.write().map_err(package_error)?,
        removed,
    })
}

// ---- files ----

/// Whether `a` and `b` name the same file (by path, or by canonical path when both exist).
fn same_file(a: &Path, b: &Path) -> bool {
    a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y)
}

/// Where a share copy goes. With `path` that path (never the document's own file); without,
/// a new folder in the system temp folder, named after the document, that the caller deletes
/// after use ([`remove_share_temp`], [`sweep_share_temp`]).
pub(crate) fn destination(
    path: Option<&Path>,
    source: Option<&Path>,
    file_name: &str,
    roots: &[PathBuf],
) -> Result<PathBuf, CommandError> {
    match path {
        Some(p) => {
            if source.is_some_and(|s| same_file(p, s)) {
                return Err(CommandError::Share(
                    "the target is the document itself; a share copy never replaces the original"
                        .to_owned(),
                ));
            }
            Ok(p.to_path_buf())
        }
        None => {
            if !roots.is_empty() {
                return Err(CommandError::Share(
                    "a temporary copy cannot be made while writes are limited to folders; name a path inside one"
                        .to_owned(),
                ));
            }
            let dir = std::env::temp_dir().join(format!("{SHARE_TEMP_PREFIX}{}", unique()));
            std::fs::create_dir(&dir).map_err(|source| CommandError::Io {
                op: "create",
                path: dir.display().to_string(),
                source,
            })?;
            Ok(dir.join(file_name))
        }
    }
}

fn unique() -> String {
    let name = crate::target::temp_name();
    name.trim_start_matches('.')
        .trim_end_matches(".open365-tmp")
        .to_owned()
}

/// The document's file name for a copy, or `fallback` for an unsaved document.
pub(crate) fn copy_name(
    source: Option<&Path>,
    fallback: &str,
    suffix: &str,
    ext: Option<&str>,
) -> String {
    let stem = source
        .and_then(|s| s.file_stem())
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(fallback);
    let ext = ext
        .map(str::to_owned)
        .or_else(|| {
            source
                .and_then(|s| s.extension())
                .and_then(|e| e.to_str())
                .map(str::to_owned)
        })
        .unwrap_or_default();
    if ext.is_empty() {
        format!("{stem}{suffix}")
    } else {
        format!("{stem}{suffix}.{ext}")
    }
}

/// Writes `bytes` to `path`. Without `overwrite` the file is created exclusively and an
/// existing file is [`CommandError::Exists`]; with it the bytes replace the file atomically.
pub(crate) fn write_share_file(
    path: &Path,
    bytes: &[u8],
    overwrite: bool,
    roots: &[PathBuf],
) -> Result<(), CommandError> {
    use std::io::Write;
    if overwrite {
        return write_atomic_within(path, bytes, roots);
    }
    let io = |op: &'static str, source| CommandError::Io {
        op,
        path: path.display().to_string(),
        source,
    };
    if !roots.is_empty() {
        let parent = match path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => PathBuf::from("."),
        };
        let inside = parent
            .canonicalize()
            .is_ok_and(|p| roots.iter().any(|r| p.starts_with(r)));
        if !inside {
            return Err(io(
                "create",
                std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "the destination folder is outside the allowed folders",
                ),
            ));
        }
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                CommandError::Exists {
                    path: path.display().to_string(),
                }
            } else {
                io("create", e)
            }
        })?;
    let written = file.write_all(bytes).and_then(|()| file.flush());
    if let Err(e) = written {
        drop(file);
        let _ = std::fs::remove_file(path);
        return Err(io("write", e));
    }
    Ok(())
}

/// Deletes a temporary share copy and its folder, if `path` is inside a folder this module
/// made. Returns whether anything was removed. Any other path is left alone.
pub fn remove_share_temp(path: &Path) -> bool {
    let Some(dir) = path.parent() else {
        return false;
    };
    let ours = dir
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with(SHARE_TEMP_PREFIX))
        && dir
            .parent()
            .is_some_and(|p| same_file(p, &std::env::temp_dir()));
    ours && std::fs::remove_dir_all(dir).is_ok()
}

/// Deletes every leftover share folder in the system temp folder (call at start-up). Returns
/// how many were removed.
pub fn sweep_share_temp() -> usize {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.starts_with(SHARE_TEMP_PREFIX))
                && e.file_type().is_ok_and(|t| t.is_dir())
                && std::fs::remove_dir_all(e.path()).is_ok()
        })
        .count()
}

/// The request the UI hands to the operating system's share sheet. This only describes the
/// request; no operating system API is called here.
pub(crate) fn os_sheet_request(path: &Path) -> Result<Value, CommandError> {
    let meta = std::fs::metadata(path).map_err(|source| CommandError::Io {
        op: "read",
        path: path.display().to_string(),
        source,
    })?;
    if !meta.is_file() {
        return Err(CommandError::Share(format!(
            "{} is not a file",
            path.display()
        )));
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let mime = match ext.as_str() {
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "docm" => "application/vnd.ms-word.document.macroEnabled.12",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "xlsm" => "application/vnd.ms-excel.sheet.macroEnabled.12",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    };
    Ok(json!({
        "schema": "open365.os-share-request/1",
        "path": path.display().to_string(),
        "file_name": path.file_name().and_then(|n| n.to_str()).unwrap_or(""),
        "mime": mime,
        "bytes": meta.len(),
        "temporary": path
            .parent()
            .and_then(|d| d.file_name())
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with(SHARE_TEMP_PREFIX)),
        "performed": false,
    }))
}

pub(crate) fn report_notes(report: &HiddenReport) -> Vec<String> {
    report.lines()
}
