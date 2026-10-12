//! Layout tab edits (page setup, sections, breaks, line numbers, hyphenation, paragraph indent
//! and spacing) and the part helpers the Design tab shares (`ext_design.rs`).
//!
//! Everything here edits raw XML in place (`w:sectPr`, `w:pPr`, `word/settings.xml`), keeps the
//! schema order of the children it writes and leaves every other child alone.

use open365_opc::{PartName, Relationships, TargetMode, rels_part_name, resolve_target};

use crate::DocxError;
use crate::document::Document;
use crate::model::{Block, Break, BreakKind, Inline, RunContent};
use crate::ppr::{remove_child, upsert_child};
use crate::ranges::DocPos;
use crate::section::{Orientation, SectionType};
use crate::xml::{self, Attr, ElementHead, RawXml, XmlElement};

const CONTENT_TYPES_PART: &str = "[Content_Types].xml";

/// Children of `w:sectPr` in schema order (ECMA-376 §17.6.17).
const SECT_ORDER: [&str; 21] = [
    "headerReference",
    "footerReference",
    "footnotePr",
    "endnotePr",
    "type",
    "pgSz",
    "pgMar",
    "paperSrc",
    "pgBorders",
    "lnNumType",
    "pgNumType",
    "cols",
    "formProt",
    "vAlign",
    "noEndnote",
    "titlePg",
    "textDirection",
    "bidi",
    "rtlGutter",
    "docGrid",
    "printerSettings",
];

/// Children of `w:settings` in schema order (§17.15.1), the ones this crate writes and their
/// neighbours.
const SETTINGS_ORDER: [&str; 24] = [
    "writeProtection",
    "view",
    "zoom",
    "removePersonalInformation",
    "removeDateAndTime",
    "doNotDisplayPageBoundaries",
    "displayBackgroundShape",
    "printPostScriptOverText",
    "printFractionalCharacterWidth",
    "printFormsData",
    "embedTrueTypeFonts",
    "embedSystemFonts",
    "saveSubsetFonts",
    "saveFormsData",
    "mirrorMargins",
    "alignBordersAndEdges",
    "bordersDoNotSurroundHeader",
    "bordersDoNotSurroundFooter",
    "gutterAtTop",
    "defaultTabStop",
    "autoHyphenation",
    "consecutiveHyphenLimit",
    "hyphenationZone",
    "doNotHyphenateCaps",
];

/// The rank of `local` in `order`; unknown names sort last.
fn rank_in(order: &[&str], local: &str) -> usize {
    order
        .iter()
        .position(|n| *n == local)
        .unwrap_or(order.len())
}

/// Finds or creates the child `w:local` of `parent` at its schema position and edits it.
pub(crate) fn upsert_ordered(
    parent: &mut XmlElement,
    order: &[&str],
    local: &str,
    edit: impl FnOnce(&mut XmlElement),
) {
    if let Some(RawXml::Element(e)) = parent
        .children
        .iter_mut()
        .find(|c| matches!(c, RawXml::Element(e) if e.head.is_w(local)))
    {
        edit(e);
        return;
    }
    let mut element = parent.sibling(local);
    edit(&mut element);
    let mine = rank_in(order, local);
    let at = parent
        .children
        .iter()
        .position(|c| match c {
            RawXml::Element(e) => rank_in(order, e.head.local_name()) > mine,
            _ => false,
        })
        .unwrap_or(parent.children.len());
    parent.children.insert(at, RawXml::Element(element));
}

/// Finds or creates the child `w:local` of a `w:sectPr` at its schema position.
pub(crate) fn upsert_sect(sect: &mut XmlElement, local: &str, edit: impl FnOnce(&mut XmlElement)) {
    upsert_ordered(sect, &SECT_ORDER, local, edit);
}

fn remove_all(parent: &mut XmlElement, local: &str) -> bool {
    let before = parent.children.len();
    parent
        .children
        .retain(|c| !matches!(c, RawXml::Element(e) if e.head.is_w(local)));
    parent.children.len() != before
}

fn child<'a>(parent: &'a XmlElement, local: &str) -> Option<&'a XmlElement> {
    parent.children.iter().find_map(|c| match c {
        RawXml::Element(e) if e.head.is_w(local) => Some(e),
        _ => None,
    })
}

