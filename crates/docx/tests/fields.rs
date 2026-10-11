//! Field update against Microsoft Word: every `fixtures/docx/fields-<set>.docx` is opened, its
//! fields are updated, and each field's result (and each paragraph's text and style) is
//! compared with what Word produced (`fixtures/reference/docx/fields-<set>/fields.json`,
//! recorded by `tools/office/fields-oracle.ps1`).

mod common;

use std::collections::BTreeMap;
use std::fmt::Write as _;

use open365_docx::{CivilDateTime, Document, FieldContext, PageInfo, PageProvider, SkipReason};
use open365_opc::Limits;

use common::repo_root;

// ---------------------------------------------------------------------------
// A small JSON reader (the crate has no serde_json dependency)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
#[allow(dead_code)]
enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    fn get(&self, key: &str) -> &Json {
        match self {
            Json::Obj(items) => items
                .iter()
                .find(|(k, _)| k == key)
                .map_or(&Json::Null, |(_, v)| v),
            _ => &Json::Null,
        }
    }
    fn str(&self) -> &str {
        match self {
            Json::Str(s) => s,
            _ => "",
        }
    }
    fn num(&self) -> f64 {
        match self {
            Json::Num(n) => *n,
            _ => 0.0,
        }
    }
    fn arr(&self) -> &[Json] {
        match self {
            Json::Arr(a) => a,
            _ => &[],
        }
    }
}

struct Reader<'a> {
    s: &'a [u8],
    i: usize,
}

impl Reader<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn value(&mut self) -> Json {
        self.ws();
        match self.s[self.i] {
            b'{' => {
                self.i += 1;
                let mut items = Vec::new();
                loop {
                    self.ws();
                    if self.s[self.i] == b'}' {
                        self.i += 1;
                        break;
                    }
                    if self.s[self.i] == b',' {
                        self.i += 1;
                        continue;
                    }
                    let Json::Str(key) = self.value() else {
                        panic!("object key")
                    };
                    self.ws();
                    assert_eq!(self.s[self.i], b':');
                    self.i += 1;
                    items.push((key, self.value()));
                }
                Json::Obj(items)
            }
            b'[' => {
                self.i += 1;
                let mut items = Vec::new();
                loop {
                    self.ws();
                    if self.s[self.i] == b']' {
                        self.i += 1;
                        break;
                    }
                    if self.s[self.i] == b',' {
                        self.i += 1;
                        continue;
                    }
                    items.push(self.value());
                }
                Json::Arr(items)
            }
            b'"' => {
                self.i += 1;
                let mut out: Vec<u16> = Vec::new();
                loop {
                    let c = self.s[self.i];
                    self.i += 1;
                    match c {
                        b'"' => break,
                        b'\\' => {
                            let e = self.s[self.i];
                            self.i += 1;
                            match e {
                                b'n' => out.push(10),
                                b'r' => out.push(13),
                                b't' => out.push(9),
                                b'b' => out.push(8),
                                b'f' => out.push(12),
                                b'u' => {
                                    let hex = std::str::from_utf8(&self.s[self.i..self.i + 4])
                                        .expect("hex");
                                    out.push(u16::from_str_radix(hex, 16).expect("hex digits"));
                                    self.i += 4;
                                }
                                other => out.push(u16::from(other)),
                            }
                        }
                        other => out.push(u16::from(other)),
                    }
                }
                Json::Str(String::from_utf16_lossy(&out))
            }
            b'n' => {
                self.i += 4;
                Json::Null
            }
            b't' => {
                self.i += 4;
                Json::Bool(true)
            }
            b'f' => {
                self.i += 5;
                Json::Bool(false)
            }
            _ => {
                let start = self.i;
                while self.i < self.s.len()
                    && matches!(
                        self.s[self.i],
                        b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9'
                    )
                {
                    self.i += 1;
                }
                Json::Num(
                    std::str::from_utf8(&self.s[start..self.i])
                        .expect("number")
                        .parse()
                        .expect("number"),
                )
            }
        }
    }
}

