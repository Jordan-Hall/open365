//! Comments outside the main story: headers, footers, footnotes, endnotes and text boxes.
//!
//! ECMA-376 lets `w:commentRangeStart`, `w:commentRangeEnd` and `w:commentReference` appear in
//! any story. Word 16 refuses to create them there (`Comments.Add` raises "can only be added
//! to the main story", see `cmt-stories` in `tools/office/make-revision-refs.ps1`) and drops
//! them when it opens such a file (`fixtures/reference/docx/comments-stories`), so these
//! comments exist for round trips and for other consumers of the format. They are read and
//! edited in the parts' XML directly.

use std::collections::BTreeMap;

use open365_opc::{PartName, Relationships, TargetMode, rels_part_name, resolve_target};

use super::{MC, NewComment, PartXml, comment_id, is_el};
use crate::DocxError;
use crate::document::Document;
use crate::model::{Inline, RunContent};
use crate::ranges::DocPos;
use crate::xml::{self, ElementHead, RawXml, XmlElement};

const REL_HEADER: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/header";
const REL_FOOTER: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer";
const REL_FOOTNOTES: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes";
const REL_ENDNOTES: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/endnotes";

/// The story a comment's markers live in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CommentStory {
    /// The main story (`word/document.xml`, outside text boxes).
    #[default]
    Main,
    /// A text box (`w:txbxContent`) anchored in the main story.
    TextBox,
    Header,
    Footer,
    Footnote,
    Endnote,
}

/// Where a comment sits outside the main story.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoryAnchor {
    pub story: CommentStory,
    /// The part holding the markers (the main part for a text box).
    pub part: String,
    /// Index of the paragraph the range starts in, counting `w:p` in document order within
    /// the part (within its text boxes for [`CommentStory::TextBox`]).
    pub para: usize,
}

/// A position in a story other than the main one: paragraph index and character offset (final
/// view, `w:t` characters, tabs and breaks).
pub type StoryPos = DocPos;

#[derive(Debug, Default, Clone)]
pub(super) struct StoryMark {
    pub anchor: Option<StoryAnchor>,
    pub segments: Vec<String>,
    opened: bool,
}

impl StoryMark {
    pub fn scope_text(&self) -> String {
        self.segments.join("\n")
    }
}

fn local_is(e: &XmlElement, ns: &str, local: &str) -> bool {
    is_el(e, ns, local)
}

fn is_fallback(e: &XmlElement) -> bool {
    local_is(e, MC, "Fallback")
}

struct Scan<'a> {
    story: CommentStory,
    part: &'a str,
    /// Only paragraphs inside `w:txbxContent` count.
    text_boxes_only: bool,
    paras: usize,
    current: usize,
    open: Vec<u32>,
    marks: &'a mut BTreeMap<u32, StoryMark>,
}