fn num_attr(e: &XmlElement, name: &str) -> Option<i64> {
    e.head.attr_w(name).and_then(|v| v.trim().parse().ok())
}

/// Page lengths are kept to whole twips between 1 pt and 22 in (the limit Word applies).
fn check_len(twips: i64) -> Result<i64, DocxError> {
    if (20..=31680).contains(&twips) {
        Ok(twips)
    } else {
        Err(DocxError::UnsupportedRange(format!(
            "{twips} twips is outside the 1 pt to 22 in range"
        )))
    }
}

/// How `w:lnNumType` restarts (`w:restart`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineNumberRestart {
    Continuous,
    NewPage,
    NewSection,
}

/// The break `Layout > Breaks` inserts into the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextBreak {
    Page,
    Column,
    /// Text wrapping (line) break.
    TextWrapping,
}

/// One `w:col`: width and the space after it, in twips.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnSpec {
    pub width_twips: u32,
    pub space_twips: u32,
}

impl Document {
    // ---- sections ----------------------------------------------------------------------

    /// The number of the section that holds paragraph `para`.
    pub fn section_of_paragraph(&self, para: usize) -> usize {
        let sections = self.sections();
        sections
            .iter()
            .find(|s| s.end_paragraph.is_none_or(|e| e >= para))
            .map_or(sections.len().saturating_sub(1), |s| s.index)
    }

    /// The `w:sectPr` of section `section`, created (empty) for the last section when the body
    /// has none.
    pub(crate) fn section_pr_mut(&mut self, section: usize) -> Result<&mut XmlElement, DocxError> {
        let sections = self.sections();
        let Some(s) = sections.get(section) else {
            return Err(DocxError::UnsupportedRange(format!(
                "no section {section} (the document has {})",
                sections.len()
            )));
        };
        if let Some(p) = s.end_paragraph {
            let para = self.paragraph_mut(p)?;
            let props = para
                .props
                .as_mut()
                .ok_or_else(|| DocxError::UnsupportedRange("section break lost".to_owned()))?;
            let at = props
                .children
                .iter()
                .position(|c| matches!(c, RawXml::Element(e) if e.head.is_w("sectPr")))
                .ok_or_else(|| DocxError::UnsupportedRange("section break lost".to_owned()))?;
            return match &mut props.children[at] {
                RawXml::Element(e) => Ok(e),
                _ => Err(DocxError::UnsupportedRange("section break lost".to_owned())),
            };
        }
        let body = self.body_mut();
        let at = body
            .blocks
            .iter()
            .rposition(|b| matches!(b, Block::Raw(RawXml::Element(e)) if e.head.is_w("sectPr")));
        let at = match at {
            Some(at) => at,
            None => {
                let element = body.head.sibling("sectPr").into_empty();
                body.blocks.push(Block::Raw(RawXml::Element(element)));
                body.blocks.len() - 1
            }
        };
        match &mut body.blocks[at] {
            Block::Raw(RawXml::Element(e)) => Ok(e),
            _ => Err(DocxError::UnsupportedRange(
                "section properties lost".to_owned(),
            )),
        }
    }

    /// Runs `edit` on the `w:sectPr` of section `section`.
    ///
    /// # Errors
    /// `section` does not exist.
    pub fn edit_section(
        &mut self,
        section: usize,
        edit: impl FnOnce(&mut XmlElement),
    ) -> Result<(), DocxError> {
        edit(self.section_pr_mut(section)?);
        self.edited();
        Ok(())
    }

    /// Page margins of `section` in twips (`w:pgMar`). Header and footer distances and the
    /// gutter are kept (a first-time `w:pgMar` gets 720 and 0).
    ///
    /// # Errors
    /// `section` does not exist or a margin is not a sane length.
    pub fn set_page_margins(
        &mut self,
        section: usize,
        top: i32,
        bottom: i32,
        left: i32,
        right: i32,
        gutter: Option<i32>,
    ) -> Result<(), DocxError> {
        for v in [top, bottom, left, right] {
            if !(0..=31680).contains(&v) {
                return Err(DocxError::UnsupportedRange(format!("margin of {v} twips")));
            }
        }
        let sect = self.section_pr_mut(section)?;
        upsert_ordered(sect, &SECT_ORDER, "pgMar", |e| {
            for (name, v) in [
                ("top", top),
                ("right", right),
                ("bottom", bottom),
                ("left", left),
            ] {
                e.head.set_attr_w(name, &v.to_string());
            }
            for (name, v) in [("header", 720), ("footer", 720), ("gutter", 0)] {
                if e.head.attr_w(name).is_none() {
                    e.head.set_attr_w(name, &v.to_string());
                }
            }
            if let Some(g) = gutter {
                e.head.set_attr_w("gutter", &g.to_string());
            }
        });
        self.edited();
        Ok(())
    }

