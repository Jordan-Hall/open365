//! Deadlines cancel the work: layout, PDF export and formula evaluation stop at their next check
//! instead of running on in the background, a cancelled export leaves no file, and the cached
//! engine is usable afterwards. Documents are generated here; no fixture holds personal data.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use open365_commands::{CommandError, FormulaEngine, Target};
use open365_layout::{LayoutOptions, layout_document, layout_document_cancellable};
use open365_mcp::{Config, Server};
use open365_text::FontLibrary;
use serde_json::{Value, json};

fn zip_of(parts: &[(&str, String)]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, data) in parts {
        w.start_file(*name, zip::write::SimpleFileOptions::default())
            .expect("start");
        w.write_all(data.as_bytes()).expect("write");
    }
    w.finish().expect("finish").into_inner()
}

const ROOT_RELS: &str = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="PART"/></Relationships>"#;

/// A document of `paragraphs` paragraphs of generated words.
fn big_docx(paragraphs: usize) -> Vec<u8> {
    let types = r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#;
    let text: String = (0..80)
        .map(|i| format!("word{} ", i % 17))
        .collect::<String>();
    let mut body = String::new();
    for _ in 0..paragraphs {
        body.push_str(&format!("<w:p><w:r><w:t>{text}</w:t></w:r></w:p>"));
    }
    let doc = format!(
        r#"<?xml version="1.0"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#
    );
    zip_of(&[
        ("[Content_Types].xml", types.to_owned()),
        (
            "_rels/.rels",
            ROOT_RELS.replace("PART", "word/document.xml"),
        ),
        ("word/document.xml", doc),
    ])
}

/// A workbook whose `rows` formulas build a string of Z1 characters: cheap while Z1 is empty,
/// slow once a probe sets it.
fn heavy_xlsx(rows: u32) -> Vec<u8> {
    let types = r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#;
    let book = r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="S" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
    let book_rels = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#;
    let mut data = String::new();
    let sum = [r#"LEN(SUBSTITUTE(UPPER(REPT("x",Z1)),"X","yy"))"#; 2].join("+");
    for r in 1..=rows {
        data.push_str(&format!(
            r#"<row r="{r}"><c r="B{r}"><f>{sum}+{r}</f><v>{r}</v></c></row>"#
        ));
    }
    let sheet = format!(
        r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>{data}</sheetData></worksheet>"#
    );
    zip_of(&[
        ("[Content_Types].xml", types.to_owned()),
        ("_rels/.rels", ROOT_RELS.replace("PART", "xl/workbook.xml")),
        ("xl/workbook.xml", book.to_owned()),
        ("xl/_rels/workbook.xml.rels", book_rels.to_owned()),
        ("xl/worksheets/sheet1.xml", sheet),
    ])
}

fn after(delay: Duration, flag: &Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
    let flag = Arc::clone(flag);
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        flag.store(true, Ordering::Relaxed);
    })
}

#[test]
fn layout_stops_soon_after_the_flag_is_set() {
    let target = Target::open_docx(&big_docx(1500)).expect("open");
    let doc = target.docx().expect("docx");
    let fonts = FontLibrary::bundled();

    let started = Instant::now();
    let full = layout_document(doc, &fonts, LayoutOptions::default());
    let full_time = started.elapsed();
    assert!(full.pages.len() > 10, "the generated document is long");

    // Already cancelled: nothing is laid out.
    let set = Arc::new(AtomicBool::new(true));
    let started = Instant::now();
    assert!(layout_document_cancellable(doc, &fonts, LayoutOptions::default(), &set).is_none());
    assert!(started.elapsed() < full_time / 4 + Duration::from_millis(100));

    // Cancelled part way: the run ends well before a full layout would.
    let flag = Arc::new(AtomicBool::new(false));
    let timer = after(full_time / 10, &flag);
    let started = Instant::now();
    let out = layout_document_cancellable(doc, &fonts, LayoutOptions::default(), &flag);
    let took = started.elapsed();
    timer.join().expect("timer");
    assert!(out.is_none(), "a cancelled layout returns nothing");
    assert!(
        took < full_time / 2,
        "cancelled after {took:?}, a full layout takes {full_time:?}"
    );

    // The flag is per call: an unset one lays out as before.
    let clear = Arc::new(AtomicBool::new(false));
    let again = layout_document_cancellable(doc, &fonts, LayoutOptions::default(), &clear)
        .expect("not cancelled");
    assert_eq!(again.pages.len(), full.pages.len());
}

