//! Fill: weekday and month names continue, a date counts days, Ctrl+D and Ctrl+R copy.

#![allow(clippy::unwrap_used)]

use open365_preview::sheet::SheetModel;

fn select(m: &mut SheetModel, a: (u32, u32), b: (u32, u32)) {
    m.select_cell(a);
    m.focus = b;
}

#[test]
fn weekdays_months_and_dates_continue_when_the_handle_is_dragged() {
    let mut m = SheetModel::blank();
    m.enter(1, 1, "Mon").unwrap();
    m.enter(1, 2, "December").unwrap();
    select(&mut m, (1, 1), (1, 2));
    assert!(m.fill_to((5, 2)).unwrap());
    assert_eq!(m.source(2, 1), "Tue");
    assert_eq!(m.source(5, 1), "Fri");
    assert_eq!(m.source(2, 2), "January");
    assert_eq!(m.source(3, 2), "February");
    // A weekday list wraps.
    m.enter(10, 1, "Sat").unwrap();
    m.enter(11, 1, "Mon").unwrap();
    select(&mut m, (10, 1), (11, 1));
    assert!(m.fill_to((13, 1)).unwrap());
    assert_eq!(m.source(12, 1), "Wed");
}

#[test]
fn ctrl_d_and_ctrl_r_copy_and_shift_formulas() {
    let mut m = SheetModel::blank();
    m.enter(1, 1, "5").unwrap();
    m.enter(1, 2, "=A1*2").unwrap();
    select(&mut m, (1, 1), (4, 2));
    assert!(m.fill_copy(true).unwrap());
    assert_eq!(m.source(4, 1), "5");
    assert_eq!(m.source(3, 2), "=A3*2");
    assert_eq!(m.display(4, 2), "10");
    m.enter(6, 1, "x").unwrap();
    select(&mut m, (6, 1), (6, 3));
    assert!(m.fill_copy(false).unwrap());
    assert_eq!(m.source(6, 3), "x");
    // A single cell copies from the cell above.
    select(&mut m, (6, 2), (6, 2));
    m.enter(5, 2, "above").unwrap();
    select(&mut m, (6, 2), (6, 2));
    assert!(m.fill_copy(true).unwrap());
    assert_eq!(m.source(6, 2), "above");
}