    /// Mirror margins (`w:mirrorMargins` in the settings): inside and outside instead of
    /// left and right.
    ///
    /// # Errors
    /// The settings part cannot be edited.
    pub fn set_mirror_margins(&mut self, on: bool) -> Result<(), DocxError> {
        self.edit_settings(|s| {
            if on {
                upsert_ordered(s, &SETTINGS_ORDER, "mirrorMargins", |_| {});
            } else {
                remove_all(s, "mirrorMargins");
            }
        })
    }

    /// Paper size of `section` in twips (`w:pgSz`), keeping the orientation: a landscape
    /// section keeps the long side across. `code` is the printer paper code, `None` for custom.
    ///
    /// # Errors
    /// `section` does not exist or a side is not a sane length.
    pub fn set_page_size(
        &mut self,
        section: usize,
        width: i64,
        height: i64,
        code: Option<u32>,
    ) -> Result<(), DocxError> {
        let (w, h) = (check_len(width)?, check_len(height)?);
        let sect = self.section_pr_mut(section)?;
        let landscape =
            child(sect, "pgSz").is_some_and(|e| e.head.attr_w("orient") == Some("landscape"));
        let (w, h) = if landscape {
            (w.max(h), w.min(h))
        } else {
            (w, h)
        };
        upsert_ordered(sect, &SECT_ORDER, "pgSz", |e| {
            e.head.set_attr_w("w", &w.to_string());
            e.head.set_attr_w("h", &h.to_string());
            match code {
                Some(c) => e.head.set_attr_w("code", &c.to_string()),
                None => e.head.remove_attr_w("code"),
            }
        });
        self.edited();
        Ok(())
    }

    /// Orientation of `section`: swaps the page sides and turns the margins a quarter turn, as
    /// Word does (portrait to landscape: top takes the old left, left the old bottom, bottom the
    /// old right, right the old top). Nothing changes when the section already has it.
    ///
    /// # Errors
    /// `section` does not exist.
    pub fn set_orientation(
        &mut self,
        section: usize,
        orientation: Orientation,
    ) -> Result<(), DocxError> {
        let current = self.sections().get(section).map(|s| s.page.orient);
        if current.is_none() {
            return Err(DocxError::UnsupportedRange(format!("no section {section}")));
        }
        let sect = self.section_pr_mut(section)?;
        let (w, h) = child(sect, "pgSz").map_or((12240, 15840), |e| {
            (
                num_attr(e, "w").unwrap_or(12240),
                num_attr(e, "h").unwrap_or(15840),
            )
        });
        let was_landscape =
            child(sect, "pgSz").is_some_and(|e| e.head.attr_w("orient") == Some("landscape"));
        let landscape = orientation == Orientation::Landscape;
        let (w, h) = if landscape {
            (w.max(h), w.min(h))
        } else {
            (w.min(h), w.max(h))
        };
        upsert_ordered(sect, &SECT_ORDER, "pgSz", |e| {
            e.head.set_attr_w("w", &w.to_string());
            e.head.set_attr_w("h", &h.to_string());
            if landscape {
                e.head.set_attr_w("orient", "landscape");
            } else {
                e.head.remove_attr_w("orient");
            }
        });
        if was_landscape != landscape {
            upsert_ordered(sect, &SECT_ORDER, "pgMar", |e| {
                let get = |e: &XmlElement, n: &str, d: i64| num_attr(e, n).unwrap_or(d);
                let (t, r, b, l) = (
                    get(e, "top", 1440),
                    get(e, "right", 1440),
                    get(e, "bottom", 1440),
                    get(e, "left", 1440),
                );
                let (t, r, b, l) = if landscape {
                    (l, t, r, b)
                } else {
                    (r, b, l, t)
                };
                for (n, v) in [("top", t), ("right", r), ("bottom", b), ("left", l)] {
                    e.head.set_attr_w(n, &v.to_string());
                }
            });
        }
        self.edited();
        Ok(())
    }

