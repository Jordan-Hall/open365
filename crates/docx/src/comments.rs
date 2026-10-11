//! Comments (ECMA-376 Part 1 §17.13.4 and the Word 2012+ extensions).
//!
//! * `word/comments.xml`: the comment bodies (`w:comment`).
//! * `word/commentsExtended.xml` (`w15:commentEx`): replies (`paraIdParent`) and resolved
//!   state (`done`), keyed by the `w14:paraId` of a comment's last paragraph.
//! * `word/commentsIds.xml` and `word/commentsExtensible.xml`: durable ids; kept, and given an
//!   entry for each comment added here.
//! * the main story: `w:commentRangeStart`, `w:commentRangeEnd` and a run holding
//!   `w:commentReference`.
//!
//! The parts are parsed when the document opens and written back only when a comment call
//! changed them; untouched they stay byte-identical.

use std::collections::{BTreeMap, HashSet};

mod stories;
pub use stories::{CommentStory, StoryAnchor, StoryPos};

use open365_opc::{Package, PartName, Relationships, TargetMode, rels_part_name, resolve_target};

use crate::DocxError;
use crate::document::Document;
use crate::edit::split_runs_at;
use crate::model::{Inline, Paragraph, Run, RunContent};
use crate::props::RunProps;
use crate::ranges::DocPos;
use crate::review::run_markup_text;
use crate::xml::{self, Attr, ElementHead, ParsedXml, RawXml, W_NS, XmlElement};

const REL_COMMENTS: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments";
const REL_EXTENDED: &str =
    "http://schemas.microsoft.com/office/2011/relationships/commentsExtended";
const REL_IDS: &str = "http://schemas.microsoft.com/office/2016/09/relationships/commentsIds";
const REL_EXTENSIBLE: &str =
    "http://schemas.microsoft.com/office/2018/08/relationships/commentsExtensible";
const CT_COMMENTS: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml";
const REL_PEOPLE: &str = "http://schemas.microsoft.com/office/2011/relationships/people";
const CT_PEOPLE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.people+xml";
const CT_EXTENDED: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.commentsExtended+xml";
const W14: &str = "http://schemas.microsoft.com/office/word/2010/wordml";
const W15: &str = "http://schemas.microsoft.com/office/word/2012/wordml";
const W16CID: &str = "http://schemas.microsoft.com/office/word/2016/wordml/cid";
const W16CEX: &str = "http://schemas.microsoft.com/office/word/2018/wordml/cex";
const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
const CONTENT_TYPES_PART: &str = "[Content_Types].xml";

/// Where a comment is anchored: the final-view text between its range markers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommentAnchor {
    pub start: DocPos,
    pub end: DocPos,
}

/// A comment author listed in `word/people.xml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    pub author: String,
    /// `w15:presenceInfo/@w15:providerId` (`None`, `AD`, `Windows Live`...).
    pub provider_id: Option<String>,
    /// `w15:presenceInfo/@w15:userId`.
    pub user_id: Option<String>,
}

/// One comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    /// `w:id`.
    pub id: u32,
    pub author: String,
    pub initials: String,
    pub date: Option<String>,
    /// The body; paragraphs separated by `\n`.
    pub text: String,
    /// The comment this one replies to.
    pub parent: Option<u32>,
    /// `w15:done`: the thread is resolved.
    pub resolved: bool,
    /// The range markers in the main story; `None` when the comment is not anchored there.
    pub anchor: Option<CommentAnchor>,
    /// Text of the anchor in the markup view, paragraphs separated by `\n`. Deleted text is
    /// included, as in Word's `Comment.Scope.Text`.
    pub scope_text: String,
    /// `w14:paraId` of the body's last paragraph, the key of the extension parts.
    pub para_id: Option<String>,
    /// The story the range markers are in. Outside the main story the anchor is `None` and
    /// `story_anchor` says where; see [`Document::add_story_comment`].
    pub story: CommentStory,
    pub story_anchor: Option<StoryAnchor>,
}

/// A comment to add.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewComment {
    pub author: String,
    pub initials: String,
    pub text: String,
    /// `w:date`; `None` stamps the current UTC time.
    pub date: Option<String>,
}

impl NewComment {
    /// A comment by `author`; the initials are the first letters of the name's words.
    pub fn new(author: &str, text: &str) -> Self {
        let initials = author
            .split_whitespace()
            .filter_map(|w| w.chars().next())
            .collect();
        Self {
            author: author.to_owned(),
            initials,
            text: text.to_owned(),
            date: None,
        }
    }

    pub fn with_date(mut self, date: &str) -> Self {
        self.date = Some(date.to_owned());
        self
    }
}

#[derive(Debug)]
struct PartXml {
    name: PartName,
    xml: ParsedXml,
    dirty: bool,
    /// Not in the package yet: needs a relationship and a content type on flush.
    created: bool,
}

/// The parsed comment parts of a document.
#[derive(Debug, Default)]
pub(crate) struct CommentParts {
    comments: Option<PartXml>,
    extended: Option<PartXml>,
    ids: Option<PartXml>,
    extensible: Option<PartXml>,
    /// `word/people.xml` (`w15:person`): the authors of comments.
    people: Option<PartXml>,
    /// Header, footer, note and similar parts opened for editing, by part name.
    stories: BTreeMap<String, PartXml>,
    /// A comment part exists but cannot be parsed: comment edits are refused.
    broken: Option<String>,
}

