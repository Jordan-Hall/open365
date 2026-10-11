//! Hand-made hostile inputs. Each case runs in a child process (`o365-probe`) so a stack
//! overflow or runaway allocation cannot take the test binary down. The assertions are:
//! no panic, no crash, no hang past the time limit, no growth past the memory limit, and
//! every refusal is an error value.
//!
//! A case that exposes a bug is kept, ignored, with its finding id (`#[ignore = "R-xx"]`);
//! its reproducer lives in `fixtures/robustness/`.

use open365_robustness::{
    Outcome, fixture, patch_headers, read_zip, run_child, with_part, write_zip,
};

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const S: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const PKG_RELS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

fn docx_base() -> Vec<u8> {
    fixture("docx", "comments.docx")
}

fn xlsx_base() -> Vec<u8> {
    fixture("xlsx", "chart-area.xlsx")
}

fn docx_body(body: &str) -> Vec<u8> {
    let xml = format!(
        "<?xml version=\"1.0\"?><w:document xmlns:w=\"{W}\" xmlns:r=\"{R}\"><w:body>{body}</w:body></w:document>"
    );
    with_part(&docx_base(), "word/document.xml", xml.into_bytes())
}

fn xlsx_sheet(inner: &str) -> Vec<u8> {
    let xml = format!(
        "<?xml version=\"1.0\"?><worksheet xmlns=\"{S}\" xmlns:r=\"{R}\">{inner}</worksheet>"
    );
    with_part(&xlsx_base(), "xl/worksheets/sheet1.xml", xml.into_bytes())
}

fn xlsx_sst(inner: &str) -> Vec<u8> {
    let xml = format!("<?xml version=\"1.0\"?><sst xmlns=\"{S}\" {inner}");
    with_part(&xlsx_base(), "xl/sharedStrings.xml", xml.into_bytes())
}

fn rels(items: &[(&str, &str, &str, bool)]) -> Vec<u8> {
    let mut xml = format!("<?xml version=\"1.0\"?><Relationships xmlns=\"{PKG_RELS}\">");
    for (id, ty, target, external) in items {
        let mode = if *external {
            " TargetMode=\"External\""
        } else {
            ""
        };
        xml.push_str(&format!(
            "<Relationship Id=\"{id}\" Type=\"{ty}\" Target=\"{target}\"{mode}/>"
        ));
    }
    xml.push_str("</Relationships>");
    xml.into_bytes()
}

thread_local! {
    static FAILS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Reports every recorded failure when the test ends, so one run lists all findings.
struct Guard;

impl Drop for Guard {
    fn drop(&mut self) {
        let fails = FAILS.with(|f| std::mem::take(&mut *f.borrow_mut()));
        if !std::thread::panicking() {
            assert!(
                fails.is_empty(),
                "{}",
                fails.join(
                    "
"
                )
            );
        }
    }
}

fn survive(kind: &str, name: &str, bytes: &[u8]) {
    let outcome = run_child(kind, name, bytes);
    if !outcome.is_clean() {
        FAILS.with(|f| f.borrow_mut().push(format!("{name}: {outcome:?}")));
    }
}

fn deflate_zip_of_zeros(name: &str, len: usize) -> Vec<u8> {
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    w.start_file(
        name,
        SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated),
    )
    .expect("start");
    let chunk = vec![0u8; 1 << 20];
    let mut left = len;
    while left > 0 {
        let n = left.min(chunk.len());
        w.write_all(&chunk[..n]).expect("write");
        left -= n;
    }
    w.finish().expect("finish").into_inner()
}

// ---- OPC container --------------------------------------------------------------------

#[test]
fn zip_bomb_single_part_over_part_limit() {
    let _guard = Guard;
    // 100 MiB of zeros in ~100 KiB: over the 32 MiB part limit and the ratio limit.
    let bomb = deflate_zip_of_zeros("word/document.xml", 100 * 1024 * 1024);
    survive("package", "bomb-part", &bomb);
}

#[test]
fn zip_bomb_many_parts_under_part_limit_over_total() {
    let _guard = Guard;
    // 5 parts of 30 MiB zeros: each under max_part_bytes, together over max_total_bytes.
    let zeros = vec![0u8; 30 * 1024 * 1024];
    let entries: Vec<(String, Vec<u8>)> = (0..5)
        .map(|i| (format!("word/p{i}.bin"), zeros.clone()))
        .collect();
    survive("package", "bomb-total", &write_zip(&entries));
}

#[test]
fn zip_bomb_xml_part_of_repeated_text() {
    let _guard = Guard;
    // 30 MiB of one long text run: legal size, extreme ratio, then parsed as XML.
    let mut xml = String::from("<w:p><w:r><w:t>");
    xml.push_str(&"A".repeat(30 * 1024 * 1024));
    xml.push_str("</w:t></w:r></w:p>");
    survive("docx", "huge-run", &docx_body(&xml));
}