impl Scan<'_> {
    fn anchor(&self) -> StoryAnchor {
        StoryAnchor {
            story: self.story,
            part: self.part.to_owned(),
            para: self.current,
        }
    }

    fn push_text(&mut self, text: &str) {
        for id in &self.open {
            if let Some(seg) = self.marks.get_mut(id).and_then(|m| m.segments.last_mut()) {
                seg.push_str(text);
            }
        }
    }

    fn walk(&mut self, el: &XmlElement, in_box: bool) {
        for child in &el.children {
            let RawXml::Element(c) = child else {
                if let RawXml::Text(t) = child
                    && (el.head.is_w("t") || el.head.is_w("delText"))
                    && (!self.text_boxes_only || in_box)
                {
                    self.push_text(t);
                }
                continue;
            };
            if is_fallback(c) {
                continue;
            }
            let active = !self.text_boxes_only || in_box;
            if c.head.is_w("txbxContent") {
                self.walk(c, true);
                continue;
            }
            if active && c.head.is_w("p") {
                self.current = self.paras;
                self.paras += 1;
                for id in self.open.clone() {
                    if let Some(m) = self.marks.get_mut(&id) {
                        m.segments.push(String::new());
                    }
                }
            } else if active && c.head.is_w("commentRangeStart") {
                if let Some(id) = comment_id(c) {
                    let anchor = self.anchor();
                    let m = self.marks.entry(id).or_default();
                    if !m.opened {
                        m.opened = true;
                        m.anchor = Some(anchor);
                        m.segments = vec![String::new()];
                        self.open.push(id);
                    }
                }
            } else if active && c.head.is_w("commentRangeEnd") {
                if let Some(id) = comment_id(c) {
                    self.open.retain(|o| *o != id);
                }
            } else if active && c.head.is_w("commentReference") {
                if let Some(id) = comment_id(c) {
                    let anchor = self.anchor();
                    let m = self.marks.entry(id).or_default();
                    if m.anchor.is_none() {
                        m.anchor = Some(anchor);
                    }
                }
            } else if active && c.head.is_w("tab") {
                self.push_text("\t");
            } else if active && (c.head.is_w("br") || c.head.is_w("cr")) {
                self.push_text("\n");
            }
            self.walk(c, in_box);
        }
    }
}

fn scan_root(
    root: &XmlElement,
    story: CommentStory,
    part: &str,
    text_boxes_only: bool,
    marks: &mut BTreeMap<u32, StoryMark>,
) {
    let mut scan = Scan {
        story,
        part,
        text_boxes_only,
        paras: 0,
        current: 0,
        open: Vec::new(),
        marks,
    };
    scan.walk(root, false);
}

/// Characters a run contributes in the final view.
fn run_len(r: &XmlElement) -> usize {
    r.children
        .iter()
        .filter_map(RawXml::as_element)
        .map(|c| {
            if c.head.is_w("t") {
                c.children
                    .iter()
                    .map(|t| match t {
                        RawXml::Text(s) => s.chars().count(),
                        _ => 0,
                    })
                    .sum()
            } else if c.head.is_w("tab") || c.head.is_w("br") || c.head.is_w("cr") {
                1
            } else {
                0
            }
        })
        .sum()
}

fn text_len(e: &XmlElement) -> usize {
    if e.head.is_w("r") {
        return run_len(e);
    }
    if e.head.is_w("del") || e.head.is_w("moveFrom") || e.head.is_w("pPr") || is_fallback(e) {
        return 0;
    }
    e.children
        .iter()
        .filter_map(RawXml::as_element)
        .map(text_len)
        .sum()
}

fn set_preserve(t: &mut XmlElement) {
    if !t.head.attrs.iter().any(xml::Attr::is_xml_space) {
        t.head.attrs.push(xml::Attr {
            name: "xml:space".to_owned(),
            ns: Some("http://www.w3.org/XML/1998/namespace".into()),
            value: "preserve".to_owned(),
        });
    }
}

/// Splits run `r` after `at` characters (0 < at < len); both halves keep the run properties.
fn split_run(r: &XmlElement, at: usize) -> Option<(XmlElement, XmlElement)> {
    let mut left = XmlElement {
        head: r.head.clone(),
        children: Vec::new(),
    };
    let mut right = left.clone();
    let mut seen = 0;
    for child in &r.children {
        let RawXml::Element(c) = child else {
            continue;
        };
        if c.head.is_w("rPr") {
            left.children.push(child.clone());
            right.children.push(child.clone());
            continue;
        }
        let len = run_len(&XmlElement {
            head: r.head.clone(),
            children: vec![child.clone()],
        });
        if seen + len <= at {
            left.children.push(child.clone());
        } else if seen >= at {
            right.children.push(child.clone());
        } else if c.head.is_w("t") {
            let text: String = c
                .children
                .iter()
                .filter_map(|t| match t {
                    RawXml::Text(s) => Some(s.as_str()),
                    _ => None,
                })
                .collect();
            let k = at - seen;
            let (a, b): (String, String) = (
                text.chars().take(k).collect(),
                text.chars().skip(k).collect(),
            );
            let mut l = c.clone();
            l.children = vec![RawXml::Text(a)];
            set_preserve(&mut l);
            let mut rr = c.clone();
            rr.children = vec![RawXml::Text(b)];
            set_preserve(&mut rr);
            left.children.push(RawXml::Element(l));
            right.children.push(RawXml::Element(rr));
        } else {
            return None;
        }
        seen += len;
    }
    Some((left, right))
}

