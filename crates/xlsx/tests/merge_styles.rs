//! What Excel does to the styles of the cells a merge covers
//! (`fixtures/reference/xlsx/autofit-merge-styles/merge.json`, `tools/office/make-autofit-refs.ps1`):
//! each scenario's "before" cells are rebuilt, merged with `Workbook::merge_cells`, and the fill and
//! borders of every cell must equal Excel's saved result.

mod common;

use open365_opc::Limits;
use open365_xlsx::{
    BorderEdge, BorderPatch, BorderStyle, Color, ColorSource, FillSpec, FormatPatch, MergeKind,
    Rgb, Workbook,
};
use serde_json::Value;

use common::{fixture, repo_root};

fn edge(text: &Value) -> BorderEdge {
    let Some(text) = text.as_str() else {
        return BorderEdge::default();
    };
    let (style, color) = text.split_once(' ').expect("style and colour");
    let style = match style {
        "thin" => BorderStyle::Thin,
        "medium" => BorderStyle::Medium,
        other => panic!("border style {other}"),
    };
    BorderEdge {
        style,
        color: colour(color),
    }
}

fn colour(text: &str) -> Color {
    match text
        .strip_prefix("rgb:FF")
        .and_then(|h| u32::from_str_radix(h, 16).ok())
    {
        Some(v) => Color {
            source: ColorSource::Rgb(Rgb::new((v >> 16) as u8, (v >> 8) as u8, v as u8)),
            tint: 0.0,
        },
        None => match text.strip_prefix("indexed:").and_then(|i| i.parse().ok()) {
            Some(i) => Color {
                source: ColorSource::Indexed(i),
                tint: 0.0,
            },
            None => Color::AUTO,
        },
    }
}

fn cell_ref(name: &str) -> (u32, u32) {
    let (col, row) = name.split_at(name.find(|c: char| c.is_ascii_digit()).expect("present"));
    (
        row.parse().expect("present"),
        col.bytes().fold(0, |n, b| n * 26 + u32::from(b - b'A') + 1),
    )
}

fn range(name: &str) -> (u32, u32, u32, u32) {
    let (a, b) = name.split_once(':').unwrap_or((name, name));
    let ((r1, c1), (r2, c2)) = (cell_ref(a), cell_ref(b));
    (r1, c1, r2, c2)
}

#[test]
fn merging_restyles_the_covered_cells_as_excel_does() {
    let path = repo_root().join("fixtures/reference/xlsx/autofit-merge-styles/merge.json");
    let json: Value =
        serde_json::from_slice(&std::fs::read(path).expect("present")).expect("present");
    let mut checked = 0;
    for s in json["scenarios"].as_array().expect("present") {
        let how = s["how"].as_str().expect("present");
        if how == "unmerge" {
            continue;
        }
        let mut wb = Workbook::open(&fixture("merged-cells"), &Limits::default()).expect("present");
        let area = range(s["range"].as_str().expect("present"));
        for (name, c) in s["before"]["cells"].as_object().expect("present") {
            let (r, col) = cell_ref(name);
            if r > 3 || col > 4 || r < 2 || col < 2 {
                continue;
            }
            let fill = c["fill"].as_str().expect("present");
            let patch = FormatPatch {
                fill: fill
                    .strip_prefix("solid ")
                    .map(|f| FillSpec::Solid(colour(f))),
                border: BorderPatch {
                    left: Some(edge(&c["left"])),
                    right: Some(edge(&c["right"])),
                    top: Some(edge(&c["top"])),
                    bottom: Some(edge(&c["bottom"])),
                    touch: true,
                    ..BorderPatch::default()
                },
                ..FormatPatch::default()
            };
            wb.format_range(0, (r, col, r, col), &patch)
                .expect("present");
        }
        let kind = if how == "across" {
            MergeKind::Across
        } else {
            MergeKind::Merge
        };
        wb.merge_cells(0, area, kind).expect("present");
        for (name, c) in s["after"]["cells"].as_object().expect("present") {
            let (r, col) = cell_ref(name);
            if r > 3 || col > 4 || r < 2 || col < 2 {
                continue;
            }
            let f = wb.cell_format(0, r, col);
            let label = format!("{} / {name}", s["name"].as_str().expect("present"));
            for (side, got, want) in [
                ("left", f.border.left, &c["left"]),
                ("right", f.border.right, &c["right"]),
                ("top", f.border.top, &c["top"]),
                ("bottom", f.border.bottom, &c["bottom"]),
            ] {
                assert_eq!(got, edge(want), "{label}: {side}");
                checked += 1;
            }
            let want_fill = c["fill"]
                .as_str()
                .expect("present")
                .strip_prefix("solid ")
                .map(colour);
            let got_fill = (f.fill.pattern != open365_xlsx::FillPattern::None).then_some(f.fill.fg);
            assert_eq!(got_fill, want_fill, "{label}: fill");
            checked += 1;
        }
    }
    assert!(checked > 80, "only {checked} checks");
}