fn parse_json(text: &str) -> Json {
    Reader {
        s: text.as_bytes(),
        i: 0,
    }
    .value()
}

// ---------------------------------------------------------------------------
// Reference loading
// ---------------------------------------------------------------------------

struct Pages {
    /// Per paragraph: (physical page, shown number, section).
    by_paragraph: Vec<(u32, u32)>,
    total: u32,
    section_of_paragraph: Vec<usize>,
    section_pages: Vec<u32>,
    number_styles: Vec<u32>,
}

impl PageProvider for Pages {
    fn page_at(&self, pos: open365_docx::DocPos) -> Option<PageInfo> {
        let (page, number) = *self.by_paragraph.get(pos.para)?;
        Some(PageInfo {
            page,
            number,
            label: {
                let section = *self.section_of_paragraph.get(pos.para)?;
                let roman = |lower: bool| {
                    let mut n = number;
                    let mut out = String::new();
                    for (v, s) in [
                        (1000, "m"),
                        (900, "cm"),
                        (500, "d"),
                        (400, "cd"),
                        (100, "c"),
                        (90, "xc"),
                        (50, "l"),
                        (40, "xl"),
                        (10, "x"),
                        (9, "ix"),
                        (5, "v"),
                        (4, "iv"),
                        (1, "i"),
                    ] {
                        while n >= v {
                            out.push_str(s);
                            n -= v;
                        }
                    }
                    if lower { out } else { out.to_uppercase() }
                };
                match self.number_styles.get(section) {
                    Some(1) => Some(roman(false)),
                    Some(2) => Some(roman(true)),
                    _ => None,
                }
            },
            section: *self.section_of_paragraph.get(pos.para)?,
        })
    }
    fn total_pages(&self) -> Option<u32> {
        Some(self.total)
    }
    fn section_pages(&self, section: usize) -> Option<u32> {
        self.section_pages.get(section).copied()
    }
}

struct Reference {
    json: Json,
    fixture: Vec<u8>,
}

fn load(set: &str) -> Reference {
    let root = repo_root();
    let json_path = root
        .join("fixtures/reference/docx")
        .join(format!("fields-{set}"))
        .join("fields.json");
    let text = std::fs::read_to_string(&json_path)
        .unwrap_or_else(|e| panic!("{}: {e}", json_path.display()));
    let fixture = std::fs::read(
        root.join("fixtures/docx")
            .join(format!("fields-{set}.docx")),
    )
    .expect("fixture");
    Reference {
        json: parse_json(&text),
        fixture,
    }
}

/// `_Toc123456789` -> `_Toc#`: Word numbers the bookmarks it creates, the numbers carry no meaning.
fn norm_code(code: &str) -> String {
    let mut out = String::new();
    let mut rest = code;
    while let Some(i) = rest.find("_Toc") {
        out.push_str(&rest[..i + 4]);
        rest = &rest[i + 4..];
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        if digits > 0 {
            out.push('#');
        }
        rest = &rest[digits..];
    }
    out.push_str(rest);
    out
}

fn civil(text: &str) -> CivilDateTime {
    CivilDateTime::parse_utc(text).expect("clock")
}

fn clean(s: &str) -> String {
    s.trim_end_matches(['\r', '\u{7}']).to_owned()
}

/// Text before the first character that is not printable ASCII (Word sometimes appends
/// uninitialised memory to error messages).
fn ascii_prefix(s: &str) -> &str {
    let end = s
        .char_indices()
        .find(|(_, c)| {
            !(c.is_ascii() && (!c.is_control() || *c == '\t' || *c == '\r' || *c == '\n'))
        })
        .map_or(s.len(), |(i, _)| i);
    &s[..end]
}

