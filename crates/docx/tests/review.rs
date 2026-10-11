#![allow(clippy::unwrap_used)]
//! Tracked changes and comments against Word's own results (`tools/office/review-oracle.ps1`,
//! `fixtures/reference/docx/review-*/review.tsv`), plus recording and round-trip tests.

use open365_docx::{
    Alignment, DocPos, Document, NewComment, Revision, RevisionKind, TextView, TrackChanges,
};
use open365_opc::Limits;

const NAMES: [&str; 6] = [
    "review-ins-del",
    "review-para-marks",
    "review-format",
    "review-move",
    "review-rows",
    "review-comments",
];
const SEP: &str = "\u{241e}";

fn path(rel: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(rel)
}

fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(path(&format!("docx/{name}.docx"))).expect("fixture")
}

fn open(name: &str) -> Document {
    Document::open(&bytes(name), &Limits::default()).expect("opens")
}

fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('n') => out.push('\n'),
            Some('u') => {
                let hex: String = it.by_ref().take(4).collect();
                out.push(char::from_u32(u32::from_str_radix(&hex, 16).unwrap()).unwrap());
            }
            Some(o) => out.push(o),
            None => {}
        }
    }
    out
}

/// Word text with paragraph and cell marks as newlines.
fn norm(s: &str) -> String {
    s.replace("\r\u{7}", "\n").replace('\r', "\n")
}

fn reference(name: &str) -> Vec<Vec<String>> {
    let text = std::fs::read_to_string(path(&format!("reference/docx/{name}/review.tsv")))
        .expect("reference");
    text.lines()
        .map(|l| l.split('\t').map(unescape).collect())
        .collect()
}

fn lines<'a>(r: &'a [Vec<String>], kind: &str) -> Vec<&'a Vec<String>> {
    r.iter().filter(|l| l[0] == kind).collect()
}

fn word_type(k: RevisionKind) -> &'static str {
    match k {
        RevisionKind::Insertion
        | RevisionKind::ParagraphMarkInserted
        | RevisionKind::RowInserted => "Insert",
        RevisionKind::Deletion | RevisionKind::ParagraphMarkDeleted | RevisionKind::RowDeleted => {
            "Delete"
        }
        RevisionKind::MovedFrom => "MovedFrom",
        RevisionKind::MovedTo => "MovedTo",
        RevisionKind::RunFormat => "Property",
        RevisionKind::ParagraphFormat => "ParagraphProperty",
        RevisionKind::TableFormat => "TableProperty",
        RevisionKind::SectionFormat => "SectionProperty",
    }
}

/// Word 16 drops the `w:tblPrChange` of the probe, so it is not compared.
fn comparable(doc: &Document) -> Vec<Revision> {
    doc.revisions()
        .into_iter()
        .filter(|r| r.kind != RevisionKind::TableFormat)
        .collect()
}

fn markup(doc: &Document) -> Vec<String> {
    (0..doc.paragraph_count())
        .map(|i| doc.paragraph_text_view(i, TextView::Markup).unwrap())
        .collect()
}

fn align(doc: &Document, i: usize) -> u8 {
    match doc.paragraph(i).unwrap().direct_alignment() {
        Some(Alignment::Center) => 1,
        Some(Alignment::Right) => 2,
        Some(Alignment::Justify) => 3,
        _ => 0,
    }
}

#[test]
fn revision_lists_match_word() {
    let mut matched = 0;
    for name in NAMES {
        let r = reference(name);
        let doc = open(name);
        let ours = comparable(&doc);
        let theirs = lines(&r, "rev");
        let pairs: Vec<(String, String)> = ours
            .iter()
            .map(|v| (word_type(v.kind).to_owned(), v.author.clone()))
            .collect();
        let expected: Vec<(String, String)> = theirs
            .iter()
            .map(|l| (l[2].clone(), l[3].clone()))
            .collect();
        assert_eq!(pairs, expected, "{name}: revision types and authors");
        for (v, l) in ours.iter().zip(&theirs) {
            if matches!(
                v.kind,
                RevisionKind::Insertion
                    | RevisionKind::Deletion
                    | RevisionKind::MovedFrom
                    | RevisionKind::MovedTo
            ) {
                assert_eq!(v.text, norm(&l[4]), "{name}: revision {} text", v.index);
            }
        }
        matched += ours.len();
    }
    assert!(matched >= 19, "compared {matched} revisions");
}

