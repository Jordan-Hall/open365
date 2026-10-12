//! No ribbon label is cut off. Office collapses a group to one button as the window narrows; it
//! never shows half a word. Every control label and group label of an expanded group must fit
//! the box the layout gives it, measured with the UI font at the size the painter uses.

use egui::{FontId, Ui, vec2};
use egui_kittest::Harness;
use open365_ui_shell::fonts;
use open365_ui_shell::layout::{LayoutInput, compute, fit_large_labels, ribbon_label, wrap_label};
use open365_ui_shell::spec::{AppSpec, Kind, Scale, Size};
use open365_ui_shell::theme::size;

const PPP: f32 = 1.25;

/// Widths from "maximised" down to the narrowest the ribbon is specified for.
const WIDTHS: [i32; 12] = [
    1900, 1800, 1700, 1600, 1500, 1400, 1300, 1200, 1100, 1000, 900, 800,
];

fn misses(spec: &AppSpec, ui: &Ui) -> Vec<String> {
    let text_w = |s: &str, px: f32| {
        ui.painter()
            .layout_no_wrap(s.to_owned(), FontId::proportional(px), egui::Color32::BLACK)
            .size()
            .x
    };
    let mut out = Vec::new();
    let inset = spec.pane[0];
    for width in WIDTHS {
        for tab in 0..spec.tabs.len() {
            let lay = compute(
                spec,
                &LayoutInput {
                    scale: PPP,
                    window: (width, spec.ref_window[1]),
                    inset: (inset, spec.pane[1], inset, inset),
                    tab,
                    ribbon_open: true,
                    formula_expanded: true,
                },
            );
            for g in &lay.groups {
                if g.scale == Scale::Collapsed {
                    continue;
                }
                let at = format!(
                    "{} {width} {}/{}",
                    spec.app, spec.tabs[tab].label, g.group.label
                );
                // A group squeezed below its label wraps it onto two lines, at a word gap.
                let avail = g.label.w as f32 / PPP + 1.0;
                let group_w = |s: &str| text_w(s, size::GROUP);
                let lines = if group_w(&g.group.label) > avail {
                    wrap_label(&g.group.label, 0, avail, group_w)
                } else {
                    vec![g.group.label.clone()]
                };
                if lines.iter().any(|l| group_w(l) > avail) {
                    out.push(format!("{at}: group label does not fit {avail:.1}"));
                }
                for pc in &g.controls {
                    let c = pc.control;
                    if c.label.is_empty()
                        || !matches!(c.kind, Kind::Button | Kind::Split | Kind::Dropdown)
                    {
                        continue;
                    }
                    // A smaller control drops its label for its icon when the label does not
                    // fit; a large one has nowhere to go, so its label must fit.
                    if c.size != Size::Large {
                        continue;
                    }
                    let font = |s: &str| text_w(s, size::LABEL);
                    let room = pc.rect.w as f32 / PPP;
                    let lines = wrap_label(ribbon_label(&c.label), 7, room, font);
                    let need = lines.iter().map(|l| font(l)).fold(0.0, f32::max);
                    if need > room + 0.5 {
                        out.push(format!("{at}/{}: needs {need:.1}, has {room:.1}", c.label));
                    }
                }
            }
        }
    }
    out
}

fn run(spec: AppSpec) -> Vec<String> {
    let mut h = Harness::builder()
        .with_size(vec2(300.0, 100.0))
        .build_ui_state(
            |ui, (spec, out): &mut (AppSpec, Vec<String>)| {
                fonts::install(ui.ctx());
                let w = |s: &str| {
                    ui.painter()
                        .layout_no_wrap(
                            s.to_owned(),
                            FontId::proportional(size::LABEL),
                            egui::Color32::BLACK,
                        )
                        .size()
                        .x
                };
                // The shell widens large buttons to their labels before the first frame is laid out.
                fit_large_labels(spec, w, PPP);
                *out = misses(spec, ui);
            },
            (spec, Vec::new()),
        );
    h.run();
    h.run();
    h.state().1.clone()
}

