//! Notes (legacy comments): `commentsN.xml`, the VML drawing Excel writes for them, and the
//! `legacyDrawing` element.

use open365_opc::{PartName, TargetMode, resolve_target};

use super::{MAIN, R_NS, XML_DECL, content_type, esc, rel, relative_target};
use crate::XlsxError;
use crate::refs::{MAX_COL, MAX_ROW, cell_ref_string, parse_cell_ref};
use crate::workbook::Workbook;
use crate::xml::{self, RawXml, XmlElement};

/// A note on a cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteSpec {
    pub row: u32,
    pub col: u32,
    pub author: String,
    /// The note text.
    pub text: String,
    /// Start the text with the author's name in bold, as New Note does.
    pub author_prefix: bool,
}

const VML_HEAD: &str = "<xml xmlns:v=\"urn:schemas-microsoft-com:vml\" xmlns:o=\"urn:schemas-microsoft-com:office:office\" xmlns:x=\"urn:schemas-microsoft-com:office:excel\"><o:shapelayout v:ext=\"edit\"><o:idmap v:ext=\"edit\" data=\"IDMAP\"/></o:shapelayout><v:shapetype id=\"_x0000_t202\" coordsize=\"21600,21600\" o:spt=\"202\" path=\"m,l,21600r21600,l21600,xe\"><v:stroke joinstyle=\"miter\"/><v:path gradientshapeok=\"t\" o:connecttype=\"rect\"/></v:shapetype>";

fn run(bold: bool, text: &str) -> String {
    let preserve = if text.starts_with(char::is_whitespace) || text.ends_with(char::is_whitespace) {
        " xml:space=\"preserve\""
    } else {
        ""
    };
    format!(
        "<r><rPr>{}<sz val=\"9\"/><color indexed=\"81\"/><rFont val=\"Tahoma\"/><family val=\"2\"/></rPr><t{preserve}>{}</t></r>",
        if bold { "<b/>" } else { "" },
        esc(text)
    )
}

fn comment_xml(spec: &NoteSpec, author_id: usize) -> String {
    let text = if spec.author_prefix {
        format!(
            "{}{}",
            run(true, &format!("{}:", spec.author)),
            run(false, &format!("\n{}", spec.text))
        )
    } else {
        run(false, &spec.text)
    };
    format!(
        "<comment xmlns=\"{MAIN}\" ref=\"{}\" authorId=\"{author_id}\" shapeId=\"0\"><text>{text}</text></comment>",
        cell_ref_string(spec.row, spec.col)
    )
}

fn shape_xml(id: u32, row: u32, col: u32) -> String {
    let (r0, c0) = (row - 1, col - 1);
    let top = r0.saturating_sub(1);
    format!(
        "<v:shape id=\"_x0000_s{id}\" type=\"#_x0000_t202\" style='position:absolute;margin-left:57pt;margin-top:1.2pt;width:100.8pt;height:60.6pt;z-index:1;visibility:hidden' fillcolor=\"infoBackground [80]\" strokecolor=\"none [81]\" o:insetmode=\"auto\"><v:fill color2=\"infoBackground [80]\"/><v:shadow color=\"none [81]\" obscured=\"t\"/><v:path o:connecttype=\"none\"/><v:textbox style='mso-direction-alt:auto'><div style='text-align:left'></div></v:textbox><x:ClientData ObjectType=\"Note\"><x:MoveWithCells/><x:SizeWithCells/><x:Anchor>{}, 12, {top}, 1, {}, 18, {}, 5</x:Anchor><x:AutoFill>False</x:AutoFill><x:Row>{r0}</x:Row><x:Column>{c0}</x:Column></x:ClientData></v:shape>",
        c0 + 1,
        c0 + 3,
        top + 4,
    )
}

/// The `(start, end)` byte range of every `v:shape` element of a VML document.
fn shape_ranges(vml: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(rel) = vml[at..].find("<v:shape ") {
        let start = at + rel;
        let Some(end_rel) = vml[start..].find("</v:shape>") else {
            break;
        };
        let end = start + end_rel + "</v:shape>".len();
        out.push((start, end));
        at = end;
    }
    out
}

