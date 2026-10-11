//! Template and macro-enabled seed kinds built from our own fixtures: `.dotx`, `.dotm`,
//! `.docm`, `.xltx`, `.xltm` and `.xlsm`. The VBA project is a made-up blob (an OLE
//! signature plus filler): it is preserved and never executed.

use open365_docx::{Document, DocumentKind};
use open365_opc::{Limits, Package, PartName};
use open365_xlsx::{Workbook, WorkbookKind};

const VBA_REL: &str = "http://schemas.microsoft.com/office/2006/relationships/vbaProject";
const VBA_TYPE: &str = "application/vnd.ms-office.vbaProject";

/// A stand-in `vbaProject.bin`: the compound-file signature followed by filler. Not a real
/// project, so nothing in it could run even if something tried.
fn vba_blob() -> Vec<u8> {
    let mut blob = vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
    blob.extend((0..504u32).map(|i| u8::try_from(i % 251).unwrap_or(0)));
    blob
}

/// The package with `from` replaced by `to` in `[Content_Types].xml`, plus a VBA project
/// related to `main` when `vba` is set.
fn rebuild(bytes: &[u8], main: &str, from: &str, to: &str, vba: bool) -> Option<Vec<u8>> {
    let mut package = Package::read(bytes, &Limits::default()).ok()?;
    let ct_name = PartName::new("[Content_Types].xml").ok()?;
    let mut ct = String::from_utf8(package.part("[Content_Types].xml")?.data().to_vec()).ok()?;
    if !ct.contains(from) {
        return None;
    }
    ct = ct.replacen(from, to, 1);
    if vba {
        if !ct.contains(VBA_TYPE) {
            let at = ct.find("<Types")?;
            let end = at + ct[at..].find('>')? + 1;
            ct.insert_str(
                end,
                &format!("<Default Extension=\"bin\" ContentType=\"{VBA_TYPE}\"/>"),
            );
        }
        let dir = main.rsplit_once('/').map_or("", |(d, _)| d);
        let file = main.rsplit_once('/').map_or(main, |(_, f)| f);
        let rels_name = if dir.is_empty() {
            format!("_rels/{file}.rels")
        } else {
            format!("{dir}/_rels/{file}.rels")
        };
        let rel =
            format!("<Relationship Id=\"rIdVba1\" Type=\"{VBA_REL}\" Target=\"vbaProject.bin\"/>");
        let rels = match package.part(&rels_name) {
            Some(p) => {
                let text = String::from_utf8(p.data().to_vec()).ok()?;
                let at = text.rfind("</Relationships>")?;
                format!("{}{rel}{}", &text[..at], &text[at..])
            }
            None => format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
                 <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{rel}</Relationships>"
            ),
        };
        package.set_part(PartName::new(&rels_name).ok()?, rels.into_bytes());
        let bin = if dir.is_empty() {
            "vbaProject.bin".to_owned()
        } else {
            format!("{dir}/vbaProject.bin")
        };
        package.set_part(PartName::new(&bin).ok()?, vba_blob());
    }
    package.set_part(ct_name, ct.into_bytes());
    package.write().ok()
}

/// `(extension, bytes)` for every other docx kind built from a plain `.docx`.
pub fn docx_variants(bytes: &[u8]) -> Vec<(&'static str, Vec<u8>)> {
    let Ok(doc) = Document::open(bytes, &Limits::default()) else {
        return Vec::new();
    };
    let main = doc.main_part().as_str().trim_start_matches('/').to_owned();
    let from = doc.kind().content_type();
    [
        ("dotx", DocumentKind::Template, false),
        ("dotm", DocumentKind::MacroEnabledTemplate, true),
        ("docm", DocumentKind::MacroEnabledDocument, true),
    ]
    .into_iter()
    .filter_map(|(ext, kind, vba)| {
        Some((ext, rebuild(bytes, &main, from, kind.content_type(), vba)?))
    })
    .collect()
}

/// `(extension, bytes)` for every other xlsx kind built from a plain `.xlsx`.
pub fn xlsx_variants(bytes: &[u8]) -> Vec<(&'static str, Vec<u8>)> {
    let Ok(wb) = Workbook::open(bytes, &Limits::default()) else {
        return Vec::new();
    };
    let main = wb
        .workbook_part()
        .as_str()
        .trim_start_matches('/')
        .to_owned();
    let from = wb.kind().content_type();
    [
        ("xltx", WorkbookKind::Template, false),
        ("xltm", WorkbookKind::MacroEnabledTemplate, true),
        ("xlsm", WorkbookKind::MacroEnabledWorkbook, true),
    ]
    .into_iter()
    .filter_map(|(ext, kind, vba)| {
        Some((ext, rebuild(bytes, &main, from, kind.content_type(), vba)?))
    })
    .collect()
}