/// Style and text of every paragraph against Word's (table-row marks left out).
fn paragraph_diffs(doc: &Document, json: &Json) -> Vec<String> {
    // Page breaks and line breaks are both `\n` in the model.
    let norm = |t: &str| t.trim_end_matches('\r').replace(['\n', '\u{c}'], "\u{b}");
    let key = |s: &str| s.replace(' ', "").to_ascii_lowercase();
    let want: Vec<(String, String)> = json
        .get("paragraphs")
        .arr()
        .iter()
        .filter(|p| p.get("text").str() != "\r\u{7}")
        .map(|p| (key(p.get("style").str()), norm(p.get("text").str())))
        .collect();
    let styles = doc.styles();
    let have: Vec<(String, String)> = doc
        .body()
        .paragraphs()
        .into_iter()
        .map(|p| {
            let name = p
                .style_id()
                .and_then(|id| styles.and_then(|s| s.style(id)))
                .map_or_else(|| "Normal".to_owned(), |i| i.name.clone());
            (key(&name), norm(&p.text()))
        })
        .collect();
    let mut out = Vec::new();
    if want.len() != have.len() {
        out.push(format!(
            "paragraph count: ours {} vs Word {}",
            have.len(),
            want.len()
        ));
    }
    for (i, (w, h)) in want.iter().zip(&have).enumerate() {
        if w != h {
            out.push(format!(
                "paragraph #{i}\n    ours : {h:?}\n    word : {w:?}"
            ));
        }
    }
    out
}

/// Positions, in Word's paragraph list (split-result paragraphs left out), of the end-of-row marks of
/// the tables in `blocks`.
fn row_end_positions(blocks: &[open365_docx::Block]) -> Vec<usize> {
    fn walk(blocks: &[open365_docx::Block], n: &mut usize, ends: &mut Vec<usize>) {
        for b in blocks {
            match b {
                open365_docx::Block::Paragraph(_) => *n += 1,
                open365_docx::Block::Table(t) => {
                    for child in &t.children {
                        let open365_docx::TableChild::Row(row) = child else {
                            continue;
                        };
                        for c in &row.children {
                            if let open365_docx::RowChild::Cell(cell) = c {
                                walk(&cell.blocks, n, ends);
                            }
                        }
                        ends.push(*n);
                        *n += 1;
                    }
                }
                _ => {}
            }
        }
    }
    let mut ends = Vec::new();
    walk(blocks, &mut 0, &mut ends);
    ends
}

struct Outcome {
    total: usize,
    matched: usize,
    skipped: usize,
    mismatches: Vec<String>,
}