#[test]
fn zip_with_100k_entries() {
    let _guard = Guard;
    let entries: Vec<(String, Vec<u8>)> = (0..100_000)
        .map(|i| (format!("x/{i}.txt"), Vec::new()))
        .collect();
    survive("package", "100k-entries", &write_zip(&entries));
}

#[test]
fn hostile_part_names() {
    let _guard = Guard;
    let names = [
        "/word/document.xml",
        "../evil.xml",
        "..\\evil.xml",
        "word/../../evil.xml",
        "word//document.xml",
        "word/./document.xml",
        "C:/evil.xml",
        "\\\\host\\share\\evil.xml",
        "word/document.xml/",
        "word/%2e%2e/evil.xml",
        "word/doc\u{0}ument.xml",
        "CON",
        "word/NUL.xml",
        "",
        " ",
        "[Content_Types].xml ",
    ];
    for (i, name) in names.iter().enumerate() {
        let entries = vec![
            (name.to_string(), b"<x/>".to_vec()),
            ("[Content_Types].xml".to_string(), b"<Types/>".to_vec()),
        ];
        survive("package", &format!("name-{i}"), &write_zip(&entries));
    }
    let long = "a/".repeat(30_000) + "b.xml";
    survive(
        "package",
        "name-long",
        &write_zip(&[(long, b"<x/>".to_vec())]),
    );
}

#[test]
fn duplicate_entries_exact_and_case_variants() {
    let _guard = Guard;
    // The writer refuses duplicates, so write two names of equal length and patch one.
    let entries = vec![
        ("word/aaaa.xml".to_string(), b"<a/>".to_vec()),
        ("word/bbbb.xml".to_string(), b"<b/>".to_vec()),
    ];
    let zip = write_zip(&entries);
    let mut exact = zip.clone();
    for window in 0..exact.len().saturating_sub(13) {
        if &exact[window..window + 13] == b"word/bbbb.xml" {
            exact[window..window + 13].copy_from_slice(b"word/aaaa.xml");
        }
    }
    survive("package", "dup-exact", &exact);
    let mut case = zip;
    for window in 0..case.len().saturating_sub(13) {
        if &case[window..window + 13] == b"word/bbbb.xml" {
            case[window..window + 13].copy_from_slice(b"word/AAAA.xml");
        }
    }
    survive("package", "dup-case", &case);
}

#[test]
fn duplicate_main_document_entries() {
    let _guard = Guard;
    let base = docx_base();
    let mut entries = read_zip(&base);
    entries.push(("word/document.xmL".into(), b"<w:document/>".to_vec()));
    survive("docx", "dup-main", &write_zip(&entries));
}

#[test]
fn huge_declared_sizes() {
    let _guard = Guard;
    let mut zip = docx_base();
    // Central directory: compressed size at +20, uncompressed size at +24.
    patch_headers(&mut zip, 0x0201_4b50, 24, 0xFFFF_FFFE);
    survive("package", "declared-uncompressed", &zip);
    let mut zip = docx_base();
    patch_headers(&mut zip, 0x0201_4b50, 20, 0xFFFF_FFFE);
    survive("package", "declared-compressed", &zip);
    let mut zip = docx_base();
    patch_headers(&mut zip, 0x0201_4b50, 24, 0xFFFF_FFFF);
    patch_headers(&mut zip, 0x0403_4b50, 22, 0xFFFF_FFFF);
    survive("package", "declared-zip64-marker", &zip);
    // Declared size smaller than the real data.
    let mut zip = docx_base();
    patch_headers(&mut zip, 0x0201_4b50, 24, 1);
    survive("package", "declared-too-small", &zip);
}

#[test]
fn eocd_lies_about_entry_count() {
    let _guard = Guard;
    let mut zip = docx_base();
    let eocd = zip
        .windows(4)
        .rposition(|w| w == [0x50, 0x4b, 0x05, 0x06])
        .expect("eocd");
    zip[eocd + 8..eocd + 10].copy_from_slice(&0xFFFFu16.to_le_bytes());
    zip[eocd + 10..eocd + 12].copy_from_slice(&0xFFFFu16.to_le_bytes());
    survive("package", "eocd-count", &zip);
    let mut zip = docx_base();
    let eocd = zip
        .windows(4)
        .rposition(|w| w == [0x50, 0x4b, 0x05, 0x06])
        .expect("eocd");
    zip[eocd + 12..eocd + 16].copy_from_slice(&0xFFFF_FFF0u32.to_le_bytes());
    survive("package", "eocd-cd-size", &zip);
}