    /// Columns of `section` (`w:cols`). One column removes the element. `widths` gives unequal
    /// columns (`w:equalWidth="0"`); otherwise `count` equal columns `space` twips apart.
    ///
    /// # Errors
    /// `section` does not exist or `count` is not 1 to 45.
    pub fn set_columns(
        &mut self,
        section: usize,
        count: u32,
        space: u32,
        separator: Option<bool>,
        widths: Option<&[ColumnSpec]>,
    ) -> Result<(), DocxError> {
        if !(1..=45).contains(&count) {
            return Err(DocxError::UnsupportedRange(format!("{count} columns")));
        }
        let sect = self.section_pr_mut(section)?;
        let line = separator.unwrap_or_else(|| {
            child(sect, "cols").is_some_and(|c| c.head.attr_w("sep") == Some("1"))
        });
        if count == 1 && !line {
            remove_all(sect, "cols");
        } else {
            upsert_ordered(sect, &SECT_ORDER, "cols", |e| {
                e.children.clear();
                for stale in ["num", "space", "sep", "equalWidth"] {
                    e.head.remove_attr_w(stale);
                }
                e.head.set_attr_w("num", &count.to_string());
                if line {
                    e.head.set_attr_w("sep", "1");
                }
                match widths {
                    Some(specs) if specs.len() == count as usize => {
                        e.head.set_attr_w("equalWidth", "0");
                        for (i, spec) in specs.iter().enumerate() {
                            let mut col = e.sibling("col");
                            col.head.set_attr_w("w", &spec.width_twips.to_string());
                            if i + 1 < specs.len() {
                                col.head.set_attr_w("space", &spec.space_twips.to_string());
                            }
                            e.children.push(RawXml::Element(col));
                        }
                    }
                    _ => {
                        e.head.set_attr_w("space", &space.to_string());
                    }
                }
            });
        }
        self.edited();
        Ok(())
    }

    /// How section `section` starts (`w:type`); next page is the default and is written as an
    /// absent element.
    ///
    /// # Errors
    /// `section` does not exist.
    pub fn set_section_type(&mut self, section: usize, kind: SectionType) -> Result<(), DocxError> {
        let sect = self.section_pr_mut(section)?;
        let val = match kind {
            SectionType::NextPage => None,
            SectionType::Continuous => Some("continuous"),
            SectionType::NextColumn => Some("nextColumn"),
            SectionType::EvenPage => Some("evenPage"),
            SectionType::OddPage => Some("oddPage"),
        };
        match val {
            Some(v) => upsert_ordered(sect, &SECT_ORDER, "type", |e| e.head.set_attr_w("val", v)),
            None => {
                remove_all(sect, "type");
            }
        }
        self.edited();
        Ok(())
    }

    /// Line numbering of `section` (`w:lnNumType`); `None` removes it. `start` is the number
    /// shown on the first line (stored zero-based).
    ///
    /// # Errors
    /// `section` does not exist.
    pub fn set_line_numbering(
        &mut self,
        section: usize,
        numbering: Option<(u32, u32, Option<u32>, LineNumberRestart)>,
    ) -> Result<(), DocxError> {
        let sect = self.section_pr_mut(section)?;
        match numbering {
            None => {
                remove_all(sect, "lnNumType");
            }
            Some((count_by, start, distance, restart)) => {
                upsert_ordered(sect, &SECT_ORDER, "lnNumType", |e| {
                    e.head.attrs.clear();
                    e.head.set_attr_w("countBy", &count_by.max(1).to_string());
                    if start > 1 {
                        e.head.set_attr_w("start", &(start - 1).to_string());
                    }
                    if let Some(d) = distance {
                        e.head.set_attr_w("distance", &d.to_string());
                    }
                    e.head.set_attr_w(
                        "restart",
                        match restart {
                            LineNumberRestart::Continuous => "continuous",
                            LineNumberRestart::NewPage => "newPage",
                            LineNumberRestart::NewSection => "newSection",
                        },
                    );
                });
            }
        }
        self.edited();
        Ok(())
    }

