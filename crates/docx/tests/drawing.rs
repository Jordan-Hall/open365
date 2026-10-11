//! Typed access to `w:drawing`: inline and anchored pictures, wrapping, positions, image parts.

mod common;

use open365_docx::{
    Align, Document, HorizontalRelativeFrom, Inline, Position, VerticalRelativeFrom, WrapSide,
    WrapType,
};
use open365_opc::Limits;

use common::repo_root;

fn open_fixture(name: &str) -> Document {
    let path = repo_root()
        .join("fixtures/docx")
        .join(format!("{name}.docx"));
    let bytes = std::fs::read(path).expect("fixture");
    Document::open(&bytes, &Limits::default()).expect("opens")
}

fn drawings(doc: &Document) -> Vec<open365_docx::Drawing> {
    fn walk(content: &[Inline], out: &mut Vec<open365_docx::Drawing>) {
        for inline in content {
            match inline {
                Inline::Run(r) => out.extend(r.content.iter().filter_map(|c| c.drawing())),
                Inline::Group(g) => walk(&g.content, out),
                Inline::Raw(_) => {}
            }
        }
    }
    let mut out = Vec::new();
    for p in doc.body().paragraphs() {
        walk(&p.content, &mut out);
    }
    out
}

#[test]
fn inline_picture_of_the_word_fixture() {
    let doc = open_fixture("inline-image");
    let d = drawings(&doc);
    assert_eq!(d.len(), 1);
    let d = &d[0];
    assert!(!d.anchored);
    assert_eq!(d.wrap, WrapType::Inline);
    assert_eq!(d.extent_emu, (304_843, 304_843));
    assert!((d.size_pt().0 - 24.0).abs() < 0.01);
    assert_eq!(d.effect_extent_emu, [0; 4]);
    assert_eq!(d.distance_emu, [0; 4]);
    let embed = d.embed.as_deref().expect("blip embed");
    let part = doc.image_part(embed).expect("image part");
    assert_eq!(part.content_type, "image/png");
    assert!(part.bytes.starts_with(b"\x89PNG"));
    assert!(doc.image_part("rId999").is_none());
}

#[test]
fn anchored_pictures_of_the_word_fixtures() {
    let tb = drawings(&open_fixture("layout-img-tb"));
    assert_eq!(tb.len(), 4);
    assert!(
        tb.iter()
            .all(|d| d.anchored && d.wrap == WrapType::TopAndBottom)
    );
    assert_eq!(tb[1].horizontal.1, Position::Align(Align::Left));
    assert_eq!(tb[0].horizontal.0, HorizontalRelativeFrom::Page);
    assert_eq!(tb[0].vertical.0, VerticalRelativeFrom::Page);
    assert!(matches!(tb[0].vertical.1, Position::Offset(o) if o > 0));
    // Distances set in Word: top 14 pt, bottom 20 pt.
    let d = tb[3].distance_pt();
    assert!(
        (d[0] - 14.0).abs() < 0.1 && (d[1] - 20.0).abs() < 0.1,
        "{d:?}"
    );

    let sq = drawings(&open_fixture("layout-img-square"));
    assert_eq!(sq.len(), 6);
    assert_eq!(sq[0].wrap, WrapType::Square);
    assert_eq!(sq[4].wrap, WrapType::Tight);
    assert_eq!(sq[5].wrap_side, WrapSide::Largest);
    assert!(sq.iter().all(|d| d.embed.is_some()));

    let none = drawings(&open_fixture("layout-img-none"));
    assert_eq!(none.len(), 2);
    assert!(none.iter().all(|d| d.wrap == WrapType::None));
    assert!(none[1].behind_text);
    assert!(!none[0].behind_text);
}

#[test]
fn alt_text_and_hostile_numbers() {
    let body = r#"<w:p><w:r><w:drawing><wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" distT="99999999999999999999" distB="-5" behindDoc="1"><wp:positionH relativeFrom="margin"><wp:posOffset>9223372036854775807</wp:posOffset></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:align>center</wp:align></wp:positionV><wp:extent cx="-4" cy="914400"/><wp:wrapSquare wrapText="left" distL="12700"/><wp:docPr id="7" name="Pic" descr="A red square"/><a:graphic><a:graphicData><a:blip r:embed="rId9"/></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r></w:p>"#;
    let bytes = common::docx_with_body(body);
    let doc = Document::open(&bytes, &Limits::default()).expect("opens");
    let d = drawings(&doc);
    assert_eq!(d.len(), 1);
    let d = &d[0];
    assert_eq!(d.description.as_deref(), Some("A red square"));
    assert_eq!(d.extent_emu, (0, 914_400));
    assert_eq!(d.distance_emu[0], 0, "unparsable distance reads as zero");
    assert_eq!(d.distance_emu[2], 12_700);
    assert_eq!(d.wrap, WrapType::Square);
    assert_eq!(d.wrap_side, WrapSide::Left);
    assert!(d.behind_text);
    assert_eq!(d.horizontal.0, HorizontalRelativeFrom::Margin);
    assert!(matches!(d.horizontal.1, Position::Offset(o) if o <= 2_000_000_000));
    assert_eq!(d.vertical.1, Position::Align(Align::Center));
    assert_eq!(d.embed.as_deref(), Some("rId9"));
    assert!(doc.image_part("rId9").is_none());
}
