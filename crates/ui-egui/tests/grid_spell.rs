//! Spell checking in the Sheets cell editor: a red wavy underline under a misspelled word in the text being edited,
//! none under formulas, and the right-click menu with the suggestions, Ignore All and Add to Dictionary.

use std::time::Duration;

use egui::{Event, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use open365_commands::Target;
use open365_ui_egui::GridView;

struct State {
    view: GridView,
    target: Target,
}

fn harness() -> Harness<'static, State> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/xlsx/chart-area.xlsx");
    let target = Target::open_xlsx(&std::fs::read(path).expect("fixture")).expect("open");
    let mut h = Harness::builder()
        .with_size(vec2(900.0, 500.0))
        .build_ui_state(
            |ui, s: &mut State| {
                s.view.show(ui, &mut s.target);
            },
            State {
                view: GridView::new(),
                target,
            },
        );
    h.run();
    h
}

fn squiggles(h: &Harness<'_, State>) -> usize {
    h.output()
        .shapes
        .iter()
        .filter(|s| match &s.shape {
            egui::Shape::Path(p) => {
                p.stroke.color
                    == egui::epaint::ColorMode::Solid(egui::Color32::from_rgb(0xff, 0x4a, 0x4a))
            }
            _ => false,
        })
        .count()
}

fn edit(h: &mut Harness<'_, State>, text: &str) {
    let ctx = h.ctx.clone();
    h.state_mut().view.start_edit(&ctx, text.to_owned());
    for _ in 0..3 {
        h.step();
    }
}

fn run_until(h: &mut Harness<'_, State>, f: impl Fn(&Harness<'_, State>) -> bool) -> bool {
    for _ in 0..300 {
        h.step();
        if f(h) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

#[test]
fn a_misspelled_word_in_the_cell_editor_is_underlined() {
    let mut h = harness();
    edit(&mut h, "Total quik fox");
    assert!(run_until(&mut h, |h| squiggles(h) >= 1));
}

#[test]
fn formulas_are_not_checked() {
    let mut h = harness();
    edit(&mut h, "=SUM(A1:A3)+quik");
    for _ in 0..40 {
        h.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(squiggles(&h), 0);
}

#[test]
fn right_click_in_the_editor_offers_suggestions_ignore_all_and_add() {
    let mut h = harness();
    edit(&mut h, "Total quik fox");
    assert!(run_until(&mut h, |h| squiggles(h) >= 1));
    // The squiggle's polyline says where the word is.
    let pos = h
        .output()
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Path(p) => {
                let a = p.points.first()?;
                let b = p.points.last()?;
                Some(Pos2::new((a.x + b.x) / 2.0, a.y - 6.0))
            }
            _ => None,
        })
        .expect("squiggle");
    h.event(Event::PointerMoved(pos));
    h.event(Event::PointerButton {
        pos,
        button: PointerButton::Secondary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    h.event(Event::PointerButton {
        pos,
        button: PointerButton::Secondary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.step();
    h.step();
    assert!(h.state().view.spell_menu().is_some());
    h.get_by_label("Ignore All");
    h.get_by_label("Add to Dictionary");
    h.get_by_label("quick");
    h.get_by_label("Ignore All").click();
    h.step();
    h.step();
    assert!(run_until(&mut h, |h| squiggles(h) == 0));
}
