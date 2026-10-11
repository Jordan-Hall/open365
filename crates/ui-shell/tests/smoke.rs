//! Draws one frame of each shell headlessly and checks the chrome reaches the host.

use egui::vec2;
use egui_kittest::Harness;
use open365_ui_shell::{AppKind, CommandState, Mode, Shell, ShellHost};

struct Host {
    ran: Vec<String>,
    centre_drawn: bool,
}

impl ShellHost for Host {
    fn state(&self, id: &str) -> CommandState {
        if id == "qat.filesave" {
            CommandState::ENABLED
        } else {
            CommandState::DISABLED
        }
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
    fn central(&mut self, ui: &mut egui::Ui) {
        self.centre_drawn = true;
        ui.label("document");
    }
}

#[test]
fn both_shells_draw_a_frame() {
    for (kind, tabs) in [(AppKind::Writer, 9), (AppKind::Sheets, 10)] {
        let shell = Shell::new(kind, Mode::Light);
        assert_eq!(shell.spec().tabs.len(), tabs);
        let host = Host {
            ran: Vec::new(),
            centre_drawn: false,
        };
        let mut h = Harness::builder()
            .with_size(vec2(1280.0, 800.0))
            .build_ui_state(
                |ui, (shell, host): &mut (Shell, Host)| shell.show(ui, host),
                (shell, host),
            );
        h.run();
        assert!(h.state().1.centre_drawn, "document area was drawn");
    }
}

#[test]
fn specs_have_control_ids() {
    for spec in [
        open365_ui_shell::spec::AppSpec::writer(),
        open365_ui_shell::spec::AppSpec::sheets(),
    ] {
        for t in &spec.tabs {
            for g in &t.groups {
                for b in &g.blocks {
                    for r in &b.rows {
                        for c in &r.items {
                            assert!(!c.id.is_empty());
                        }
                    }
                }
            }
        }
    }
}
