#![allow(clippy::unwrap_used)]
//! Tracked changes and comments measured against Word 16 itself.
//!
//! `fixtures/docx/{rev,cmt}-*.docx` were created by Word (Application.TrackRevisions,
//! Comments.Add) and `fixtures/reference/docx/<name>/review.tsv` records what Word reports for
//! each: the revision list, the comments, and the paragraph and table state after
//! `Revisions.AcceptAll`, `RejectAll`, each revision accepted or rejected alone, and each
//! paragraph's `Range.Revisions` accepted or rejected (see `tools/office/make-revision-refs.ps1`).
//! The model must reach the same state. Known gaps are listed in `KNOWN_GAPS` and must still
//! fail, so the list cannot go stale.

use std::collections::BTreeMap;

use open365_docx::{
    Alignment, Block, Document, RevisionKind, RowChild, TableChild, TableWidth, TextView,
    VerticalMerge,
};
use open365_opc::Limits;

fn path(rel: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(rel)
}

fn open(name: &str) -> Document {
    let bytes = std::fs::read(path(&format!("docx/{name}.docx"))).expect("fixture");
    Document::open(&bytes, &Limits::default()).expect("opens")
}

fn names() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(path("docx"))
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().into_string().ok())
        .filter_map(|n| n.strip_suffix(".docx").map(str::to_owned))
        .filter(|n| n.starts_with("rev-") || n.starts_with("cmt-"))
        .filter(|n| std::env::var("REV_ONLY").map_or(true, |o| *n == o))
        .collect();
    v.sort();
    v
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

fn reference(name: &str) -> Vec<Vec<String>> {
    let text = std::fs::read_to_string(path(&format!("reference/docx/{name}/review.tsv")))
        .expect("reference");
    text.lines()
        .map(|l| l.split('\t').map(unescape).collect())
        .collect()
}

/// Word text: paragraph marks as newlines.
fn norm(s: &str) -> String {
    s.replace("\r\u{7}", "\n").replace('\r', "\n")
}

#[derive(Debug, Clone, PartialEq)]
struct ParaSig {
    text: String,
    align: u8,
    style: String,
    list: bool,
    runs: Vec<(String, usize)>,
}

#[derive(Debug, Clone, PartialEq)]
struct CellSig {
    row: usize,
    col: usize,
    width: f64,
    shade: i64,
    text: String,
}

type State = (Vec<ParaSig>, Vec<Vec<CellSig>>);

fn parse_runs(s: &str) -> Vec<(String, usize)> {
    s.split(';')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, n) = p.rsplit_once('x').unwrap();
            (k.to_owned(), n.parse().unwrap())
        })
        .collect()
}

fn ref_state(r: &[Vec<String>], label: &str) -> State {
    let paras = r
        .iter()
        .filter(|l| l[0] == "para" && l[1] == label)
        .map(|l| {
            // A section break ends its paragraph with U+000C in Word's text; a note reference is U+0002.
            let formfeed = l[3].contains('\u{c}');
            let mut runs = parse_runs(&l[7]);
            if formfeed && let Some(last) = runs.last_mut() {
                last.1 -= 1;
            }
            ParaSig {
                text: norm(&l[3]).replace(['\u{c}', '\u{2}'], ""),
                align: l[4].parse().unwrap(),
                style: l[5].to_lowercase(),
                list: l[6] != "0",
                runs,
            }
        })
        .collect();
    let tables = r
        .iter()
        .filter(|l| l[0] == "tbl" && l[1] == label)
        .map(|l| {
            l[3].split('\u{241e}')
                .filter(|c| !c.is_empty())
                .map(|c| {
                    let mut it = c.splitn(5, ',');
                    CellSig {
                        row: it.next().unwrap().parse().unwrap(),
                        col: it.next().unwrap().parse().unwrap(),
                        width: it.next().unwrap().parse().unwrap(),
                        shade: it.next().unwrap().parse().unwrap(),
                        text: norm(it.next().unwrap()),
                    }
                })
                .collect()
        })
        .collect();
    (paras, tables)
}