#[test]
fn writer_labels_are_never_truncated() {
    let m = run(AppSpec::writer());
    assert!(m.is_empty(), "{} truncated:\n{}", m.len(), m.join("\n"));
}

#[test]
fn sheets_labels_are_never_truncated() {
    let m = run(AppSpec::sheets());
    assert!(m.is_empty(), "{} truncated:\n{}", m.len(), m.join("\n"));
}

fn control_labels(spec: &AppSpec) -> Vec<(String, String)> {
    let mut v = Vec::new();
    for t in &spec.tabs {
        for g in &t.groups {
            v.push((g.id.clone(), g.label.clone()));
            let blocks = std::iter::once(&g.blocks).chain(g.states.iter().map(|s| &s.blocks));
            for b in blocks.flatten() {
                for it in b.rows.iter().flat_map(|r| &r.items) {
                    v.push((it.id.clone(), it.label.clone()));
                }
            }
        }
    }
    v
}

/// Labels that deliberately start with a lowercase letter (a product spelling Office itself uses).
const LOWERCASE_START: &[&str] = &["eSignature", "eSignature fields"];

fn lowercase_starts(spec: &AppSpec) -> Vec<String> {
    control_labels(spec)
        .into_iter()
        .filter(|(_, l)| l.chars().next().is_some_and(char::is_lowercase))
        .filter(|(_, l)| !LOWERCASE_START.contains(&l.as_str()))
        .map(|(id, l)| format!("{id}: {l:?}"))
        .collect()
}

#[test]
fn no_label_starts_with_a_lowercase_letter() {
    for spec in [AppSpec::writer(), AppSpec::sheets()] {
        let bad = lowercase_starts(&spec);
        assert!(bad.is_empty(), "{}: {}", spec.app, bad.join(", "));
    }
}

#[test]
fn proofread_buttons_keep_their_full_label() {
    let spec = AppSpec::writer();
    let found: Vec<_> = control_labels(&spec)
        .into_iter()
        .filter(|(id, _)| id.ends_with(".proofread.proofread") || id == "review.proofing.proofread")
        .collect();
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(found.iter().all(|(_, l)| l == "Proofread"), "{found:?}");
}

#[test]
fn a_large_button_grows_to_fit_its_label() {
    let mut spec = AppSpec::writer();
    // A stand-in font: 7 pt per character, so "Proofread" (63 pt, 79 u) cannot fit 52 u.
    let w = |s: &str| 7.0 * s.chars().count() as f32;
    let width_of = |spec: &AppSpec, id: &str| {
        spec.tabs
            .iter()
            .flat_map(|t| &t.groups)
            .flat_map(|g| g.blocks.iter().flat_map(|b| &b.rows).flat_map(|r| &r.items))
            .find(|c| c.id == id)
            .map(|c| c.w)
            .unwrap()
    };
    let group_w = |spec: &AppSpec, id: &str| {
        spec.tabs
            .iter()
            .flat_map(|t| &t.groups)
            .find(|g| g.id == id)
            .unwrap()
            .w
    };
    let (cw, gw) = (
        width_of(&spec, "review.proofing.proofread"),
        group_w(&spec, "review.proofing"),
    );
    fit_large_labels(&mut spec, w, PPP);
    let (cw2, gw2) = (
        width_of(&spec, "review.proofing.proofread"),
        group_w(&spec, "review.proofing"),
    );
    assert!(cw2 > cw && cw2 as f32 >= 63.0 * PPP, "{cw} -> {cw2}");
    assert!(gw2 > gw, "the group grows with its button: {gw} -> {gw2}");
    // Fitting is stable.
    let again = width_of(
        &{
            let mut s = spec.clone();
            fit_large_labels(&mut s, w, PPP);
            s
        },
        "review.proofing.proofread",
    );
    assert_eq!(again, cw2);
}
