//! Finds content a user may not see in Word or Excel: hidden text, comments, tracked
//! changes, hidden sheets/rows/columns, defined names, external links, macros, custom XML,
//! thumbnails, unreachable parts and author metadata. Read-only; never fetches anything.

use std::collections::HashSet;

use open365_opc::{
    Limits, OpcError, Package, PartName, Relationships, TargetMode, rels_part_name, resolve_target,
};

use crate::xmlscan::{El, scan};

/// One thing a user may not see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiddenFinding {
    /// Stable category: `hidden_text`, `comment`, `tracked_change`, `hidden_sheet`,
    /// `hidden_row`, `hidden_column`, `defined_name`, `external_link`, `vba`, `custom_xml`,
    /// `thumbnail`, `unreachable_part`, `metadata`, `unparseable`.
    pub category: &'static str,
    pub part: String,
    pub detail: String,
}

fn truthy(el: &El, key: &str) -> bool {
    !matches!(el.attr(key), Some("0" | "false" | "off"))
}

fn flag(el: &El, key: &str) -> bool {
    matches!(el.attr(key), Some("1" | "true"))
}

fn add(out: &mut Vec<HiddenFinding>, category: &'static str, part: &str, detail: String) {
    out.push(HiddenFinding {
        category,
        part: part.to_owned(),
        detail,
    });
}

/// Reads `bytes` with default limits and lists hidden content.
pub fn inspect_hidden(bytes: &[u8]) -> Result<Vec<HiddenFinding>, OpcError> {
    let package = Package::read(bytes, &Limits::default())?;
    Ok(hidden_findings(&package))
}

/// Lower-cased names of parts (and `.rels` parts) reachable from the package root through
/// internal relationships.
fn reachable(package: &Package) -> HashSet<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut queue: Vec<Option<PartName>> = vec![None];
    let mut visited: HashSet<Option<String>> = HashSet::new();
    while let Some(source) = queue.pop() {
        if !visited.insert(source.as_ref().map(|s| s.as_str().to_ascii_lowercase())) {
            continue;
        }
        let rels_name = rels_part_name(source.as_ref());
        let Some(rels_part) = package.part(rels_name.as_str()) else {
            continue;
        };
        seen.insert(rels_name.as_str().to_ascii_lowercase());
        let Ok(rels) = Relationships::parse(rels_part.data()) else {
            continue;
        };
        for rel in rels.iter() {
            if rel.target_mode == TargetMode::External {
                continue;
            }
            let Ok(target) = resolve_target(source.as_ref(), &rel.target) else {
                continue;
            };
            if package.part(target.as_str()).is_some() {
                seen.insert(target.as_str().to_ascii_lowercase());
                queue.push(Some(target));
            }
        }
    }
    seen
}

pub(crate) fn hidden_findings(package: &Package) -> Vec<HiddenFinding> {
    let mut out = Vec::new();
    let seen = reachable(package);
    for part in package.parts() {
        let name = part.name().as_str();
        let lower = name.to_ascii_lowercase();
        let is_rels = lower.ends_with(".rels");
        let size = part.data().len();
        if lower != "[content_types].xml" && !is_rels && !seen.contains(&lower) {
            add(
                &mut out,
                "unreachable_part",
                name,
                format!("{size} bytes, no relationship path from the package root"),
            );
        }
        if lower.ends_with("vbaproject.bin") || lower.contains("/vbadata") {
            add(&mut out, "vba", name, "macro project".to_owned());
        } else if lower.starts_with("customxml/") && !is_rels {
            if lower.ends_with(".xml") && !lower.contains("props") {
                add(&mut out, "custom_xml", name, format!("{size} bytes"));
            }
        } else if lower.starts_with("docprops/thumbnail") {
            add(&mut out, "thumbnail", name, format!("{size} bytes"));
        } else if lower.starts_with("xl/externallinks/") && !is_rels {
            add(
                &mut out,
                "external_link",
                name,
                "external workbook link".to_owned(),
            );
        }
        if is_rels {
            external_rels(name, part.data(), &mut out);
        }
        if !lower.ends_with(".xml") {
            continue;
        }
        let els = match scan(part.data()) {
            Ok(els) => els,
            Err(e) => {
                if lower.starts_with("word/")
                    || lower.starts_with("xl/")
                    || lower == "docprops/core.xml"
                {
                    add(&mut out, "unparseable", name, e);
                }
                continue;
            }
        };
        if lower.starts_with("word/") {
            word_part(name, &lower, &els, &mut out);
        } else if lower.starts_with("xl/") {
            excel_part(name, &lower, &els, &mut out);
        } else if lower == "docprops/core.xml" {
            for el in &els {
                if matches!(el.name.as_str(), "creator" | "lastModifiedBy")
                    && !el.text.trim().is_empty()
                {
                    add(
                        &mut out,
                        "metadata",
                        name,
                        format!("{}: {}", el.name, el.text.trim()),
                    );
                }
            }
        }
    }
    out
}