    // ---- breaks ------------------------------------------------------------------------

    /// Inserts a break at `pos`. Page and column breaks end the paragraph: the text after `pos`
    /// moves to a new paragraph (Word 2013 and later, which is where the caret goes); a text
    /// wrapping break stays in the paragraph.
    ///
    /// # Errors
    /// `pos` is outside the document.
    pub fn insert_break(&mut self, pos: DocPos, kind: TextBreak) -> Result<(), DocxError> {
        self.check_pos(pos)?;
        if kind == TextBreak::TextWrapping {
            return self.insert_text(pos.para, pos.offset, "\n");
        }
        self.split_paragraph(pos)?;
        let para = self.paragraph_mut(pos.para)?;
        let mut run = para.new_run();
        let owner = run.head.clone();
        run.content.push(RunContent::Break(Break::new(
            if kind == TextBreak::Page {
                BreakKind::Page
            } else {
                BreakKind::Column
            },
            &owner,
        )));
        para.content.push(Inline::Run(run));
        self.edited();
        Ok(())
    }

    /// Inserts a section break at `pos`: the text before it ends the earlier section (a copy of
    /// the section `pos` is in) and the text after starts the new one, which begins as `kind`.
    /// Returns the number of the new section.
    ///
    /// # Errors
    /// `pos` is outside the document.
    pub fn insert_section_break(
        &mut self,
        pos: DocPos,
        kind: SectionType,
    ) -> Result<usize, DocxError> {
        self.check_pos(pos)?;
        let current = self.section_of_paragraph(pos.para);
        let mut copy = self.section_pr_mut(current)?.clone();
        remove_all(&mut copy, "type");
        // The earlier part keeps how the old section began; the old element carries on after.
        let began = child(self.section_pr_mut(current)?, "type").cloned();
        if let Some(t) = began {
            upsert_ordered(&mut copy, &SECT_ORDER, "type", |e| *e = t);
        }
        self.split_paragraph(pos)?;
        {
            let para = self.paragraph_mut(pos.para)?;
            let props = para.props_mut();
            remove_child(props, "sectPr");
            props.children.push(RawXml::Element(copy));
        }
        self.set_section_type(current + 1, kind)?;
        self.edited();
        Ok(current + 1)
    }

    // ---- paragraphs --------------------------------------------------------------------

    /// Runs `edit` on the `w:pPr` of paragraph `para` (created when missing).
    ///
    /// # Errors
    /// `para` is out of range.
    pub fn edit_paragraph_props(
        &mut self,
        para: usize,
        edit: impl FnOnce(&mut XmlElement),
    ) -> Result<(), DocxError> {
        edit(self.paragraph_mut(para)?.props_mut());
        self.edited();
        Ok(())
    }

    /// Left and right indent of paragraph `para` in twips (`w:ind`); `None` leaves a side as it
    /// is. The first-line and hanging indent are kept.
    ///
    /// # Errors
    /// `para` is out of range.
    pub fn set_para_indent(
        &mut self,
        para: usize,
        left: Option<i32>,
        right: Option<i32>,
    ) -> Result<(), DocxError> {
        self.edit_paragraph_props(para, |ppr| {
            upsert_child(ppr, "ind", |e| {
                if let Some(v) = left {
                    for stale in ["start", "leftChars", "startChars"] {
                        e.head.remove_attr_w(stale);
                    }
                    e.head.set_attr_w("left", &v.to_string());
                }
                if let Some(v) = right {
                    for stale in ["end", "rightChars", "endChars"] {
                        e.head.remove_attr_w(stale);
                    }
                    e.head.set_attr_w("right", &v.to_string());
                }
            });
        })
    }

    /// Space before and after paragraph `para` in twips (`w:spacing`); `None` leaves a side as
    /// it is. Automatic spacing and line-based spacing on that side are dropped.
    ///
    /// # Errors
    /// `para` is out of range.
    pub fn set_para_spacing(
        &mut self,
        para: usize,
        before: Option<i32>,
        after: Option<i32>,
    ) -> Result<(), DocxError> {
        self.edit_paragraph_props(para, |ppr| {
            upsert_child(ppr, "spacing", |e| {
                if let Some(v) = before {
                    for stale in ["beforeLines", "beforeAutospacing"] {
                        e.head.remove_attr_w(stale);
                    }
                    e.head.set_attr_w("before", &v.max(0).to_string());
                }
                if let Some(v) = after {
                    for stale in ["afterLines", "afterAutospacing"] {
                        e.head.remove_attr_w(stale);
                    }
                    e.head.set_attr_w("after", &v.max(0).to_string());
                }
            });
        })
    }

