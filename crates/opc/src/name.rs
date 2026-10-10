use std::fmt;

use crate::OpcError;

/// A validated zip entry name such as `word/document.xml` or `[Content_Types].xml`.
///
/// Names are stored as written in the package (no leading `/`). Validation rejects
/// anything that could escape a directory or alias another entry: absolute paths,
/// backslashes, empty, `.` or `..` segments, and control characters.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PartName(String);

impl PartName {
    pub fn new(name: &str) -> Result<Self, OpcError> {
        let invalid = |reason: &'static str| OpcError::InvalidPartName {
            name: name.to_owned(),
            reason,
        };
        if name.is_empty() {
            return Err(invalid("empty"));
        }
        if name.starts_with('/') {
            return Err(invalid("absolute path"));
        }
        if name.contains('\\') {
            return Err(invalid("backslash"));
        }
        if name.chars().any(char::is_control) {
            return Err(invalid("control character"));
        }
        if name
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
        {
            return Err(invalid("empty, '.' or '..' segment"));
        }
        Ok(Self(name.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// OPC part names compare case-insensitively (ASCII), so two entries that differ
    /// only in case are the same part.
    pub(crate) fn key(&self) -> String {
        self.0.to_ascii_lowercase()
    }
}

impl fmt::Display for PartName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::PartName;

    #[test]
    fn accepts_ooxml_names() {
        for name in [
            "[Content_Types].xml",
            "_rels/.rels",
            "word/document.xml",
            "xl/worksheets/sheet1.xml",
        ] {
            assert_eq!(PartName::new(name).unwrap().as_str(), name);
        }
    }

    #[test]
    fn rejects_escaping_or_ambiguous_names() {
        for name in [
            "",
            "/word/document.xml",
            "../evil.xml",
            "word/../../evil.xml",
            "word\\document.xml",
            "word//document.xml",
            "word/./document.xml",
            "word/",
            "word/doc\0.xml",
        ] {
            assert!(PartName::new(name).is_err(), "accepted {name:?}");
        }
    }
}