fn max_shape_number(vml: &str) -> u32 {
    let mut best = 0;
    let mut at = 0;
    while let Some(rel) = vml[at..].find("_x0000_s") {
        let s = at + rel + "_x0000_s".len();
        let digits: String = vml[s..].chars().take_while(char::is_ascii_digit).collect();
        if let Ok(n) = digits.parse::<u32>() {
            best = best.max(n);
        }
        at = s;
    }
    best
}

struct NoteParts {
    comments: PartName,
    vml: Option<PartName>,
}

impl Workbook {
    fn note_parts(&self, sheet: usize) -> Option<NoteParts> {
        let sheet_part = self.sheet_part(sheet)?;
        let rels = self.relationships(Some(sheet_part));
        let find = |ty: &str| {
            rels.iter()
                .find(|r| r.rel_type == ty && r.target_mode == TargetMode::Internal)
                .and_then(|r| resolve_target(Some(sheet_part), &r.target).ok())
                .filter(|p| self.package.part(p.as_str()).is_some())
        };
        Some(NoteParts {
            comments: find(rel::COMMENTS)?,
            vml: find(rel::VML_DRAWING),
        })
    }

    /// The notes of a sheet (author, plain text; the author prefix, when present, is removed).
    pub fn notes(&self, sheet: usize) -> Vec<NoteSpec> {
        let Some(parts) = self.note_parts(sheet) else {
            return Vec::new();
        };
        let Some(doc) = self
            .part_bytes(parts.comments.as_str())
            .and_then(|b| xml::parse(b).ok())
        else {
            return Vec::new();
        };
        let authors: Vec<String> = doc
            .root
            .child_main("authors")
            .map(|a| a.elements().map(XmlElement::own_text).collect())
            .unwrap_or_default();
        let Some(list) = doc.root.child_main("commentList") else {
            return Vec::new();
        };
        list.elements()
            .filter_map(|c| {
                let (row, col) = parse_cell_ref(c.head.attr("ref")?)?;
                let author = c
                    .head
                    .attr("authorId")
                    .and_then(|v| v.trim().parse::<usize>().ok())
                    .and_then(|i| authors.get(i).cloned())
                    .unwrap_or_default();
                let mut text = String::new();
                fn collect(e: &XmlElement, out: &mut String) {
                    for ch in e.elements() {
                        match ch.head.local_name() {
                            "t" => out.push_str(&ch.own_text()),
                            "rPr" | "phoneticPr" | "rPh" => {}
                            _ => collect(ch, out),
                        }
                    }
                }
                if let Some(t) = c.child_main("text") {
                    collect(t, &mut text);
                }
                let prefix = format!("{author}:\n");
                let (text, author_prefix) = match text.strip_prefix(&prefix) {
                    Some(rest) if !author.is_empty() => (rest.to_owned(), true),
                    _ => (text, false),
                };
                Some(NoteSpec {
                    row,
                    col,
                    author,
                    text,
                    author_prefix,
                })
            })
            .collect()
    }

