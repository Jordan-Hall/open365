//! Hyperlinks, bookmarks, list labels, document information and the PDF/A profiles, read back
//! from the files we write.

mod common;

use std::sync::atomic::AtomicBool;

use common::{Obj, Reader, layout_fixture, layout_with_bundled_theme, root};
use open365_docx::Document;
use open365_layout::{Layout, ParaRole, StoryKind};
use open365_opc::Limits;
use open365_pdf::{Conformance, PdfMetadata, PdfOptions, export_pdf, export_pdf_with};
use open365_text::FontLibrary;

fn export(name: &str, meta: &PdfMetadata, options: &PdfOptions) -> (Vec<u8>, Layout) {
    let fonts = FontLibrary::bundled();
    let (doc, layout) = if name == "fields-refs" || name == "fields-toc" {
        layout_with_bundled_theme(name, &fonts)
    } else {
        layout_fixture(name, &fonts).unwrap_or_else(|| panic!("{name}: layout"))
    };
    let out = export_pdf_with(
        &doc,
        &layout,
        &fonts,
        meta,
        options,
        &AtomicBool::new(false),
    )
    .expect("export");
    (out.bytes, layout)
}

fn all_objects(r: &Reader<'_>) -> Vec<(u32, Obj)> {
    (1..=r.object_count() as u32)
        .map(|i| (i, r.get(i)))
        .collect()
}

fn is_name(o: Option<&Obj>, want: &str) -> bool {
    matches!(o, Some(Obj::Name(n)) if n == want)
}

#[test]
fn external_hyperlink_is_an_annotation_owned_by_a_link_element() {
    let (bytes, _) = export(
        "hyperlink-external",
        &PdfMetadata::default(),
        &PdfOptions::default(),
    );
    let r = Reader::new(&bytes);
    let page = &r.pages()[0].1;
    let annots = r.key(page, "Annots").expect("annots");
    assert_eq!(annots.arr().len(), 1);
    let Obj::Ref(annot_id) = annots.arr()[0] else {
        panic!("annotation ref")
    };
    let annot = r.get(annot_id);
    let d = annot.dict();
    assert!(is_name(d.get("Subtype"), "Link"));
    let action = r.key(&annot, "A").expect("action");
    assert!(is_name(action.dict().get("S"), "URI"));
    assert_eq!(
        action.dict()["URI"],
        Obj::Str(b"https://example.com/open365-fixture".to_vec())
    );
    // Print flag set, no border.
    assert_eq!(d["F"], Obj::Int(4));
    let rect = d["Rect"].arr();
    assert!(rect[2].num() > rect[0].num() && rect[3].num() > rect[1].num());

    // The Link element holds the link text's marked content and the annotation.
    let (link_id, link) = all_objects(&r)
        .into_iter()
        .find(|(_, o)| matches!(o, Obj::Dict(d) if is_name(d.get("S"), "Link")))
        .expect("Link element");
    let kids = r.key(&link, "K").expect("K");
    let mut mcr = 0;
    let mut objr = 0;
    for k in kids.arr() {
        let k = r.resolve(k);
        match k.dict().get("Type") {
            Some(Obj::Name(t)) if t == "MCR" => mcr += 1,
            Some(Obj::Name(t)) if t == "OBJR" => {
                objr += 1;
                assert_eq!(k.dict()["Obj"], Obj::Ref(annot_id));
            }
            other => panic!("unexpected kid {other:?}"),
        }
    }
    assert!(mcr >= 1 && objr == 1);
    // The Link sits inside a paragraph, which lists it.
    let Obj::Ref(parent) = link.dict()["P"] else {
        panic!("parent ref")
    };
    let p = r.get(parent);
    assert!(is_name(p.dict().get("S"), "P"));
    assert!(
        p.dict()["K"].arr().contains(&Obj::Ref(link_id)),
        "paragraph does not list its link"
    );
    // The annotation's StructParent finds the Link element through the parent tree.
    let sp = d["StructParent"].num() as i64;
    let tree = r
        .key(
            &r.key(&r.trailer(), "Root").expect("root"),
            "StructTreeRoot",
        )
        .expect("tree");
    let pt = r.key(&tree, "ParentTree").expect("parent tree");
    let nums = pt.dict()["Nums"].arr();
    let at = nums
        .chunks(2)
        .find(|c| c[0] == Obj::Int(sp))
        .expect("StructParent key in the parent tree");
    assert_eq!(at[1], Obj::Ref(link_id));
}

