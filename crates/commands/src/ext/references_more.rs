//! Writer References tab, citations and cross-references: a `CITATION` field with its in-text
//! result, a `BIBLIOGRAPHY` field over the formatted entries, and a `REF` cross-reference to a
//! bookmark that this operation puts around the target. Each is one undo step.

use open365_docx::{DocPos, Document, DocxError, escape_xml};
use serde::{Deserialize, Serialize};

use crate::CommandError;
use crate::command::EffectClass;

/// One piece of a bibliography entry (`italic` for titles).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntrySeg {
    pub text: String,
    pub italic: bool,
}

/// One edit of this group.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MoreOp {
    /// A `CITATION` field at `at` showing `result`.
    InsertCitation {
        at: DocPos,
        instruction: String,
        result: String,
    },
    /// A heading (optional) and one paragraph per entry, inside one `BIBLIOGRAPHY` field.
    InsertBibliography {
        at: DocPos,
        heading: Option<String>,
        entries: Vec<Vec<EntrySeg>>,
    },
    /// A `REF` field at `at` to the bookmark `bookmark`, which is placed around
    /// `target_start..target_end` (it moves there when it exists). `result` is the text shown.
    CrossReference {
        at: DocPos,
        target_start: DocPos,
        target_end: DocPos,
        bookmark: String,
        /// `REF` or `PAGEREF`.
        field: String,
        switches: String,
        result: String,
    },
}

impl MoreOp {
    #[must_use]
    pub fn effect(&self) -> EffectClass {
        EffectClass::Edit
    }
}

fn run(text: &str, italic: bool) -> String {
    format!(
        "<w:r>{}<w:t xml:space=\"preserve\">{}</w:t></w:r>",
        if italic { "<w:rPr><w:i/></w:rPr>" } else { "" },
        escape_xml(text)
    )
}

fn marker(kind: &str) -> String {
    format!("<w:r><w:fldChar w:fldCharType=\"{kind}\"/></w:r>")
}

fn instr(instruction: &str) -> String {
    format!(
        "<w:r><w:instrText xml:space=\"preserve\"> {} </w:instrText></w:r>",
        escape_xml(instruction)
    )
}

fn field(instruction: &str, result: &str) -> String {
    format!(
        "{}{}{}{result}{}",
        marker("begin"),
        instr(instruction),
        marker("separate"),
        marker("end")
    )
}

fn valid_name(name: &str) -> bool {
    name.chars()
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && name.chars().count() <= 40
}

fn insertion_index(doc: &mut Document, at: DocPos) -> Result<usize, DocxError> {
    let len =
        doc.paragraph(at.para)
            .map(|p| p.char_len())
            .ok_or(DocxError::ParagraphOutOfRange {
                index: at.para,
                count: doc.paragraph_count(),
            })?;
    if at.offset == 0 {
        Ok(at.para)
    } else if at.offset >= len {
        Ok(at.para + 1)
    } else {
        doc.split_paragraph(at)?;
        Ok(at.para + 1)
    }
}

fn bad(message: &str) -> CommandError {
    CommandError::Docx(DocxError::UnsupportedRange(message.to_owned()))
}

/// Runs `op` on `doc`.
pub fn apply(doc: &mut Document, op: &MoreOp) -> Result<(), CommandError> {
    match op {
        MoreOp::InsertCitation {
            at,
            instruction,
            result,
        } => {
            doc.insert_inline_xml(*at, &field(instruction, &run(result, false)))?;
            Ok(())
        }
        MoreOp::InsertBibliography {
            at,
            heading,
            entries,
        } => {
            if entries.is_empty() {
                return Err(bad("add a source first"));
            }
            let idx = insertion_index(doc, *at)?;
            let mut xml = String::new();
            if let Some(h) = heading {
                xml.push_str(&format!(
                    "<w:p><w:pPr><w:pStyle w:val=\"Heading1\"/></w:pPr>{}</w:p>",
                    run(h, false)
                ));
            }
            let last = entries.len() - 1;
            for (i, e) in entries.iter().enumerate() {
                let body: String = e.iter().map(|s| run(&s.text, s.italic)).collect();
                let head = if i == 0 {
                    format!(
                        "{}{}{}",
                        marker("begin"),
                        instr("BIBLIOGRAPHY \\l 1033"),
                        marker("separate")
                    )
                } else {
                    String::new()
                };
                let tail = if i == last {
                    marker("end")
                } else {
                    String::new()
                };
                xml.push_str(&format!(
                    "<w:p><w:pPr><w:pStyle w:val=\"Bibliography\"/></w:pPr>{head}{body}{tail}</w:p>"
                ));
            }
            doc.insert_blocks_xml(idx, &xml)?;
            Ok(())
        }
        MoreOp::CrossReference {
            at,
            target_start,
            target_end,
            bookmark,
            field: name,
            switches,
            result,
        } => {
            if !valid_name(bookmark) {
                return Err(bad("the bookmark name is not valid"));
            }
            if target_end < target_start {
                return Err(bad("the cross-reference target is reversed"));
            }
            // A hidden bookmark (`_Ref...`) around the target, once per name. The id comes from
            // the digits of the name, so one name always has one id.
            let exists = String::from_utf8_lossy(&doc.serialize_main())
                .contains(&format!("w:name=\"{bookmark}\""));
            if !exists {
                let id: u64 = bookmark
                    .chars()
                    .filter(char::is_ascii_digit)
                    .collect::<String>()
                    .parse()
                    .unwrap_or(900_000_000);
                // The end goes in first so the start position stays valid.
                doc.insert_inline_xml(*target_end, &format!("<w:bookmarkEnd w:id=\"{id}\"/>"))?;
                doc.insert_inline_xml(
                    *target_start,
                    &format!("<w:bookmarkStart w:id=\"{id}\" w:name=\"{bookmark}\"/>"),
                )?;
            }
            // The bookmark markers have no width, so `at` is still where it was.
            let instruction = format!("{name} {bookmark} {switches}")
                .trim_end()
                .to_owned();
            doc.insert_inline_xml(*at, &field(&instruction, &run(result, false)))?;
            Ok(())
        }
    }
}