#[test]
fn truncated_and_garbage_containers() {
    let _guard = Guard;
    let zip = docx_base();
    for cut in [0, 1, 4, 22, zip.len() / 2, zip.len() - 1] {
        survive("package", &format!("trunc-{cut}"), &zip[..cut]);
    }
    survive("package", "garbage", &vec![0x50u8; 4096]);
    let mut prefixed = vec![b'M'; 5000];
    prefixed.extend_from_slice(&zip);
    survive("package", "prefixed", &prefixed);
}

// ---- XML structure --------------------------------------------------------------------

#[test]
fn deep_nesting_docx_body() {
    let _guard = Guard;
    for depth in [1_000usize, 10_000, 200_000] {
        let body = format!(
            "{}<w:p/>{}",
            "<w:sdt><w:sdtContent>".repeat(depth),
            "</w:sdtContent></w:sdt>".repeat(depth)
        );
        survive("docx", &format!("deep-sdt-{depth}"), &docx_body(&body));
    }
}

#[test]
fn deep_nesting_docx_tables_and_runs() {
    let _guard = Guard;
    for depth in [1_000usize, 50_000] {
        let body = format!(
            "{}<w:p/>{}",
            "<w:tbl><w:tr><w:tc>".repeat(depth),
            "</w:tc></w:tr></w:tbl>".repeat(depth)
        );
        survive("docx", &format!("deep-tbl-{depth}"), &docx_body(&body));
        let body = format!(
            "<w:p>{}x{}</w:p>",
            "<w:hyperlink>".repeat(depth),
            "</w:hyperlink>".repeat(depth)
        );
        survive(
            "docx",
            &format!("deep-hyperlink-{depth}"),
            &docx_body(&body),
        );
    }
}

#[test]
fn deep_nesting_xlsx_sheet() {
    let _guard = Guard;
    for depth in [1_000usize, 100_000] {
        let inner = format!(
            "{}{}",
            "<sheetData>".repeat(depth),
            "</sheetData>".repeat(depth)
        );
        survive("xlsx", &format!("deep-sheet-{depth}"), &xlsx_sheet(&inner));
    }
}

#[test]
fn deep_nesting_unclosed_and_wide() {
    let _guard = Guard;
    survive("docx", "unclosed", &docx_body(&"<w:p>".repeat(300_000)));
    survive(
        "docx",
        "wide-attrs",
        &docx_body(&format!("<w:p {}/>", "a=\"1\" ")),
    );
    let attrs: String = (0..200_000).map(|i| format!("a{i}=\"1\" ")).collect();
    survive("docx", "many-attrs", &docx_body(&format!("<w:p {attrs}/>")));
    let siblings = "<w:p/>".repeat(2_000_000);
    survive("docx", "many-siblings", &docx_body(&siblings));
}

fn laughs() -> String {
    let mut dtd = String::from("<!DOCTYPE lolz [<!ENTITY lol \"lol\">");
    for i in 1..=9 {
        let prev = if i == 1 {
            "lol".to_string()
        } else {
            format!("lol{}", i - 1)
        };
        let refs = format!("&{prev};").repeat(10);
        dtd.push_str(&format!("<!ENTITY lol{i} \"{refs}\">"));
    }
    dtd.push_str("]>");
    dtd
}

#[test]
fn billion_laughs_in_every_part_kind() {
    let _guard = Guard;
    let dtd = laughs();
    let docx = format!(
        "<?xml version=\"1.0\"?>{dtd}<w:document xmlns:w=\"{W}\"><w:body><w:p><w:r><w:t>&lol9;</w:t></w:r></w:p></w:body></w:document>"
    );
    survive(
        "docx",
        "laughs-document",
        &with_part(&docx_base(), "word/document.xml", docx.into_bytes()),
    );
    for part in [
        "word/styles.xml",
        "word/settings.xml",
        "word/comments.xml",
        "[Content_Types].xml",
    ] {
        let xml = format!("<?xml version=\"1.0\"?>{dtd}<root>&lol9;</root>");
        survive(
            "docx",
            &format!("laughs-{}", part.replace('/', "_")),
            &with_part(&docx_base(), part, xml.into_bytes()),
        );
    }
    let rel = format!("<?xml version=\"1.0\"?>{dtd}<Relationships xmlns=\"{PKG_RELS}\"/>");
    survive(
        "docx",
        "laughs-rels",
        &with_part(&docx_base(), "_rels/.rels", rel.into_bytes()),
    );
    let sheet = format!(
        "<?xml version=\"1.0\"?>{dtd}<worksheet xmlns=\"{S}\"><sheetData><row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>&lol9;</t></is></c></row></sheetData></worksheet>"
    );
    survive(
        "xlsx",
        "laughs-sheet",
        &with_part(&xlsx_base(), "xl/worksheets/sheet1.xml", sheet.into_bytes()),
    );
    let sst =
        format!("<?xml version=\"1.0\"?>{dtd}<sst xmlns=\"{S}\"><si><t>&lol9;</t></si></sst>");
    survive(
        "xlsx",
        "laughs-sst",
        &with_part(&xlsx_base(), "xl/sharedStrings.xml", sst.into_bytes()),
    );
}