#[test]
fn views_match_word() {
    for name in NAMES {
        let r = reference(name);
        let doc = open(name);
        let want: Vec<String> = lines(&r, "markup").iter().map(|l| norm(&l[2])).collect();
        assert_eq!(markup(&doc), want, "{name}: markup view");
        let want: Vec<String> = lines(&r, "final").iter().map(|l| norm(&l[2])).collect();
        let got: Vec<String> = doc
            .final_view_paragraphs()
            .into_iter()
            .map(|p| p.text)
            .collect();
        // Word's final view keeps the cells of a deleted row as empty paragraphs.
        assert_eq!(got, want, "{name}: final view");
        for i in 0..doc.paragraph_count() {
            assert_eq!(
                doc.paragraph_text_view(i, TextView::Final).unwrap(),
                doc.paragraph(i).unwrap().text()
            );
        }
    }
}

#[test]
fn accept_and_reject_all_match_word() {
    for name in NAMES {
        let r = reference(name);
        for (kind, accept) in [("accept", true), ("reject", false)] {
            let mut doc = open(name);
            if accept {
                doc.accept_all_revisions();
            } else {
                doc.reject_all_revisions();
            }
            let want = lines(&r, kind);
            let got = markup(&doc);
            assert_eq!(
                got,
                want.iter().map(|l| norm(&l[2])).collect::<Vec<_>>(),
                "{name}: {kind} all text"
            );
            if doc
                .revisions()
                .iter()
                .all(|v| v.kind != RevisionKind::TableFormat)
            {
                assert!(doc.revisions().is_empty(), "{name}: {kind} left revisions");
            }
            for (i, l) in want.iter().enumerate() {
                if l.len() > 3 {
                    assert_eq!(
                        align(&doc, i).to_string(),
                        l[3],
                        "{name}: {kind} paragraph {i} alignment"
                    );
                }
            }
            // The result is a valid package that reopens with the same text.
            let saved = doc.save().expect("save");
            let again = Document::open(&saved, &Limits::default()).expect("reopens");
            assert_eq!(markup(&again), got, "{name}: {kind} reopen");
        }
    }
}

#[test]
fn single_revisions_match_word() {
    for name in NAMES {
        let r = reference(name);
        let count = comparable(&open(name)).len();
        if open(name).revisions().len() != count {
            continue;
        }
        for (kind, accept) in [("accept1", true), ("reject1", false)] {
            for l in lines(&r, kind) {
                let idx: usize = l[1].parse().unwrap();
                let mut doc = open(name);
                if accept {
                    doc.accept_revision(idx).unwrap();
                } else {
                    doc.reject_revision(idx).unwrap();
                }
                let got = markup(&doc).join(SEP);
                assert_eq!(got, norm(&l[2]).replace('\n', SEP), "{name}: {kind} {idx}");
            }
        }
    }
}

#[test]
fn comments_match_word() {
    let r = reference("review-comments");
    let doc = open("review-comments");
    let ours = doc.comments();
    let theirs = lines(&r, "comment");
    assert_eq!(ours.len(), theirs.len());
    for (c, l) in ours.iter().zip(&theirs) {
        assert_eq!(c.author, l[2]);
        assert_eq!(c.initials, l[3]);
        assert_eq!(c.scope_text, norm(&l[4]), "scope of {}", c.id);
        assert_eq!(c.text, norm(&l[5]));
        assert_eq!(u8::from(c.resolved).to_string(), l[6]);
        let parent = c
            .parent
            .and_then(|p| ours.iter().position(|o| o.id == p))
            .map_or(-1, |p| i64::try_from(p).unwrap());
        assert_eq!(parent.to_string(), l[7]);
        assert_eq!(doc.comment_replies(c.id).len().to_string(), l[8]);
    }
}