/// Model indices of the paragraphs Word's `Paragraphs` lists: it leaves out the paragraphs of a
/// vertical merge's continuation cells.
fn visible_paragraphs(doc: &Document) -> Vec<usize> {
    fn walk(blocks: &[Block], hidden: bool, next: &mut usize, out: &mut Vec<usize>) {
        for b in blocks {
            match b {
                Block::Paragraph(_) => {
                    if !hidden {
                        out.push(*next);
                    }
                    *next += 1;
                }
                Block::Table(t) => {
                    for row in t.rows() {
                        for cell in row.cells() {
                            let cont = cell.properties().v_merge == VerticalMerge::Continue;
                            walk(&cell.blocks, hidden || cont, next, out);
                        }
                    }
                }
                Block::Sdt(s) => {
                    for c in s.content() {
                        walk(&c.blocks, hidden, next, out);
                    }
                }
                Block::Raw(_) => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(&doc.body().blocks, false, &mut 0, &mut out);
    out
}

fn model_state(doc: &Document) -> State {
    let styles = doc.styles();
    let paras = visible_paragraphs(doc)
        .into_iter()
        .map(|i| {
            let p = doc.paragraph(i).unwrap();
            let align = match styles.map(|s| s.effective_para_props(p).alignment) {
                Some(Alignment::Center) => 1,
                Some(Alignment::Right) => 2,
                Some(Alignment::Justify) => 3,
                _ => 0,
            };
            let mut runs: Vec<(String, usize)> = Vec::new();
            for r in doc.paragraph_format_runs(i, TextView::Markup).unwrap() {
                let f = &r.format;
                let key = format!(
                    "B{}I{}U{}S{}",
                    u8::from(f.bold),
                    u8::from(f.italic),
                    u8::from(f.underline),
                    f.size_half_points
                );
                let n = r.text.chars().count();
                match runs.last_mut() {
                    Some(last) if last.0 == key => last.1 += n,
                    _ => runs.push((key, n)),
                }
            }
            ParaSig {
                text: p.markup_text(),
                align,
                style: doc
                    .paragraph_style_name(i)
                    .unwrap_or_default()
                    .to_lowercase(),
                list: doc
                    .numbering()
                    .and_then(|n| n.paragraph_numbering(styles, p))
                    .is_some(),
                runs,
            }
        })
        .collect();
    let mut tables = Vec::new();
    collect_tables(&doc.body().blocks, &mut tables);
    (paras, tables)
}

fn collect_tables(blocks: &[Block], out: &mut Vec<Vec<CellSig>>) {
    for b in blocks {
        match b {
            Block::Table(t) => {
                let mut cells = Vec::new();
                let mut ri = 0;
                for child in &t.children {
                    let TableChild::Row(row) = child else {
                        continue;
                    };
                    ri += 1;
                    let mut ci = 0;
                    for rc in &row.children {
                        let RowChild::Cell(cell) = rc else { continue };
                        ci += 1;
                        let props = cell.properties();
                        // Word's Cells leave out the continuation of a vertical merge.
                        if props.v_merge == VerticalMerge::Continue {
                            continue;
                        }
                        let text = cell
                            .blocks
                            .iter()
                            .filter_map(|b| match b {
                                Block::Paragraph(p) => Some(p.markup_text()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        let shade = props
                            .shading
                            .and_then(|s| s.fill)
                            .map_or(-16_777_216, |[r, g, b]| {
                                i64::from(r) + i64::from(g) * 256 + i64::from(b) * 65_536
                            });
                        cells.push(CellSig {
                            row: ri,
                            col: ci,
                            width: match props.width {
                                Some(TableWidth::Points(p)) => p,
                                _ => -1.0,
                            },
                            shade,
                            text,
                        });
                    }
                }
                out.push(cells);
                // Tables nested in cells are not generated by the oracle.
            }
            Block::Sdt(_) | Block::Paragraph(_) | Block::Raw(_) => {}
        }
    }
}

fn state_diff(label: &str, ours: &State, theirs: &State) -> Option<String> {
    if ours.0.len() != theirs.0.len() {
        return Some(format!(
            "{label}: {} paragraphs, Word {}",
            ours.0.len(),
            theirs.0.len()
        ));
    }
    for (i, (a, b)) in ours.0.iter().zip(&theirs.0).enumerate() {
        if a != b {
            return Some(format!(
                "{label}: paragraph {i}\n   ours {a:?}\n   word {b:?}"
            ));
        }
    }
    if ours.1.len() != theirs.1.len() {
        return Some(format!(
            "{label}: {} tables, Word {}",
            ours.1.len(),
            theirs.1.len()
        ));
    }
    for (t, (a, b)) in ours.1.iter().zip(&theirs.1).enumerate() {
        let same = a.len() == b.len()
            && a.iter().zip(b).all(|(x, y)| {
                x.row == y.row
                    && x.col == y.col
                    && x.text == y.text
                    && x.shade == y.shade
                    && (x.width - y.width).abs() < 0.15
            });
        if !same {
            return Some(format!("{label}: table {t}\n   ours {a:?}\n   word {b:?}"));
        }
    }
    None
}

/// (fixture, label prefix) pairs where the model is known not to reach Word's state yet.
const KNOWN_GAPS: &[(&str, &str)] = &[];

fn is_known_gap(name: &str, label: &str) -> bool {
    KNOWN_GAPS.iter().any(|(n, p)| *n == name && label == *p)
}

fn word_type(k: RevisionKind) -> &'static str {
    k.word_type_name()
}

/// Measures every fixture; returns (compared, matched, failure messages).
fn run(only: Option<&str>) -> (usize, usize, Vec<String>) {
    let mut compared = 0;
    let mut matched = 0;
    let mut failures = Vec::new();
    for name in names() {
        if only.is_some_and(|o| o != name) {
            continue;
        }
        let r = reference(&name);
        let labels: Vec<String> = {
            let mut seen = Vec::new();
            for l in &r {
                if (l[0] == "para" || l[0] == "tbl") && !seen.contains(&l[1]) {
                    seen.push(l[1].clone());
                }
            }
            seen
        };
        for label in labels {
            if label == "final" {
                continue;
            }
            let mut doc = open(&name);
            let revs = doc.revisions();
            let outcome = if label == "markup" {
                Ok(())
            } else if label == "accept" {
                doc.accept_all_revisions();
                Ok(())
            } else if label == "reject" {
                doc.reject_all_revisions();
                Ok(())
            } else if let Some(i) = label.strip_prefix("accept1:") {
                doc.accept_revision(i.parse().unwrap())
            } else if let Some(i) = label.strip_prefix("reject1:") {
                doc.reject_revision(i.parse().unwrap())
            } else if let Some(i) = label.strip_prefix("acceptp:") {
                let i = visible_paragraphs(&doc)[i.parse::<usize>().unwrap()];
                doc.accept_revisions_in_paragraphs(i, i).map(drop)
            } else if let Some(i) = label.strip_prefix("rejectp:") {
                let i = visible_paragraphs(&doc)[i.parse::<usize>().unwrap()];
                doc.reject_revisions_in_paragraphs(i, i).map(drop)
            } else {
                panic!("unknown label {label}")
            };
            compared += 1;
            let diff = match outcome {
                Err(e) => Some(format!("{label}: {e} ({} revisions)", revs.len())),
                Ok(()) => state_diff(&label, &model_state(&doc), &ref_state(&r, &label)),
            };
            match diff {
                None if is_known_gap(&name, &label) => {
                    failures.push(format!("{name}: known gap {label} now matches; remove it"));
                }
                None => matched += 1,
                Some(_) if is_known_gap(&name, &label) => matched += 1,
                Some(d) => failures.push(format!("{name}: {d}")),
            }
        }
    }
    (compared, matched, failures)
}

#[test]
fn accept_and_reject_states_match_word() {
    let (compared, matched, failures) = run(None);
    println!("states compared {compared}, matched {matched}");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert!(compared >= 100, "compared {compared}");
}

#[test]
fn revision_lists_match_word() {
    let mut compared = 0;
    let mut failures = Vec::new();
    for name in names() {
        let r = reference(&name);
        let doc = open(&name);
        let ours = doc.revisions();
        let theirs: Vec<&Vec<String>> = r.iter().filter(|l| l[0] == "rev").collect();
        let a: Vec<(String, String, Option<String>)> = ours
            .iter()
            .map(|v| {
                (
                    word_type(v.kind).to_owned(),
                    v.author.clone(),
                    v.date.clone(),
                )
            })
            .collect();
        let b: Vec<(String, String, Option<String>)> = theirs
            .iter()
            .map(|l| (l[2].clone(), l[3].clone(), Some(l[4].clone())))
            .collect();
        compared += b.len();
        if a != b {
            failures.push(format!("{name}: revisions\n   ours {a:?}\n   word {b:?}"));
            continue;
        }
        for (v, l) in ours.iter().zip(&theirs) {
            if matches!(
                v.kind,
                RevisionKind::Insertion
                    | RevisionKind::Deletion
                    | RevisionKind::MovedFrom
                    | RevisionKind::MovedTo
            ) && v.text != norm(&l[5])
            {
                failures.push(format!(
                    "{name}: revision {} text {:?} vs {:?}",
                    v.index, v.text, l[5]
                ));
            }
        }
    }
    println!("revisions compared {compared}");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn comments_match_word() {
    let mut compared = 0;
    for name in names() {
        let r = reference(&name);
        let doc = open(&name);
        let ours = doc.comments();
        let theirs: Vec<&Vec<String>> = r.iter().filter(|l| l[0] == "comment").collect();
        assert_eq!(ours.len(), theirs.len(), "{name}: comment count");
        let by_index: BTreeMap<usize, u32> =
            ours.iter().enumerate().map(|(i, c)| (i, c.id)).collect();
        for (i, (c, l)) in ours.iter().zip(&theirs).enumerate() {
            assert_eq!(c.author, l[2], "{name}: comment {i} author");
            assert_eq!(c.initials, l[3], "{name}: comment {i} initials");
            // Word's Scope.Text still contains deleted text, and so does `scope_text`.
            assert_eq!(
                norm(&c.scope_text),
                norm(&l[4]),
                "{name}: comment {i} scope"
            );
            assert_eq!(c.text, l[5].replace('\r', "\n"), "{name}: comment {i} text");
            assert_eq!(c.resolved, l[6] == "1", "{name}: comment {i} done");
            let anc: i64 = l[7].parse().unwrap();
            let parent = c
                .parent
                .and_then(|p| by_index.iter().find(|(_, id)| **id == p).map(|(i, _)| *i));
            assert_eq!(
                parent.map_or(-1, |i| i as i64),
                anc,
                "{name}: comment {i} parent"
            );
            let replies: usize = l[8].parse().unwrap();
            assert_eq!(
                doc.comment_replies(c.id).len(),
                replies,
                "{name}: comment {i} replies"
            );
            compared += 1;
        }
    }
    assert!(compared >= 10, "compared {compared}");
}

#[test]
fn comment_parts_round_trip_and_people_follow_new_authors() {
    use open365_docx::{DocPos, NewComment};
    let parts = [
        "word/comments.xml",
        "word/commentsExtended.xml",
        "word/commentsIds.xml",
        "word/commentsExtensible.xml",
        "word/people.xml",
    ];
    for name in ["cmt-basic", "cmt-ranges", "cmt-tracked"] {
        let original = open(name);
        let mut copy = open(name);
        let saved = copy.save().unwrap();
        let again = Document::open(&saved, &Limits::default()).unwrap();
        for part in parts {
            let a = original.package().part(part).map(|p| p.data().to_vec());
            let b = again.package().part(part).map(|p| p.data().to_vec());
            assert!(a.is_some(), "{name}: Word wrote {part}");
            assert_eq!(a, b, "{name}: {part} unchanged by an open and save");
        }
        assert_eq!(original.comments(), again.comments());
        assert_eq!(original.people(), again.people());
        assert_eq!(original.people()[0].author, "Author A");
    }
    let mut doc = open("cmt-basic");
    let id = doc
        .add_comment(
            DocPos { para: 2, offset: 0 },
            DocPos { para: 2, offset: 5 },
            &NewComment::new("Author B", "New comment"),
        )
        .unwrap();
    let saved = doc.save().unwrap();
    let again = Document::open(&saved, &Limits::default()).unwrap();
    assert!(again.comment(id).is_some());
    let authors: Vec<String> = again.people().into_iter().map(|p| p.author).collect();
    assert_eq!(authors, ["Author A", "Author B"]);
    // The other comment parts gained exactly one entry each.
    assert_eq!(again.comments().len(), original_count("cmt-basic") + 1);
}

fn original_count(name: &str) -> usize {
    open(name).comments().len()
}
