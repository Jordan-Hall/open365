#![allow(clippy::unwrap_used)]
//! Comments in every story: headers, footers, footnotes, endnotes and text boxes, with replies,
//! resolved state and `people.xml`.
//!
//! `fixtures/docx/comments-stories.docx` is generated (authors "Author A" and "Author B").
//! `fixtures/reference/docx/comments-stories/comments.tsv` is what Word 16 reports for it: only
//! the two main-story comments. Word drops every comment placed in another story when it opens
//! the file, and `Comments.Add` refuses to create one there (`cmt-stories` in
//! `tools/office/make-revision-refs.ps1`), so parity ends at the main story; for the other
//! stories these tests check lossless reading, editing and round trips.

use open365_docx::{Comment, CommentStory, DocPos, Document, NewComment};
use open365_opc::{Limits, Package};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/docx")
            .join(name),
    )
    .expect("fixture")
}

fn open() -> Document {
    Document::open(&fixture("comments-stories.docx"), &Limits::default()).unwrap()
}

fn by_text<'a>(all: &'a [Comment], text: &str) -> &'a Comment {
    all.iter().find(|c| c.text == text).unwrap()
}

#[test]
fn main_story_comments_match_word() {
    let reference = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/reference/docx/comments-stories/comments.tsv"),
    )
    .unwrap();
    let lines: Vec<Vec<&str>> = reference.lines().map(|l| l.split('\t').collect()).collect();
    let all = open().comments();
    // Word lists the main story's two comments (and again under the comments story).
    for l in lines.iter().take(2) {
        let c = by_text(&all, l[5]);
        assert_eq!(c.author, l[2]);
        assert_eq!(c.initials, l[3]);
        assert_eq!(c.scope_text, l[4]);
        assert_eq!(u8::from(c.resolved).to_string(), l[6]);
        assert_eq!(c.story, CommentStory::Main);
    }
    let reply = by_text(&all, "Main reply");
    assert_eq!(reply.parent, Some(by_text(&all, "Main comment").id));
}

#[test]
fn reads_comments_in_every_story() {
    let doc = open();
    let all = doc.comments();
    assert_eq!(all.len(), 8);
    let expect = [
        (
            "Footnote comment",
            CommentStory::Footnote,
            "note words",
            true,
        ),
        ("Footnote reply", CommentStory::Footnote, "note words", true),
        ("Endnote comment", CommentStory::Endnote, "end words", false),
        (
            "Text box comment",
            CommentStory::TextBox,
            "Boxed words",
            false,
        ),
        (
            "Header comment",
            CommentStory::Header,
            "header words",
            false,
        ),
        ("Footer comment", CommentStory::Footer, "footer words", true),
    ];
    for (text, story, scope, resolved) in expect {
        let c = by_text(&all, text);
        assert_eq!(c.story, story, "{text}");
        assert_eq!(c.scope_text, scope, "{text}");
        assert_eq!(c.resolved, resolved, "{text}");
        assert!(c.anchor.is_none());
        assert_eq!(c.story_anchor.as_ref().unwrap().story, story);
    }
    let note = by_text(&all, "Footnote comment");
    assert_eq!(by_text(&all, "Footnote reply").parent, Some(note.id));
    assert_eq!(doc.comment_replies(note.id).len(), 1);
    assert_eq!(doc.people().len(), 2);
}

#[test]
fn untouched_save_keeps_every_part() {
    let original = fixture("comments-stories.docx");
    let mut doc = Document::open(&original, &Limits::default()).unwrap();
    let saved = doc.save().unwrap();
    let a = Package::read(&original, &Limits::default()).unwrap();
    let b = Package::read(&saved, &Limits::default()).unwrap();
    for part in a.parts() {
        assert_eq!(
            part.data(),
            b.part(part.name().as_str()).unwrap().data(),
            "{}",
            part.name()
        );
    }
}