#[test]
fn untouched_documents_save_byte_identical_parts() {
    for name in NAMES {
        let original = bytes(name);
        let mut doc = Document::open(&original, &Limits::default()).unwrap();
        let saved = doc.save().unwrap();
        let a = open365_opc::Package::read(&original, &Limits::default()).unwrap();
        let b = open365_opc::Package::read(&saved, &Limits::default()).unwrap();
        for part in a.parts() {
            assert_eq!(
                part.data(),
                b.part(part.name().as_str()).unwrap().data(),
                "{name}: {}",
                part.name()
            );
        }
    }
}

#[test]
fn recording_edits_as_tracked_changes() {
    let mut doc = open("review-ins-del");
    let before = doc.revisions().len();
    doc.set_track_changes(Some(TrackChanges::with_date(
        "Reviewer C",
        "2026-02-03T04:05:00Z",
    )));
    // Insert in plain text, delete plain text, delete own insertion.
    let p = doc.paragraph_text(0).unwrap();
    assert_eq!(p, "Alpha beta gamma epsilon");
    doc.insert_text(0, 0, "NEW ").unwrap();
    doc.delete_range(DocPos::new(0, 4), DocPos::new(0, 9))
        .unwrap(); // "Alpha"
    assert_eq!(doc.paragraph_text(0).unwrap(), "NEW  beta gamma epsilon");
    let c: Vec<_> = doc
        .revisions()
        .into_iter()
        .filter(|r| r.author == "Reviewer C")
        .collect();
    assert_eq!(c.len(), 2);
    assert_eq!(c[0].kind, RevisionKind::Insertion);
    assert_eq!(c[1].kind, RevisionKind::Deletion);
    // Deleting text the author inserted removes it with no new revision.
    doc.delete_range(DocPos::new(0, 0), DocPos::new(0, 4))
        .unwrap();
    assert_eq!(doc.paragraph_text(0).unwrap(), " beta gamma epsilon");
    assert_eq!(
        doc.revisions()
            .iter()
            .filter(|r| r.author == "Reviewer C")
            .count(),
        1
    );
    // Typing inside another author's insertion splits it.
    doc.insert_text(0, 3, "Q").unwrap();
    assert_eq!(doc.paragraph_text(0).unwrap(), " beQta gamma epsilon");
    // Enter marks the paragraph mark as inserted; Backspace over it removes it again.
    let n = doc.paragraph_count();
    doc.split_paragraph(DocPos::new(0, 2)).unwrap();
    assert_eq!(doc.paragraph_count(), n + 1);
    assert_eq!(
        doc.paragraph(0).unwrap().mark_revision(),
        Some(RevisionKind::ParagraphMarkInserted)
    );
    doc.delete_range(DocPos::new(0, 2), DocPos::new(1, 0))
        .unwrap();
    assert_eq!(doc.paragraph_count(), n);
    // The result saves, reopens, and rejecting everything by Reviewer C is possible.
    let saved = doc.save().unwrap();
    let mut again = Document::open(&saved, &Limits::default()).unwrap();
    assert!(again.revisions().len() > before);
    again.reject_all_revisions();
    again.accept_all_revisions();
    assert!(again.revisions().is_empty());
}

#[test]
fn adding_replying_and_resolving_comments() {
    let mut doc = open("review-ins-del");
    assert!(doc.comments().is_empty());
    let id = doc
        .add_comment(
            DocPos::new(0, 0),
            DocPos::new(0, 5),
            &NewComment::new("Reviewer A", "Check this").with_date("2026-01-02T03:04:00Z"),
        )
        .unwrap();
    let reply = doc
        .reply_to_comment(id, &NewComment::new("Reviewer B", "Done"))
        .unwrap();
    doc.set_comment_resolved(id, true).unwrap();
    let saved = doc.save().unwrap();
    let again = Document::open(&saved, &Limits::default()).unwrap();
    let all = again.comments();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].scope_text, "Alpha");
    assert_eq!(all[0].initials, "RA");
    assert!(all[0].resolved && all[1].resolved);
    assert_eq!(all[1].id, reply);
    assert_eq!(all[1].parent, Some(id));
    assert_eq!(all[1].scope_text, "Alpha");
    assert_eq!(again.paragraph_text(0).unwrap(), "Alpha beta gamma epsilon");
    let mut again = again;
    again.delete_comment(id).unwrap();
    assert!(again.comments().is_empty());
}
