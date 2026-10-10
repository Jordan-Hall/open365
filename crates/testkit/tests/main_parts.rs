//! Round-trip invariants hold for every main-part type, with and without VBA parts, and a
//! dangling content-type `Override` is reported.

use std::io::{Cursor, Write};

use open365_opc::{IntegrityIssue, MainPartKind, VBA_DATA_CONTENT_TYPE, VBA_PROJECT_CONTENT_TYPE};
use open365_testkit::{inspect, roundtrip};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

fn package(kind: MainPartKind, vba: bool, dangling: bool) -> Vec<u8> {
    let (dir, main) = if kind.is_word() {
        ("word", "document.xml")
    } else {
        ("xl", "workbook.xml")
    };
    let mut types = format!(
        "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
         <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
         <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
         <Override PartName=\"/{dir}/{main}\" ContentType=\"{}\"/>",
        kind.content_type()
    );
    if vba {
        types.push_str(&format!(
            "<Override PartName=\"/{dir}/vbaProject.bin\" ContentType=\"{VBA_PROJECT_CONTENT_TYPE}\"/>\
             <Override PartName=\"/{dir}/vbaData.xml\" ContentType=\"{VBA_DATA_CONTENT_TYPE}\"/>"
        ));
    }
    if dangling {
        types.push_str(
            "<Override PartName=\"/_xmlsignatures/sig1.xml\" ContentType=\"application/xml\"/>",
        );
    }
    types.push_str("</Types>");

    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let mut add = |name: &str, data: &[u8]| {
        writer
            .start_file(
                name,
                SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
            )
            .expect("start entry");
        writer.write_all(data).expect("write entry");
    };
    add("[Content_Types].xml", types.as_bytes());
    add(
        "_rels/.rels",
        format!(
            "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
             <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"{dir}/{main}\"/></Relationships>"
        )
        .as_bytes(),
    );
    add(&format!("{dir}/{main}"), b"<root/>");
    if vba {
        add(&format!("{dir}/vbaProject.bin"), b"\xd0\xcf\x11\xe0");
        add(&format!("{dir}/vbaData.xml"), b"<vbaData/>");
    }
    writer.finish().expect("finish zip").into_inner()
}

#[test]
fn every_main_part_type_roundtrips_losslessly() {
    for kind in MainPartKind::ALL {
        for vba in [false, true] {
            let bytes = package(kind, vba, false);
            let report = roundtrip(&bytes).expect("roundtrip");
            assert!(
                report.is_lossless(),
                "{kind:?} vba={vba}: {}",
                report.describe()
            );
            assert_eq!(report.main_kind, Some(kind));
            let info = inspect(&bytes).expect("inspect");
            assert_eq!(info.integrity_issues, Vec::new(), "{kind:?} vba={vba}");
        }
    }
}

#[test]
fn dangling_override_is_an_integrity_issue() {
    let info = inspect(&package(MainPartKind::Docx, false, true)).expect("inspect");
    assert_eq!(
        info.integrity_issues,
        vec![IntegrityIssue::DanglingOverride {
            part: "_xmlsignatures/sig1.xml".to_owned()
        }]
    );
}