/// Inserts `items` at character offset `target` of `children`; `acc` is the offset of the
/// first child. Returns true when placed.
fn insert_at(
    children: &mut Vec<RawXml>,
    target: usize,
    acc: &mut usize,
    items: &mut Option<Vec<RawXml>>,
) -> bool {
    let mut i = 0;
    while i < children.len() {
        let RawXml::Element(e) = &children[i] else {
            i += 1;
            continue;
        };
        if e.head.is_w("pPr") {
            i += 1;
            continue;
        }
        let len = text_len(e);
        if *acc == target {
            if let Some(items) = items.take() {
                children.splice(i..i, items);
            }
            return true;
        }
        if target > *acc && target < *acc + len {
            if e.head.is_w("r") {
                if let Some((l, r)) = split_run(e, target - *acc)
                    && let Some(items) = items.take()
                {
                    let mut new = vec![RawXml::Element(l)];
                    new.extend(items);
                    new.push(RawXml::Element(r));
                    children.splice(i..=i, new);
                    return true;
                }
                return false;
            }
            if let RawXml::Element(e) = &mut children[i] {
                return insert_at(&mut e.children, target, acc, items);
            }
        }
        *acc += len;
        i += 1;
    }
    if *acc == target
        && let Some(items) = items.take()
    {
        children.extend(items);
        return true;
    }
    false
}

fn insert_after_where(
    children: &mut Vec<RawXml>,
    find: &dyn Fn(&XmlElement) -> bool,
    items: &mut Option<Vec<RawXml>>,
) -> bool {
    if let Some(at) = children
        .iter()
        .position(|c| matches!(c, RawXml::Element(e) if find(e)))
    {
        if let Some(items) = items.take() {
            children.splice(at + 1..at + 1, items);
        }
        return true;
    }
    for c in children.iter_mut() {
        if let RawXml::Element(e) = c
            && insert_after_where(&mut e.children, find, items)
        {
            return true;
        }
    }
    false
}

fn has_reference(e: &XmlElement, id: u32) -> bool {
    e.head.is_w("r")
        && e.children.iter().any(|c| {
            matches!(c, RawXml::Element(r)
                if r.head.is_w("commentReference") && comment_id(r) == Some(id))
        })
}

fn remove_markers(el: &mut XmlElement, doomed: &std::collections::HashSet<u32>) -> bool {
    let before = el.children.len();
    el.children.retain(|c| match c {
        RawXml::Element(e) => {
            !(((e.head.is_w("commentRangeStart") || e.head.is_w("commentRangeEnd"))
                && comment_id(e).is_some_and(|i| doomed.contains(&i)))
                || doomed.iter().any(|id| has_reference(e, *id)))
        }
        _ => true,
    });
    let mut changed = el.children.len() != before;
    for c in &mut el.children {
        if let RawXml::Element(e) = c {
            changed |= remove_markers(e, doomed);
        }
    }
    changed
}

/// The `w:p` number `*n` counting down (document order), or none. With `boxes_only` only
/// paragraphs inside `w:txbxContent` count.
fn nth_paragraph<'a>(
    el: &'a mut XmlElement,
    n: &mut usize,
    boxes_only: bool,
    in_box: bool,
) -> Option<&'a mut XmlElement> {
    for c in &mut el.children {
        let RawXml::Element(e) = c else { continue };
        if is_fallback(e) {
            continue;
        }
        let box_here = in_box || e.head.is_w("txbxContent");
        if (!boxes_only || in_box) && e.head.is_w("p") {
            if *n == 0 {
                return Some(e);
            }
            *n -= 1;
        }
        if let Some(found) = nth_paragraph(e, n, boxes_only, box_here) {
            return Some(found);
        }
    }
    None
}

