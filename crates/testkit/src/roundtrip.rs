//! Read -> write -> read invariant checker.

use std::collections::HashMap;

use open365_opc::{IntegrityIssue, Limits, OpcError, Package};

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
}

impl RoundtripReport {
    pub fn is_lossless(&self) -> bool {
        self.changed_parts.is_empty()
            && !self.order_changed
            && self.missing_parts.is_empty()
            && self.added_parts.is_empty()
            && self.new_issues.is_empty()
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
        out.join("; ")
    }
}

/// Reads `bytes`, writes and re-reads them, and reports every difference.
pub fn roundtrip(bytes: &[u8]) -> Result<RoundtripReport, OpcError> {
    let limits = Limits::default();
    let before = Package::read(bytes, &limits)?;
    let after = Package::read(&before.write()?, &limits)?;

    let names =
        |p: &Package| -> Vec<String> { p.parts().map(|x| x.name().as_str().to_owned()).collect() };
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
    let added_parts: Vec<String> = after_names
        .iter()
        .filter(|n| !before_names.contains(n))
        .cloned()
        .collect();

    let issues_before = before.check_integrity();
    let new_issues = after
        .check_integrity()
        .into_iter()
        .filter(|i| !issues_before.contains(i))
        .collect();

    Ok(RoundtripReport {
        part_count: before_names.len(),
        changed_parts,
        order_changed: missing_parts.is_empty()
            && added_parts.is_empty()
            && before_names != after_names,
        missing_parts,
        added_parts,
        new_issues,
    })
}
