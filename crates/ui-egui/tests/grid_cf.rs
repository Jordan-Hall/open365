//! Conditional formats in the grid, against Excel's `Range.DisplayFormat`
//! (`fixtures/reference/xlsx/grid-cf/displayformat.json`): fill, font colour and style and the
//! dxf number format text of every cell the grid draws, then data bars and icon sets, and the
//! re-evaluation after an edit and its undo.

use std::path::PathBuf;

use egui::{Color32, Event, Key, vec2};
use egui_kittest::Harness;
use open365_ui_egui::{App, PaintedCell};
use open365_xlsx::parse_cell_ref;
use serde_json::Value as Json;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn reference() -> Json {
    let path = root().join("fixtures/reference/xlsx/grid-cf/displayformat.json");
    serde_json::from_slice(&std::fs::read(path).expect("reference")).expect("json")
}

fn harness() -> Harness<'static, App> {
    let today = reference()["today"].as_f64().expect("today");
    let mut app = App::new(Some(root().join("fixtures/xlsx/grid-cf.xlsx")));
    app.grid.set_cf_today(Some(today));
    let mut h = Harness::builder()
        .with_size(vec2(5200.0, 420.0))
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app);
    h.run();
    h.run();
    h
}

fn painted(h: &Harness<'_, App>, row: u32, col: u32) -> Option<PaintedCell> {
    h.state()
        .grid
        .painted()
        .into_iter()
        .find(|p| p.row == row && p.col == col)
}

/// Excel's BGR integer as the colour the grid should draw.
fn bgr(v: i64) -> Color32 {
    Color32::from_rgb(
        (v & 0xFF) as u8,
        ((v >> 8) & 0xFF) as u8,
        ((v >> 16) & 0xFF) as u8,
    )
}

#[test]
fn drawn_cells_match_excel_display_format() {
    let h = harness();
    let reference = reference();
    let mut compared = 0;
    let mut failures = Vec::new();
    for cell in reference["cells"].as_array().expect("cells") {
        let at = cell["ref"].as_str().expect("ref");
        let (row, col) = parse_cell_ref(at).expect("address");
        let Some(p) = painted(&h, row, col) else {
            // A blank cell has no text to log.
            if cell["text"].as_str() == Some("") {
                continue;
            }
            failures.push(format!("{at}: not drawn"));
            continue;
        };
        compared += 1;
        let mut bad = Vec::new();
        if !p.hashes && p.text != cell["text"].as_str().unwrap() {
            bad.push(format!("text {:?} != {:?}", p.text, cell["text"]));
        }
        // Data bars and icon sets have no DisplayFormat equivalent.
        if (36..=38).contains(&col) {
            continue;
        }
        if cell["pattern"].as_i64().unwrap() != -4142
            && p.fill != Some(bgr(cell["fill"].as_i64().unwrap()))
        {
            bad.push(format!("fill {:?} != {}", p.fill, cell["fill"]));
        }
        let want_color = cell["font_color"].as_i64().unwrap();
        if want_color != 0 && p.font_color != bgr(want_color) {
            bad.push(format!("font colour {:?} != {want_color}", p.font_color));
        }
        if p.bold != cell["bold"].as_bool().unwrap()
            || p.italic != cell["italic"].as_bool().unwrap()
            || p.strike != cell["strike"].as_bool().unwrap()
        {
            bad.push("font style".to_owned());
        }
        if !bad.is_empty() {
            failures.push(format!("{at}: {}", bad.join("; ")));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {compared}: {failures:#?}",
        failures.len()
    );
    assert!(compared >= 490, "{compared} cells compared");
}

#[test]
fn data_bars_and_icons_are_drawn() {
    let h = harness();
    let bars: Vec<_> = (2..=13)
        .filter_map(|r| painted(&h, r, 36))
        .filter(|p| p.data_bar.is_some())
        .collect();
    assert!(bars.len() >= 10, "data bar column: {} bars", bars.len());
    let icons: Vec<_> = (2..=13)
        .flat_map(|r| [37, 38].into_iter().map(move |c| (r, c)))
        .filter_map(|(r, c)| painted(&h, r, c))
        .filter(|p| p.icon.is_some())
        .collect();
    assert!(!icons.is_empty(), "icon set columns draw icons");
}

#[test]
fn edit_and_undo_re_evaluate_the_rules() {
    let mut h = harness();
    // D2:D13 strikes values up to 21; D3 holds 87.
    assert!(!painted(&h, 3, 4).unwrap().strike);
    h.state_mut().grid.set_active(3, 4);
    h.run();
    h.event(Event::Text("5".to_owned()));
    h.run();
    h.key_press(Key::Enter);
    h.run();
    h.run();
    assert!(painted(&h, 3, 4).unwrap().strike, "5 is within the rule");
    h.state_mut().undo();
    h.run();
    h.run();
    assert!(!painted(&h, 3, 4).unwrap().strike, "undo restores 87");
}