fn visit_raw_mut(content: &mut [Inline], f: &mut dyn FnMut(&mut XmlElement) -> bool) -> bool {
    for inline in content {
        match inline {
            Inline::Run(r) => {
                for item in &mut r.content {
                    if let RunContent::Raw(RawXml::Element(e)) = item
                        && f(e)
                    {
                        return true;
                    }
                }
            }
            Inline::Group(g) => {
                if visit_raw_mut(&mut g.content, f) {
                    return true;
                }
            }
            Inline::Raw(RawXml::Element(e)) => {
                if f(e) {
                    return true;
                }
            }
            Inline::Raw(_) => {}
        }
    }
    false
}

impl Document {
    /// Header, footer, footnote and endnote parts of the main part, in relationship order.
    fn story_parts(&self) -> Vec<(CommentStory, String)> {
        let main = self.main_part();
        let Some(rels) = self
            .package()
            .part(rels_part_name(Some(main)).as_str())
            .and_then(|p| Relationships::parse(p.data()).ok())
        else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for rel in rels.iter() {
            if rel.target_mode != TargetMode::Internal {
                continue;
            }
            let story = match rel.rel_type.as_str() {
                REL_HEADER => CommentStory::Header,
                REL_FOOTER => CommentStory::Footer,
                REL_FOOTNOTES => CommentStory::Footnote,
                REL_ENDNOTES => CommentStory::Endnote,
                _ => continue,
            };
            if let Ok(name) = resolve_target(Some(main), &rel.target) {
                out.push((story, name.as_str().to_owned()));
            }
        }
        out
    }

    fn story_root(&self, part: &str) -> Option<XmlElement> {
        if let Some(p) = self.comments.stories.get(part) {
            return Some(p.xml.root.clone());
        }
        let bytes = self.package().part(part)?.data().to_vec();
        xml::parse(&bytes).ok().map(|p| p.root)
    }

    /// Markers of comments found outside the main story, by comment id.
    pub(super) fn story_marks(&self) -> BTreeMap<u32, StoryMark> {
        let mut marks = BTreeMap::new();
        for (story, part) in self.story_parts() {
            if let Some(root) = self.story_root(&part) {
                scan_root(&root, story, &part, false, &mut marks);
            }
        }
        if let Ok(parsed) = xml::parse(&self.serialize_main()) {
            let main = self.main_part().as_str().to_owned();
            scan_root(&parsed.root, CommentStory::TextBox, &main, true, &mut marks);
        }
        marks
    }

    fn story_part_mut(&mut self, part: &str) -> Result<&mut PartXml, DocxError> {
        if !self.comments.stories.contains_key(part) {
            let bytes = self
                .package()
                .part(part)
                .ok_or_else(|| DocxError::CommentsUnavailable(format!("no part {part}")))?
                .data()
                .to_vec();
            let xml = xml::parse(&bytes)?;
            self.comments.stories.insert(
                part.to_owned(),
                PartXml {
                    name: PartName::new(part)?,
                    xml,
                    dirty: false,
                    created: false,
                },
            );
        }
        self.comments
            .stories
            .get_mut(part)
            .ok_or_else(|| DocxError::CommentsUnavailable(format!("no part {part}")))
    }

