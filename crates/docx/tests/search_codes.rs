//! Find and Replace special codes, wildcard groups, one-match replacement and section margins.

mod common;

use open365_docx::{DocMatch, DocPos, Document, FindError, FindOptions, MarginEdit};
use open365_opc::Limits;

use common::docx_with_body;

fn doc(body: &str) -> Document {
    Document::open(&docx_with_body(body), &Limits::default()).expect("opens")
}

fn p(text: &str) -> String {
    format!("<w:p><w:r><w:t>{text}</w:t></w:r></w:p>")
}

fn paras(texts: &[&str]) -> Document {
    doc(&texts.iter().map(|t| p(t)).collect::<String>())
}

fn texts(d: &Document) -> Vec<String> {
    (0..d.paragraph_count())
        .map(|i| d.paragraph_text(i).expect("paragraph"))
        .collect()
}

fn xml(d: &Document) -> String {
    String::from_utf8(d.serialize_main()).expect("utf8")
}

fn find(pattern: &str) -> FindOptions {
    FindOptions {
        pattern: pattern.to_owned(),
        ..FindOptions::default()
    }
}

fn wild(pattern: &str) -> FindOptions {
    FindOptions {
        wildcards: true,
        ..find(pattern)
    }
}

#[test]
fn caret_p_finds_paragraph_marks_and_spans_paragraphs() {
    let d = paras(&["one", "two", "three"]);
    let all = d.find_all(&find("^p")).unwrap();
    assert_eq!(all.len(), 3);
    assert_eq!(
        all[0],
        DocMatch {
            para: 0,
            start: 3,
            end: 0,
            end_para: 1
        }
    );
    let two = d.find_all(&find("e^pt")).unwrap();
    assert_eq!(two.len(), 1);
    assert_eq!((two[0].para, two[0].start), (0, 2));
    assert_eq!((two[0].end_para, two[0].end), (1, 1));
}

#[test]
fn replace_all_joins_paragraphs_when_the_mark_goes() {
    let mut d = paras(&["one", "two", "three"]);
    // The document's last mark cannot go; the other two join their paragraphs.
    assert_eq!(d.replace_all(&find("^p"), " ").unwrap(), 2);
    assert_eq!(texts(&d), ["one two three"]);
}

#[test]
fn replace_with_caret_p_splits_paragraphs() {
    let mut d = paras(&["a,b,c", "d"]);
    assert_eq!(d.replace_all(&find(","), "^p").unwrap(), 2);
    assert_eq!(texts(&d), ["a", "b", "c", "d"]);
}

#[test]
fn replace_with_line_and_page_breaks() {
    let mut d = paras(&["a;b"]);
    d.replace_all(&find(";"), "^l").unwrap();
    assert_eq!(texts(&d), ["a\nb"]);
    assert!(!xml(&d).contains("w:type=\"page\""));

    let mut d = paras(&["a;b"]);
    d.replace_all(&find(";"), "^m").unwrap();
    assert!(xml(&d).contains("w:type=\"page\""), "{}", xml(&d));
    // The page break is found again by ^m, and a line break is not.
    assert_eq!(d.find_all(&find("^m")).unwrap().len(), 1);
    assert_eq!(d.find_all(&find("^l")).unwrap().len(), 0);

    let mut d = paras(&["a;b"]);
    d.replace_all(&find(";"), "^n").unwrap();
    assert!(xml(&d).contains("w:type=\"column\""));
}

#[test]
fn hyphen_and_space_codes() {
    let mut d = paras(&["a-b c"]);
    d.replace_all(&find("-"), "^~").unwrap();
    d.replace_all(&find(" "), "^s").unwrap();
    assert_eq!(texts(&d), ["a\u{2011}b\u{a0}c"]);
    assert_eq!(d.find_all(&find("^~")).unwrap().len(), 1);
    assert_eq!(d.find_all(&find("^s")).unwrap().len(), 1);
    d.replace_all(&find("^s"), "^-").unwrap();
    assert_eq!(texts(&d), ["a\u{2011}b\u{ad}c"]);
}

#[test]
fn any_digit_letter_and_whitespace_codes() {
    let d = paras(&["ab 12 c3"]);
    assert_eq!(d.find_all(&find("^#")).unwrap().len(), 3);
    assert_eq!(d.find_all(&find("^$")).unwrap().len(), 3);
    assert_eq!(d.find_all(&find("^$^$")).unwrap().len(), 1);
    assert_eq!(d.find_all(&find("c^?")).unwrap().len(), 1);
    // Two spaces and the paragraph mark.
    assert_eq!(d.find_all(&find("^w")).unwrap().len(), 3);
}

#[test]
fn unsupported_codes_are_errors() {
    let d = paras(&["a"]);
    for code in ["^f", "^e", "^d", "^q"] {
        assert!(matches!(
            d.find_all(&find(code)),
            Err(FindError::Pattern(_))
        ));
    }
    let mut d = paras(&["a"]);
    for rep in ["^q", "^f", "x^"] {
        assert!(matches!(
            d.replace_all(&find("a"), rep),
            Err(FindError::Replacement(_))
        ));
    }
    assert_eq!(texts(&d), ["a"]);
}

#[test]
fn clipboard_code_in_the_replacement() {
    let mut d = paras(&["x"]);
    assert!(d.replace_all(&find("x"), "^c").is_err());
    assert_eq!(
        d.replace_all_with_clipboard(&find("x"), "[^c]", Some("one\r\ntwo"))
            .unwrap(),
        1
    );
    assert_eq!(texts(&d), ["[one", "two]"]);
}

