//! Invariants over every Word-made fixture in `fixtures/docx`.

mod common;

use std::collections::HashSet;

use open365_docx::{Document, HygieneAction, Inline, Paragraph, Run};
use open365_opc::{Limits, Package};

use common::{assert_infoset_eq, fixtures, repo_root};

const INSERTED: &str = "Edited: ";

fn open(bytes: &[u8]) -> Document {
    Document::open(bytes, &Limits::default()).expect("fixture opens")
}

fn bold_runs(paragraph: &Paragraph) -> Vec<&Run> {
    fn walk<'a>(content: &'a [Inline], out: &mut Vec<&'a Run>) {
        for inline in content {
            match inline {
                Inline::Run(r) if r.props.bold() == Some(true) => out.push(r),
                Inline::Group(g) => walk(&g.content, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(&paragraph.content, &mut out);
    out
}

#[test]
fn fixtures_exist() {
    assert!(fixtures().len() >= 14, "expected the Word-made fixture set");
}

#[test]
fn open_succeeds_and_nothing_is_dropped() {
    for (name, bytes) in fixtures() {
        let doc = open(&bytes);
        assert!(doc.dropped().is_empty(), "{name}: {:?}", doc.dropped());
        assert!(doc.paragraph_count() > 0, "{name}: no paragraphs");
    }
}

#[test]
fn unedited_main_part_reserialises_to_an_equal_infoset() {
    for (name, bytes) in fixtures() {
        let doc = open(&bytes);
        let original = Package::read(&bytes, &Limits::default()).expect("package");
        let original = original.part(doc.main_part().as_str()).expect("main part");
        let rewritten = doc.serialize_main();
        if let Err(diff) = assert_infoset_eq(original.data(), &rewritten) {
            panic!("{name}: {diff}");
        }
    }
}

#[test]
fn unedited_save_keeps_every_part_byte_identical() {
    for (name, bytes) in fixtures() {
        let mut doc = open(&bytes);
        let saved = doc.save().expect("save");
        let before = Package::read(&bytes, &Limits::default()).expect("package");
        let after = Package::read(&saved, &Limits::default()).expect("package");
        let names = |p: &Package| -> Vec<String> {
            p.parts().map(|part| part.name().to_string()).collect()
        };
        assert_eq!(names(&before), names(&after), "{name}: part list changed");
        for part in before.parts() {
            let other = after.part(part.name().as_str()).expect("part present");
            assert_eq!(part.data(), other.data(), "{name}: {} changed", part.name());
        }
    }
}

#[test]
fn edit_save_reopen_keeps_everything_else() {
    let out_dir = repo_root().join("target").join("docx-edited");
    std::fs::create_dir_all(&out_dir).expect("output dir");

    for (name, bytes) in fixtures() {
        let mut doc = open(&bytes);
        let main = doc.main_part().to_string();
        let texts_before: Vec<String> = (0..doc.paragraph_count())
            .map(|i| doc.paragraph_text(i).expect("paragraph"))
            .collect();

        doc.insert_text(0, 0, INSERTED).expect("insert");
        let bold_len = "Edited".chars().count();
        doc.set_bold(0, 0..bold_len, true).expect("bold");
        let saved = doc.save().expect("save");
        std::fs::write(out_dir.join(format!("{name}.docx")), &saved).expect("write output");

        // Reopen and check the edit.
        let reopened = open(&saved);
        assert!(reopened.dropped().is_empty(), "{name}");
        assert_eq!(reopened.paragraph_count(), texts_before.len(), "{name}");
        assert_eq!(
            reopened.paragraph_text(0).expect("p0"),
            format!("{INSERTED}{}", texts_before[0]),
            "{name}"
        );
        for (i, before) in texts_before.iter().enumerate().skip(1) {
            assert_eq!(
                &reopened.paragraph_text(i).expect("paragraph"),
                before,
                "{name} p{i}"
            );
        }
        let paragraph = reopened.paragraph(0).expect("p0");
        let bold: Vec<String> = bold_runs(paragraph).iter().map(|r| r.text()).collect();
        assert!(
            bold.iter().any(|t| t == "Edited"),
            "{name}: bold runs are {bold:?}"
        );

        // Every other part is byte-identical, apart from stale parts hygiene removed.
        let before = Package::read(&bytes, &Limits::default()).expect("package");
        let after = Package::read(&saved, &Limits::default()).expect("package");
        let removed: HashSet<String> = doc
            .hygiene_log()
            .iter()
            .filter_map(|a| match a {
                HygieneAction::RemovedPart { name, .. } => Some(name.clone()),
                HygieneAction::RemovedRelationship { .. } => None,
            })
            .collect();
        let rels_edited = !doc.hygiene_log().is_empty();
        for part in before.parts() {
            let pname = part.name().to_string();
            if removed.contains(&pname) {
                assert!(
                    after.part(&pname).is_none(),
                    "{name}: {pname} should be gone"
                );
                continue;
            }
            let other = after
                .part(&pname)
                .unwrap_or_else(|| panic!("{name}: {pname} missing after save"));
            if pname == main || (rels_edited && pname == "_rels/.rels") {
                continue;
            }
            assert_eq!(part.data(), other.data(), "{name}: {pname} changed");
        }
        assert_eq!(
            after.parts().count(),
            before.parts().count() - removed.len(),
            "{name}: part count"
        );

        // No integrity issue that the original did not have.
        let old_issues = before.check_integrity();
        for issue in after.check_integrity() {
            assert!(old_issues.contains(&issue), "{name}: new issue {issue:?}");
        }
    }
}
