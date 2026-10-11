//! `open365 docx|xlsx inspect`: package structure plus, with `--hidden`, what a user may not see.

use std::path::Path;

use open365_testkit::{HiddenFinding, Inspection, inspect, inspect_hidden};
use serde_json::{Value, json};

use crate::output::Report;
use crate::{CliError, read};

/// Which Office format the file is expected to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Docx,
    Xlsx,
}

impl Kind {
    fn marker(self) -> &'static str {
        match self {
            Self::Docx => "wordprocessingml",
            Self::Xlsx => "spreadsheetml",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Docx => "docx",
            Self::Xlsx => "xlsx",
        }
    }
}

/// Fails unless some part's content type belongs to the expected format.
fn check_kind(info: &Inspection, kind: Kind, file: &Path) -> Result<(), CliError> {
    let ok = info
        .parts
        .iter()
        .filter_map(|p| p.content_type.as_deref())
        .any(|ct| ct.contains(kind.marker()) && ct.contains("main+xml"));
    if ok {
        Ok(())
    } else {
        Err(CliError::Other(format!(
            "{}: not a {} package (no {} main part)",
            file.display(),
            kind.name(),
            kind.marker()
        )))
    }
}

pub fn run(kind: Kind, file: &Path, hidden: bool) -> Result<Report, CliError> {
    let bytes = read(file)?;
    let info = inspect(&bytes).map_err(|e| CliError::Opc(file.to_owned(), e))?;
    check_kind(&info, kind, file)?;
    let findings = if hidden {
        inspect_hidden(&bytes).map_err(|e| CliError::Opc(file.to_owned(), e))?
    } else {
        Vec::new()
    };
    let shown = hidden.then_some(findings.as_slice());
    Ok(Report::new(
        to_json(&info, shown),
        render_text(&info, shown),
    ))
}

pub fn to_json(info: &Inspection, hidden: Option<&[HiddenFinding]>) -> Value {
    let mut doc = json!({
        "parts": info.parts.iter().map(|p| json!({
            "name": p.name,
            "size": p.size,
            "compression": format!("{:?}", p.compression),
            "content_type": p.content_type,
        })).collect::<Vec<_>>(),
        "relationships": info.relationships.iter().map(|r| json!({
            "rels_part": r.rels_part,
            "id": r.id,
            "type": r.rel_type,
            "target": r.target,
            "external": r.external,
        })).collect::<Vec<_>>(),
        "content_types": {
            "defaults": info.default_types.iter().map(|(e, t)| json!({"extension": e, "content_type": t})).collect::<Vec<_>>(),
            "overrides": info.override_types.iter().map(|(p, t)| json!({"part": p, "content_type": t})).collect::<Vec<_>>(),
        },
        "integrity_issues": info.integrity_issues.iter().map(|i| format!("{i:?}")).collect::<Vec<_>>(),
    });
    if let (Some(findings), Some(map)) = (hidden, doc.as_object_mut()) {
        map.insert(
            "hidden".to_owned(),
            Value::Array(
                findings
                    .iter()
                    .map(|f| json!({"category": f.category, "part": f.part, "detail": f.detail}))
                    .collect(),
            ),
        );
    }
    doc
}

pub fn render_text(info: &Inspection, hidden: Option<&[HiddenFinding]>) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{:>10}  {:<8}  {:<60}  PART",
        "SIZE", "METHOD", "CONTENT-TYPE"
    );
    for p in &info.parts {
        let _ = writeln!(
            out,
            "{:>10}  {:<8}  {:<60}  {}",
            p.size,
            format!("{:?}", p.compression),
            p.content_type.as_deref().unwrap_or("-"),
            p.name
        );
    }
    let _ = writeln!(out, "\nRELATIONSHIPS");
    for r in &info.relationships {
        let _ = writeln!(
            out,
            "  {} {} {} -> {}{}",
            r.rels_part,
            r.id,
            r.rel_type,
            r.target,
            if r.external { " (external)" } else { "" }
        );
    }
    let _ = writeln!(out, "\nCONTENT TYPES");
    for (e, t) in &info.default_types {
        let _ = writeln!(out, "  default .{e} = {t}");
    }
    for (p, t) in &info.override_types {
        let _ = writeln!(out, "  override /{p} = {t}");
    }
    let _ = writeln!(
        out,
        "\n{} parts, {} relationships, {} integrity issues",
        info.parts.len(),
        info.relationship_count,
        info.integrity_issues.len()
    );
    for issue in &info.integrity_issues {
        let _ = writeln!(out, "  issue: {issue:?}");
    }
    if let Some(findings) = hidden {
        let _ = writeln!(out, "\nHIDDEN CONTENT ({} findings)", findings.len());
        for f in findings {
            let _ = writeln!(out, "  {:<16} {}: {}", f.category, f.part, f.detail);
        }
    }
    out
}