#[test]
fn wildcard_groups_and_back_references() {
    let mut d = paras(&["John Smith", "Jane Doe"]);
    let n = d
        .replace_all(&wild("(<[A-Z][a-z]@>) (<[A-Z][a-z]@>)"), "\\2, \\1")
        .unwrap();
    assert_eq!(n, 2);
    assert_eq!(texts(&d), ["Smith, John", "Doe, Jane"]);

    let d = paras(&["abab abba"]);
    assert_eq!(d.find_all(&wild("(ab)\\1")).unwrap().len(), 1);
}

#[test]
fn bad_wildcard_patterns_and_replacements_error() {
    let mut d = paras(&["abc"]);
    for bad in ["(a", "a)", "((a))", "\\1", "[b-a]", "a{2", "()"] {
        let e = d.find_all(&wild(bad)).unwrap_err();
        assert!(
            e.to_string()
                .contains("Pattern Match expression which is not valid"),
            "{bad}: {e}"
        );
    }
    let e = d.replace_all(&wild("(a)"), "\\2").unwrap_err();
    assert!(matches!(e, FindError::Replacement(_)));
    assert_eq!(texts(&d), ["abc"]);
}

#[test]
fn wildcard_star_crosses_paragraphs_like_word() {
    let mut d = paras(&["start", "middle", "end"]);
    assert_eq!(d.replace_all(&wild("s*d"), "X").unwrap(), 1);
    assert_eq!(texts(&d), ["Xdle", "end"]);
}

#[test]
fn replace_match_replaces_only_that_match() {
    let mut d = paras(&["cat cat cat"]);
    let opts = find("cat");
    let all = d.find_all(&opts).unwrap();
    let caret = d.replace_match(&opts, "dog", None, &all[1]).unwrap();
    assert_eq!(texts(&d), ["cat dog cat"]);
    assert_eq!(caret, DocPos::new(0, 7));
}

#[test]
fn replace_match_splits_with_caret_p() {
    let mut d = paras(&["a b c"]);
    let opts = find(" ");
    let all = d.find_all(&opts).unwrap();
    let caret = d.replace_match(&opts, "^p", None, &all[0]).unwrap();
    assert_eq!(texts(&d), ["a", "b c"]);
    assert_eq!(caret, DocPos::new(1, 0));
    // Line break variant.
    let all = d.find_all(&opts).unwrap();
    d.replace_match(&opts, "^l", None, &all[0]).unwrap();
    assert_eq!(texts(&d), ["a", "b\nc"]);
}

#[test]
fn replace_match_with_groups_and_a_stale_match() {
    let mut d = paras(&["ab cd"]);
    let opts = wild("(<?@>) (<?@>)");
    let all = d.find_all(&opts).unwrap();
    d.replace_match(&opts, "\\2 \\1", None, &all[0]).unwrap();
    assert_eq!(texts(&d), ["cd ab"]);
    // The range now holds other text: the match is refused and nothing changes.
    let stale = DocMatch {
        para: 0,
        start: 1,
        end: 4,
        end_para: 0,
    };
    assert!(d.replace_match(&opts, "x", None, &stale).is_err());
    assert_eq!(texts(&d), ["cd ab"]);
}

#[test]
fn replace_match_across_paragraphs() {
    let mut d = paras(&["one", "two"]);
    let opts = find("e^pt");
    let all = d.find_all(&opts).unwrap();
    d.replace_match(&opts, "-", None, &all[0]).unwrap();
    assert_eq!(texts(&d), ["on-wo"]);
}

#[test]
fn replace_match_keeps_the_first_run_format_and_found_copies() {
    let mut d = doc(
        "<w:p><w:r><w:rPr><w:b/></w:rPr><w:t>bold</w:t></w:r><w:r><w:t> plain</w:t></w:r></w:p>",
    );
    let opts = find("bold");
    let all = d.find_all(&opts).unwrap();
    d.replace_match(&opts, "very ^&", None, &all[0]).unwrap();
    assert_eq!(texts(&d), ["very bold plain"]);
    let x = xml(&d);
    assert!(x.contains("<w:b/>"), "{x}");
}

#[test]
fn section_margins_set_every_value_and_validate() {
    let mut d = paras(&["a"]);
    d.set_section_margins(
        0,
        &MarginEdit {
            top: Some(1000),
            bottom: Some(1100),
            left: Some(1200),
            right: Some(1300),
            gutter: Some(100),
            header: Some(500),
            footer: Some(600),
        },
    )
    .unwrap();
    let m = d.sections()[0].margins;
    assert_eq!(
        (
            m.top_twips,
            m.bottom_twips,
            m.left_twips,
            m.right_twips,
            m.gutter_twips,
            m.header_twips,
            m.footer_twips
        ),
        (1000, 1100, 1200, 1300, 100, 500, 600)
    );
    // Only the left margin moves (a ruler drag); the rest stays.
    d.set_section_margins(
        0,
        &MarginEdit {
            left: Some(2000),
            ..MarginEdit::default()
        },
    )
    .unwrap();
    let m = d.sections()[0].margins;
    assert_eq!(
        (m.left_twips, m.right_twips, m.top_twips),
        (2000, 1300, 1000)
    );
    // Refused edits change nothing.
    for bad in [
        MarginEdit {
            left: Some(-1),
            ..MarginEdit::default()
        },
        MarginEdit {
            left: Some(12000),
            ..MarginEdit::default()
        },
        MarginEdit {
            top: Some(31000),
            ..MarginEdit::default()
        },
    ] {
        assert!(d.set_section_margins(0, &bad).is_err());
    }
    assert!(d.set_section_margins(3, &MarginEdit::default()).is_err());
    assert_eq!(d.sections()[0].margins.left_twips, 2000);
}