#[test]
fn quadratic_blowup_entity() {
    let _guard = Guard;
    let big = "A".repeat(50_000);
    let refs = "&a;".repeat(50_000);
    let xml = format!(
        "<?xml version=\"1.0\"?><!DOCTYPE d [<!ENTITY a \"{big}\">]><w:document xmlns:w=\"{W}\"><w:body><w:p><w:r><w:t>{refs}</w:t></w:r></w:p></w:body></w:document>"
    );
    survive(
        "docx",
        "quadratic",
        &with_part(&docx_base(), "word/document.xml", xml.into_bytes()),
    );
}

#[test]
fn character_reference_extremes() {
    let _guard = Guard;
    let body =
        "<w:p><w:r><w:t>&#x110000;&#xD800;&#0;&#x99999999999;&#;&#x;&bogus;&amp</w:t></w:r></w:p>";
    survive("docx", "charrefs", &docx_body(body));
}

// ---- xlsx specifics ------------------------------------------------------------------

#[test]
fn huge_shared_string_counts() {
    let _guard = Guard;
    survive(
        "xlsx",
        "sst-count",
        &xlsx_sst("count=\"4294967295\" uniqueCount=\"4294967295\"><si><t>a</t></si></sst>"),
    );
    survive(
        "xlsx",
        "sst-count-neg",
        &xlsx_sst("count=\"-1\" uniqueCount=\"99999999999999999999\"/>"),
    );
    // A real million-entry table.
    let many = format!(
        "count=\"1000000\" uniqueCount=\"1000000\">{}</sst>",
        "<si><t>x</t></si>".repeat(1_000_000)
    );
    survive("xlsx", "sst-million", &xlsx_sst(&many));
}

#[test]
fn shared_string_index_out_of_range() {
    let _guard = Guard;
    let sheet = "<sheetData><row r=\"1\"><c r=\"A1\" t=\"s\"><v>4294967295</v></c><c r=\"B1\" t=\"s\"><v>-1</v></c><c r=\"C1\" t=\"s\"><v>99999999999999999999</v></c><c r=\"D1\" t=\"s\"><v>1e3</v></c><c r=\"E1\" t=\"s\"/></row></sheetData>";
    survive("xlsx", "sst-index", &xlsx_sheet(sheet));
}

#[test]
fn giant_row_and_column_indexes() {
    let _guard = Guard;
    let cases = [
        "<row r=\"4294967295\"><c r=\"A4294967295\"><v>1</v></c></row>",
        "<row r=\"1048577\"><c r=\"XFE1048577\"><v>1</v></c></row>",
        "<row r=\"2147483648\"><c r=\"A1\"><v>1</v></c></row>",
        "<row r=\"0\"><c r=\"A0\"><v>1</v></c></row>",
        "<row r=\"-5\"><c r=\"A-5\"><v>1</v></c></row>",
        "<row r=\"1\"><c r=\"ZZZZZZZZZZ1\"><v>1</v></c></row>",
        "<row r=\"1\"><c r=\"XFD1048576\"><v>1</v></c></row>",
        "<row r=\"1\"><c r=\"A99999999999999999999\"><v>1</v></c></row>",
        "<row r=\"1\"><c r=\"\"><v>1</v></c><c><v>2</v></c><c r=\"$A$1\"><v>3</v></c></row>",
        "<row r=\"4294967295\" spans=\"1:4294967295\" ht=\"1e308\" s=\"4294967295\" customFormat=\"1\"/>",
        "<row r=\"1\"><c r=\"A1\" s=\"4294967295\"><v>1</v></c></row>",
    ];
    for (i, row) in cases.iter().enumerate() {
        survive(
            "xlsx",
            &format!("index-{i}"),
            &xlsx_sheet(&format!("<sheetData>{row}</sheetData>")),
        );
    }
}