#[test]
fn adds_replies_resolves_and_deletes_in_each_story() {
    for (story, part, start, end, expect) in [
        (CommentStory::Header, "word/header1.xml", 2, 6, "ad h"),
        (CommentStory::Footer, "word/footer1.xml", 0, 4, "Foot"),
        (CommentStory::Footnote, "word/footnotes.xml", 1, 5, "note"),
        (CommentStory::Endnote, "word/endnotes.xml", 1, 4, "end"),
        (CommentStory::TextBox, "word/document.xml", 6, 11, "words"),
    ] {
        let mut doc = open();
        let n = doc.comments().len();
        let id = doc
            .add_story_comment(
                story,
                None,
                DocPos::new(
                    if story == CommentStory::Footnote || story == CommentStory::Endnote {
                        // footnote 2 is the third paragraph (separators come first)
                        2
                    } else {
                        0
                    },
                    start,
                ),
                DocPos::new(
                    if story == CommentStory::Footnote || story == CommentStory::Endnote {
                        2
                    } else {
                        0
                    },
                    end,
                ),
                &NewComment::new("Author C", "New story comment").with_date("2026-01-02T03:04:00Z"),
            )
            .unwrap();
        let reply = doc
            .reply_to_comment(id, &NewComment::new("Author A", "Story reply"))
            .unwrap();
        doc.set_comment_resolved(id, true).unwrap();
        let saved = doc.save().unwrap();
        // Set to a scratch directory to open the results in Word by hand.
        if let Ok(dir) = std::env::var("OPEN365_COMMENT_STORIES_DUMP") {
            std::fs::write(
                std::path::Path::new(&dir).join(format!("{story:?}.docx")),
                &saved,
            )
            .unwrap();
        }

        let again = Document::open(&saved, &Limits::default()).unwrap();
        let all = again.comments();
        assert_eq!(all.len(), n + 2, "{story:?}");
        let c = by_text(&all, "New story comment");
        assert_eq!(c.story, story, "{story:?}");
        assert!(c.resolved, "{story:?}");
        assert_eq!(c.initials, "AC");
        assert_eq!(c.scope_text, expect, "{story:?}");
        let r = by_text(&all, "Story reply");
        assert_eq!(r.id, reply);
        assert_eq!(r.parent, Some(id));
        assert_eq!(r.story, story);
        assert_eq!(r.scope_text, c.scope_text);
        assert!(r.resolved);
        assert!(again.people().iter().any(|p| p.author == "Author C"));
        // Only the edited part changed apart from the comment parts and the main story.
        let before = Package::read(&fixture("comments-stories.docx"), &Limits::default()).unwrap();
        let after = Package::read(&saved, &Limits::default()).unwrap();
        for p in before.parts() {
            let name = p.name().as_str();
            if matches!(
                name,
                "word/comments.xml"
                    | "word/commentsExtended.xml"
                    | "word/people.xml"
                    | "word/document.xml"
            ) || name == part
            {
                continue;
            }
            assert_eq!(
                p.data(),
                after.part(name).unwrap().data(),
                "{story:?}: {name}"
            );
        }

        let mut again = again;
        again.delete_comment(id).unwrap();
        let left = again.comments();
        assert_eq!(left.len(), n, "{story:?}");
        assert!(left.iter().all(|c| c.text != "New story comment"));
        let reopened = Document::open(&again.save().unwrap(), &Limits::default()).unwrap();
        assert_eq!(reopened.comments().len(), n);
    }
}

#[test]
fn refuses_ranges_outside_the_story() {
    let mut doc = open();
    let n = doc.comments().len();
    let err = doc.add_story_comment(
        CommentStory::Header,
        None,
        DocPos::new(0, 0),
        DocPos::new(0, 999),
        &NewComment::new("Author C", "Nope"),
    );
    assert!(err.is_err());
    assert_eq!(doc.comments().len(), n);
    let err = doc.add_story_comment(
        CommentStory::Footer,
        Some("word/missing.xml"),
        DocPos::new(0, 0),
        DocPos::new(0, 1),
        &NewComment::new("Author C", "Nope"),
    );
    assert!(err.is_err());
}

#[test]
fn deleting_a_story_comment_removes_its_markers() {
    let mut doc = open();
    let id = by_text(&doc.comments(), "Header comment").id;
    doc.delete_comment(id).unwrap();
    let saved = doc.save().unwrap();
    let pkg = Package::read(&saved, &Limits::default()).unwrap();
    let header = String::from_utf8(pkg.part("word/header1.xml").unwrap().data().to_vec()).unwrap();
    assert!(!header.contains("comment"), "{header}");
    assert!(header.contains("header words"));
}