    /// Adds or replaces the note on a cell. The first note of a sheet creates `xl/commentsN.xml`, the
    /// VML drawing `xl/drawings/vmlDrawingN.vml` Excel needs to show it, both relationships and the
    /// `legacyDrawing` element.
    ///
    /// # Errors
    /// The cell is outside the grid, the sheet is not a worksheet, or the sheet already has a legacy
    /// drawing that is not a notes drawing.
    pub fn set_note(&mut self, sheet: usize, spec: &NoteSpec) -> Result<(), XlsxError> {
        self.require_transitional()?;
        if spec.row == 0 || spec.col == 0 || spec.row > MAX_ROW || spec.col > MAX_COL {
            return Err(XlsxError::CellOutOfRange {
                row: spec.row,
                col: spec.col,
            });
        }
        let sheet_part = self
            .sheet_part(sheet)
            .cloned()
            .ok_or(XlsxError::NotAWorksheet(sheet))?;
        if let Some(c) = spec
            .text
            .chars()
            .find(|c| matches!(c, '\u{0}'..='\u{8}' | '\u{b}' | '\u{c}' | '\u{e}'..='\u{1f}'))
        {
            return Err(XlsxError::InvalidText(c));
        }
        self.worksheet_mut(sheet)?;
        self.edited()?;
        let parts = match self.note_parts(sheet) {
            Some(p) => p,
            None => self.create_note_parts(sheet, &sheet_part)?,
        };
        // comments part
        let mut doc = xml::parse(
            self.part_bytes(parts.comments.as_str())
                .ok_or_else(|| XlsxError::MissingPart(parts.comments.to_string()))?,
        )?;
        let mut authors: Vec<String> = doc
            .root
            .child_main("authors")
            .map(|a| a.elements().map(XmlElement::own_text).collect())
            .unwrap_or_default();
        let author_id = match authors.iter().position(|a| *a == spec.author) {
            Some(i) => i,
            None => {
                authors.push(spec.author.clone());
                authors.len() - 1
            }
        };
        let authors_el = crate::sheet::parse_fragment(
            &format!(
                "<authors xmlns=\"{MAIN}\">{}</authors>",
                authors
                    .iter()
                    .map(|a| format!("<author>{}</author>", esc(a)))
                    .collect::<String>()
            ),
            &doc.root.head,
        )?;
        let reference = cell_ref_string(spec.row, spec.col);
        let new_comment =
            crate::sheet::parse_fragment(&comment_xml(spec, author_id), &doc.root.head)?;
        let mut list = doc.root.child_main("commentList").cloned().map_or_else(
            || {
                crate::sheet::parse_fragment(
                    &format!("<commentList xmlns=\"{MAIN}\"/>"),
                    &doc.root.head,
                )
            },
            Ok,
        )?;
        let key = |e: &XmlElement| {
            e.head
                .attr("ref")
                .and_then(parse_cell_ref)
                .unwrap_or((0, 0))
        };
        let existed = list
            .elements()
            .any(|c| c.head.attr("ref") == Some(reference.as_str()));
        list.children.retain(|c| {
            c.as_element()
                .is_none_or(|e| e.head.attr("ref") != Some(reference.as_str()))
        });
        let at = list
            .children
            .iter()
            .position(|c| {
                c.as_element()
                    .is_some_and(|e| key(e) > (spec.row, spec.col))
            })
            .unwrap_or(list.children.len());
        list.children.insert(at, RawXml::Element(new_comment));
        doc.root.children.retain(|c| {
            c.as_element()
                .is_none_or(|e| !(e.head.is_main("authors") || e.head.is_main("commentList")))
        });
        doc.root.children.insert(0, RawXml::Element(authors_el));
        doc.root.children.insert(1, RawXml::Element(list));
        self.put_part_bytes(parts.comments.as_str(), doc.to_bytes())?;

        // VML shape
        if !existed && let Some(vml_part) = &parts.vml {
            let mut vml =
                String::from_utf8_lossy(self.part_bytes(vml_part.as_str()).unwrap_or_default())
                    .into_owned();
            let id = max_shape_number(&vml).max(idmap_base(&vml)) + 1;
            let shape = shape_xml(id, spec.row, spec.col);
            match vml.rfind("</xml>") {
                Some(end) => vml.insert_str(end, &shape),
                None => {
                    return Err(XlsxError::Malformed {
                        part: vml_part.to_string(),
                        reason: "no closing xml element".to_owned(),
                    });
                }
            }
            self.put_part_bytes(vml_part.as_str(), vml.into_bytes())?;
        }
        self.sheets[sheet].dirty = true;
        Ok(())
    }

    fn create_note_parts(
        &mut self,
        sheet: usize,
        sheet_part: &PartName,
    ) -> Result<NoteParts, XlsxError> {
        if self.sheets[sheet]
            .data
            .as_ref()
            .is_some_and(|ws| ws.root_element("legacyDrawing").is_some())
        {
            return Err(XlsxError::UnsupportedEdit(
                "the sheet already has a legacy drawing that is not a notes drawing".to_owned(),
            ));
        }
        let vml_name = self.next_part_name("xl/drawings", "vmlDrawing", "vml");
        let n = vml_name
            .trim_start_matches("xl/drawings/vmlDrawing")
            .trim_end_matches(".vml")
            .to_owned();
        let comments_name = self.next_part_name("xl", "comments", "xml");
        let head = VML_HEAD.replace("IDMAP", &n);
        let vml = self.add_part(
            &vml_name,
            content_type::VML_DRAWING,
            format!("{head}</xml>").into_bytes(),
        )?;
        let vml_rid = self.add_relationship(
            Some(sheet_part),
            rel::VML_DRAWING,
            &relative_target(Some(sheet_part), vml.as_str()),
            TargetMode::Internal,
        )?;
        let comments = self.add_part(
            &comments_name,
            content_type::COMMENTS,
            format!("{XML_DECL}<comments xmlns=\"{MAIN}\"><authors/><commentList/></comments>")
                .into_bytes(),
        )?;
        self.add_relationship(
            Some(sheet_part),
            rel::COMMENTS,
            &relative_target(Some(sheet_part), comments.as_str()),
            TargetMode::Internal,
        )?;
        let ws = self.worksheet_mut(sheet)?;
        let el = ws.fragment(&format!(
            "<legacyDrawing xmlns=\"{MAIN}\" xmlns:r=\"{R_NS}\" r:id=\"{vml_rid}\"/>"
        ))?;
        ws.put_root_element("legacyDrawing", Some(el));
        Ok(NoteParts {
            comments,
            vml: Some(vml),
        })
    }