impl CommentParts {
    pub(crate) fn load(package: &Package, main: &PartName) -> Self {
        let mut parts = Self::default();
        let Some(rels) = package
            .part(rels_part_name(Some(main)).as_str())
            .and_then(|p| Relationships::parse(p.data()).ok())
        else {
            return parts;
        };
        let load = |rel_type: &str, broken: &mut Option<String>| -> Option<PartXml> {
            let rel = rels
                .iter()
                .find(|r| r.target_mode == TargetMode::Internal && r.rel_type == rel_type)?;
            let name = resolve_target(Some(main), &rel.target).ok()?;
            let part = package.part(name.as_str())?;
            match xml::parse(part.data()) {
                Ok(xml) => Some(PartXml {
                    name,
                    xml,
                    dirty: false,
                    created: false,
                }),
                Err(e) => {
                    *broken = Some(format!("{name}: {e}"));
                    None
                }
            }
        };
        let mut broken = None;
        parts.comments = load(REL_COMMENTS, &mut broken);
        parts.extended = load(REL_EXTENDED, &mut broken);
        parts.ids = load(REL_IDS, &mut broken);
        parts.extensible = load(REL_EXTENSIBLE, &mut broken);
        parts.people = load(REL_PEOPLE, &mut broken);
        parts.broken = broken;
        parts
    }

    pub(crate) fn is_dirty(&self) -> bool {
        [
            &self.comments,
            &self.extended,
            &self.ids,
            &self.extensible,
            &self.people,
        ]
        .into_iter()
        .flatten()
        .any(|p| p.dirty)
    }

    /// Writes the changed parts into `package`, registering new ones.
    pub(crate) fn flush(
        &mut self,
        package: &mut Package,
        main: &PartName,
    ) -> Result<(), DocxError> {
        for (slot, rel_type, content_type) in [
            (&mut self.comments, REL_COMMENTS, Some(CT_COMMENTS)),
            (&mut self.extended, REL_EXTENDED, Some(CT_EXTENDED)),
            (&mut self.ids, REL_IDS, None),
            (&mut self.extensible, REL_EXTENSIBLE, None),
            (&mut self.people, REL_PEOPLE, Some(CT_PEOPLE)),
        ] {
            let Some(part) = slot else { continue };
            if !part.dirty {
                continue;
            }
            if part.created {
                let file = part.name.as_str().rsplit('/').next().unwrap_or_default();
                add_relationship(package, main, rel_type, file)?;
                if let Some(ct) = content_type {
                    add_override(package, part.name.as_str(), ct)?;
                }
                part.created = false;
            }
            package.set_part(part.name.clone(), part.xml.to_bytes());
            part.dirty = false;
        }
        for part in self.stories.values_mut() {
            if part.dirty {
                package.set_part(part.name.clone(), part.xml.to_bytes());
                part.dirty = false;
            }
        }
        Ok(())
    }
}

fn add_relationship(
    package: &mut Package,
    main: &PartName,
    rel_type: &str,
    target: &str,
) -> Result<(), DocxError> {
    let name = rels_part_name(Some(main));
    let mut parsed = match package.part(name.as_str()) {
        Some(p) => xml::parse(p.data())?,
        None => xml::parse(
            br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>"#,
        )?,
    };
    let mut n = 1;
    let used: HashSet<String> = parsed
        .root
        .children
        .iter()
        .filter_map(RawXml::as_element)
        .flat_map(|e| e.head.attrs.iter())
        .filter(|a| a.name == "Id")
        .map(|a| a.value.clone())
        .collect();
    while used.contains(&format!("rId{n}")) {
        n += 1;
    }
    let attr = |name: &str, value: String| Attr {
        name: name.to_owned(),
        ns: None,
        value,
    };
    let head = ElementHead {
        name: "Relationship".to_owned(),
        ns: parsed.root.head.ns.clone(),
        attrs: vec![
            attr("Id", format!("rId{n}")),
            attr("Type", rel_type.to_owned()),
            attr("Target", target.to_owned()),
        ],
    };
    parsed.root.children.push(RawXml::Element(XmlElement {
        head,
        children: Vec::new(),
    }));
    package.set_part(name, parsed.to_bytes());
    Ok(())
}

fn add_override(package: &mut Package, part: &str, content_type: &str) -> Result<(), DocxError> {
    let Some(types) = package.part(CONTENT_TYPES_PART) else {
        return Err(DocxError::Xml(
            "package has no [Content_Types].xml".to_owned(),
        ));
    };
    let mut parsed = xml::parse(types.data())?;
    let attr = |name: &str, value: String| Attr {
        name: name.to_owned(),
        ns: None,
        value,
    };
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
    let name = PartName::new(CONTENT_TYPES_PART)?;
    package.set_part(name, parsed.to_bytes());
    Ok(())
}

// ---------------------------------------------------------------------------
// XML helpers
// ---------------------------------------------------------------------------

fn attr_ns<'a>(head: &'a ElementHead, ns: &str, local: &str) -> Option<&'a str> {
    head.attrs
        .iter()
        .find(|a| a.ns.as_deref() == Some(ns) && a.local_name() == local)
        .map(|a| a.value.as_str())
}

fn set_attr_ns(head: &mut ElementHead, ns: &str, prefix: &str, local: &str, value: &str) {
    if let Some(a) = head
        .attrs
        .iter_mut()
        .find(|a| a.ns.as_deref() == Some(ns) && a.local_name() == local)
    {
        value.clone_into(&mut a.value);
        return;
    }
    head.attrs.push(Attr {
        name: format!("{prefix}:{local}"),
        ns: Some(ns.into()),
        value: value.to_owned(),
    });
}

fn is_el(e: &XmlElement, ns: &str, local: &str) -> bool {
    e.head.ns.as_deref() == Some(ns) && e.head.local_name() == local
}

/// The prefix the root binds to `ns`; declares `preferred` on the root when there is none.
fn ensure_prefix(root: &mut XmlElement, ns: &str, preferred: &str) -> String {
    let existing = root.head.attrs.iter().find_map(|a| {
        (a.value == ns)
            .then(|| a.name.strip_prefix("xmlns:"))
            .flatten()
            .map(str::to_owned)
    });
    if let Some(p) = existing {
        return p;
    }
    root.head.attrs.push(Attr {
        name: format!("xmlns:{preferred}"),
        ns: None,
        value: ns.to_owned(),
    });
    preferred.to_owned()
}