#[test]
fn giant_dimensions_merges_and_columns() {
    let _guard = Guard;
    let parts = [
        "<dimension ref=\"A1:XFD1048576\"/><sheetData/>",
        "<dimension ref=\"A1:ZZZZZZ99999999\"/><sheetData/>",
        "<cols><col min=\"1\" max=\"4294967295\" width=\"1e308\"/></cols><sheetData/>",
        "<cols><col min=\"4294967295\" max=\"1\" width=\"-1\"/></cols><sheetData/>",
        "<sheetData/><mergeCells count=\"4294967295\"><mergeCell ref=\"A1:XFD1048576\"/></mergeCells>",
        "<sheetData/><conditionalFormatting sqref=\"A1:XFD1048576\"><cfRule type=\"expression\" priority=\"1\"><formula>A1&gt;1</formula></cfRule></conditionalFormatting>",
        "<sheetFormatPr defaultRowHeight=\"1e308\" defaultColWidth=\"-1\" outlineLevelRow=\"4294967295\"/><sheetData/>",
        "<sheetData/><hyperlinks><hyperlink ref=\"A1:XFD1048576\" r:id=\"rId9\"/></hyperlinks>",
        "<sheetData/><dataValidations count=\"4294967295\"><dataValidation sqref=\"A1:XFD1048576\" type=\"list\"><formula1>\"a,b\"</formula1></dataValidation></dataValidations>",
    ];
    for (i, inner) in parts.iter().enumerate() {
        survive("xlsx", &format!("dims-{i}"), &xlsx_sheet(inner));
    }
}

#[test]
fn many_dense_rows_and_cells() {
    let _guard = Guard;
    let mut inner = String::from("<sheetData>");
    for r in 1..=200_000u32 {
        inner.push_str(&format!(
            "<row r=\"{r}\"><c r=\"A{r}\"><v>{r}</v></c></row>"
        ));
    }
    inner.push_str("</sheetData>");
    survive("xlsx", "dense-rows", &xlsx_sheet(&inner));
}

#[test]
fn hostile_formulas_inside_cells() {
    let _guard = Guard;
    let deep = format!("{}1{}", "(".repeat(100_000), ")".repeat(100_000));
    let sheet = format!(
        "<sheetData><row r=\"1\"><c r=\"A1\"><f>{deep}</f></c><c r=\"B1\"><f>B1+1</f></c><c r=\"C1\"><f t=\"shared\" ref=\"C1:C1048576\" si=\"4294967295\">A1</f></c><c r=\"D1\"><f t=\"array\" ref=\"A1:XFD1048576\">ROW(A1)</f></c></row></sheetData>"
    );
    survive("xlsx", "cell-formulas", &xlsx_sheet(&sheet));
}

// ---- relationships --------------------------------------------------------------------

#[test]
fn cyclic_relationships() {
    let _guard = Guard;
    let office =
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";
    let styles = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles";
    // Package root points to itself and to a rels part.
    let root = rels(&[
        ("rId1", office, "word/document.xml", false),
        ("rId2", styles, "_rels/.rels", false),
        ("rId3", styles, "/", false),
        ("rId4", styles, "", false),
    ]);
    survive(
        "docx",
        "cycle-root",
        &with_part(&docx_base(), "_rels/.rels", root),
    );
    // Main part points to itself and to a part that points back.
    let doc_rels = rels(&[
        ("rId1", styles, "document.xml", false),
        ("rId2", styles, "styles.xml", false),
        ("rId3", styles, "../word/../word/document.xml", false),
    ]);
    let base = with_part(&docx_base(), "word/_rels/document.xml.rels", doc_rels);
    let back = rels(&[("rId1", styles, "document.xml", false)]);
    survive(
        "docx",
        "cycle-main",
        &with_part(&base, "word/_rels/styles.xml.rels", back),
    );
    // The main document relationship is the workbook itself.
    let wb = rels(&[
        ("rId1", office, "workbook.xml", false),
        (
            "rId2",
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet",
            "workbook.xml",
            false,
        ),
    ]);
    survive(
        "xlsx",
        "cycle-workbook",
        &with_part(&xlsx_base(), "xl/_rels/workbook.xml.rels", wb),
    );
    // Sheet -> drawing -> sheet.
    let sheet_rels = rels(&[(
        "rId1",
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing",
        "../worksheets/sheet1.xml",
        false,
    )]);
    survive(
        "xlsx",
        "cycle-sheet-drawing",
        &with_part(
            &xlsx_base(),
            "xl/worksheets/_rels/sheet1.xml.rels",
            sheet_rels,
        ),
    );
}