#[test]
fn a_cancelled_evaluation_leaves_the_engine_usable() {
    let bytes = heavy_xlsx(300);
    let target = Target::open_xlsx(&bytes).expect("open");
    let wb = target.xlsx().expect("xlsx");
    let keep = |_: usize, _: u32, _: u32| true;
    let mut engine = FormulaEngine::build(wb, &keep);

    let started = Instant::now();
    engine
        .evaluate(wb, 0, 1, 26, Some("30000"), &keep)
        .expect("full evaluation");
    let full_time = started.elapsed();

    let flag = Arc::new(AtomicBool::new(false));
    let timer = after(full_time / 10, &flag);
    let started = Instant::now();
    let err = engine
        .evaluate_cancellable(wb, (0, 1, 26), Some("30000"), &keep, &flag)
        .expect_err("cancelled");
    let took = started.elapsed();
    timer.join().expect("timer");
    assert!(matches!(err, CommandError::Cancelled), "{err}");
    assert!(
        took < full_time / 2,
        "cancelled after {took:?}, a full evaluation takes {full_time:?}"
    );

    // The probe cell was put back and the engine still answers correctly.
    let got = engine
        .evaluate(wb, 0, 1, 26, Some("2+3"), &keep)
        .expect("evaluation after a cancel");
    assert_eq!(
        serde_json::to_value(&got.result).expect("json"),
        json!({"type": "number", "value": 5.0})
    );
    let cell = engine
        .evaluate(wb, 0, 1, 26, Some("B1"), &keep)
        .expect("B1");
    assert_eq!(
        serde_json::to_value(&cell.result).expect("json"),
        json!({"type": "number", "value": 0.0}),
        "Z1 is empty again, so B1 is back to 0"
    );
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("open365-cancel-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn call(server: &mut Server, id: u64, tool: &str, args: Value) -> (Value, bool) {
    let line = json!({"jsonrpc":"2.0","id":id,"method":"tools/call",
                      "params":{"name":tool,"arguments":args}});
    let reply = server.handle_line(&line.to_string()).expect("reply");
    let reply: Value = serde_json::from_str(&reply).expect("json");
    let result = &reply["result"];
    let text = result["content"][0]["text"].as_str().expect("text");
    (
        serde_json::from_str(text).expect("tool json"),
        result["isError"].as_bool().expect("isError"),
    )
}

#[test]
fn a_tiny_deadline_times_out_fast_and_the_engine_cache_survives() {
    let scratch = Scratch::new("deadline");
    std::fs::write(scratch.0.join("big.docx"), big_docx(6000)).expect("write");
    std::fs::write(scratch.0.join("heavy.xlsx"), heavy_xlsx(300)).expect("write");
    std::fs::write(scratch.0.join("small.docx"), big_docx(2)).expect("write");
    let mut server = Server::new(
        Config {
            roots: vec![scratch.0.clone()],
            allow_edit: true,
            deadline: Duration::from_millis(1000),
            ..Config::default()
        },
        Box::new(std::io::sink()),
    )
    .expect("server");
    let mut n = 1;
    let mut next = move || {
        n += 1;
        n
    };
    let mut open = |server: &mut Server, name: &str| -> String {
        let p = scratch.0.join(name);
        let (v, err) = call(
            server,
            next(),
            "open_document",
            json!({"path": p.to_str().expect("utf8")}),
        );
        assert!(!err, "{v}");
        v["document"].as_str().expect("id").to_owned()
    };
    let big = open(&mut server, "big.docx");
    let small = open(&mut server, "small.docx");
    let heavy = open(&mut server, "heavy.xlsx");

    // Layout and export on a large document time out, fast, and write nothing.
    let started = Instant::now();
    let (v, err) = call(&mut server, 100, "layout_summary", json!({"document": big}));
    assert!(err && v["status"] == "timeout", "{v}");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    let started = Instant::now();
    let (v, err) = call(
        &mut server,
        101,
        "export_pdf",
        json!({"document": big, "path": "late.pdf"}),
    );
    assert!(err && v["status"] == "timeout", "{v}");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );

    // The engine is built (cheap: Z1 is empty), then a heavy evaluation is cancelled.
    let eval = |server: &mut Server, id: u64, formula: &str| {
        call(
            server,
            id,
            "evaluate_formula",
            json!({"document": heavy, "sheet": 0, "cell": "Z1", "formula": formula}),
        )
    };
    let (v, err) = eval(&mut server, 102, "0");
    assert!(!err && v["result"]["value"] == 0.0, "{v}");
    let started = Instant::now();
    let (v, err) = eval(&mut server, 103, "30000");
    assert!(err && v["status"] == "timeout", "{v}");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    // The cancelled job ends at its next check, so this does not queue behind it for long and
    // the same engine answers.
    let (v, err) = eval(&mut server, 104, "2+3");
    assert!(!err, "{v}");
    assert_eq!(v["result"]["value"], 5.0, "{v}");

    // Other documents are unaffected.
    let (v, err) = call(
        &mut server,
        105,
        "layout_summary",
        json!({"document": small}),
    );
    assert!(!err, "{v}");

    // No partial or late PDF appears, and no temporary file is left behind.
    std::thread::sleep(Duration::from_millis(1500));
    assert!(!scratch.0.join("late.pdf").exists());
    assert!(std::fs::read_dir(&scratch.0).expect("ls").all(|e| {
        !e.expect("e")
            .file_name()
            .to_string_lossy()
            .ends_with(".open365-tmp")
    }));
}
