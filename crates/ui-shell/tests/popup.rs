//! The popup of a collapsed ribbon group: opens on click, closes on Escape or a click outside, and
//! its key tips still work.

use egui::{Event, Key, Modifiers, PointerButton, Pos2, pos2, vec2};
use egui_kittest::Harness;
use open365_ui_shell::layout::{LayoutInput, compute, popup_group};
use open365_ui_shell::spec::{AppSpec, Scale};
use open365_ui_shell::{AppKind, CommandState, Mode, Shell, ShellHost};

const W: f32 = 700.0;
const H: f32 = 600.0;
/// The test harness's panel margin around the shell.
const M: f32 = 8.0;

#[derive(Default)]
struct Host {
    ran: Vec<String>,
    /// Times the shell said a pop-up closed (the document takes the keyboard back).
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
    fn chrome_closed(&mut self) {
        self.closed += 1;
    }
    fn central(&mut self, ui: &mut egui::Ui) {
        ui.label("document");
    }
}

type Rig = Harness<'static, (Shell, Host)>;

fn rig() -> Rig {
    let mut h = Harness::builder().with_size(vec2(W, H)).build_ui_state(
        |ui, (shell, host): &mut (Shell, Host)| shell.show(ui, host),
        (Shell::new(AppKind::Writer, Mode::Dark), Host::default()),
    );
    h.run();
    h
}

fn click(h: &mut Rig, pos: Pos2) {
    h.event(Event::PointerMoved(pos));
    h.step();
    // One frame each: a press held across several simulated frames is too slow to be a click.
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

fn popup(h: &Rig) -> Option<&str> {
    h.state().0.open_group_popup()
}

/// Where the Writer Home tab puts its groups at this window width.
fn home() -> (AppSpec, Vec<(String, Scale, Pos2, Pos2)>) {
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
    let groups = lay
        .groups
        .iter()
        .map(|g| {
            let r = g.rect;
            (
                g.group.id.clone(),
                g.scale,
                pos2(
                    M + r.x as f32 + r.w as f32 / 2.0,
                    M + r.y as f32 + r.h as f32 / 2.0,
                ),
                pos2(M + r.x as f32, M + (r.y + r.h) as f32),
            )
        })
        .collect();
    (spec, groups)
}

#[test]
fn click_opens_escape_and_outside_click_close() {
    let (_, groups) = home();
    let (id, _, centre, _) = groups
        .iter()
        .find(|g| g.0 == "home.font")
        .expect("font group");
    assert_eq!(
        groups.iter().find(|g| g.0 == *id).map(|g| g.1),
        Some(Scale::Collapsed),
        "the Font group is collapsed at {W} px"
    );
    let mut h = rig();
    assert_eq!(popup(&h), None);
    click(&mut h, *centre);
    assert_eq!(popup(&h), Some("home.font"));

    // The same button toggles it shut.
    click(&mut h, *centre);
    assert_eq!(popup(&h), None);

    // Escape closes it.
    click(&mut h, *centre);
    assert_eq!(popup(&h), Some("home.font"));
    h.key_press(Key::Escape);
    h.run();
    assert_eq!(popup(&h), None);

    // So does a click in the document area.
    click(&mut h, *centre);
    assert_eq!(popup(&h), Some("home.font"));
    click(&mut h, pos2(W / 2.0, H - 100.0));
    assert_eq!(popup(&h), None);
    // Every close told the host once (toggle, Escape, outside click).
    assert_eq!(h.state().1.closed, 3);
}

#[test]
fn a_click_inside_runs_the_command_and_closes() {
    let (spec, groups) = home();
    let (_, _, centre, below) = groups
        .iter()
        .find(|g| g.0 == "home.font")
        .expect("font group");
    let group = spec.tabs[0]
        .groups
        .iter()
        .find(|g| g.id == "home.font")
        .expect("group");
    let full = popup_group(&spec, group, 1.0);
    let bold = full
        .controls
        .iter()
        .find(|c| c.control.id == "home.font.bold")
        .expect("bold");
    let at = pos2(
        below.x + 4.0 + bold.rect.x as f32 + bold.rect.w as f32 / 2.0,
        below.y + 4.0 + bold.rect.y as f32 + bold.rect.h as f32 / 2.0,
    );
    let mut h = rig();
    click(&mut h, *centre);
    assert_eq!(popup(&h), Some("home.font"));
    click(&mut h, at);
    assert_eq!(h.state().1.ran, ["home.font.bold"]);
    assert_eq!(popup(&h), None);
}

#[test]
fn key_tips_work_inside_the_popup() {
    let mut h = rig();
    // Alt on its own shows the tips; H opens the Home tab's tips; ZF is the Font group button.
    h.event(Event::ModifiersChanged(Modifiers::ALT));
    h.run();
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run();
    h.event(Event::Text("h".to_owned()));
    h.run();
    assert_eq!(popup(&h), None);
    h.event(Event::Text("zf".to_owned()));
    h.run();
    assert_eq!(popup(&h), Some("home.font"), "the group's key tip opens it");
    h.event(Event::Text("1".to_owned()));
    h.run();
    assert_eq!(
        h.state().1.ran,
        ["home.font.bold"],
        "Bold's own tip runs inside"
    );
    assert_eq!(popup(&h), None);
}
