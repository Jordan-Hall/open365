//! The formula bar and the cell editor show dates, times and percentages the way Excel does.

#![allow(clippy::unwrap_used)]

use open365_calc::FormatLocale;
use open365_preview::sheet::SheetModel;

fn dates() -> SheetModel {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/xlsx/dates-1900.xlsx");
    SheetModel::open(&path).unwrap()
}

#[test]
fn a_date_cell_shows_a_date_in_the_formula_bar_not_its_serial() {
    let mut m = dates();
    m.set_locale(FormatLocale::for_lcid(2057).unwrap());
    let mut found = 0;
    for r in 1..=12 {
        for c in 1..=4 {
            let raw = m.source(r, c);
            let shown = m.display(r, c);
            let is_date = raw.parse::<f64>().is_ok() && shown.contains(['/', '-']) && shown != raw;
            if !is_date {
                continue;
            }
            found += 1;
            let bar = m.bar_text(r, c);
            assert_ne!(bar, raw, "{r},{c}: the serial leaked into the bar");
            assert!(bar.matches('/').count() == 2, "{bar} is not dd/mm/yyyy");
            // Entering the text the bar shows changes nothing.
            let rev = m.is_dirty();
            m.enter(r, c, &bar).unwrap();
            assert_eq!(m.source(r, c), raw, "the number survives the round trip");
            assert_eq!(m.is_dirty(), rev);
        }
    }
    assert!(found > 0, "the fixture has date cells");
}

#[test]
fn history_state_follows_the_command_history() {
    let mut m = SheetModel::blank();
    assert!(!m.can_undo() && !m.can_redo());
    m.enter(1, 1, "5").unwrap();
    assert!(m.can_undo() && !m.can_redo());
    assert!(m.undo());
    assert!(!m.can_undo() && m.can_redo());
    assert!(m.redo());
    assert!(m.can_undo() && !m.can_redo());
}
