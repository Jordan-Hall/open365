//! Hide, unhide, zero-size and select-all size edits against Excel:
//! `fixtures/reference/xlsx/grid-hide/xml.json` holds the `sheetFormatPr`, `cols` and `row` start
//! tags Excel wrote after each edit scenario applied to `fixtures/xlsx/grid-hide.xlsx`. The
//! commands must produce the same attributes (`spans` and the Office `dyDescent` hint aside), and
//! undo must give back the original sheet part byte for byte.

use std::collections::BTreeSet;
use std::path::PathBuf;

use open365_commands::{Command, Target, apply};
use open365_opc::{Limits, Package};
use serde_json::Value;

const SCENARIOS: [&str; 6] = ["Hide", "Zero", "Unhide", "AllRows", "AllCols", "SetVisible"];
const MAX_ROW: u32 = 1_048_576;
const MAX_COL: u32 = 16_384;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn open() -> Target {
    Target::open_path(&root().join("fixtures/xlsx/grid-hide.xlsx")).expect("opens")
}

fn sheet_part(target: &mut Target, index: usize) -> String {
    let bytes = target.snapshot_bytes().expect("saves");
    let pkg = Package::read(&bytes, &Limits::default()).expect("package");
    let name = format!("xl/worksheets/sheet{}.xml", index + 1);
    String::from_utf8(pkg.part(&name).expect("sheet part").data().to_vec()).expect("utf-8")
}

/// Start tags beginning with `prefix` (`"<row "`), in order.
fn tags(xml: &str, prefix: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(at) = rest.find(prefix) {
        let tail = &rest[at..];
        let end = tail.find('>').expect("tag end") + 1;
        out.push(tail[..end].to_owned());
        rest = &tail[end..];
    }
    out
}

/// Attributes of a start tag as `name=value`, without `spans` and `x14ac:dyDescent`.
fn attrs(tag: &str) -> BTreeSet<String> {
    let body = tag
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim_end_matches('/');
    let mut out = BTreeSet::new();
    let mut rest = body.split_once(' ').map_or("", |(_, r)| r);
    while let Some((name, tail)) = rest.split_once("=\"") {
        let (value, after) = tail.split_once('"').expect("closing quote");
        let name = name.trim();
        if name != "spans" && name != "x14ac:dyDescent" {
            out.insert(format!("{name}={value}"));
        }
        rest = after;
    }
    out
}

fn all(tags: &[String]) -> Vec<BTreeSet<String>> {
    tags.iter().map(|t| attrs(t)).collect()
}

fn edit(name: &str) -> Vec<Command> {
    let cols = |first, last, width| Command::SetColumnWidth {
        sheet: 0,
        first,
        last,
        width,
    };
    let rows = |first, last, points| Command::SetRowHeight {
        sheet: 0,
        first,
        last,
        points,
    };
    let sheet = SCENARIOS.iter().position(|s| *s == name).expect("known");
    let mut cmds = match name {
        "Hide" => vec![
            Command::HideColumns {
                sheet,
                first: 2,
                last: 2,
            },
            Command::HideColumns {
                sheet,
                first: 3,
                last: 3,
            },
            Command::HideColumns {
                sheet,
                first: 4,
                last: 5,
            },
            Command::HideRows {
                sheet,
                first: 3,
                last: 3,
            },
            Command::HideRows {
                sheet,
                first: 4,
                last: 4,
            },
            Command::HideRows {
                sheet,
                first: 6,
                last: 6,
            },
        ],
        "Zero" => vec![
            cols(2, 2, 0.0),
            cols(6, 6, 0.0),
            rows(3, 3, 0.0),
            rows(4, 4, 0.0),
            rows(6, 6, 0.0),
        ],
        // 8.88671875 is the default width Excel stored on the 120 dpi machine of the run.
        "Unhide" => vec![
            Command::UnhideColumns {
                sheet,
                first: 2,
                last: 4,
                default_width: Some(8.88671875),
            },
            Command::UnhideRows {
                sheet,
                first: 3,
                last: 6,
            },
        ],
        "AllRows" => vec![rows(1, MAX_ROW, 25.05)],
        "AllCols" => vec![cols(1, MAX_COL, 15.77734375)],
        "SetVisible" => vec![
            cols(2, 3, 12.77734375),
            rows(3, 3, 19.95),
            rows(4, 4, 19.95),
        ],
        _ => unreachable!(),
    };
    // The plain size commands address sheet 0 above; point them at the scenario's sheet.
    for c in &mut cmds {
        match c {
            Command::SetColumnWidth { sheet: s, .. } | Command::SetRowHeight { sheet: s, .. } => {
                *s = sheet;
            }
            _ => {}
        }
    }
    cmds
}

#[test]
fn every_scenario_writes_what_excel_writes_and_undoes_cleanly() {
    let reference: Value = serde_json::from_slice(
        &std::fs::read(root().join("fixtures/reference/xlsx/grid-hide/xml.json")).expect("ref"),
    )
    .expect("json");
    for (index, name) in SCENARIOS.iter().enumerate() {
        let mut target = open();
        let original = sheet_part(&mut target, index);
        let commands = edit(name);
        for c in &commands {
            apply(&mut target, c.clone()).unwrap_or_else(|e| panic!("{name}: {c:?}: {e}"));
        }
        let got = sheet_part(&mut target, index);
        let want = &reference["scenarios"][*name];

        let want_fmt = attrs(want["sheetFormatPr"].as_str().unwrap());
        let got_fmt = attrs(&tags(&got, "<sheetFormatPr")[0]);
        assert_eq!(got_fmt, want_fmt, "{name}: sheetFormatPr");

        let want_cols: Vec<String> = tags(want["cols"].as_str().unwrap(), "<col ");
        let got_cols = tags(&got, "<col ");
        assert_eq!(all(&got_cols), all(&want_cols), "{name}: cols");

        let want_rows: Vec<String> = want["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect();
        let got_rows = tags(&got, "<row ");
        assert_eq!(all(&got_rows), all(&want_rows), "{name}: rows");

        for _ in &commands {
            target
                .undo()
                .unwrap_or_else(|e| panic!("{name}: undo: {e}"));
        }
        assert_eq!(sheet_part(&mut target, index), original, "{name}: undo");
    }
}
