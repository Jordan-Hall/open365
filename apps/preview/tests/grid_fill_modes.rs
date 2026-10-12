//! Fill handle variants: Ctrl toggles copy and series, month-spaced dates continue by month,
//! custom lists continue, and the AutoFill Options redo the last fill.

#![allow(clippy::unwrap_used)]

use open365_preview::sheet::{FillMode, SheetModel};

fn select(m: &mut SheetModel, a: (u32, u32), b: (u32, u32)) {
    m.select_cell(a);
    m.focus = b;
}

fn num(m: &SheetModel, r: u32, c: u32) -> f64 {
    m.source(r, c).parse::<f64>().unwrap()
}

#[test]
fn a_lone_number_copies_and_ctrl_makes_it_count() {
    let mut m = SheetModel::blank();
    m.enter(1, 1, "5").unwrap();
    select(&mut m, (1, 1), (1, 1));
    assert!(m.fill_to((3, 1)).unwrap());
    assert_eq!((m.source(2, 1), m.source(3, 1)), ("5".into(), "5".into()));
    m.enter(1, 3, "5").unwrap();
    select(&mut m, (1, 3), (1, 3));
    assert!(m.fill_with((3, 3), FillMode::Ctrl).unwrap());
    assert_eq!((m.source(2, 3), m.source(3, 3)), ("6".into(), "7".into()));
}

#[test]
fn ctrl_copies_text_that_would_count_and_a_number_run() {
    let mut m = SheetModel::blank();
    m.enter(1, 1, "Item1").unwrap();
    m.enter(1, 2, "1").unwrap();
    m.enter(2, 2, "2").unwrap();
    select(&mut m, (1, 1), (1, 1));
    m.fill_with((3, 1), FillMode::Ctrl).unwrap();
    assert_eq!(m.source(3, 1), "Item1");
    select(&mut m, (1, 2), (2, 2));
    m.fill_with((4, 2), FillMode::Ctrl).unwrap();
    assert_eq!((m.source(3, 2), m.source(4, 2)), ("1".into(), "2".into()));
    // Without Ctrl the run counts.
    select(&mut m, (1, 2), (2, 2));
    m.fill_to((6, 2)).unwrap();
    assert_eq!(m.source(6, 2), "6");
}

#[test]
fn custom_lists_continue_and_wrap() {
    let mut m = SheetModel::blank();
    m.custom_lists
        .push(vec!["Low".into(), "Medium".into(), "High".into()]);
    m.enter(1, 1, "Medium").unwrap();
    select(&mut m, (1, 1), (1, 1));
    m.fill_to((3, 1)).unwrap();
    assert_eq!(
        (m.source(2, 1), m.source(3, 1)),
        ("High".into(), "Low".into())
    );
}

#[test]
fn a_date_counts_days_and_keeps_its_format() {
    let mut m = SheetModel::blank();
    m.select_cell((1, 3));
    m.enter_now(false).unwrap();
    m.fill_to((3, 3)).unwrap();
    assert_eq!(num(&m, 2, 3) - num(&m, 1, 3), 1.0);
    assert_eq!(num(&m, 3, 3) - num(&m, 1, 3), 2.0);
    assert_eq!(
        m.display(3, 3).matches('/').count(),
        2,
        "{}",
        m.display(3, 3)
    );
}

#[test]
fn autofill_options_redo_the_last_fill() {
    let mut m = SheetModel::blank();
    m.enter(1, 1, "Mon").unwrap();
    select(&mut m, (1, 1), (1, 1));
    m.fill_to((3, 1)).unwrap();
    assert_eq!(m.source(3, 1), "Wed");
    assert!(m.refill(FillMode::Copy).unwrap());
    assert_eq!(
        (m.source(2, 1), m.source(3, 1)),
        ("Mon".into(), "Mon".into())
    );
    assert!(m.refill(FillMode::Series).unwrap());
    assert_eq!(m.source(3, 1), "Wed");
}