    /// Runs `f` over each root element a story's markers can be in: the part root, or every
    /// raw element of the main story for text boxes. Stops when `f` returns true.
    fn with_story_roots(
        &mut self,
        story: CommentStory,
        part: &str,
        f: &mut dyn FnMut(&mut XmlElement) -> bool,
    ) -> Result<bool, DocxError> {
        if story == CommentStory::TextBox {
            for i in 0..self.paragraph_count() {
                let para = self.paragraph_mut(i)?;
                if visit_raw_mut(&mut para.content, f) {
                    return Ok(true);
                }
            }
            return Ok(false);
        }
        let p = self.story_part_mut(part)?;
        let hit = f(&mut p.xml.root);
        if hit {
            p.dirty = true;
        }
        Ok(hit)
    }

    fn resolve_story_part(
        &self,
        story: CommentStory,
        part: Option<&str>,
    ) -> Result<String, DocxError> {
        if story == CommentStory::TextBox {
            return Ok(self.main_part().as_str().to_owned());
        }
        let parts = self.story_parts();
        match part {
            Some(p) => parts
                .iter()
                .find(|(s, n)| {
                    *s == story && n.trim_start_matches('/') == p.trim_start_matches('/')
                })
                .map(|(_, n)| n.clone()),
            None => parts
                .iter()
                .find(|(s, _)| *s == story)
                .map(|(_, n)| n.clone()),
        }
        .ok_or_else(|| DocxError::CommentsUnavailable(format!("no {story:?} part")))
    }

    /// Adds `comment` to a story other than the main one. `part` names the header, footer,
    /// footnotes or endnotes part (`None`: the first of that kind); it is ignored for text
    /// boxes. `start` and `end` give paragraph indices within that part (within its text boxes
    /// for a text box) and character offsets. Word does not read comments placed here.
    pub fn add_story_comment(
        &mut self,
        story: CommentStory,
        part: Option<&str>,
        start: StoryPos,
        end: StoryPos,
        comment: &NewComment,
    ) -> Result<u32, DocxError> {
        if story == CommentStory::Main {
            return self.add_comment(start, end, comment);
        }
        let part = self.resolve_story_part(story, part)?;
        let (s, e) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        let id = self.append_comment_body(comment, None)?;
        let has_style = self
            .styles()
            .is_some_and(|st| st.has_style("CommentReference"));
        let boxes = story == CommentStory::TextBox;
        let place =
            |root: &mut XmlElement, pos: StoryPos, mk: &dyn Fn(&ElementHead) -> Vec<RawXml>| {
                let mut n = pos.para;
                let Some(p) = nth_paragraph(root, &mut n, boxes, false) else {
                    return false;
                };
                let items = mk(&p.head);
                insert_at(&mut p.children, pos.offset, &mut 0, &mut Some(items))
            };
        let end_items = |head: &ElementHead| {
            vec![
                RawXml::Element(marker(head, "commentRangeEnd", id)),
                RawXml::Element(reference_run(head, id, has_style)),
            ]
        };
        let start_items =
            |head: &ElementHead| vec![RawXml::Element(marker(head, "commentRangeStart", id))];
        let mut placed_end = false;
        let mut placed_start = false;
        // End first: with a collapsed range the start marker then lands before it.
        self.with_story_roots(story, &part, &mut |root| {
            if !placed_end {
                placed_end = place(root, e, &end_items);
            }
            if placed_end && !placed_start {
                placed_start = place(root, s, &start_items);
            }
            placed_end && placed_start
        })?;
        if !(placed_end && placed_start) {
            // Undo: leave no half-placed range or orphan body.
            let doomed = std::iter::once(id).collect();
            self.with_story_roots(story, &part, &mut |root| remove_markers(root, &doomed))?;
            self.remove_comment_body(id);
            return Err(DocxError::CommentsUnavailable(
                "the range is outside the story".to_owned(),
            ));
        }
        self.edited();
        Ok(id)
    }