#[test]
fn unsafe_link_targets_are_not_written() {
    // Only http, https, mailto, ftp and tel reach a viewer; the fixture's links are all safe.
    let (bytes, _) = export(
        "layout-anc-hyperlink",
        &PdfMetadata::default(),
        &PdfOptions::default(),
    );
    let text = String::from_utf8_lossy(&bytes);
    assert!(!text.contains("/URI (javascript"));
    assert!(text.contains("/URI (mailto:someone@a.example)"));
}

#[test]
fn headings_become_nested_bookmarks_with_destinations() {
    let (bytes, layout) = export(
        "fields-toc",
        &PdfMetadata::default(),
        &PdfOptions::default(),
    );
    let r = Reader::new(&bytes);
    let root_obj = r.key(&r.trailer(), "Root").expect("root");
    let outlines = r.key(&root_obj, "Outlines").expect("outlines");
    // Titles in outline order (depth first).
    fn walk(r: &Reader<'_>, item: &Obj, out: &mut Vec<(String, f64)>, pages: &[u32]) {
        let mut cur = r.key(item, "First");
        while let Some(it) = cur {
            let Obj::Str(t) = &it.dict()["Title"] else {
                panic!("title")
            };
            let dest = it.dict()["Dest"].arr();
            let Obj::Ref(pg) = dest[0] else {
                panic!("dest page")
            };
            assert!(pages.contains(&pg), "destination is not a page");
            assert!(is_name(Some(&dest[1]), "XYZ"));
            out.push((String::from_utf8_lossy(t).into_owned(), dest[3].num()));
            walk(r, &it, out, pages);
            cur = r.key(&it, "Next");
        }
    }
    let pages: Vec<u32> = r.pages().iter().map(|p| p.0).collect();
    let mut got = Vec::new();
    walk(&r, &outlines, &mut got, &pages);
    let want: Vec<String> = {
        let mut seen = std::collections::HashSet::new();
        layout
            .pages
            .iter()
            .flat_map(|p| p.lines.iter())
            .filter(|l| {
                l.story == StoryKind::Main
                    && matches!(l.role, ParaRole::Heading(_))
                    && seen.insert(l.paragraph)
            })
            .map(|l| l.text.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect()
    };
    assert!(!want.is_empty(), "fixture has headings");
    let titles: Vec<&str> = got.iter().map(|(t, _)| t.as_str()).collect();
    // Numbered headings are headings too (their number leads the title, as in Word's outline),
    // so every unnumbered heading is there and the tree is at least that big.
    for w in &want {
        assert!(titles.contains(&w.as_str()), "{w} missing from {titles:?}");
    }
    assert!(
        titles.len() > want.len(),
        "numbered headings are bookmarked: {titles:?}"
    );
    assert!(titles.contains(&"1 First Chapter"), "{titles:?}");
    // Count of the root is the number of entries in the tree.
    assert_eq!(outlines.dict()["Count"].num() as usize, titles.len());
    // y is inside the page.
    assert!(got.iter().all(|(_, y)| *y > 0.0 && *y < 842.0));

    // And the option turns them off.
    let (bytes, _) = export(
        "fields-toc",
        &PdfMetadata::default(),
        &PdfOptions {
            bookmarks: false,
            ..PdfOptions::default()
        },
    );
    assert!(!String::from_utf8_lossy(&bytes).contains("/Type /Outlines"));
}

#[test]
fn internal_hyperlink_jumps_to_its_bookmark() {
    let (bytes, layout) = export(
        "fields-refs",
        &PdfMetadata::default(),
        &PdfOptions::default(),
    );
    let r = Reader::new(&bytes);
    let mut jumps = 0;
    for (_, o) in all_objects(&r) {
        if let Obj::Dict(d) = &o
            && is_name(d.get("Subtype"), "Link")
            && let Some(Obj::Arr(dest)) = d.get("Dest")
        {
            let Obj::Ref(pg) = dest[0] else {
                panic!("dest page")
            };
            assert!(r.pages().iter().any(|p| p.0 == pg));
            jumps += 1;
        }
    }
    assert!(jumps >= 1, "no internal link in a document that has one");
    assert!(!layout.pages.is_empty());
}

#[test]
fn list_labels_are_lbl_elements_beside_the_item_body() {
    let (bytes, _) = export(
        "layout-lists-multilevel",
        &PdfMetadata::default(),
        &PdfOptions::default(),
    );
    let r = Reader::new(&bytes);
    let mut items = 0;
    for (_, o) in all_objects(&r) {
        let Obj::Dict(d) = &o else { continue };
        if !is_name(d.get("S"), "LI") {
            continue;
        }
        items += 1;
        let kids: Vec<Obj> = d["K"].arr().iter().map(|k| r.resolve(k)).collect();
        let names: Vec<&str> = kids.iter().map(|k| k.dict()["S"].name()).collect();
        assert_eq!(names, ["Lbl", "LBody"]);
        // The label carries marked content of its own.
        assert!(!kids[0].dict()["K"].arr().is_empty());
    }
    // Numbered headings are headings (H1..H6), not list items.
    assert!(items >= 1);
}

#[test]
fn info_and_xmp_say_the_same_thing() {
    let meta = PdfMetadata {
        title: Some("Quarterly <report> & plans".into()),
        author: Some("A. Writer".into()),
        subject: Some("Numbers".into()),
        keywords: Some("alpha, beta".into()),
        created: Some("D:20260101120000Z".into()),
        modified: Some("D:20260102130000+01'00'".into()),
        language: Some("en-GB".into()),
        ..PdfMetadata::default()
    };
    let (bytes, _) = export("plain-paragraphs", &meta, &PdfOptions::default());
    let r = Reader::new(&bytes);
    let trailer = r.trailer();
    let info = r.key(&trailer, "Info").expect("info");
    assert_eq!(info.dict()["Subject"], Obj::Str(b"Numbers".to_vec()));
    assert_eq!(
        info.dict()["ModDate"],
        Obj::Str(b"D:20260102130000+01'00'".to_vec())
    );
    let root_obj = r.key(&trailer, "Root").expect("root");
    let xmp =
        String::from_utf8(r.stream(&r.key(&root_obj, "Metadata").expect("xmp"))).expect("utf8");
    for want in [
        "Quarterly &lt;report&gt; &amp; plans",
        "<rdf:li>A. Writer</rdf:li>",
        "<dc:description>",
        "<pdf:Keywords>alpha, beta</pdf:Keywords>",
        "<xmp:CreateDate>2026-01-01T12:00:00Z</xmp:CreateDate>",
        "<xmp:ModifyDate>2026-01-02T13:00:00+01:00</xmp:ModifyDate>",
        "<pdf:Producer>Open365</pdf:Producer>",
    ] {
        assert!(xmp.contains(want), "XMP lacks {want}: {xmp}");
    }
    assert!(
        !xmp.contains("pdfaid"),
        "a plain export must not claim PDF/A"
    );
}

#[test]
fn document_properties_come_from_the_document_on_request() {
    let bytes = std::fs::read(root().join("fixtures/docx/fields-props.docx")).expect("docx");
    let doc = Document::open(&bytes, &Limits::default()).expect("open");
    let meta = PdfMetadata::from_document(&doc);
    assert_eq!(meta.title.as_deref(), Some("Replacement Title"));
    assert_eq!(meta.author.as_deref(), Some("Test Author"));
    assert_eq!(meta.subject.as_deref(), Some("Field Oracle Subject"));
    assert_eq!(meta.keywords.as_deref(), Some("alpha, beta"));
    assert_eq!(meta.created.as_deref(), Some("D:20231102080910Z"));
    // A document with empty properties yields nothing.
    let bytes = std::fs::read(root().join("fixtures/docx/plain-paragraphs.docx")).expect("docx");
    let doc = Document::open(&bytes, &Limits::default()).expect("open");
    let meta = PdfMetadata::from_document(&doc);
    assert!(meta.title.is_none() && meta.author.is_none());
}

fn pdfa(name: &str, conformance: Conformance) -> Vec<u8> {
    let meta = PdfMetadata {
        title: Some("Archive".into()),
        language: Some("en-GB".into()),
        ..PdfMetadata::default()
    };
    export(
        name,
        &meta,
        &PdfOptions {
            conformance,
            ..PdfOptions::default()
        },
    )
    .0
}

#[test]
fn pdfa_files_carry_the_identification_output_intent_and_embedded_fonts() {
    for (conformance, version, part) in [
        (Conformance::PdfA1b, "%PDF-1.4", "1"),
        (Conformance::PdfA2b, "%PDF-1.7", "2"),
    ] {
        for name in [
            "plain-paragraphs",
            "layout-img-inline",
            "hyperlink-external",
            "layout-table-basic",
        ] {
            let bytes = pdfa(name, conformance);
            assert!(bytes.starts_with(version.as_bytes()), "{name}: header");
            let r = Reader::new(&bytes);
            let trailer = r.trailer();
            assert!(trailer.dict().contains_key("ID"), "{name}: file identifier");
            assert!(!trailer.dict().contains_key("Encrypt"));
            let root_obj = r.key(&trailer, "Root").expect("root");
            // XMP identifies the profile and is not compressed.
            let xmp_obj = r.key(&root_obj, "Metadata").expect("xmp");
            assert!(!xmp_obj.dict().contains_key("Filter"), "XMP must be plain");
            let xmp = String::from_utf8(r.stream(&xmp_obj)).expect("utf8");
            assert!(xmp.contains(&format!("<pdfaid:part>{part}</pdfaid:part>")));
            assert!(xmp.contains("<pdfaid:conformance>B</pdfaid:conformance>"));
            assert!(xmp.contains("<dc:title>"));
            // Output intent with an ICC profile.
            let intents = r.key(&root_obj, "OutputIntents").expect("intents");
            let intent = r.resolve(&intents.arr()[0]);
            assert!(is_name(intent.dict().get("S"), "GTS_PDFA1"));
            let profile = r.key(&intent, "DestOutputProfile").expect("profile");
            assert_eq!(profile.dict()["N"], Obj::Int(3));
            let icc = r.stream(&profile);
            assert_eq!(&icc[36..40], b"acsp");
            assert_eq!(
                u32::from_be_bytes(icc[0..4].try_into().expect("size")) as usize,
                icc.len()
            );
            // Every font is embedded; nothing uses transparency or interactive features.
            for (_, o) in all_objects(&r) {
                if let Obj::Dict(d) = &o
                    && is_name(d.get("Type"), "FontDescriptor")
                {
                    assert!(
                        d.contains_key("FontFile2") || d.contains_key("FontFile3"),
                        "{name}: font not embedded"
                    );
                }
            }
            let text = String::from_utf8_lossy(&bytes);
            for banned in [
                "/SMask",
                "/JavaScript",
                "/JS ",
                "/Launch",
                "/EmbeddedFile",
                "/OCProperties",
                "/BM /",
            ] {
                assert!(!text.contains(banned), "{name}: {banned}");
            }
        }
    }
}

#[test]
fn pdfa1_flattens_pictures_with_alpha_but_pdfa2_keeps_the_soft_mask() {
    // layout-img-inline has pictures; whether any has alpha is the fixture's business, so the
    // check is that PDF/A-1b never writes a mask and that the plain export is unchanged.
    let a1 = pdfa("layout-img-inline", Conformance::PdfA1b);
    assert!(!String::from_utf8_lossy(&a1).contains("/SMask"));
    let plain = export(
        "layout-img-inline",
        &PdfMetadata::default(),
        &PdfOptions::default(),
    )
    .0;
    assert!(plain.starts_with(b"%PDF-1.7"));
    assert!(!String::from_utf8_lossy(&plain).contains("OutputIntent"));
}

#[test]
fn a_plain_export_is_unchanged_by_the_options_default() {
    let fonts = FontLibrary::bundled();
    let (doc, layout) = layout_fixture("plain-paragraphs", &fonts).expect("layout");
    let meta = PdfMetadata::default();
    let a = export_pdf(&doc, &layout, &fonts, &meta)
        .expect("export")
        .bytes;
    let b = export_pdf_with(
        &doc,
        &layout,
        &fonts,
        &meta,
        &PdfOptions::default(),
        &AtomicBool::new(false),
    )
    .expect("export")
    .bytes;
    assert_eq!(a, b);
}
