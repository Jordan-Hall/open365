//! Page selection, copies, N-up and the ink map on a PDF built here with `lopdf`.

use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, Stream, dictionary};
use open365_print::{InkItem, PdfSource, compose};

/// A PDF of `n` pages of `w` by `h` points; page `k` shows the text "Page k" and a filled box
/// whose width is `10 * k` points, so a page is told apart by its ink.
fn sample(n: usize, w: f32, h: f32) -> Vec<u8> {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let font = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
    });
    let resources = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font } });
    let mut kids = Vec::new();
    for k in 1..=n {
        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 12.into()]),
                Operation::new("Td", vec![72.into(), 700.into()]),
                Operation::new("Tj", vec![Object::string_literal(format!("Page {k}"))]),
                Operation::new("ET", vec![]),
                Operation::new(
                    "re",
                    vec![100.into(), 100.into(), (10 * k as i64).into(), 20.into()],
                ),
                Operation::new("f", vec![]),
            ],
        };
        let cid = doc.add_object(Stream::new(
            Dictionary::new(),
            content.encode().expect("encode"),
        ));
        let page = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), w.into(), h.into()],
            "Resources" => resources, "Contents" => cid,
        });
        kids.push(Object::Reference(page));
    }
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages", "Kids" => kids, "Count" => n as i64,
        }),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    let mut out = Vec::new();
    doc.save_to(&mut out).expect("save");
    out
}

fn box_width(src: &PdfSource, page: usize) -> f64 {
    let ink = src.ink(page).expect("ink");
    ink.items
        .iter()
        .find_map(|i| match i {
            InkItem::Rect { w, .. } => Some(*w),
            _ => None,
        })
        .expect("a filled box")
}

#[test]
fn reads_sizes_and_ink() {
    let src = PdfSource::load(&sample(3, 595.0, 842.0)).expect("loads");
    assert_eq!(src.page_count(), 3);
    assert_eq!(src.page_size(0), Some((595.0, 842.0)));
    let ink = src.ink(1).expect("ink");
    assert!(
        ink.items.iter().any(
            |i| matches!(i, InkItem::Text { x, w, .. } if (*x - 72.0).abs() < 0.01 && *w > 20.0)
        )
    );
    assert!((box_width(&src, 2) - 30.0).abs() < 0.01);
}

#[test]
fn selects_and_repeats_pages_in_order() {
    let src = PdfSource::load(&sample(4, 595.0, 842.0)).expect("loads");
    // Pages 2 and 4, two copies uncollated: 2 2 4 4.
    let out = compose(&src, &[1, 1, 3, 3], (595.0, 842.0), 1).expect("composes");
    let job = PdfSource::load(&out).expect("job loads");
    assert_eq!(job.page_count(), 4);
    // Which form each job page draws: 2 2 4 4 is form A, A, B, B.
    let doc = Document::load_mem(&out).expect("lopdf loads");
    let forms: Vec<_> = doc
        .get_pages()
        .values()
        .map(|id| {
            let page = doc.get_dictionary(*id).expect("page");
            let res = page
                .get(b"Resources")
                .and_then(Object::as_dict)
                .expect("resources");
            let xo = res
                .get(b"XObject")
                .and_then(Object::as_dict)
                .expect("xobject");
            xo.get(b"P0").and_then(Object::as_reference).expect("form")
        })
        .collect();
    assert_eq!(forms[0], forms[1]);
    assert_eq!(forms[2], forms[3]);
    assert_ne!(forms[1], forms[2]);
}

#[test]
fn n_up_places_pages_on_one_sheet() {
    let src = PdfSource::load(&sample(4, 595.0, 842.0)).expect("loads");
    let out = compose(&src, &[0, 1, 2, 3], (595.0, 842.0), 4).expect("composes");
    let job = PdfSource::load(&out).expect("loads");
    assert_eq!(job.page_count(), 1);
    assert_eq!(job.page_size(0), Some((595.0, 842.0)));
    let content = String::from_utf8_lossy(&out);
    assert!(content.contains("/P3 Do"), "four placements");
    assert!(
        content.contains("0.5 0 0 0.5"),
        "each page is half size: {content}"
    );
}

#[test]
fn smaller_sheet_scales_down_and_never_enlarges() {
    let src = PdfSource::load(&sample(1, 600.0, 800.0)).expect("loads");
    let small = compose(&src, &[0], (300.0, 400.0), 1).expect("composes");
    assert!(String::from_utf8_lossy(&small).contains("0.5 0 0 0.5"));
    let big = compose(&src, &[0], (1200.0, 1600.0), 1).expect("composes");
    let text = String::from_utf8_lossy(&big);
    assert!(
        text.contains("q 1 0 0 1 300 400 cm"),
        "centred at full size: {text}"
    );
}

#[test]
fn rejects_empty_and_unknown_pages() {
    let src = PdfSource::load(&sample(2, 595.0, 842.0)).expect("loads");
    assert!(compose(&src, &[], (595.0, 842.0), 1).is_err());
    assert!(compose(&src, &[2], (595.0, 842.0), 1).is_err());
    assert!(PdfSource::load(b"not a pdf").is_err());
}
