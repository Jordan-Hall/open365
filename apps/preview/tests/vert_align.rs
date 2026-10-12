//! Sub/superscript toggles read the effective state, so a character style that sets
//! superscript counts as superscript, and the toggle turns it off as Word does.

use std::io::Write;

use open365_commands::Target;
use open365_docx::DocPos;
use open365_preview::doc::{Nav, Toggle, WriterPane};

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

fn docx(body: &str, styles: &str) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    let parts = [
        ("[Content_Types].xml", r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/></Types>"#.to_owned()),
        ("_rels/.rels", r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.to_owned()),
        ("word/_rels/document.xml.rels", r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#.to_owned()),
        ("word/styles.xml", format!(r#"<?xml version="1.0" encoding="UTF-8"?><w:styles xmlns:w="{W}">{styles}</w:styles>"#)),
        ("word/document.xml", format!(r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="{W}"><w:body>{body}<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="708" w:footer="708" w:gutter="0"/></w:sectPr></w:body></w:document>"#)),
    ];
    for (name, content) in parts {
        zip.start_file(name, opts).expect("start");
        zip.write_all(content.as_bytes()).expect("write");
    }
    zip.finish().expect("finish").into_inner()
}

/// "plain " then "sup" in a character style `Sup` whose only property is superscript, then " end".
fn pane() -> WriterPane {
    let styles = r#"<w:style w:type="character" w:styleId="Sup"><w:name w:val="Sup"/><w:rPr><w:vertAlign w:val="superscript"/></w:rPr></w:style>"#;
    let body = r#"<w:p><w:r><w:t xml:space="preserve">plain </w:t></w:r><w:r><w:rPr><w:rStyle w:val="Sup"/></w:rPr><w:t>sup</w:t></w:r><w:r><w:t xml:space="preserve"> end</w:t></w:r></w:p>"#;
    WriterPane::new(Target::open_docx(&docx(body, styles)).expect("open"))
}

#[test]
fn caret_in_a_superscript_style_reports_superscript() {
    let mut w = pane();
    w.set_caret(DocPos::new(0, 8));
    assert!(w.toggle_state(Toggle::Sup) && !w.toggle_state(Toggle::Sub));
    w.set_caret(DocPos::new(0, 2));
    assert!(!w.toggle_state(Toggle::Sup));
}

#[test]
fn toggling_style_inherited_superscript_turns_it_off() {
    let mut w = pane();
    w.set_caret(DocPos::new(0, 6));
    for _ in 0..3 {
        w.move_caret(Nav::Right, true, false);
    }
    assert!(w.has_selection());
    w.toggle(Toggle::Sup);
    w.set_caret(DocPos::new(0, 8));
    assert!(
        !w.toggle_state(Toggle::Sup),
        "the selection went back to baseline"
    );
    // A caret typing into style-inherited superscript flips it off too.
    let mut w = pane();
    w.set_caret(DocPos::new(0, 8));
    assert!(w.toggle_state(Toggle::Sup));
    w.toggle(Toggle::Sup);
    assert!(!w.toggle_state(Toggle::Sup));
    // Subscript replaces the inherited superscript.
    w.toggle(Toggle::Sub);
    assert!(w.toggle_state(Toggle::Sub) && !w.toggle_state(Toggle::Sup));
}
