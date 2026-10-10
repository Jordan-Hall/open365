//! A scripted editing session (type, Enter, Backspace across a paragraph boundary, bold and
//! italic over a range, style change, alignment) applied to every Word-made fixture. The
//! results are written to `target/docx-edited2/` for the Word oracle, and the whole session
//! is undone through snapshots.

mod common;

use open365_docx::{Alignment, DocPos, Document, RunPropsPatch};
use open365_opc::{Limits, Package};

use common::{assert_infoset_eq, fixtures, repo_root};

fn open(bytes: &[u8]) -> Document {
    Document::open(bytes, &Limits::default()).expect("fixture opens")
}

/// Index of the first paragraph with text, so edits land on visible content.
fn first_with_text(doc: &Document) -> usize {
    (0..doc.paragraph_count())
        .find(|&i| doc.paragraph(i).is_some_and(|p| p.char_len() > 3))
        .unwrap_or(0)
}

#[test]
fn scripted_session_on_every_fixture_saves_reopens_and_undoes() {
    let out_dir = repo_root().join("target").join("docx-edited2");
    std::fs::create_dir_all(&out_dir).expect("output dir");
    let mut joined_somewhere = false;

    for (name, bytes) in fixtures() {
        let mut doc = open(&bytes);
        let original_main = doc.serialize_main();
        let count_before = doc.paragraph_count();
        let target = first_with_text(&doc);
        let original_len = doc.paragraph(target).expect("target").char_len();
        let mut snapshots = Vec::new();

        // 1. Type at the start of the paragraph.
        snapshots.push(doc.capture(target, target).expect("capture"));
        let caret = doc
            .replace_range(DocPos::new(target, 0), DocPos::new(target, 0), "Typed: ")
            .expect("type");
        assert_eq!(caret, DocPos::new(target, 7), "{name}");

        // 2. Enter after the typed text.
        snapshots.push(doc.capture(target, target).expect("capture"));
        doc.split_paragraph(caret).expect("enter");
        assert_eq!(doc.paragraph_count(), count_before + 1, "{name}");
        assert_eq!(doc.paragraph_text(target).expect("p"), "Typed: ", "{name}");

        // 3. Bold and italic over the first characters of the second half.
        let second_len = doc.paragraph(target + 1).expect("p").char_len();
        let end = second_len.min(3);
        snapshots.push(doc.capture(target + 1, target + 1).expect("capture"));
        doc.set_run_props(
            DocPos::new(target + 1, 0),
            DocPos::new(target + 1, end),
            &RunPropsPatch {
                bold: Some(true),
                italic: Some(true),
                ..RunPropsPatch::default()
            },
        )
        .expect("format");

        // 4. Style and alignment of the first half.
        snapshots.push(doc.capture(target, target).expect("capture"));
        doc.set_paragraph_style(target, "Heading2").expect("style");
        doc.set_alignment(target, Alignment::Center).expect("align");

        // 5. Backspace across the boundary of two paragraphs in one container.
        let mut joined = None;
        for i in (target + 1)..doc.paragraph_count().saturating_sub(1) {
            let len = doc.paragraph(i).expect("p").char_len();
            let Ok(snapshot) = doc.capture(i, i + 1) else {
                continue;
            };
            if doc
                .delete_range(DocPos::new(i, len), DocPos::new(i + 1, 0))
                .is_ok()
            {
                joined = Some((snapshot, i));
                break;
            }
        }
        if let Some((snapshot, i)) = joined {
            joined_somewhere = true;
            assert!(doc.paragraph(i).is_some(), "{name}");
            snapshots.push(snapshot);
        }
        let paragraphs_after = doc.paragraph_count();

        // Save, reopen, check.
        let saved = doc.save().expect("save");
        std::fs::write(out_dir.join(format!("{name}.docx")), &saved).expect("write output");
        let reopened = open(&saved);
        assert!(reopened.dropped().is_empty(), "{name}");
        assert_eq!(reopened.paragraph_count(), paragraphs_after, "{name}");
        assert_eq!(
            reopened.paragraph_text(target).expect("p"),
            "Typed: ",
            "{name}"
        );
        assert_eq!(
            reopened.paragraph(target).expect("p").style_id(),
            Some("Heading2"),
            "{name}"
        );
        assert_eq!(
            reopened.paragraph(target).expect("p").direct_alignment(),
            Some(Alignment::Center),
            "{name}"
        );
        assert!(
            reopened.styles().is_some_and(|s| s.has_style("Heading2")),
            "{name}: Heading2 must be defined for Word to apply it"
        );
        // Everything except the main part, styles and stale-part hygiene is untouched.
        let before = Package::read(&bytes, &Limits::default()).expect("package");
        let after = Package::read(&saved, &Limits::default()).expect("package");
        let styles_name = doc.styles().map(|s| s.part_name().to_string());
        for part in before.parts() {
            let pname = part.name().to_string();
            let Some(other) = after.part(&pname) else {
                continue;
            };
            if pname == doc.main_part().as_str()
                || Some(&pname) == styles_name.as_ref()
                || pname == "_rels/.rels"
            {
                continue;
            }
            assert_eq!(part.data(), other.data(), "{name}: {pname} changed");
        }

        // Undo the whole session, newest first.
        while let Some(snapshot) = snapshots.pop() {
            doc.restore(&snapshot).expect("undo");
        }
        assert_eq!(doc.paragraph_count(), count_before, "{name}");
        assert_eq!(
            doc.paragraph(target).expect("p").char_len(),
            original_len,
            "{name}"
        );
        if let Err(diff) = assert_infoset_eq(&original_main, &doc.serialize_main()) {
            panic!("{name}: undo did not restore the original: {diff}");
        }
        assert!(doc.edit_log().is_empty(), "{name}");
    }
    assert!(joined_somewhere, "no fixture allowed a paragraph join");
}
