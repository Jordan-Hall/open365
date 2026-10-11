//! Copy, cut and paste on the Sheets grid: TSV plus HTML on the clipboard, formulas that shift
//! when pasted, and the confirmation a paste over existing data needs. A memory clipboard stands
//! in for the system one so the tests never touch the person's clipboard.

#![allow(clippy::unwrap_used)]

use open365_preview::sheet::MemoryClipboard;
use open365_preview::{AppKind, Preview};
use open365_ui_core::{InputEvent, Key, Modifiers};

fn sheets() -> (Preview, MemoryClipboard) {
    let mut p = Preview::new(AppKind::Sheets, false, 1600, 1000, 1.25);
    let clip = MemoryClipboard::default();
    p.shell.sheet_mut().set_clipboard(clip.clone());
    (p, clip)
}

fn ctrl(p: &mut Preview, c: char) {
    for pressed in [true, false] {
        p.input(InputEvent::Key {
            key: Key::Char(c),
            pressed,
            modifiers: Modifiers {
                ctrl: true,
                ..Modifiers::default()
            },
        });
    }
}

fn fill_block(p: &mut Preview) {
    let s = p.shell.sheet_mut();
    s.enter(1, 1, "1").unwrap();
    s.enter(1, 2, "two").unwrap();
    s.enter(2, 1, "3").unwrap();
    s.enter(2, 2, "=A1+A2").unwrap();
}

#[test]
fn copy_puts_tsv_and_html_on_the_clipboard() {
    let (mut p, clip) = sheets();
    fill_block(&mut p);
    {
        let s = p.shell.sheet_mut();
        s.select_cell((1, 1));
        s.extend_to((2, 2));
    }
    ctrl(&mut p, 'C');
    assert_eq!(clip.tsv(), "1\ttwo\n3\t4\n");
    let html = clip.html();
    assert!(
        html.contains("<table>") && html.contains("<td>two</td>"),
        "{html}"
    );
    assert!(
        html.contains("<td style=\"text-align:right\">4</td>"),
        "{html}"
    );
}

#[test]
fn paste_into_empty_cells_needs_no_confirmation_and_shifts_formulas() {
    let (mut p, _clip) = sheets();
    fill_block(&mut p);
    {
        let s = p.shell.sheet_mut();
        s.select_cell((1, 1));
        s.extend_to((2, 2));
    }
    ctrl(&mut p, 'C');
    p.shell.sheet_mut().select_cell((5, 4));
    ctrl(&mut p, 'V');
    assert!(!p.shell.paste_pending());
    let s = p.shell.sheet();
    assert_eq!(s.display(5, 4), "1");
    assert_eq!(s.display(5, 5), "two");
    assert_eq!(s.source(6, 5), "=D5+D6", "the formula moved with the block");
    assert_eq!(s.display(6, 5), "4");
    assert_eq!(s.range(), (5, 4, 6, 5), "the pasted block is selected");
}

#[test]
fn paste_over_data_waits_for_confirmation() {
    let (mut p, clip) = sheets();
    fill_block(&mut p);
    clip.put_text("x\ty\n");
    p.shell.sheet_mut().select_cell((1, 1));
    ctrl(&mut p, 'V');
    assert!(p.shell.paste_pending(), "destructive: it asks first");
    assert_eq!(p.shell.sheet().display(1, 1), "1", "nothing changed yet");
    // Escape keeps the cells.
    p.key(Key::Escape);
    assert!(!p.shell.paste_pending());
    assert_eq!(p.shell.sheet().display(1, 1), "1");
    // Paste again and confirm with Enter.
    ctrl(&mut p, 'V');
    assert!(p.shell.paste_pending());
    p.key(Key::Enter);
    assert!(!p.shell.paste_pending());
    assert_eq!(p.shell.sheet().display(1, 1), "x");
    assert_eq!(p.shell.sheet().display(1, 2), "y");
    // One undo restores the whole block.
    ctrl(&mut p, 'Z');
    assert_eq!(p.shell.sheet().display(1, 1), "1");
    assert_eq!(p.shell.sheet().display(1, 2), "two");
}

#[test]
fn cut_moves_the_cells_on_paste() {
    let (mut p, _clip) = sheets();
    fill_block(&mut p);
    {
        let s = p.shell.sheet_mut();
        s.select_cell((1, 1));
        s.extend_to((1, 2));
    }
    ctrl(&mut p, 'X');
    assert_eq!(
        p.shell.sheet().display(1, 1),
        "1",
        "a cut clears only on paste"
    );
    p.shell.sheet_mut().select_cell((8, 1));
    ctrl(&mut p, 'V');
    let s = p.shell.sheet();
    assert_eq!(
        (s.display(8, 1).as_str(), s.display(8, 2).as_str()),
        ("1", "two")
    );
    assert_eq!(s.display(1, 1), "");
    assert_eq!(s.display(1, 2), "");
}

#[test]
fn text_from_another_program_pastes_as_tsv() {
    let (mut p, clip) = sheets();
    clip.put_text("a\tb\r\n\"c\td\"\t5\r\n");
    ctrl(&mut p, 'V');
    let s = p.shell.sheet();
    assert_eq!(s.display(1, 1), "a");
    assert_eq!(s.display(1, 2), "b");
    assert_eq!(s.display(2, 1), "c\td");
    assert_eq!(s.display(2, 2), "5");
}
