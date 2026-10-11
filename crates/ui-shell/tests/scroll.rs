//! The "Column right" / "Column left" buttons of the tab row and of the ribbon body scroll their strip.

use egui::{Event, Modifiers, PointerButton, Pos2, pos2, vec2};
use egui_kittest::Harness;
use open365_ui_shell::layout::{LayoutInput, PxRect, compute};
use open365_ui_shell::spec::AppSpec;
use open365_ui_shell::{AppKind, CommandState, Mode, Shell, ShellHost};

const W: f32 = 460.0;
const H: f32 = 600.0;
const M: f32 = 8.0;

struct Host;

impl ShellHost for Host {
    fn state(&self, _id: &str) -> CommandState {
        CommandState::ENABLED
    }
    fn run(&mut self, _id: &str, _value: Option<f64>) {}
    fn title(&self) -> String {
        "Document1".to_owned()
    }
    fn status_text(&self, _id: &str) -> Option<String> {
        None
    }
    fn central(&mut self, ui: &mut egui::Ui) {
        ui.label("document");
    }
}

type Rig = Harness<'static, (Shell, Host)>;

fn rig() -> Rig {
    let mut h = Harness::builder().with_size(vec2(W, H)).build_ui_state(
        |ui, (shell, host): &mut (Shell, Host)| shell.show(ui, host),
        (Shell::new(AppKind::Writer, Mode::Dark), Host),
    );
    h.run();
    h
}

fn click(h: &mut Rig, pos: Pos2) {
    h.event(Event::PointerMoved(pos));
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.step();
    }
    h.run();
}

fn centre(r: PxRect) -> Pos2 {
    pos2(
        M + r.x as f32 + r.w as f32 / 2.0,
        M + r.y as f32 + r.h as f32 / 2.0,
    )
}

#[test]
fn tab_row_and_ribbon_scroll_buttons_move_their_strip() {
    let spec = AppSpec::writer();
    let lay = compute(
        &spec,
        &LayoutInput {
            scale: 1.0,
            window: ((W - 2.0 * M) as i32, (H - 2.0 * M) as i32),
            inset: (0, 0, 0, 0),
            tab: 0,
            ribbon_open: true,
            formula_expanded: true,
        },
    );
    let tab_btn = lay.tab_scroll.expect("tab row overflows at this width");
    let ribbon_btn = lay
        .ribbon_scroll
        .expect("ribbon overflows at this width")
        .button;
    let mut h = rig();
    assert_eq!(h.state().0.tab_scroll_offset(), 0.0);
    click(&mut h, centre(tab_btn));
    assert!(
        h.state().0.tab_scroll_offset() > 0.0,
        "Column right moves the tabs"
    );
    let moved = h.state().0.tab_scroll_offset();
    // Column left appears at the strip's left end and walks back.
    let first = lay.tab_rects[0];
    click(
        &mut h,
        pos2(
            M + first.x as f32 + 10.0,
            M + first.y as f32 + first.h as f32 / 2.0,
        ),
    );
    assert!(
        h.state().0.tab_scroll_offset() < moved,
        "Column left moves them back"
    );

    assert_eq!(h.state().0.ribbon_scroll_offset(), 0);
    click(&mut h, centre(ribbon_btn));
    assert!(h.state().0.ribbon_scroll_offset() > 0, "the ribbon scrolls");
}
