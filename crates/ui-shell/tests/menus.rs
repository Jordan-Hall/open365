//! Menu, colour picker, gallery and list geometry against the popups the reference measured
//! (`fixtures/reference/ui/excel-popups.json`, 125 %, physical px = `u`), +-1 px.
//!
//! Widths follow the text and so are not compared (the test passes the measured width as the minimum);
//! heights and every row position are. The Excel colour picker has a "Gallery Filters" strip (52 u) that
//! Open365 does not draw, so the picker is compared without it.

#![allow(clippy::unwrap_used, clippy::cast_possible_truncation)]

use open365_ui_shell::menu::plan;
use open365_ui_shell::{EntryKind, MenuEntry, Swatch, Tile};
use serde_json::Value;

type Row = (String, String, String, f32, f32);

fn fixture() -> Value {
    let p = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/reference/ui/excel-popups.json"
    );
    serde_json::from_str(&std::fs::read_to_string(p).expect("fixture")).expect("json")
}

/// The first popup window of control `id`: (width, height, rows as (type, name, class, y, h)).
fn popup(fx: &Value, id: &str) -> (f32, f32, Vec<Row>) {
    let p = fx["popups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == id)
        .unwrap_or_else(|| panic!("no popup {id}"));
    let w = &p["windows"][0];
    let items = w["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            let f = |k: usize| i[k].as_f64().unwrap() as f32;
            (
                i[0].as_str().unwrap().to_owned(),
                i[1].as_str().unwrap().to_owned(),
                i[3].as_str().unwrap().to_owned(),
                f(5),
                f(7),
            )
        })
        .collect();
    (
        w["rect"][2].as_f64().unwrap() as f32,
        w["rect"][3].as_f64().unwrap() as f32,
        items,
    )
}

fn text_w(_: &str) -> f32 {
    10.0
}

/// Rebuilds a plain menu from the recorded rows: groups with a name become headings, a 3 u hole
/// between two rows a separator, row heights are taken as recorded.
fn menu_from(items: &[Row]) -> Vec<MenuEntry> {
    let mut entries = Vec::new();
    let mut end = 2.0_f32;
    for (ty, name, _cls, y, h) in items {
        let leaf = ty == "MenuItem" || ty == "ListItem";
        let head = ty == "Group" && !name.trim().is_empty();
        if !leaf && !head {
            continue;
        }
        if head {
            entries.push(MenuEntry::heading(name));
            end = *y + 37.0;
            continue;
        }
        if *y - end >= 2.5 {
            entries.push(MenuEntry::separator());
        }
        let mut e = MenuEntry::new("x", name, true);
        if (*h - 36.0).abs() > 0.5 {
            e.height = Some(*h as u32);
        }
        entries.push(e);
        end = *y + *h;
    }
    entries
}

fn check_menu(fx: &Value, id: &str) {
    let (w, h, items) = popup(fx, id);
    let entries = menu_from(&items);
    let p = plan(&entries, w, 10_000.0, &text_w);
    assert!(
        (p.height - h).abs() <= 1.0,
        "{id}: height {} vs {h}",
        p.height
    );
    let mut k = 0;
    for (ty, name, _cls, y, rh) in &items {
        if ty == "MenuItem" || ty == "ListItem" {
            while entries[k].kind != EntryKind::Item {
                k += 1;
            }
            let r = &p.rows[k];
            assert!(
                (r.rect[1] - y).abs() <= 1.0 && (r.rect[3] - rh).abs() <= 1.0,
                "{id}: row {name}: {:?} vs y {y} h {rh}",
                r.rect
            );
            k += 1;
        }
    }
}

#[test]
fn text_menus_match_the_reference() {
    let fx = fixture();
    for id in [
        "UnderlineGallery_Dropdown",
        "AccountingFormatMenu_Dropdown",
        "FormatCellsMenu",
        "BordersGallery_Dropdown",
        "ClearMenu",
        "FillMenu",
        "ConditionalFormattingMenu",
    ] {
        check_menu(&fx, id);
    }
}

#[test]
fn combo_lists_match_the_reference() {
    let fx = fixture();
    check_menu(&fx, "NumberFormatGallery");
    // The size list: 30 u rows, 16 of them visible, then a 21 u scroll bar.
    let sizes: Vec<MenuEntry> = (0..17)
        .map(|i| {
            let mut e = MenuEntry::new("s", &i.to_string(), true);
            e.height = Some(30);
            e.centred = true;
            e
        })
        .collect();
    let (w, h, items) = popup(&fx, "FontSize");
    let p = plan(&sizes, 67.0, 16.0 * 30.0 + 12.0, &text_w);
    assert!(p.scrolls);
    assert!(
        (p.height - h).abs() <= 1.0,
        "size list height {} vs {h}",
        p.height
    );
    assert!(
        (p.width - w).abs() <= 1.0,
        "size list width {} vs {w}",
        p.width
    );
    let first = items.iter().find(|i| i.2 == "NetUIGalleryButton").unwrap();
    assert!((p.rows[0].rect[1] - first.3).abs() <= 1.0);
    assert!((p.rows[1].rect[1] - p.rows[0].rect[1] - 30.0).abs() <= 1.0);
}