    /// Deletes the note on a cell; the last note of a sheet takes its comments part, VML drawing,
    /// relationships and `legacyDrawing` with it.
    ///
    /// # Errors
    /// The sheet is not a worksheet.
    pub fn delete_note(&mut self, sheet: usize, row: u32, col: u32) -> Result<bool, XlsxError> {
        let sheet_part = self
            .sheet_part(sheet)
            .cloned()
            .ok_or(XlsxError::NotAWorksheet(sheet))?;
        let Some(parts) = self.note_parts(sheet) else {
            return Ok(false);
        };
        let mut doc = xml::parse(
            self.part_bytes(parts.comments.as_str())
                .ok_or_else(|| XlsxError::MissingPart(parts.comments.to_string()))?,
        )?;
        let reference = cell_ref_string(row, col);
        let Some(list) = doc
            .root
            .children
            .iter_mut()
            .filter_map(RawXml::as_element_mut)
            .find(|e| e.head.is_main("commentList"))
        else {
            return Ok(false);
        };
        let before = list.elements().count();
        list.children.retain(|c| {
            c.as_element()
                .is_none_or(|e| e.head.attr("ref") != Some(reference.as_str()))
        });
        let left = list.elements().count();
        if left == before {
            return Ok(false);
        }
        self.edited()?;
        if let Some(vml_part) = &parts.vml {
            let mut vml =
                String::from_utf8_lossy(self.part_bytes(vml_part.as_str()).unwrap_or_default())
                    .into_owned();
            let (r0, c0) = (row - 1, col - 1);
            if let Some(&(s, e)) = shape_ranges(&vml).iter().find(|&&(s, e)| {
                let t = &vml[s..e];
                t.contains(&format!("<x:Row>{r0}</x:Row>"))
                    && t.contains(&format!("<x:Column>{c0}</x:Column>"))
            }) {
                vml.replace_range(s..e, "");
            }
            if left > 0 {
                self.put_part_bytes(vml_part.as_str(), vml.into_bytes())?;
            }
        }
        if left > 0 {
            self.put_part_bytes(parts.comments.as_str(), doc.to_bytes())?;
        } else {
            // Last note: remove the parts and their wiring.
            let rels = self.relationships(Some(&sheet_part));
            let ids: Vec<String> = rels
                .iter()
                .filter(|r| {
                    resolve_target(Some(&sheet_part), &r.target)
                        .is_ok_and(|t| t == parts.comments || parts.vml.as_ref() == Some(&t))
                })
                .map(|r| r.id.clone())
                .collect();
            for id in ids {
                self.remove_relationship(Some(&sheet_part), &id)?;
            }
            self.drop_part(parts.comments.as_str());
            if let Some(v) = &parts.vml {
                self.drop_part(v.as_str());
            }
            self.worksheet_mut(sheet)?
                .put_root_element("legacyDrawing", None);
        }
        self.sheets[sheet].dirty = true;
        Ok(true)
    }
}

fn idmap_base(vml: &str) -> u32 {
    vml.find("data=\"")
        .map(|i| &vml[i + 6..])
        .and_then(|s| s.split('"').next())
        .and_then(|d| d.split(',').next())
        .and_then(|d| d.trim().parse::<u32>().ok())
        .map_or(0, |n| n * 1024)
}
