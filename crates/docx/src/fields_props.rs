//! Core, extended and custom document properties for `AUTHOR`, `TITLE`, `DOCPROPERTY` and the
//! date fields.

use open365_opc::{Package, Relationships, TargetMode, resolve_target};

use crate::fields_date::CivilDateTime;
use crate::xml::{self, RawXml, XmlElement};

const CORE_TYPES: [&str; 1] =
    ["http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties"];
const APP_TYPES: [&str; 1] =
    ["http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties"];
const CUSTOM_TYPES: [&str; 1] =
    ["http://schemas.openxmlformats.org/officeDocument/2006/relationships/custom-properties"];

/// A custom property value (`docProps/custom.xml`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CustomValue {
    Text(String),
    Int(i64),
    Real(f64),
    Bool(bool),
    /// UTC instant.
    Date(CivilDateTime),
}

#[derive(Debug, Clone, Default)]
pub(crate) struct DocProps {
    core: Vec<(String, String)>,
    app: Vec<(String, String)>,
    custom: Vec<(String, CustomValue)>,
}

impl DocProps {
    pub(crate) fn load(package: &Package) -> Self {
        let mut props = Self::default();
        let Some(rels) = package
            .part("_rels/.rels")
            .and_then(|p| Relationships::parse(p.data()).ok())
        else {
            return props;
        };
        let root_of = |types: &[&str]| -> Option<XmlElement> {
            let rel = rels.iter().find(|r| {
                r.target_mode == TargetMode::Internal && types.contains(&r.rel_type.as_str())
            })?;
            let name = resolve_target(None, &rel.target).ok()?;
            let part = package.part(name.as_str())?;
            Some(xml::parse(part.data()).ok()?.root)
        };
        if let Some(root) = root_of(&CORE_TYPES) {
            props.core = text_children(&root);
        }
        if let Some(root) = root_of(&APP_TYPES) {
            props.app = text_children(&root);
        }
        if let Some(root) = root_of(&CUSTOM_TYPES) {
            for child in root.children.iter().filter_map(RawXml::as_element) {
                if child.head.local_name() != "property" {
                    continue;
                }
                let name = child
                    .head
                    .attrs
                    .iter()
                    .find(|a| a.name == "name")
                    .map(|a| a.value.clone());
                let value = child.children.iter().filter_map(RawXml::as_element).next();
                if let (Some(name), Some(value)) = (name, value) {
                    let text = element_text(value);
                    let parsed = match value.head.local_name() {
                        "lpwstr" | "lpstr" | "bstr" => Some(CustomValue::Text(text)),
                        "i1" | "i2" | "i4" | "i8" | "int" | "ui1" | "ui2" | "ui4" | "ui8"
                        | "uint" => text.trim().parse().ok().map(CustomValue::Int),
                        "r4" | "r8" | "decimal" => text.trim().parse().ok().map(CustomValue::Real),
                        "bool" => Some(CustomValue::Bool(matches!(text.trim(), "true" | "1"))),
                        "filetime" | "date" => {
                            CivilDateTime::parse_utc(&text).map(CustomValue::Date)
                        }
                        _ => Some(CustomValue::Text(text)),
                    };
                    if let Some(v) = parsed {
                        props.custom.push((name, v));
                    }
                }
            }
        }
        props
    }