fn run(set: &str) -> Outcome {
    let reference = load(set);
    let ctx_json = reference.json.get("context");
    let mut doc = Document::open(&reference.fixture, &Limits::default()).expect("open fixture");
    // Page numbers: a paragraph's page is the page of its start.
    let paragraphs = reference.json.get("paragraphs").arr();
    let mut by_paragraph = Vec::new();
    let mut section_of_paragraph = Vec::new();
    if set == "toc" {
        // The fixture is stale: its tables of contents differ from the updated ones, so its
        // paragraph numbers differ from the reference's. Match the body paragraphs by text.
        let norm = |t: &str| t.trim_end_matches('\r').replace(['\n', '\u{b}'], " ");
        let is_toc_style = |s: &str| {
            let s = s.replace(' ', "").to_ascii_lowercase();
            s.starts_with("toc") || s.starts_with("tableoffigures")
        };
        let wanted: Vec<(String, u32, u32)> = paragraphs
            .iter()
            .filter(|p| !is_toc_style(p.get("style").str()))
            .map(|p| {
                (
                    norm(p.get("text").str()),
                    p.get("page").num() as u32,
                    p.get("label").num() as u32,
                )
            })
            .collect();
        let styles = doc.styles().cloned();
        let mut j = 0usize;
        let mut last = (1u32, 1u32);
        for p in doc.body().paragraphs() {
            let in_toc = p
                .style_id()
                .and_then(|id| styles.as_ref().and_then(|s| s.style(id)))
                .is_some_and(|i| is_toc_style(&i.name));
            let text = norm(&p.text());
            if !in_toc
                && !text.is_empty()
                && let Some(k) = (j..wanted.len()).find(|&k| wanted[k].0 == text)
            {
                last = (wanted[k].1, wanted[k].2);
                j = k + 1;
            }
            by_paragraph.push(last);
            section_of_paragraph.push(0);
        }
    } else {
        // Word's list has an end-of-row mark (`\r\u{7}`) after the cells of every table row; the model
        // has none. Marks are found by walking the model's tables (an empty cell reads the same).
        // A field result that ends in a paragraph mark (REF to a whole-paragraph bookmark) splits
        // its paragraph in Word's list: the extra empty paragraph has no counterpart in the model.
        let split_results: Vec<&str> = reference
            .json
            .get("fields")
            .arr()
            .iter()
            .map(|f| f.get("result").str())
            .filter(|r| r.len() > 1 && r.ends_with('\r'))
            .collect();
        let row_ends = row_end_positions(doc.body().blocks.as_slice());
        let sections = doc.sections();
        let mut idx = 0usize; // model paragraph index
        let mut word_pos = 0usize; // position in Word's list without the extra split paragraphs
        let mut previous = "";
        for p in paragraphs {
            let text = p.get("text").str();
            let extra = text == "\r" && split_results.contains(&previous);
            previous = text;
            if extra {
                continue;
            }
            let pos = word_pos;
            word_pos += 1;
            if row_ends.contains(&pos) {
                continue;
            }
            by_paragraph.push((p.get("page").num() as u32, p.get("label").num() as u32));
            let section = sections
                .iter()
                .position(|s| s.end_paragraph.is_none_or(|e| e >= idx))
                .unwrap_or(0);
            section_of_paragraph.push(section);
            idx += 1;
        }
    }
    let mut section_pages = vec![0u32; section_of_paragraph.iter().copied().max().unwrap_or(0) + 1];
    {
        let mut seen: Vec<Vec<u32>> = vec![Vec::new(); section_pages.len()];
        for (i, (page, _)) in by_paragraph.iter().enumerate() {
            let s = section_of_paragraph[i];
            if !seen[s].contains(page) {
                seen[s].push(*page);
            }
        }
        for (i, pages) in seen.iter().enumerate() {
            section_pages[i] = u32::try_from(pages.len()).unwrap_or(0);
        }
    }
    let provider = Pages {
        by_paragraph,
        total: ctx_json.get("pages").num() as u32,
        section_of_paragraph,
        section_pages,
        number_styles: ctx_json
            .get("number_styles")
            .arr()
            .iter()
            .map(|n| n.num() as u32)
            .collect(),
    };
    let mut ctx = FieldContext::new(civil(ctx_json.get("now_before").str()));
    ctx.utc_offset_minutes = ctx_json.get("utc_offset_minutes").num() as i32;
    ctx.file_name = Some(ctx_json.get("file_name").str().to_owned());
    ctx.file_path = Some(ctx_json.get("file_path").str().to_owned());
    let ctx = ctx.with_pages(&provider);
    let report = doc.update_fields(&ctx).expect("update");

    // Fields in order. Word lists nested fields too, in begin order.
    // Word lists `w:hyperlink` elements as HYPERLINK fields; this model keeps them as text.
    let is_link = |code: &str| code.trim_start().starts_with("HYPERLINK");
    let actual: Vec<_> = doc
        .fields()
        .into_iter()
        .filter(|f| !is_link(&f.instruction.text))
        .collect();
    let expected: Vec<&Json> = reference
        .json
        .get("fields")
        .arr()
        .iter()
        .filter(|f| !is_link(f.get("code").str()))
        .collect();
    let mut mismatches = Vec::new();
    let mut matched = 0usize;
    let mut skipped_names: BTreeMap<String, usize> = BTreeMap::new();
    for s in &report.skipped {
        let reason = match s.reason {
            SkipReason::Locked => "locked",
            SkipReason::UnsupportedType => "unsupported",
            SkipReason::MissingInput => "missing-input",
        };
        *skipped_names
            .entry(format!("{reason}:{}", s.name))
            .or_default() += 1;
    }
    if actual.len() != expected.len() {
        mismatches.push(format!(
            "field count: ours {} vs Word {}",
            actual.len(),
            expected.len()
        ));
    }
    for (i, want) in expected.iter().enumerate() {
        let Some(have) = actual.get(i) else { break };
        let code_want = norm_code(want.get("code").str().trim());
        let code_have = norm_code(have.instruction.text.trim());
        let want_result = clean(want.get("result").str());
        let have_result = clean(&have.result_text);
        let ok = if code_want != code_have {
            false
        } else if want_result == have_result {
            true
        } else {
            let prefix = ascii_prefix(&want_result);
            prefix.len() != want_result.len() && have_result.starts_with(prefix)
        };
        if ok {
            matched += 1;
        } else {
            mismatches.push(format!(
                "#{i} [{code_want}]\n    ours : {have_result:?}\n    word : {want_result:?}{}",
                if code_want == code_have {
                    String::new()
                } else {
                    format!("\n    (our code: {code_have:?})")
                }
            ));
        }
    }
    if set == "toc" {
        mismatches.extend(paragraph_diffs(&doc, &reference.json));
    }
    let mut summary = String::new();
    for (k, v) in &skipped_names {
        let _ = write!(summary, "{k}={v} ");
    }
    eprintln!(
        "fields-{set}: {matched}/{} fields match; skipped: {summary}",
        expected.len()
    );
    let dir = repo_root().join("target-pdf");
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(
        dir.join(format!("fields-{set}-mismatch.txt")),
        mismatches.join("\n"),
    );
    Outcome {
        total: expected.len(),
        matched,
        skipped: report.skipped.len(),
        mismatches,
    }
}