    /// Adds a reply to a comment whose markers are outside the main story.
    pub(super) fn reply_in_story(
        &mut self,
        parent: u32,
        anchor: &StoryAnchor,
        comment: &NewComment,
    ) -> Result<u32, DocxError> {
        let parent_para = self.ensure_para_id(parent)?;
        let id = self.append_comment_body(comment, Some(&parent_para))?;
        let has_style = self
            .styles()
            .is_some_and(|st| st.has_style("CommentReference"));
        let mut done = (false, false, false);
        self.with_story_roots(anchor.story, &anchor.part, &mut |root| {
            let head = root.head.clone();
            if !done.0 {
                let mut items = Some(vec![
                    RawXml::Element(marker(&head, "commentRangeEnd", id)),
                    RawXml::Element(reference_run(&head, id, has_style)),
                ]);
                done.0 = insert_after_where(
                    &mut root.children,
                    &|e| has_reference(e, parent),
                    &mut items,
                );
                if !done.0 {
                    let mut items = items.take().or_else(|| {
                        Some(vec![
                            RawXml::Element(marker(&head, "commentRangeEnd", id)),
                            RawXml::Element(reference_run(&head, id, has_style)),
                        ])
                    });
                    done.0 = insert_after_where(
                        &mut root.children,
                        &|e| e.head.is_w("commentRangeEnd") && comment_id(e) == Some(parent),
                        &mut items,
                    );
                }
            }
            if !done.1 {
                let mut items = Some(vec![RawXml::Element(marker(
                    &head,
                    "commentRangeStart",
                    id,
                ))]);
                done.1 = insert_after_where(
                    &mut root.children,
                    &|e| e.head.is_w("commentRangeStart") && comment_id(e) == Some(parent),
                    &mut items,
                );
            }
            done.2 = done.0 && done.1;
            done.2
        })?;
        if !done.2 {
            let doomed = std::iter::once(id).collect();
            self.with_story_roots(anchor.story, &anchor.part, &mut |root| {
                remove_markers(root, &doomed)
            })?;
            self.remove_comment_body(id);
            return Err(DocxError::CommentsUnavailable(
                "the comment's markers are missing".to_owned(),
            ));
        }
        self.edited();
        Ok(id)
    }

    /// Removes the markers of `doomed` comments from every story outside the main one.
    pub(super) fn remove_story_markers(
        &mut self,
        doomed: &std::collections::HashSet<u32>,
        marks: &BTreeMap<u32, StoryMark>,
    ) -> Result<(), DocxError> {
        let mut targets: Vec<(CommentStory, String)> = marks
            .iter()
            .filter(|(id, _)| doomed.contains(id))
            .filter_map(|(_, m)| m.anchor.as_ref().map(|a| (a.story, a.part.clone())))
            .collect();
        targets.sort_by(|a, b| (a.1.as_str(), a.0 as u8).cmp(&(b.1.as_str(), b.0 as u8)));
        targets.dedup();
        for (story, part) in targets {
            self.with_story_roots(story, &part, &mut |root| {
                remove_markers(root, doomed);
                false
            })?;
            if story != CommentStory::TextBox
                && let Some(p) = self.comments.stories.get_mut(&part)
            {
                p.dirty = true;
            }
        }
        Ok(())
    }
}

fn marker(owner: &ElementHead, local: &str, id: u32) -> XmlElement {
    let mut head = owner.sibling(local);
    head.ns = owner.ns.clone();
    head.set_attr_w("id", &id.to_string());
    XmlElement {
        head,
        children: Vec::new(),
    }
}

fn reference_run(owner: &ElementHead, id: u32, styled: bool) -> XmlElement {
    let mut run = XmlElement {
        head: owner.sibling("r"),
        children: Vec::new(),
    };
    if styled {
        let mut style = owner.sibling("rStyle");
        style.set_attr_w("val", "CommentReference");
        run.children.push(RawXml::Element(XmlElement {
            head: owner.sibling("rPr"),
            children: vec![RawXml::Element(XmlElement {
                head: style,
                children: Vec::new(),
            })],
        }));
    }
    run.children
        .push(RawXml::Element(marker(owner, "commentReference", id)));
    run
}