#[test]
fn relationship_xml_oddities() {
    let _guard = Guard;
    let styles = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles";
    let mut many = Vec::new();
    for i in 0..200_000 {
        many.push((format!("r{i}"), styles.to_string(), format!("p{i}.xml")));
    }
    let mut xml = format!("<Relationships xmlns=\"{PKG_RELS}\">");
    for (id, ty, target) in &many {
        xml.push_str(&format!(
            "<Relationship Id=\"{id}\" Type=\"{ty}\" Target=\"{target}\"/>"
        ));
    }
    xml.push_str("</Relationships>");
    survive(
        "docx",
        "rels-200k",
        &with_part(
            &docx_base(),
            "word/_rels/document.xml.rels",
            xml.into_bytes(),
        ),
    );
    let dup = rels(&[
        ("rId1", styles, "a.xml", false),
        ("rId1", styles, "b.xml", false),
    ]);
    survive(
        "docx",
        "rels-dup-id",
        &with_part(&docx_base(), "word/_rels/document.xml.rels", dup),
    );
    let long = "../".repeat(100_000) + "x.xml";
    let deep = rels(&[("rId1", styles, &long, false)]);
    survive(
        "docx",
        "rels-long-target",
        &with_part(&docx_base(), "word/_rels/document.xml.rels", deep),
    );
}

#[test]
fn external_targets_are_never_fetched() {
    let _guard = Guard;
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.set_nonblocking(true).expect("nonblocking");
    let port = listener.local_addr().expect("addr").port();
    let url = format!("http://127.0.0.1:{port}/x");
    let rel = |kind: &str| {
        format!("http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}")
    };
    let targets = [
        url.clone(),
        "file:///C:/Windows/win.ini".to_string(),
        "\\\\127.0.0.1\\share\\x.docx".to_string(),
        format!("ftp://127.0.0.1:{port}/x"),
    ];

    // DOCX: hyperlink, image, attached template, oleObject, external altChunk, plus a
    // DOCTYPE with an external entity in the main part.
    let mut items = Vec::new();
    for (i, t) in targets.iter().enumerate() {
        for kind in [
            "hyperlink",
            "image",
            "attachedTemplate",
            "oleObject",
            "afChunk",
            "subDocument",
            "styles",
        ] {
            items.push((format!("x{i}{kind}"), rel(kind), t.clone()));
        }
    }
    let list: Vec<(&str, &str, &str, bool)> = items
        .iter()
        .map(|(a, b, c)| (a.as_str(), b.as_str(), c.as_str(), true))
        .collect();
    let docx = with_part(&docx_base(), "word/_rels/document.xml.rels", rels(&list));
    let body = "<w:p><w:hyperlink r:id=\"x0hyperlink\"><w:r><w:t>l</w:t></w:r></w:hyperlink></w:p><w:altChunk r:id=\"x0afChunk\"/>";
    let docx = with_part(
        &docx,
        "word/document.xml",
        format!("<w:document xmlns:w=\"{W}\" xmlns:r=\"{R}\"><w:body>{body}</w:body></w:document>")
            .into_bytes(),
    );
    survive("docx", "external-docx", &docx);

    let xxe = format!(
        "<?xml version=\"1.0\"?><!DOCTYPE d [<!ENTITY x SYSTEM \"{url}\"><!ENTITY y SYSTEM \"file:///C:/Windows/win.ini\">]><w:document xmlns:w=\"{W}\"><w:body><w:p><w:r><w:t>&x;&y;</w:t></w:r></w:p></w:body></w:document>"
    );
    survive(
        "docx",
        "xxe-docx",
        &with_part(&docx_base(), "word/document.xml", xxe.into_bytes()),
    );
    let ext_dtd = format!(
        "<?xml version=\"1.0\"?><!DOCTYPE d SYSTEM \"{url}.dtd\"><w:document xmlns:w=\"{W}\"><w:body/></w:document>"
    );
    survive(
        "docx",
        "external-dtd",
        &with_part(&docx_base(), "word/document.xml", ext_dtd.into_bytes()),
    );

    // XLSX: external links, drawings, hyperlinks, query tables, connections.
    let mut items = Vec::new();
    for (i, t) in targets.iter().enumerate() {
        for kind in [
            "externalLink",
            "hyperlink",
            "image",
            "oleObject",
            "queryTable",
            "connections",
            "drawing",
            "sharedStrings",
        ] {
            items.push((format!("x{i}{kind}"), rel(kind), t.clone()));
        }
    }
    let list: Vec<(&str, &str, &str, bool)> = items
        .iter()
        .map(|(a, b, c)| (a.as_str(), b.as_str(), c.as_str(), true))
        .collect();
    let xlsx = with_part(&xlsx_base(), "xl/_rels/workbook.xml.rels", rels(&list));
    let xlsx = with_part(&xlsx, "xl/worksheets/_rels/sheet1.xml.rels", rels(&list));
    survive("xlsx", "external-xlsx", &xlsx);

    // Root package relationships to external targets.
    let office =
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";
    let root = rels(&[
        ("rId1", office, &url, true),
        ("rId2", office, "word/document.xml", false),
    ]);
    survive(
        "docx",
        "external-root",
        &with_part(&docx_base(), "_rels/.rels", root),
    );

    std::thread::sleep(std::time::Duration::from_millis(200));
    assert!(
        listener.accept().is_err(),
        "a parser connected to an external relationship target"
    );
}