/// Makes the root declare `prefix` for `ns` and list it in `mc:Ignorable`.
fn ensure_ignorable(root: &mut XmlElement, ns: &str, preferred: &str) {
    let prefix = ensure_prefix(root, ns, preferred);
    let mc = ensure_prefix(root, MC, "mc");
    let mc_name = format!("{mc}:Ignorable");
    match root.head.attrs.iter_mut().find(|a| a.name == mc_name) {
        Some(a) => {
            if !a.value.split_whitespace().any(|p| p == prefix) {
                a.value.push(' ');
                a.value.push_str(&prefix);
            }
        }
        None => root.head.attrs.push(Attr {
            name: mc_name,
            ns: Some(MC.into()),
            value: prefix,
        }),
    }
}

fn text_of(e: &XmlElement, out: &mut String) {
    for c in &e.children {
        match c {
            RawXml::Text(t) if e.head.is_w("t") => out.push_str(t),
            RawXml::Element(c) if c.head.is_w("tab") => out.push('\t'),
            RawXml::Element(c) if c.head.is_w("br") => out.push('\n'),
            RawXml::Element(c) => text_of(c, out),
            _ => {}
        }
    }
}

fn comment_paragraphs(comment: &XmlElement) -> Vec<&XmlElement> {
    comment
        .children
        .iter()
        .filter_map(RawXml::as_element)
        .filter(|e| e.head.is_w("p"))
        .collect()
}

fn comment_id(e: &XmlElement) -> Option<u32> {
    e.head.attr_w("id")?.parse().ok()
}

fn last_para_id(comment: &XmlElement) -> Option<String> {
    comment_paragraphs(comment)
        .last()
        .and_then(|p| attr_ns(&p.head, W14, "paraId"))
        .map(str::to_owned)
}

fn hex8(v: u32) -> String {
    format!("{v:08X}")
}

// ---------------------------------------------------------------------------
// Anchors in the main story
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone, Copy)]
struct Marks {
    start: Option<DocPos>,
    end: Option<DocPos>,
    reference: Option<DocPos>,
}

fn scan_inlines(content: &[Inline], para: usize, acc: &mut usize, out: &mut BTreeMap<u32, Marks>) {
    for inline in content {
        match inline {
            Inline::Run(r) => {
                for item in &r.content {
                    if let RunContent::Raw(RawXml::Element(e)) = item
                        && e.head.is_w("commentReference")
                        && let Some(id) = comment_id(e)
                    {
                        out.entry(id).or_default().reference = Some(DocPos::new(para, *acc));
                    }
                }
                *acc += r.char_len();
            }
            Inline::Group(g) => scan_inlines(&g.content, para, acc, out),
            Inline::Raw(RawXml::Element(e)) => {
                let id = comment_id(e);
                if let Some(id) = id {
                    if e.head.is_w("commentRangeStart") {
                        out.entry(id).or_default().start = Some(DocPos::new(para, *acc));
                    } else if e.head.is_w("commentRangeEnd") {
                        out.entry(id).or_default().end = Some(DocPos::new(para, *acc));
                    }
                }
            }
            Inline::Raw(_) => {}
        }
    }
}

impl Document {
    fn marks(&self) -> BTreeMap<u32, Marks> {
        let mut out = BTreeMap::new();
        for (i, p) in self.body().paragraphs().into_iter().enumerate() {
            scan_inlines(&p.content, i, &mut 0, &mut out);
        }
        out
    }

    /// Markup-view text between the range markers of comment `id` in the main story.
    fn scope_text(&self, id: u32) -> String {
        fn walk(
            content: &[Inline],
            id: u32,
            inside: &mut bool,
            ended: &mut bool,
            touched: &mut bool,
            out: &mut String,
        ) {
            for inline in content {
                if *ended {
                    return;
                }
                match inline {
                    Inline::Run(r) if *inside => run_markup_text(r, out),
                    Inline::Run(_) => {}
                    Inline::Group(g) => walk(&g.content, id, inside, ended, touched, out),
                    Inline::Raw(RawXml::Element(e)) if comment_id(e) == Some(id) => {
                        if e.head.is_w("commentRangeStart") {
                            *inside = true;
                            *touched = true;
                        } else if e.head.is_w("commentRangeEnd") && *inside {
                            *inside = false;
                            *ended = true;
                        }
                    }
                    Inline::Raw(_) => {}
                }
            }
        }
        let mut parts = Vec::new();
        let (mut inside, mut ended) = (false, false);
        for p in self.body().paragraphs() {
            let mut touched = inside;
            let mut seg = String::new();
            walk(
                &p.content,
                id,
                &mut inside,
                &mut ended,
                &mut touched,
                &mut seg,
            );
            if touched {
                parts.push(seg);
            }
            if ended {
                break;
            }
        }
        parts.join("\n")
    }

    /// The comment authors listed in `word/people.xml`.
    pub fn people(&self) -> Vec<Person> {
        let Some(part) = &self.comments.people else {
            return Vec::new();
        };
        part.xml
            .root
            .children
            .iter()
            .filter_map(RawXml::as_element)
            .filter(|e| is_el(e, W15, "person"))
            .map(|e| {
                let presence = e
                    .children
                    .iter()
                    .filter_map(RawXml::as_element)
                    .find(|c| is_el(c, W15, "presenceInfo"));
                let get = |k: &str| {
                    presence
                        .and_then(|p| attr_ns(&p.head, W15, k))
                        .map(str::to_owned)
                };
                Person {
                    author: attr_ns(&e.head, W15, "author")
                        .unwrap_or_default()
                        .to_owned(),
                    provider_id: get("providerId"),
                    user_id: get("userId"),
                }
            })
            .collect()
    }

