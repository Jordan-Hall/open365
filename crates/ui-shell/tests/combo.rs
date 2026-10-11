//! The editable Font box of both applications: typing, inline auto-complete, Enter commits through the host and
//! Escape cancels (the same code serves the Font Size box and Sheets' Number Format).

use egui::{Event, Key, Modifiers, PointerButton, Pos2, pos2, vec2};
use egui_kittest::Harness;
use open365_ui_shell::layout::{LayoutInput, compute};
use open365_ui_shell::spec::AppSpec;
use open365_ui_shell::{AppKind, CommandState, MenuEntry, Mode, Shell, ShellHost};

const W: f32 = 1700.0;
const H: f32 = 700.0;
/// The test harness's panel margin around the shell.
const M: f32 = 8.0;
const ID: &str = "home.font.font";

#[derive(Default)]
struct Host {
    ran: Vec<String>,
    closed: u32,
}

impl ShellHost for Host {
    fn state(&self, _id: &str) -> CommandState {
        CommandState::ENABLED
    }
    fn run(&mut self, id: &str, _value: Option<f64>) {
        self.ran.push(id.to_owned());
    }
    fn title(&self) -> String {
        "Document1".to_owned()
    }
    fn status_text(&self, _id: &str) -> Option<String> {
        None
    }
    fn menu(&self, id: &str) -> Option<Vec<MenuEntry>> {
        (id == ID).then(|| {
            ["Arial", "Calibri", "Cambria", "Candara", "Times New Roman"]
                .iter()
                .map(|f| MenuEntry::new(&format!("{ID}#{f}"), f, true))
                .collect()
        })
    }
    /// An empty box: the editor then starts with nothing to replace (headless egui collapses a selection on the
    /// next frame because no IME owns the text edit, so a select-all start state cannot be observed here).
    fn value(&self, id: &str) -> Option<String> {
        (id == ID).then(String::new)
    }
    fn combo_commit(&mut self, id: &str, text: &str) -> Option<String> {
        (!text.trim().is_empty()).then(|| format!("{id}#{}", text.trim()))
    }
    fn chrome_closed(&mut self) {
        self.closed += 1;
    }
    fn central(&mut self, ui: &mut egui::Ui) {
        ui.label("document");
    }
}

type Rig = Harness<'static, (Shell, Host)>;

fn rig(kind: AppKind) -> Rig {
    let mut h = Harness::builder().with_size(vec2(W, H)).build_ui_state(
        |ui, (shell, host): &mut (Shell, Host)| shell.show(ui, host),
        (Shell::new(kind, Mode::Dark), Host::default()),
    );
    h.run();
    h
}

/// Where the Font box's text area is on the Home tab (window `W` x `H`, scale 1).
fn font_box(kind: AppKind) -> Pos2 {
    let spec = match kind {
        AppKind::Writer => AppSpec::writer(),
        AppKind::Sheets => AppSpec::sheets(),
    };
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
    let c = lay
        .groups
        .iter()
        .flat_map(|g| g.controls.iter())
        .find(|c| c.control.id == ID)
        .expect("font combo");
    let r = c.rect;
    pos2(M + r.x as f32 + 20.0, M + r.y as f32 + r.h as f32 / 2.0)
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

fn type_chars(h: &mut Rig, text: &str) {
    for c in text.chars() {
        h.event(Event::Text(c.to_string()));
        h.step();
    }
    h.run();
}

fn edited(h: &Rig) -> Option<String> {
    h.state().0.combo_edit().map(|(_, t)| t.to_owned())
}

fn each_app(check: impl Fn(AppKind)) {
    for kind in [AppKind::Writer, AppKind::Sheets] {
        check(kind);
    }
}

#[test]
fn clicking_the_text_starts_an_edit_and_the_first_letter_completes_inline() {
    each_app(|kind| {
        let mut h = rig(kind);
        assert!(h.state().0.combo_edit().is_none());
        click(&mut h, font_box(kind));
        assert_eq!(h.state().0.combo_edit().map(|e| e.0), Some(ID), "{kind:?}");
        type_chars(&mut h, "C");
        assert_eq!(
            edited(&h).as_deref(),
            Some("Calibri"),
            "first match, {kind:?}"
        );
    });
}

#[test]
fn text_without_a_match_stays_as_typed() {
    each_app(|kind| {
        let mut h = rig(kind);
        click(&mut h, font_box(kind));
        type_chars(&mut h, "Z");
        assert_eq!(edited(&h).as_deref(), Some("Z"), "{kind:?}");
    });
}

#[test]
fn enter_commits_the_text_through_the_host() {
    each_app(|kind| {
        let mut h = rig(kind);
        click(&mut h, font_box(kind));
        type_chars(&mut h, "A");
        assert_eq!(edited(&h).as_deref(), Some("Arial"), "{kind:?}");
        h.key_press(Key::Enter);
        h.run();
        assert_eq!(h.state().1.ran, vec![format!("{ID}#Arial")], "{kind:?}");
        assert!(h.state().0.combo_edit().is_none(), "{kind:?}");
        assert!(
            h.state().1.closed > 0,
            "the document takes the keyboard back, {kind:?}"
        );
    });
}

#[test]
fn escape_cancels_without_running_anything() {
    each_app(|kind| {
        let mut h = rig(kind);
        click(&mut h, font_box(kind));
        type_chars(&mut h, "T");
        assert_eq!(edited(&h).as_deref(), Some("Times New Roman"), "{kind:?}");
        h.key_press(Key::Escape);
        h.run();
        assert!(h.state().1.ran.is_empty(), "{kind:?}");
        assert!(h.state().0.combo_edit().is_none(), "{kind:?}");
        assert!(h.state().1.closed > 0, "{kind:?}");
    });
}