// ---- formulas -------------------------------------------------------------------------

fn formula(name: &str, text: &str) {
    survive("formula", name, text.as_bytes());
}

#[test]
fn formula_deep_nesting() {
    let _guard = Guard;
    for depth in [1_000usize, 10_000, 200_000] {
        formula(
            &format!("paren-{depth}"),
            &format!("{}1{}", "(".repeat(depth), ")".repeat(depth)),
        );
        formula(
            &format!("unary-{depth}"),
            &format!("{}1", "-".repeat(depth)),
        );
        formula(&format!("not-{depth}"), &format!("{}1", "+".repeat(depth)));
        formula(
            &format!("if-{depth}"),
            &format!("{}1{}", "IF(TRUE,".repeat(depth), ",2)".repeat(depth)),
        );
        formula(
            &format!("array-{depth}"),
            &format!("{}1{}", "{".repeat(depth), "}".repeat(depth)),
        );
        formula(
            &format!("sum-{depth}"),
            &format!("{}1{}", "SUM(".repeat(depth), ")".repeat(depth)),
        );
        formula(&format!("unclosed-{depth}"), &"(".repeat(depth));
        formula(
            &format!("binary-{depth}"),
            &format!("1{}", "+1".repeat(depth)),
        );
        formula(
            &format!("concat-{depth}"),
            &format!("\"a\"{}", "&\"a\"".repeat(depth)),
        );
        formula(
            &format!("power-{depth}"),
            &format!("1{}", "^1".repeat(depth)),
        );
    }
}

#[test]
fn formula_huge_literals_and_ranges() {
    let _guard = Guard;
    formula("string-literal", &format!("\"{}\"", "a".repeat(20_000_000)));
    formula("number-text", &"9".repeat(5_000_000));
    formula("exp", "1E999999999999");
    formula("name", &"a".repeat(5_000_000));
    formula("sheet-quote", &format!("'{}'!A1", "x".repeat(5_000_000)));
    formula(
        "many-args",
        &format!("SUM({})", "1,".repeat(1_000_000) + "1"),
    );
    formula(
        "union",
        &format!(
            "SUM({})",
            "(A1:B2,".repeat(10_000) + "A1" + &")".repeat(10_000)
        ),
    );
    formula("full-grid", "SUM(A1:XFD1048576)");
    formula("full-grid-mul", "SUMPRODUCT(A1:XFD1048576*A1:XFD1048576)");
    formula("row-array", "SUM(ROW(1:1048576)*COLUMN(A:XFD))");
    formula("sequence", "SEQUENCE(1000000000)");
    formula("sequence2", "SEQUENCE(100000,100000)");
    formula("rept", "REPT(\"a\",2000000000)");
    formula("rept-nested", "REPT(REPT(\"abcdefghij\",30000),30000)");
    formula("randarray", "RANDARRAY(1000000,10000)");
    formula(
        "text-format",
        &format!("TEXT(1E308,\"{}\")", "0".repeat(1_000_000)),
    );
    formula("text-format-dec", "TEXT(1,\"0.\"&REPT(\"0\",100000000))");
    formula("transpose-all", "COUNTA(TRANSPOSE(A1:XFD1048576))");
    formula("filter-all", "COUNTA(FILTER(A1:XFD1048576,TRUE))");
    formula("sort-all", "COUNTA(SORT(A1:A1048576))");
    formula("unique-all", "COUNTA(UNIQUE(A1:XFD1048576))");
    formula(
        "regex-backtrack",
        "REGEXTEST(REPT(\"a\",100000)&\"b\",\"^(a+)+$\")",
    );
    formula(
        "regex-big",
        "REGEXREPLACE(REPT(\"a\",100000),\"(a*)*\",\"$1$1\")",
    );
    formula(
        "textjoin",
        "LEN(TEXTJOIN(\",\",TRUE,REPT(\"abcdefghij\",3200),ROW(1:1048576)))",
    );
    formula(
        "large-array-literal",
        &format!("{{{}}}", "1,".repeat(2_000_000) + "1"),
    );
    formula("column-cross", "COLUMN(A:XFD)*ROW(1:1048576)");
    formula("lambda-recursion", "LET(f,LAMBDA(f,n,f(f,n+1)),f(f,1))");
    formula("lambda-self", "LAMBDA(x,x)(LAMBDA(x,x))");
    formula("reduce-big", "REDUCE(0,SEQUENCE(1000000),LAMBDA(a,b,a+b))");
    formula("makearray", "MAKEARRAY(100000,100000,LAMBDA(r,c,r*c))");
    formula("power-huge", "2^2^2^2^2^2");
    formula("fact-huge", "FACT(1E9)");
    formula("combin", "COMBIN(1E9,5E8)");
    formula("date-extreme", "TEXT(DATE(9999999,1,1),\"yyyy-mm-dd\")");
    formula("bitshift", "BITLSHIFT(1,1E9)");
    formula("base-huge", "BASE(1E300,2,1000000)");
    formula("roman", "ROMAN(1E9)");
    formula("char-huge", "CHAR(1E9)&UNICHAR(1E9)");
}