    /// Every comment, in the order of `word/comments.xml`.
    pub fn comments(&self) -> Vec<Comment> {
        let Some(part) = &self.comments.comments else {
            return Vec::new();
        };
        let marks = self.marks();
        let story_marks = self.story_marks();
        let ext = self.comments.extended.as_ref();
        let ext_of = |para_id: &str| -> Option<&XmlElement> {
            ext?.xml
                .root
                .children
                .iter()
                .filter_map(RawXml::as_element)
                .find(|e| {
                    is_el(e, W15, "commentEx") && attr_ns(&e.head, W15, "paraId") == Some(para_id)
                })
        };
        let all: Vec<&XmlElement> = part
            .xml
            .root
            .children
            .iter()
            .filter_map(RawXml::as_element)
            .filter(|e| e.head.is_w("comment"))
            .collect();
        let id_of_para = |para_id: &str| -> Option<u32> {
            all.iter()
                .find(|c| last_para_id(c).as_deref() == Some(para_id))
                .and_then(|c| comment_id(c))
        };
        all.iter()
            .filter_map(|c| {
                let id = comment_id(c)?;
                let para_id = last_para_id(c);
                let ex = para_id.as_deref().and_then(ext_of);
                let parent = ex
                    .and_then(|e| attr_ns(&e.head, W15, "paraIdParent"))
                    .and_then(id_of_para);
                let resolved = ex
                    .and_then(|e| attr_ns(&e.head, W15, "done"))
                    .is_some_and(|v| matches!(v, "1" | "true"));
                let anchor = marks.get(&id).and_then(|m| {
                    let start = m.start.or(m.reference)?;
                    let end = m.end.or(m.reference).unwrap_or(start);
                    Some(CommentAnchor { start, end })
                });
                // Word reports a reply's scope as its parent's.
                let scope_id = parent
                    .filter(|p| marks.contains_key(p) || story_marks.contains_key(p))
                    .unwrap_or(id);
                let scope_text = match story_marks.get(&scope_id) {
                    Some(m) if !marks.contains_key(&scope_id) => m.scope_text(),
                    _ => self.scope_text(scope_id),
                };
                let story_anchor = if anchor.is_none() {
                    story_marks.get(&id).and_then(|m| m.anchor.clone())
                } else {
                    None
                };
                let text = comment_paragraphs(c)
                    .into_iter()
                    .map(|p| {
                        let mut s = String::new();
                        text_of(p, &mut s);
                        s
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                Some(Comment {
                    id,
                    author: c.head.attr_w("author").unwrap_or_default().to_owned(),
                    initials: c.head.attr_w("initials").unwrap_or_default().to_owned(),
                    date: c.head.attr_w("date").map(str::to_owned),
                    text,
                    parent,
                    resolved,
                    anchor,
                    scope_text,
                    para_id,
                    story: story_anchor
                        .as_ref()
                        .map_or(CommentStory::Main, |a| a.story),
                    story_anchor,
                })
            })
            .collect()
    }

    /// The comment with `id`.
    pub fn comment(&self, id: u32) -> Option<Comment> {
        self.comments().into_iter().find(|c| c.id == id)
    }

    /// The direct replies to comment `id`, oldest first.
    pub fn comment_replies(&self, id: u32) -> Vec<Comment> {
        self.comments()
            .into_iter()
            .filter(|c| c.parent == Some(id))
            .collect()
    }

    fn require_comments_ready(&self) -> Result<(), DocxError> {
        match &self.comments.broken {
            Some(why) => Err(DocxError::CommentsUnavailable(why.clone())),
            None => Ok(()),
        }
    }

    fn comments_part_mut(&mut self) -> Result<&mut PartXml, DocxError> {
        self.require_comments_ready()?;
        let dir = self
            .main_part()
            .as_str()
            .rsplit_once('/')
            .map_or(String::new(), |(d, _)| format!("{d}/"));
        if self.comments.comments.is_none() {
            let xml = xml::parse(NEW_COMMENTS.as_bytes())?;
            self.comments.comments = Some(PartXml {
                name: PartName::new(&format!("{dir}comments.xml"))?,
                xml,
                dirty: true,
                created: true,
            });
        }
        self.comments
            .comments
            .as_mut()
            .ok_or_else(|| DocxError::CommentsUnavailable("no comments part".to_owned()))
    }

    fn extended_part_mut(&mut self) -> Result<&mut PartXml, DocxError> {
        self.require_comments_ready()?;
        let dir = self
            .main_part()
            .as_str()
            .rsplit_once('/')
            .map_or(String::new(), |(d, _)| format!("{d}/"));
        if self.comments.extended.is_none() {
            let xml = xml::parse(NEW_EXTENDED.as_bytes())?;
            self.comments.extended = Some(PartXml {
                name: PartName::new(&format!("{dir}commentsExtended.xml"))?,
                xml,
                dirty: true,
                created: true,
            });
        }
        self.comments
            .extended
            .as_mut()
            .ok_or_else(|| DocxError::CommentsUnavailable("no extended part".to_owned()))
    }

    /// Every `w14:paraId` in use in the main story and the comments part.
    fn used_para_ids(&self) -> HashSet<String> {
        let mut used = HashSet::new();
        let mut scan = |bytes: &[u8]| {
            let text = String::from_utf8_lossy(bytes);
            for piece in text.split("paraId=\"").skip(1) {
                if let Some((v, _)) = piece.split_once('"') {
                    used.insert(v.to_ascii_uppercase());
                }
            }
        };
        scan(&self.serialize_main());
        if let Some(p) = &self.comments.comments {
            scan(&p.xml.to_bytes());
        }
        used
    }

    fn fresh_hex(&self, used: &mut HashSet<String>, seed: u32) -> String {
        let mut v = 0x1000_0000_u32.wrapping_add(seed.wrapping_mul(0x0101_0101)) & 0x7FFF_FFFF;
        loop {
            let id = hex8(v);
            if v != 0 && used.insert(id.clone()) {
                return id;
            }
            v = v.wrapping_add(0x0000_1111) & 0x7FFF_FFFF;
        }
    }

    /// Adds `comment` anchored on the final-view text from `start` to `end` (either order).
    /// A collapsed range anchors the comment at a point. Returns the new comment's id.
    pub fn add_comment(
        &mut self,
        start: DocPos,
        end: DocPos,
        comment: &NewComment,
    ) -> Result<u32, DocxError> {
        let (s, e) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        self.check_pos(s)?;
        self.check_pos(e)?;
        let id = self.append_comment_body(comment, None)?;
        let reference = self.reference_run(id, s.para);
        // End first: with a collapsed range the start marker then lands before it.
        self.insert_markers(
            e,
            vec![
                Inline::Raw(RawXml::Element(self.marker("commentRangeEnd", id, e.para))),
                Inline::Run(reference),
            ],
        )?;
        self.insert_markers(
            s,
            vec![Inline::Raw(RawXml::Element(self.marker(
                "commentRangeStart",
                id,
                s.para,
            )))],
        )?;
        self.edited();
        Ok(id)
    }

    /// Adds a reply to comment `parent`. The reply is anchored on the parent's range.
    pub fn reply_to_comment(
        &mut self,
        parent: u32,
        comment: &NewComment,
    ) -> Result<u32, DocxError> {
        self.comment(parent)
            .ok_or(DocxError::NoSuchComment(parent))?;
        let marks = self.marks();
        if !marks.contains_key(&parent)
            && let Some(anchor) = self
                .story_marks()
                .get(&parent)
                .and_then(|m| m.anchor.clone())
        {
            return self.reply_in_story(parent, &anchor, comment);
        }
        let m = marks.get(&parent).copied().ok_or_else(|| {
            DocxError::CommentsUnavailable("the comment is not anchored".to_owned())
        })?;
        let (Some(start), Some(end_pos)) = (m.start, m.end.or(m.reference)) else {
            return Err(DocxError::CommentsUnavailable(
                "the comment is not anchored".to_owned(),
            ));
        };
        let parent_para = self.ensure_para_id(parent)?;
        let id = self.append_comment_body(comment, Some(&parent_para))?;
        let reference = self.reference_run(id, start.para);
        let end_items = vec![
            Inline::Raw(RawXml::Element(self.marker(
                "commentRangeEnd",
                id,
                end_pos.para,
            ))),
            Inline::Run(reference),
        ];
        let start_items = vec![Inline::Raw(RawXml::Element(self.marker(
            "commentRangeStart",
            id,
            start.para,
        )))];
        // The reply's markers sit right after the parent's, as Word writes them.
        let para = self.paragraph_mut(end_pos.para)?;
        let placed = insert_after(
            &mut para.content,
            &|i| is_reference(i, parent),
            end_items.clone(),
        )
        .or_else(|| insert_after(&mut para.content, &|i| is_range_end(i, parent), end_items));
        if placed.is_none() {
            return Err(DocxError::CommentsUnavailable(
                "the comment's end marker is missing".to_owned(),
            ));
        }
        let para = self.paragraph_mut(start.para)?;
        insert_after(
            &mut para.content,
            &|i| is_range_start(i, parent),
            start_items,
        )
        .ok_or_else(|| {
            DocxError::CommentsUnavailable("the comment's start marker is missing".to_owned())
        })?;
        self.edited();
        Ok(id)
    }

    /// Marks the comment, and the thread under it, resolved or open again.
    pub fn set_comment_resolved(&mut self, id: u32, resolved: bool) -> Result<(), DocxError> {
        if self.comment(id).is_none() {
            return Err(DocxError::NoSuchComment(id));
        }
        let thread: Vec<u32> = self
            .comments()
            .into_iter()
            .filter(|c| c.id == id || c.parent == Some(id))
            .map(|c| c.id)
            .collect();
        for cid in thread {
            let para_id = self.ensure_para_id(cid)?;
            self.ensure_extended_entry(&para_id, None)?;
            let part = self.extended_part_mut()?;
            for child in &mut part.xml.root.children {
                let RawXml::Element(e) = child else { continue };
                if is_el(e, W15, "commentEx") && attr_ns(&e.head, W15, "paraId") == Some(&para_id) {
                    set_attr_ns(
                        &mut e.head,
                        W15,
                        "w15",
                        "done",
                        if resolved { "1" } else { "0" },
                    );
                }
            }
            part.dirty = true;
        }
        Ok(())
    }

    /// Deletes comment `id`, its replies, their range markers and their reference runs.
    pub fn delete_comment(&mut self, id: u32) -> Result<(), DocxError> {
        self.require_comments_ready()?;
        let all = self.comments();
        if !all.iter().any(|c| c.id == id) {
            return Err(DocxError::NoSuchComment(id));
        }
        let doomed: HashSet<u32> = all
            .iter()
            .filter(|c| c.id == id || c.parent == Some(id))
            .map(|c| c.id)
            .collect();
        let doomed_paras: HashSet<String> = all
            .iter()
            .filter(|c| doomed.contains(&c.id))
            .filter_map(|c| c.para_id.clone())
            .collect();
        let marks = self.marks();
        let mut paras: Vec<usize> = marks
            .iter()
            .filter(|(k, _)| doomed.contains(k))
            .flat_map(|(_, m)| [m.start, m.end, m.reference])
            .flatten()
            .map(|p| p.para)
            .collect();
        paras.sort_unstable();
        paras.dedup();
        for p in paras {
            let para = self.paragraph_mut(p)?;
            remove_marks(&mut para.content, &doomed);
        }
        let story_marks = self.story_marks();
        self.remove_story_markers(&doomed, &story_marks)?;
        if let Some(part) = &mut self.comments.comments {
            part.xml.root.children.retain(|c| {
                !matches!(c, RawXml::Element(e) if e.head.is_w("comment")
                    && comment_id(e).is_some_and(|i| doomed.contains(&i)))
            });
            part.dirty = true;
        }
        for (slot, ns, local) in [
            (&mut self.comments.extended, W15, "commentEx"),
            (&mut self.comments.ids, W16CID, "commentId"),
        ] {
            if let Some(part) = slot {
                part.xml.root.children.retain(|c| {
                    !matches!(c, RawXml::Element(e) if is_el(e, ns, local)
                        && attr_ns(&e.head, ns, "paraId").is_some_and(|v| doomed_paras.contains(v)))
                });
                part.dirty = true;
            }
        }
        self.edited();
        Ok(())
    }

    /// Drops the body of comment `id` and its extension entries (undo of a failed add).
    fn remove_comment_body(&mut self, id: u32) {
        let para_ids: HashSet<String> = self
            .comments
            .comments
            .as_ref()
            .map(|p| {
                p.xml
                    .root
                    .children
                    .iter()
                    .filter_map(RawXml::as_element)
                    .filter(|e| e.head.is_w("comment") && comment_id(e) == Some(id))
                    .filter_map(last_para_id)
                    .collect()
            })
            .unwrap_or_default();
        if let Some(part) = &mut self.comments.comments {
            part.xml.root.children.retain(|c| {
                !matches!(c, RawXml::Element(e) if e.head.is_w("comment") && comment_id(e) == Some(id))
            });
            part.dirty = true;
        }
        for (slot, ns, local) in [
            (&mut self.comments.extended, W15, "commentEx"),
            (&mut self.comments.ids, W16CID, "commentId"),
        ] {
            if let Some(part) = slot {
                part.xml.root.children.retain(|c| {
                    !matches!(c, RawXml::Element(e) if is_el(e, ns, local)
                        && attr_ns(&e.head, ns, "paraId").is_some_and(|v| para_ids.contains(v)))
                });
                part.dirty = true;
            }
        }
    }

    /// The `w14:paraId` of comment `id`'s last paragraph, adding one when it has none.
    fn ensure_para_id(&mut self, id: u32) -> Result<String, DocxError> {
        let mut used = self.used_para_ids();
        let fresh = self.fresh_hex(&mut used, id.wrapping_add(7));
        let part = self.comments_part_mut()?;
        let root = &mut part.xml.root;
        let prefix = ensure_prefix(root, W14, "w14");
        let mut found: Option<String> = None;
        let mut touched = false;
        for child in &mut root.children {
            let RawXml::Element(c) = child else { continue };
            if !c.head.is_w("comment") || comment_id(c) != Some(id) {
                continue;
            }
            let Some(RawXml::Element(p)) = c
                .children
                .iter_mut()
                .rev()
                .find(|x| matches!(x, RawXml::Element(e) if e.head.is_w("p")))
            else {
                continue;
            };
            match attr_ns(&p.head, W14, "paraId") {
                Some(v) => found = Some(v.to_owned()),
                None => {
                    p.head.attrs.insert(
                        0,
                        Attr {
                            name: format!("{prefix}:paraId"),
                            ns: Some(W14.into()),
                            value: fresh.clone(),
                        },
                    );
                    found = Some(fresh.clone());
                    touched = true;
                }
            }
        }
        if touched {
            ensure_ignorable(&mut part.xml.root, W14, "w14");
            part.dirty = true;
        }
        found.ok_or(DocxError::NoSuchComment(id))
    }

    /// Makes sure `commentsExtended.xml` has an entry for `para_id` (and the parent link).
    fn ensure_extended_entry(
        &mut self,
        para_id: &str,
        parent_para: Option<&str>,
    ) -> Result<(), DocxError> {
        let part = self.extended_part_mut()?;
        let exists = part
            .xml
            .root
            .children
            .iter()
            .filter_map(RawXml::as_element)
            .any(|e| {
                is_el(e, W15, "commentEx") && attr_ns(&e.head, W15, "paraId") == Some(para_id)
            });
        if exists {
            return Ok(());
        }
        let prefix = ensure_prefix(&mut part.xml.root, W15, "w15");
        let mut head = ElementHead {
            name: format!("{prefix}:commentEx"),
            ns: Some(W15.into()),
            attrs: Vec::new(),
        };
        set_attr_ns(&mut head, W15, &prefix, "paraId", para_id);
        if let Some(parent) = parent_para {
            set_attr_ns(&mut head, W15, &prefix, "paraIdParent", parent);
        }
        set_attr_ns(&mut head, W15, &prefix, "done", "0");
        part.xml.root.children.push(RawXml::Element(XmlElement {
            head,
            children: Vec::new(),
        }));
        part.dirty = true;
        Ok(())
    }

    /// Appends the `w:comment` element (and extension entries) for `comment`.
    fn append_comment_body(
        &mut self,
        comment: &NewComment,
        parent_para: Option<&str>,
    ) -> Result<u32, DocxError> {
        self.require_comments_ready()?;
        let marks = self.marks();
        let has_comment_style = self.styles().is_some_and(|s| s.has_style("CommentText"));
        let has_ref_style = self
            .styles()
            .is_some_and(|s| s.has_style("CommentReference"));
        let mut used = self.used_para_ids();
        let date = comment.date.clone().unwrap_or_else(crate::review::now_utc);
        let part = self.comments_part_mut()?;
        let mut next = marks.keys().next_back().map_or(0, |m| m + 1);
        for e in part.xml.root.children.iter().filter_map(RawXml::as_element) {
            if let Some(i) = comment_id(e) {
                next = next.max(i + 1);
            }
        }
        let id = next;
        let seed = id.wrapping_mul(31).wrapping_add(comment.text.len() as u32);
        let root_name = part.xml.root.head.name.clone();
        let prefix = root_name.rsplit_once(':').map(|(p, _)| p.to_owned());
        let w = |local: &str| match &prefix {
            Some(p) => format!("{p}:{local}"),
            None => local.to_owned(),
        };
        let w_ns = part.xml.root.head.ns.clone();
        let el = |local: &str, attrs: Vec<(&str, String)>, children: Vec<RawXml>| {
            RawXml::Element(XmlElement {
                head: ElementHead {
                    name: w(local),
                    ns: w_ns.clone(),
                    attrs: attrs
                        .into_iter()
                        .map(|(k, v)| Attr {
                            name: w(k),
                            ns: w_ns.clone(),
                            value: v,
                        })
                        .collect(),
                },
                children,
            })
        };
        let lines: Vec<&str> = comment.text.split('\n').collect();
        let prefix14 = ensure_prefix(&mut part.xml.root, W14, "w14");
        ensure_ignorable(&mut part.xml.root, W14, "w14");
        let mut paragraphs = Vec::new();
        let mut last_para_id = String::new();
        for (n, line) in lines.iter().enumerate() {
            let mut kids = Vec::new();
            if has_comment_style {
                kids.push(el(
                    "pPr",
                    vec![],
                    vec![el(
                        "pStyle",
                        vec![("val", "CommentText".to_owned())],
                        vec![],
                    )],
                ));
            }
            if n == 0 {
                let mut run_kids = Vec::new();
                if has_ref_style {
                    run_kids.push(el(
                        "rPr",
                        vec![],
                        vec![el(
                            "rStyle",
                            vec![("val", "CommentReference".to_owned())],
                            vec![],
                        )],
                    ));
                }
                run_kids.push(el("annotationRef", vec![], vec![]));
                kids.push(el("r", vec![], run_kids));
            }
            if !line.is_empty() {
                let mut t_attrs = vec![];
                if crate::edit::needs_preserve(line) {
                    t_attrs.push(("space", "preserve".to_owned()));
                }
                let mut t = el("t", vec![], vec![RawXml::Text((*line).to_owned())]);
                if let RawXml::Element(te) = &mut t
                    && !t_attrs.is_empty()
                {
                    te.head.attrs.push(Attr {
                        name: "xml:space".to_owned(),
                        ns: Some("http://www.w3.org/XML/1998/namespace".into()),
                        value: "preserve".to_owned(),
                    });
                }
                kids.push(el("r", vec![], vec![t]));
            }
            let para_id = {
                let mut v = 0x1000_0000_u32
                    .wrapping_add(seed.wrapping_mul(0x0101_0101))
                    .wrapping_add(n as u32 * 0x11)
                    & 0x7FFF_FFFF;
                loop {
                    let candidate = hex8(v);
                    if v != 0 && used.insert(candidate.clone()) {
                        break candidate;
                    }
                    v = v.wrapping_add(0x1111) & 0x7FFF_FFFF;
                }
            };
            let mut p = el("p", vec![], kids);
            if let RawXml::Element(pe) = &mut p {
                pe.head.attrs.push(Attr {
                    name: format!("{prefix14}:paraId"),
                    ns: Some(W14.into()),
                    value: para_id.clone(),
                });
            }
            last_para_id = para_id;
            paragraphs.push(p);
        }
        let comment_el = el(
            "comment",
            vec![
                ("id", id.to_string()),
                ("author", comment.author.clone()),
                ("date", date.clone()),
                ("initials", comment.initials.clone()),
            ],
            paragraphs,
        );
        part.xml.root.children.push(comment_el);
        part.dirty = true;

        // Extension parts: commentsExtended is created when a reply needs it.
        let have_extended = self.comments.extended.is_some();
        if have_extended || parent_para.is_some() {
            self.ensure_extended_entry(&last_para_id, parent_para)?;
        }
        let mut durable_used = used;
        let durable = self.fresh_hex(&mut durable_used, id.wrapping_add(99));
        if let Some(part) = &mut self.comments.ids {
            let prefix = ensure_prefix(&mut part.xml.root, W16CID, "w16cid");
            let mut head = ElementHead {
                name: format!("{prefix}:commentId"),
                ns: Some(W16CID.into()),
                attrs: Vec::new(),
            };
            set_attr_ns(&mut head, W16CID, &prefix, "paraId", &last_para_id);
            set_attr_ns(&mut head, W16CID, &prefix, "durableId", &durable);
            part.xml.root.children.push(RawXml::Element(XmlElement {
                head,
                children: Vec::new(),
            }));
            part.dirty = true;
        }
        if let Some(part) = &mut self.comments.extensible {
            let prefix = ensure_prefix(&mut part.xml.root, W16CEX, "w16cex");
            let mut head = ElementHead {
                name: format!("{prefix}:commentExtensible"),
                ns: Some(W16CEX.into()),
                attrs: Vec::new(),
            };
            set_attr_ns(&mut head, W16CEX, &prefix, "durableId", &durable);
            set_attr_ns(&mut head, W16CEX, &prefix, "dateUtc", &date);
            part.xml.root.children.push(RawXml::Element(XmlElement {
                head,
                children: Vec::new(),
            }));
            part.dirty = true;
        }
        self.ensure_person(&comment.author)?;
        Ok(id)
    }

    /// Adds `author` to `word/people.xml` (created when missing) unless already listed.
    fn ensure_person(&mut self, author: &str) -> Result<(), DocxError> {
        if self.people().iter().any(|p| p.author == author) {
            return Ok(());
        }
        if self.comments.people.is_none() {
            let dir = self
                .main_part()
                .as_str()
                .rsplit_once('/')
                .map_or(String::new(), |(d, _)| format!("{d}/"));
            self.comments.people = Some(PartXml {
                name: PartName::new(&format!("{dir}people.xml"))?,
                xml: xml::parse(NEW_PEOPLE.as_bytes())?,
                dirty: true,
                created: true,
            });
        }
        let Some(part) = &mut self.comments.people else {
            return Ok(());
        };
        let prefix = ensure_prefix(&mut part.xml.root, W15, "w15");
        let name = |local: &str| format!("{prefix}:{local}");
        let attr = |local: &str, value: &str| Attr {
            name: name(local),
            ns: Some(W15.into()),
            value: value.to_owned(),
        };
        let presence = XmlElement {
            head: ElementHead {
                name: name("presenceInfo"),
                ns: Some(W15.into()),
                attrs: vec![attr("providerId", "None"), attr("userId", author)],
            },
            children: Vec::new(),
        };
        part.xml.root.children.push(RawXml::Element(XmlElement {
            head: ElementHead {
                name: name("person"),
                ns: Some(W15.into()),
                attrs: vec![attr("author", author)],
            },
            children: vec![RawXml::Element(presence)],
        }));
        part.dirty = true;
        Ok(())
    }

    /// An empty marker element named like the paragraph's elements.
    fn marker(&self, local: &str, id: u32, para: usize) -> XmlElement {
        let owner = self.paragraph(para).map_or_else(
            || ElementHead {
                name: "w:p".to_owned(),
                ns: Some(W_NS.into()),
                attrs: Vec::new(),
            },
            |p| p.head.clone(),
        );
        let mut head = owner.sibling(local);
        head.set_attr_w("id", &id.to_string());
        XmlElement {
            head,
            children: Vec::new(),
        }
    }

    /// The run holding `w:commentReference` for comment `id`.
    fn reference_run(&self, id: u32, para: usize) -> Run {
        let owner = self.paragraph(para);
        let mut run = owner.map_or_else(
            || Run {
                head: ElementHead {
                    name: "w:r".to_owned(),
                    ns: Some(W_NS.into()),
                    attrs: Vec::new(),
                },
                props: RunProps::default(),
                content: Vec::new(),
            },
            Paragraph::new_run,
        );
        if self
            .styles()
            .is_some_and(|s| s.has_style("CommentReference"))
        {
            let mut rpr = run.head.sibling("rPr");
            let mut style = run.head.sibling("rStyle");
            style.set_attr_w("val", "CommentReference");
            rpr.name = run.head.sibling("rPr").name;
            run.props = RunProps::from_element(XmlElement {
                head: rpr,
                children: vec![RawXml::Element(XmlElement {
                    head: style,
                    children: Vec::new(),
                })],
            });
        }
        let mut reference = run.head.sibling("commentReference");
        reference.set_attr_w("id", &id.to_string());
        run.content
            .push(RunContent::Raw(RawXml::Element(XmlElement {
                head: reference,
                children: Vec::new(),
            })));
        run
    }

    /// Inserts `items` at the run boundary for `pos`.
    fn insert_markers(&mut self, pos: DocPos, items: Vec<Inline>) -> Result<(), DocxError> {
        let para = self.paragraph_mut(pos.para)?;
        split_runs_at(&mut para.content, pos.offset, &mut 0);
        let mut acc = 0;
        let mut pending = Some(items);
        insert_at_boundary(&mut para.content, pos.offset, &mut acc, &mut pending);
        if let Some(items) = pending {
            para.content.extend(items);
        }
        Ok(())
    }
}

fn insert_at_boundary(
    content: &mut Vec<Inline>,
    target: usize,
    acc: &mut usize,
    items: &mut Option<Vec<Inline>>,
) {
    let mut i = 0;
    while i < content.len() {
        if items.is_none() {
            return;
        }
        if *acc == target {
            if let Some(items) = items.take() {
                content.splice(i..i, items);
            }
            return;
        }
        match &mut content[i] {
            Inline::Run(r) => *acc += r.char_len(),
            Inline::Group(g) => {
                let len: usize = g.content.iter().map(Inline::char_len).sum();
                if len > 0 && target > *acc && target < *acc + len {
                    insert_at_boundary(&mut g.content, target, acc, items);
                } else {
                    *acc += len;
                }
            }
            Inline::Raw(_) => {}
        }
        i += 1;
    }
    if *acc == target
        && let Some(items) = items.take()
    {
        content.extend(items);
    }
}

fn is_range_start(i: &Inline, id: u32) -> bool {
    matches!(i, Inline::Raw(RawXml::Element(e)) if e.head.is_w("commentRangeStart") && comment_id(e) == Some(id))
}

fn is_range_end(i: &Inline, id: u32) -> bool {
    matches!(i, Inline::Raw(RawXml::Element(e)) if e.head.is_w("commentRangeEnd") && comment_id(e) == Some(id))
}

fn is_reference(i: &Inline, id: u32) -> bool {
    match i {
        Inline::Run(r) => r.content.iter().any(|c| {
            matches!(c, RunContent::Raw(RawXml::Element(e))
                if e.head.is_w("commentReference") && comment_id(e) == Some(id))
        }),
        _ => false,
    }
}

/// Inserts `items` right after the first inline matching `find`, searching groups too.
fn insert_after(
    content: &mut Vec<Inline>,
    find: &dyn Fn(&Inline) -> bool,
    items: Vec<Inline>,
) -> Option<()> {
    if let Some(at) = content.iter().position(find) {
        content.splice(at + 1..at + 1, items);
        return Some(());
    }
    let mut items = Some(items);
    for inline in content.iter_mut() {
        if let Inline::Group(g) = inline
            && let Some(taken) = items.take()
        {
            match insert_after(&mut g.content, find, taken.clone()) {
                Some(()) => return Some(()),
                None => items = Some(taken),
            }
        }
    }
    None
}

fn remove_marks(content: &mut Vec<Inline>, doomed: &HashSet<u32>) {
    content.retain(|i| match i {
        Inline::Raw(RawXml::Element(e)) => {
            !((e.head.is_w("commentRangeStart") || e.head.is_w("commentRangeEnd"))
                && comment_id(e).is_some_and(|id| doomed.contains(&id)))
        }
        Inline::Run(r) => !doomed
            .iter()
            .any(|id| is_reference(&Inline::Run(r.clone()), *id)),
        Inline::Group(_) | Inline::Raw(_) => true,
    });
    for i in content {
        if let Inline::Group(g) = i {
            remove_marks(&mut g.content, doomed);
        }
    }
}

const NEW_COMMENTS: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n",
    "<w:comments xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" ",
    "xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" ",
    "xmlns:w14=\"http://schemas.microsoft.com/office/word/2010/wordml\" ",
    "mc:Ignorable=\"w14\"/>"
);

const NEW_PEOPLE: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>
",
    "<w15:people xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" ",
    "xmlns:w15=\"http://schemas.microsoft.com/office/word/2012/wordml\" ",
    "mc:Ignorable=\"w15\"/>"
);

const NEW_EXTENDED: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n",
    "<w15:commentsEx xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" ",
    "xmlns:w15=\"http://schemas.microsoft.com/office/word/2012/wordml\" ",
    "mc:Ignorable=\"w15\"/>"
);