    /// Suppress Line Numbers for paragraph `para` (`w:suppressLineNumbers`).
    ///
    /// # Errors
    /// `para` is out of range.
    pub fn set_suppress_line_numbers(&mut self, para: usize, on: bool) -> Result<(), DocxError> {
        self.edit_paragraph_props(para, |ppr| {
            if on {
                upsert_child(ppr, "suppressLineNumbers", |_| {});
            } else {
                remove_child(ppr, "suppressLineNumbers");
            }
        })
    }

    // ---- settings ----------------------------------------------------------------------

    /// Automatic hyphenation (`w:autoHyphenation`) and its options: the zone in twips
    /// (`w:hyphenationZone`, Word's default 360), the most hyphens in a row
    /// (`w:consecutiveHyphenLimit`, 0 is no limit) and whether capitalised words are
    /// hyphenated.
    ///
    /// # Errors
    /// The settings part cannot be edited.
    pub fn set_hyphenation(
        &mut self,
        auto: bool,
        zone: Option<u32>,
        limit: Option<u32>,
        hyphenate_caps: Option<bool>,
    ) -> Result<(), DocxError> {
        self.edit_settings(|s| {
            if auto {
                upsert_ordered(s, &SETTINGS_ORDER, "autoHyphenation", |_| {});
            } else {
                remove_all(s, "autoHyphenation");
            }
            if let Some(z) = zone {
                upsert_ordered(s, &SETTINGS_ORDER, "hyphenationZone", |e| {
                    e.head.set_attr_w("val", &z.to_string());
                });
            }
            if let Some(l) = limit {
                if l == 0 {
                    remove_all(s, "consecutiveHyphenLimit");
                } else {
                    upsert_ordered(s, &SETTINGS_ORDER, "consecutiveHyphenLimit", |e| {
                        e.head.set_attr_w("val", &l.to_string());
                    });
                }
            }
            if let Some(caps) = hyphenate_caps {
                if caps {
                    remove_all(s, "doNotHyphenateCaps");
                } else {
                    upsert_ordered(s, &SETTINGS_ORDER, "doNotHyphenateCaps", |_| {});
                }
            }
        })
    }

    /// Runs `edit` on `w:settings`, creating the settings part when the package has none, and
    /// re-reads the parts the model caches.
    ///
    /// # Errors
    /// The part cannot be parsed or created.
    pub fn edit_settings(&mut self, edit: impl FnOnce(&mut XmlElement)) -> Result<(), DocxError> {
        let (name, _) = self.ensure_part(
            REL_SETTINGS,
            "word/settings.xml",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml",
            W_ROOT_SETTINGS,
        )?;
        self.edit_part_xml(&name, edit)
    }

    // ---- part helpers shared with the Design tab ---------------------------------------

    /// Parses part `name`, runs `edit` on its root and writes it back.
    pub(crate) fn edit_part_xml(
        &mut self,
        name: &PartName,
        edit: impl FnOnce(&mut XmlElement),
    ) -> Result<(), DocxError> {
        let bytes = self
            .package()
            .part(name.as_str())
            .map(|p| p.data().to_vec())
            .ok_or_else(|| DocxError::Xml(format!("part {name} is missing")))?;
        let mut parsed = xml::parse(&bytes)?;
        edit(&mut parsed.root);
        self.ext_package_mut()
            .set_part(name.clone(), parsed.to_bytes());
        self.ext_reload_parts();
        Ok(())
    }