fn swatch_row(n: usize) -> Vec<Swatch> {
    (0..n)
        .map(|i| Swatch::new("c", "c", [i as u8; 3]))
        .collect()
}

fn tail(mut v: Vec<MenuEntry>) -> Vec<MenuEntry> {
    v.push(MenuEntry::separator());
    v.push(MenuEntry::new("m", "More Colours...", true));
    v
}

#[test]
fn colour_pickers_match_the_reference() {
    let fx = fixture();
    let filter = 52.0;
    let mut auto = MenuEntry::new("a", "Automatic", true);
    auto.height = Some(34);
    let font = tail(vec![
        auto,
        MenuEntry::separator(),
        MenuEntry::heading("Theme Colours"),
        MenuEntry::swatches(vec![swatch_row(10)], false),
        MenuEntry::separator(),
        MenuEntry::swatches(vec![swatch_row(10); 5], true),
        MenuEntry::separator(),
        MenuEntry::heading("Standard Colours"),
        MenuEntry::swatches(vec![swatch_row(10)], false),
    ]);
    let (w, h, _) = popup(&fx, "FontColorPicker_Dropdown");
    let p = plan(&font, 0.0, 10_000.0, &text_w);
    assert!(
        (p.width - w).abs() <= 1.0,
        "picker width {} vs {w}",
        p.width
    );
    assert!(
        (p.height + filter - h).abs() <= 1.0,
        "font picker {} vs {h}",
        p.height
    );
    // Cell rings measured at (4, 131), (177) and (384) with the 52 u filter strip above.
    let theme = &p.rows[3];
    assert!((theme.cells[0][0] - 4.0).abs() <= 1.0);
    assert!((theme.cells[0][1] - (131.0 - filter)).abs() <= 1.0);
    assert!((theme.cells[1][0] - theme.cells[0][0] - 34.0).abs() <= 1.0);
    let tints = &p.rows[5];
    assert!(
        (tints.cells[0][1] - (177.0 - filter)).abs() <= 1.0,
        "tint row {:?}",
        tints.cells[0]
    );
    assert!((tints.cells[10][1] - tints.cells[0][1] - 30.0).abs() <= 1.0);
    let std_row = &p.rows[8];
    assert!((std_row.cells[0][1] - (384.0 - filter)).abs() <= 1.0);
    let mut none = MenuEntry::new("n", "No Fill", true);
    none.height = Some(34);
    let fill = tail(vec![
        MenuEntry::heading("Theme Colours"),
        MenuEntry::swatches(vec![swatch_row(10)], false),
        MenuEntry::separator(),
        MenuEntry::swatches(vec![swatch_row(10); 5], true),
        MenuEntry::separator(),
        MenuEntry::heading("Standard Colours"),
        MenuEntry::swatches(vec![swatch_row(10)], false),
        MenuEntry::separator(),
        none,
    ]);
    let (_, h, _) = popup(&fx, "CellFillColorPicker_Dropdown");
    let p = plan(&fill, 0.0, 10_000.0, &text_w);
    assert!(
        (p.height + filter - h).abs() <= 1.0,
        "fill picker {} vs {h}",
        p.height
    );
}

#[test]
fn cell_style_gallery_matches_the_reference() {
    let fx = fixture();
    let tiles = |n: usize| -> Vec<Tile> {
        (0..n)
            .map(|i| Tile {
                id: i.to_string(),
                label: i.to_string(),
                fill: None,
                ink: None,
                checked: false,
            })
            .collect()
    };
    let mut entries = Vec::new();
    for (i, (title, n)) in [
        ("Good, Bad and Neutral", 4),
        ("Data and Model", 7),
        ("Titles and Headings", 2),
        ("Themed Cell Styles", 24),
        ("Number Format", 2),
    ]
    .into_iter()
    .enumerate()
    {
        if i > 0 {
            entries.push(MenuEntry::separator());
        }
        entries.push(MenuEntry::heading(title));
        entries.push(MenuEntry::tiles(6, (133, 36), tiles(n)));
    }
    let (w, _, items) = popup(&fx, "CellStylesGallery");
    let p = plan(&entries, 0.0, 10_000.0, &text_w);
    assert!(
        (p.width - (w - 21.0)).abs() <= 1.0,
        "gallery width {} vs {w}",
        p.width
    );
    let firsts: Vec<f32> = [
        "Normal",
        "Calculation",
        "Heading 1",
        "Light Turquoise, 20% - Accent1",
        "Comma",
    ]
    .iter()
    .map(|n| items.iter().find(|i| i.1.starts_with(n)).unwrap().3)
    .collect();
    let mut g = 0;
    for r in &p.rows {
        if matches!(entries[r.index].kind, EntryKind::Tiles { .. }) {
            assert!(
                (r.cells[0][1] - firsts[g]).abs() <= 1.0,
                "group {g}: tile y {} vs {}",
                r.cells[0][1],
                firsts[g]
            );
            g += 1;
        }
    }
    assert_eq!(g, 5);
}