fn external_rels(rels_name: &str, data: &[u8], out: &mut Vec<HiddenFinding>) {
    if let Ok(rels) = Relationships::parse(data) {
        for r in rels.iter() {
            if r.target_mode == TargetMode::External && r.rel_type.ends_with("/externalLink") {
                add(
                    out,
                    "external_link",
                    rels_name,
                    format!("{} -> {}", r.id, r.target),
                );
            }
        }
    }
}

fn word_part(name: &str, lower: &str, els: &[El], out: &mut Vec<HiddenFinding>) {
    let mut vanish = 0usize;
    let mut tracked: Vec<(&str, usize)> = Vec::new();
    let mut comments = 0usize;
    for el in els {
        match el.name.as_str() {
            "vanish" | "specVanish" if truthy(el, "val") => vanish += 1,
            "ins" | "del" | "moveFrom" | "moveTo" | "rPrChange" | "pPrChange" | "sectPrChange"
            | "tblPrChange" | "numberingChange" | "cellIns" | "cellDel" => {
                match tracked.iter_mut().find(|(n, _)| *n == el.name) {
                    Some(slot) => slot.1 += 1,
                    None => tracked.push((el.name.as_str(), 1)),
                }
            }
            "comment" if lower.contains("comments") => comments += 1,
            _ => {}
        }
    }
    if vanish > 0 {
        add(
            out,
            "hidden_text",
            name,
            format!("{vanish} w:vanish occurrence(s)"),
        );
    }
    for (n, c) in tracked {
        add(out, "tracked_change", name, format!("{c} w:{n}"));
    }
    if comments > 0 {
        add(out, "comment", name, format!("{comments} comment(s)"));
    }
}

fn excel_part(name: &str, lower: &str, els: &[El], out: &mut Vec<HiddenFinding>) {
    if lower == "xl/workbook.xml" {
        for el in els {
            match el.name.as_str() {
                "sheet" => {
                    if let Some(state @ ("hidden" | "veryHidden")) = el.attr("state") {
                        add(
                            out,
                            "hidden_sheet",
                            name,
                            format!("{:?} ({state})", el.attr("name").unwrap_or("?")),
                        );
                    }
                }
                "definedName" => add(
                    out,
                    "defined_name",
                    name,
                    format!(
                        "{} = {}{}",
                        el.attr("name").unwrap_or("?"),
                        el.text.trim(),
                        if flag(el, "hidden") { " (hidden)" } else { "" }
                    ),
                ),
                "externalReference" => add(
                    out,
                    "external_link",
                    name,
                    format!("externalReference {}", el.attr("id").unwrap_or("?")),
                ),
                _ => {}
            }
        }
    } else if lower.starts_with("xl/worksheets/") {
        let (mut rows, mut cols) = (Vec::new(), Vec::new());
        for el in els.iter().filter(|e| flag(e, "hidden")) {
            match el.name.as_str() {
                "row" => rows.push(el.attr("r").unwrap_or("?").to_owned()),
                "col" => cols.push(format!(
                    "{}-{}",
                    el.attr("min").unwrap_or("?"),
                    el.attr("max").unwrap_or("?")
                )),
                _ => {}
            }
        }
        if !rows.is_empty() {
            add(out, "hidden_row", name, format!("rows {}", rows.join(",")));
        }
        if !cols.is_empty() {
            add(
                out,
                "hidden_column",
                name,
                format!("columns {}", cols.join(",")),
            );
        }
    } else if lower.starts_with("xl/comments") || lower.starts_with("xl/threadedcomments/") {
        let n = els
            .iter()
            .filter(|e| matches!(e.name.as_str(), "comment" | "threadedComment"))
            .count();
        if n > 0 {
            add(out, "comment", name, format!("{n} comment(s)"));
        }
    }
}
