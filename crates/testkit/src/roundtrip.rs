//! Read -> write -> read invariant checker.

use std::collections::{HashMap, HashSet};

use open365_opc::{IntegrityIssue, Limits, MainPartKind, OpcError, Package};

/// Differences between a package and its read -> write -> read copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoundtripReport {
    pub part_count: usize,
    /// Parts present in both whose bytes differ.
    pub changed_parts: Vec<String>,
    /// Part order differs from the original (names compared in sequence).
    pub order_changed: bool,
    pub missing_parts: Vec<String>,
    pub added_parts: Vec<String>,
    /// Integrity issues present after but not before.
    pub new_issues: Vec<IntegrityIssue>,
    /// The main part's kind (docx, docm, dotx, dotm, xlsx, xlsm, xltx or xltm) before the
    /// round trip, when its content type is one of those.
    pub main_kind: Option<MainPartKind>,
    /// The main part's kind or name differs after the round trip.
    pub main_kind_changed: bool,
}

impl RoundtripReport {
    pub fn is_lossless(&self) -> bool {
        self.changed_parts.is_empty()
            && !self.order_changed
            && self.missing_parts.is_empty()
            && self.added_parts.is_empty()
            && self.new_issues.is_empty()
            && !self.main_kind_changed
    }

    /// One-line description of what differs; empty when lossless.
    pub fn describe(&self) -> String {
        let mut out = Vec::new();
        let mut list = |label: &str, items: &[String]| {
            if !items.is_empty() {
                out.push(format!("{label}: {}", items.join(", ")));
            }
        };
        list("changed", &self.changed_parts);
        list("missing", &self.missing_parts);
        list("added", &self.added_parts);
        if self.order_changed {
            out.push("part order changed".to_owned());
        }
        if !self.new_issues.is_empty() {
            out.push(format!("new integrity issues: {:?}", self.new_issues));
        }
        if self.main_kind_changed {
            out.push("main part kind changed".to_owned());
        }
        out.join("; ")
    }
}

/// Reads `bytes`, writes and re-reads them, and reports every difference.
pub fn roundtrip(bytes: &[u8]) -> Result<RoundtripReport, OpcError> {
    let limits = Limits::default();
    let before = Package::read(bytes, &limits)?;
    let after = Package::read(&before.write()?, &limits)?;

    fn names(p: &Package) -> Vec<&str> {
        p.parts().map(|x| x.name().as_str()).collect()
    }
    let (before_names, after_names) = (names(&before), names(&after));

    let after_data: HashMap<&str, &[u8]> = after
        .parts()
        .map(|p| (p.name().as_str(), p.data()))
        .collect();
    let mut changed_parts = Vec::new();
    let mut missing_parts = Vec::new();
    for part in before.parts() {
        match after_data.get(part.name().as_str()) {
            None => missing_parts.push(part.name().as_str().to_owned()),
            Some(data) if *data != part.data() => {
                changed_parts.push(part.name().as_str().to_owned());
            }
            Some(_) => {}
        }
    }
    let before_set: HashSet<&str> = before_names.iter().copied().collect();
    let added_parts: Vec<String> = after_names
        .iter()
        .filter(|n| !before_set.contains(*n))
        .map(|n| (*n).to_owned())
        .collect();

    let kind_before = before.main_part_kind();
    // A package that came back identical (names, bytes, compression, order) has the same
    // issues and the same main part, so the second integrity pass would only repeat the
    // first. Skip it; the check is the dominant cost on large packages.
    let (new_issues, main_kind_changed) = if before == after {
        (Vec::new(), false)
    } else {
        let issues_before: HashSet<IntegrityIssue> = before.check_integrity().into_iter().collect();
        let new_issues = after
            .check_integrity()
            .into_iter()
            .filter(|i| !issues_before.contains(i))
            .collect();
        (new_issues, kind_before != after.main_part_kind())
    };

    Ok(RoundtripReport {
        part_count: before_names.len(),
        changed_parts,
        order_changed: missing_parts.is_empty()
            && added_parts.is_empty()
            && before_names != after_names,
        missing_parts,
        added_parts,
        new_issues,
        main_kind: kind_before.map(|(_, kind)| kind),
        main_kind_changed,
    })
}