#[test]
fn formula_cycles_and_indirection() {
    let _guard = Guard;
    formula("self-ref", "E1+1");
    formula("indirect-self", "INDIRECT(\"E1\")+1");
    formula("offset-self", "OFFSET(E1,0,0)");
    formula("indirect-huge", "INDIRECT(\"A1:XFD1048576\")");
    formula("offset-huge", "SUM(OFFSET(A1,0,0,1048576,16384))");
    formula("offset-neg", "OFFSET(A1,-1000000000,-1000000000)");
    formula("index-huge", "INDEX(A1:C10,4294967295,4294967295)");
    formula("choose-big", "CHOOSE(4294967295,1,2)");
    formula("sheet3d", "SUM(S:S!A1)");
    formula("ext-ref", "[999999]S!A1+'[http://127.0.0.1:1/x.xlsx]S'!A1");
    formula("hyperlink", "HYPERLINK(\"http://127.0.0.1:1/\",\"x\")");
    formula("webservice", "WEBSERVICE(\"http://127.0.0.1:1/\")");
    formula(
        "filterxml",
        "FILTERXML(WEBSERVICE(\"http://127.0.0.1:1/\"),\"//a\")",
    );
    formula("rtd", "RTD(\"a\",\"b\",\"c\")");
    formula("call", "CALL(\"kernel32\",\"Sleep\",\"JJ\",1000000)");
    formula("register", "REGISTER.ID(\"kernel32\",\"Sleep\")");
    formula("dde", "'cmd'|'/c calc'!A1");
}

#[test]
fn formula_garbage_text() {
    let _guard = Guard;
    for (i, text) in [
        "",
        "=",
        "==",
        "\u{0}",
        "\u{FFFF}\u{FEFF}",
        "ΑΒΓ(1)",
        "#REF!#REF!",
        "A1:A2:A3:A4",
        "A1 A2 A3",
        "Table1[#Headers",
        "Table1[[",
        "\"\"\"\"\"",
        "'",
        "'''!A1",
        "1E+",
        "1..2",
        ".",
        "A0",
        "A1048577",
        "XFE1",
        "R4294967295C4294967295",
        "R[-2147483648]C[2147483647]",
        "$$A$$1",
        "1:0",
        "0:0",
        "A:A:A",
        "{1,2;3}",
        "{;;;;}",
        "{,}",
        "SUM(,,,,)",
        "IF(,,)",
        "@@@@",
        "#",
        "%%%%%%1",
        "1%%%%%%%%%%%%%%%%",
        "\u{202E}1+1",
    ]
    .iter()
    .enumerate()
    {
        formula(&format!("garbage-{i}"), text);
    }
}

// Sanity: the probe itself reports panics and errors correctly.
#[test]
fn probe_reports_rejection_as_value() {
    let _guard = Guard;
    let outcome = run_child("package", "not-a-zip", b"not a zip");
    assert!(matches!(outcome, Outcome::Rejected(_)), "{outcome:?}");
}

// ---- known findings -------------------------------------------------------------------
// Each reproducer is in fixtures/robustness/. The tests are ignored until the owning crate
// is fixed; run them with `cargo test -p open365-robustness -- --ignored`.

fn replay_formula(file: &str) {
    let _guard = Guard;
    let bytes = std::fs::read(
        open365_robustness::repo_root()
            .join("fixtures/robustness")
            .join(file),
    )
    .expect("reproducer");
    survive("formula", file, &bytes);
}

#[test]
fn r_calc_1_mmult_unbounded_output() {
    replay_formula("R-CALC-1-mmult-unbounded-output.formula");
}

#[test]
fn r_calc_2_linest_memory() {
    replay_formula("R-CALC-2-linest-memory.formula");
}

#[test]
fn r_calc_3_substitute_transient() {
    replay_formula("R-CALC-3-substitute-1gb-transient.formula");
}

#[test]
fn r_calc_4_frequency_quadratic() {
    replay_formula("R-CALC-4-frequency-quadratic.formula");
}

#[test]
fn r_calc_5_mode_mult_quadratic() {
    replay_formula("R-CALC-5-modemult-quadratic.formula");
}

#[test]
fn r_calc_6_unique_quadratic() {
    replay_formula("R-CALC-6-unique-quadratic.formula");
}