    /// The part the main part relates to with `rel_type`, created from `default` at
    /// `new_name` (with its relationship and content-type override) when there is none.
    /// Returns the part name and the relationship id.
    pub(crate) fn ensure_part(
        &mut self,
        rel_type: &str,
        new_name: &str,
        content_type: &str,
        default: &str,
    ) -> Result<(PartName, String), DocxError> {
        let main = self.main_part().clone();
        let rels_name = rels_part_name(Some(&main));
        let existing = self
            .package()
            .part(rels_name.as_str())
            .and_then(|p| Relationships::parse(p.data()).ok())
            .and_then(|rels| {
                rels.iter()
                    .find(|r| r.target_mode == TargetMode::Internal && r.rel_type == rel_type)
                    .map(|r| (r.target.clone(), r.id.clone()))
            });
        if let Some((target, id)) = existing {
            let name = resolve_target(Some(&main), &target)?;
            if self.package().part(name.as_str()).is_some() {
                return Ok((name, id));
            }
        }
        let name = PartName::new(new_name)?;
        let file = new_name.strip_prefix("word/").unwrap_or(new_name);
        let package = self.ext_package_mut();
        package.set_part(name.clone(), default.as_bytes().to_vec());
        let id = add_relationship(package, &main, rel_type, file)?;
        add_override(package, new_name, content_type)?;
        Ok((name, id))
    }

    /// Creates a new part of a kind that may occur many times (a header), numbered after the
    /// existing ones. Returns the part name and the relationship id.
    pub(crate) fn add_numbered_part(
        &mut self,
        rel_type: &str,
        stem: &str,
        content_type: &str,
        bytes: Vec<u8>,
    ) -> Result<(PartName, String), DocxError> {
        let main = self.main_part().clone();
        let mut n = 1;
        let new_name = loop {
            let candidate = format!("word/{stem}{n}.xml");
            if self.package().part(&candidate).is_none() {
                break candidate;
            }
            n += 1;
        };
        let name = PartName::new(&new_name)?;
        let file = new_name
            .strip_prefix("word/")
            .unwrap_or(&new_name)
            .to_owned();
        let package = self.ext_package_mut();
        package.set_part(name.clone(), bytes);
        let id = add_relationship(package, &main, rel_type, &file)?;
        add_override(package, &new_name, content_type)?;
        Ok((name, id))
    }
}

pub(crate) const REL_SETTINGS: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings";
const W_ROOT_SETTINGS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"/>"#;

fn attr(name: &str, value: String) -> Attr {
    Attr {
        name: name.to_owned(),
        ns: None,
        value,
    }
}

/// Adds a relationship from `main` to `target`; returns its id.
fn add_relationship(
    package: &mut open365_opc::Package,
    main: &PartName,
    rel_type: &str,
    target: &str,
) -> Result<String, DocxError> {
    let name = rels_part_name(Some(main));
    let mut parsed = match package.part(name.as_str()) {
        Some(p) => xml::parse(p.data())?,
        None => xml::parse(
            br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>"#,
        )?,
    };
    let used: std::collections::HashSet<String> = parsed
        .root
        .children
        .iter()
        .filter_map(RawXml::as_element)
        .flat_map(|e| e.head.attrs.iter())
        .filter(|a| a.name == "Id")
        .map(|a| a.value.clone())
        .collect();
    let mut n = 1;
    while used.contains(&format!("rId{n}")) {
        n += 1;
    }
    let id = format!("rId{n}");
    let head = ElementHead {
        name: "Relationship".to_owned(),
        ns: parsed.root.head.ns.clone(),
        attrs: vec![
            attr("Id", id.clone()),
            attr("Type", rel_type.to_owned()),
            attr("Target", target.to_owned()),
        ],
    };
    parsed.root.children.push(RawXml::Element(XmlElement {
        head,
        children: Vec::new(),
    }));
    package.set_part(name, parsed.to_bytes());
    Ok(id)
}

fn add_override(
    package: &mut open365_opc::Package,
    part: &str,
    content_type: &str,
) -> Result<(), DocxError> {
    let Some(types) = package.part(CONTENT_TYPES_PART) else {
        return Err(DocxError::Xml(
            "package has no [Content_Types].xml".to_owned(),
        ));
    };
    let mut parsed = xml::parse(types.data())?;
    let head = ElementHead {
        name: "Override".to_owned(),
        ns: parsed.root.head.ns.clone(),
        attrs: vec![
            attr("PartName", format!("/{part}")),
            attr("ContentType", content_type.to_owned()),
        ],
    };
    parsed.root.children.push(RawXml::Element(XmlElement {
        head,
        children: Vec::new(),
    }));
    package.set_part(PartName::new(CONTENT_TYPES_PART)?, parsed.to_bytes());
    Ok(())
}