#[test]
fn parse_instructions_without_changing_the_document() {
    let reference = load("format");
    let doc = Document::open(&reference.fixture, &Limits::default()).expect("open");
    let fields = doc.fields();
    assert!(fields.len() > 1000);
    assert!(fields.iter().all(|f| f.depth == 0));
    let mut saved_doc = Document::open(&reference.fixture, &Limits::default()).expect("open");
    let saved = saved_doc.save().expect("save");
    let before =
        open365_opc::Package::read(&reference.fixture, &Limits::default()).expect("package");
    let after = open365_opc::Package::read(&saved, &Limits::default()).expect("package");
    for part in before.parts() {
        assert_eq!(
            part.data(),
            after.part(part.name().as_str()).expect("part").data()
        );
    }
}

#[test]
fn format_set_matches_word() {
    let out = run("format");
    report_failures("format", &out);
}

/// Every date field matches Word, including the genitive month names languages use next to a day number.
#[test]
fn date_set_matches_word() {
    let out = run("date");
    report_failures("date", &out);
}

fn report_failures(set: &str, out: &Outcome) {
    let shown: Vec<&String> = out.mismatches.iter().take(25).collect();
    assert!(
        out.mismatches.is_empty(),
        "fields-{set}: {}/{} match ({} skipped); first mismatches:\n{}",
        out.matched,
        out.total,
        out.skipped,
        shown
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn refs_set_matches_word() {
    let out = run("refs");
    report_failures("refs", &out);
}

/// Table formulas over cells with separators, brackets, signs, words and exponents.
#[test]
fn tables_set_matches_word() {
    let out = run("tables");
    report_failures("tables", &out);
}

#[test]
fn toc_set_matches_word() {
    let out = run("toc");
    report_failures("toc", &out);
}

#[test]
fn props_set_matches_word() {
    let out = run("props");
    report_failures("props", &out);
}

#[test]
fn format_set_updates_quickly() {
    let start = std::time::Instant::now();
    let out = run("format");
    assert!(out.mismatches.is_empty());
    assert!(start.elapsed().as_secs() < 5, "{:?}", start.elapsed());
}