    fn core(&self, name: &str) -> Option<&str> {
        self.core
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    fn app(&self, name: &str) -> Option<&str> {
        self.app
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    /// `dc:title`.
    pub(crate) fn title(&self) -> Option<&str> {
        self.core("title")
    }

    pub(crate) fn subject(&self) -> Option<&str> {
        self.core("subject")
    }

    pub(crate) fn author(&self) -> Option<&str> {
        self.core("creator")
    }

    pub(crate) fn keywords(&self) -> Option<&str> {
        self.core("keywords")
    }

    pub(crate) fn comments(&self) -> Option<&str> {
        self.core("description")
    }

    pub(crate) fn last_saved_by(&self) -> Option<&str> {
        self.core("lastModifiedBy")
    }

    /// UTC instant of `dcterms:created`.
    pub(crate) fn created(&self) -> Option<CivilDateTime> {
        self.core("created").and_then(CivilDateTime::parse_utc)
    }

    pub(crate) fn modified(&self) -> Option<CivilDateTime> {
        self.core("modified").and_then(CivilDateTime::parse_utc)
    }

    pub(crate) fn last_printed(&self) -> Option<CivilDateTime> {
        self.core("lastPrinted").and_then(CivilDateTime::parse_utc)
    }

    /// True when `name` is a custom property (not a built-in one).
    pub(crate) fn is_custom(&self, name: &str) -> Option<bool> {
        Some(
            self.custom
                .iter()
                .any(|(n, _)| n.eq_ignore_ascii_case(name.trim())),
        )
    }

    /// A built-in or custom property by the name `DOCPROPERTY` uses.
    pub(crate) fn lookup(&self, name: &str) -> Option<PropValue> {
        let key = name.trim();
        let lower = key.to_ascii_lowercase();
        let text = |v: Option<&str>| v.map(|s| PropValue::Text(s.to_owned()));
        let number = |v: Option<&str>| -> Option<PropValue> {
            v.and_then(|s| s.trim().parse::<f64>().ok())
                .map(PropValue::Number)
        };
        let builtin = match lower.as_str() {
            "title" => text(self.title()),
            "subject" => text(self.subject()),
            "author" => text(self.author()),
            "keywords" => text(self.keywords()),
            "comments" => text(self.comments()),
            "lastsavedby" => text(self.last_saved_by()),
            "revisionnumber" => text(self.core("revision")),
            "category" => text(self.core("category")),
            "createtime" | "creationdate" => self.created().map(PropValue::Date),
            "lastsavedtime" | "savetime" => self.modified().map(PropValue::Date),
            "lastprinted" | "lastprintdate" => self.last_printed().map(PropValue::Date),
            "template" => text(self.app("Template")),
            "manager" => text(self.app("Manager")),
            "company" => text(self.app("Company")),
            "format" => text(self.app("PresentationFormat")),
            "hyperlinkbase" => text(self.app("HyperlinkBase")),
            "totaleditingtime" => number(self.app("TotalTime")),
            "numberofpages" | "numpages" => number(self.app("Pages")),
            "numberofwords" | "numwords" => number(self.app("Words")),
            "numberofcharacters" | "numchars" => number(self.app("Characters")),
            "numberofcharacterswithspaces" => number(self.app("CharactersWithSpaces")),
            "numberoflines" => number(self.app("Lines")),
            "numberofparagraphs" => number(self.app("Paragraphs")),
            "security" => number(self.app("DocSecurity")),
            _ => None,
        };
        if builtin.is_some() {
            return builtin;
        }
        self.custom
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(key))
            .map(|(_, v)| match v {
                CustomValue::Text(t) => PropValue::Text(t.clone()),
                CustomValue::Int(i) =>
                {
                    #[allow(clippy::cast_precision_loss)]
                    PropValue::Number(*i as f64)
                }
                CustomValue::Real(r) => PropValue::Number(*r),
                CustomValue::Bool(b) => PropValue::Text(if *b { "Y" } else { "N" }.to_owned()),
                CustomValue::Date(d) => PropValue::Date(*d),
            })
    }
}

/// A property as a field sees it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PropValue {
    Text(String),
    Number(f64),
    /// UTC instant.
    Date(CivilDateTime),
}

fn element_text(e: &XmlElement) -> String {
    e.children
        .iter()
        .filter_map(|c| match c {
            RawXml::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect()
}

fn text_children(root: &XmlElement) -> Vec<(String, String)> {
    root.children
        .iter()
        .filter_map(RawXml::as_element)
        .map(|e| (e.head.local_name().to_owned(), element_text(e)))
        .collect()
}
