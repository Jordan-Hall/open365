//! Spell checking in the document view: language spans from `w:lang`/`w:noProof`, red wavy underlines, and the
//! right-click menu (five suggestions at most, Ignore All, Add to Dictionary, replacement through
//! `DocxCorrectWord`). The document is generated here; it holds no personal data.

use std::io::Write;
use std::time::Duration;

use egui::{Event, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use open365_commands::Target;
use open365_ui_egui::{DocPos, DocView, SpellChoice};

struct State {
    view: DocView,
    target: Target,
}

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

fn docx(body: &str) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    let parts = [
        (
            "[Content_Types].xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#.to_owned(),
        ),
        (
            "_rels/.rels",
            r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.to_owned(),
        ),
        (
            "word/document.xml",
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="{W}"><w:body>{body}<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="708" w:footer="708" w:gutter="0"/></w:sectPr></w:body></w:document>"#
            ),
        ),
    ];
    for (name, content) in parts {
        zip.start_file(name, opts).expect("start");
        zip.write_all(content.as_bytes()).expect("write");
    }
    zip.finish().expect("finish").into_inner()
}

fn run(text: &str, props: &str) -> String {
    format!("<w:r><w:rPr>{props}</w:rPr><w:t xml:space=\"preserve\">{text}</w:t></w:r>")
}

fn harness(body: &str) -> Harness<'static, State> {
    let target = Target::open_docx(&docx(body)).expect("open");
    let mut h = Harness::builder()
        .with_size(vec2(1000.0, 700.0))
        .build_ui_state(
            |ui, s: &mut State| {
                s.view.show(ui, &mut s.target);
            },
            State {
                view: DocView::new(),
                target,
            },
        );
    h.run();
    h
}

/// Steps until the checker has answered for `pos` (or gives up).
fn wait_flagged(h: &mut Harness<'_, State>, pos: DocPos) -> bool {
    for _ in 0..300 {
        h.step();
        if h.state_mut().view.spell_target_at(pos).is_some() {
            h.step();
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

fn settle(h: &mut Harness<'_, State>) {
    for _ in 0..60 {
        h.step();
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Red wavy lines drawn last frame (paths in the squiggle colour).
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

fn text_of(h: &Harness<'_, State>) -> String {
    h.state()
        .view
        .paragraph_text(0)
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn a_misspelled_word_gets_a_wavy_underline_and_a_menu_target() {
    let mut h = harness(&format!("<w:p>{}</w:p>", run("The quik brown fox", "")));
    assert!(wait_flagged(&mut h, DocPos::new(0, 5)));
    assert_eq!(squiggles(&h), 1);
    let target = h
        .state_mut()
        .view
        .spell_target_at(DocPos::new(0, 5))
        .unwrap();
    assert_eq!(target.issue.word, "quik");
    assert!((1..=5).contains(&target.suggestions.len()));
    assert!(target.suggestions.iter().any(|s| s == "quick"));
    assert!(
        h.state_mut()
            .view
            .spell_target_at(DocPos::new(0, 1))
            .is_none()
    );
}

#[test]
fn no_proof_and_other_language_runs_are_not_underlined() {
    let body = format!(
        "<w:p>{}{}{}</w:p>",
        run("good ", ""),
        run("quik ", "<w:noProof/>"),
        run("wrongg", "<w:lang w:val=\"de-DE\"/>")
    );
    let mut h = harness(&body);
    settle(&mut h);
    assert_eq!(squiggles(&h), 0);
    assert!(
        h.state_mut()
            .view
            .spell_target_at(DocPos::new(0, 6))
            .is_none()
    );
}

#[test]
fn choosing_a_suggestion_corrects_the_word_in_one_undo_step() {
    let mut h = harness(&format!("<w:p>{}</w:p>", run("The quik brown fox", "")));
    assert!(wait_flagged(&mut h, DocPos::new(0, 5)));
    let target = h
        .state_mut()
        .view
        .spell_target_at(DocPos::new(0, 5))
        .unwrap();
    {
        let s = h.state_mut();
        s.view.apply_spell_choice(
            &mut s.target,
            &target,
            &SpellChoice::Replace("quick".into()),
        );
    }
    h.run();
    assert_eq!(text_of(&h), "The quick brown fox");
    {
        let s = h.state_mut();
        s.view.undo(&mut s.target).expect("undo");
    }
    h.run();
    assert_eq!(text_of(&h), "The quik brown fox");
}

#[test]
fn ignore_all_and_add_to_dictionary_remove_the_squiggle() {
    for choice in [SpellChoice::IgnoreAll, SpellChoice::AddToDictionary] {
        let mut h = harness(&format!("<w:p>{}</w:p>", run("A zorbleflux here", "")));
        assert!(wait_flagged(&mut h, DocPos::new(0, 4)));
        let target = h
            .state_mut()
            .view
            .spell_target_at(DocPos::new(0, 4))
            .unwrap();
        {
            let s = h.state_mut();
            s.view.apply_spell_choice(&mut s.target, &target, &choice);
        }
        settle(&mut h);
        assert_eq!(squiggles(&h), 0, "{choice:?}");
    }
}

#[test]
fn right_click_on_a_flagged_word_opens_the_menu() {
    let mut h = harness(&format!("<w:p>{}</w:p>", run("The quik brown fox", "")));
    assert!(wait_flagged(&mut h, DocPos::new(0, 5)));
    let want = DocPos::new(0, 6);
    let mut pos = Pos2::ZERO;
    'find: for y in (60..400).step_by(2) {
        for x in (60..600).step_by(2) {
            let c = Pos2::new(x as f32, y as f32);
            if h.state().view.position_at(c) == want {
                pos = c;
                break 'find;
            }
        }
    }
    assert_ne!(pos, Pos2::ZERO);
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
}
