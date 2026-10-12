//! Writer File commands run through `Shell::run_command`.

#![allow(clippy::unwrap_used)]

use open365_preview::doc::WriterPane;
use open365_preview::ribbon::Handled;
use open365_preview::{AppKind, Preview};

fn rig(text: &str) -> Preview {
    let mut p = Preview::new(AppKind::Writer, false, 1600, 1000, 1.25);
    let mut pane = WriterPane::blank();
    pane.insert_text(text);
    p.shell.set_writer(&mut p.ui, pane);
    p
}

fn run(p: &mut Preview, id: &str) {
    assert_eq!(p.shell.run_command(&mut p.ui, id), Handled::Yes, "{id}");
}

fn dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("open365-rwfile-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn save_as_open_export_close() {
    let d = dir("a");
    let docx = d.join("a.docx");
    let mut p = rig("hello file");
    run(&mut p, &format!("file.save-as={}", docx.display()));
    assert!(docx.exists());
    assert!(!p.shell.writer().unwrap().is_dirty());

    run(&mut p, "file.export-pdf");
    let pdf = d.join("a.pdf");
    assert!(std::fs::read(&pdf).unwrap().starts_with(b"%PDF"));

    run(&mut p, "file.new");
    assert_eq!(p.shell.writer().unwrap().text(), "");
    run(&mut p, &format!("file.open={}", docx.display()));
    let w = p.shell.writer().unwrap();
    assert_eq!(w.text(), "hello file");
    assert_eq!(w.path(), Some(docx.as_path()));
    assert!(w.canvas().is_some());

    run(&mut p, "file.save");
    run(&mut p, "file.print");
    run(&mut p, "file.share");
    run(&mut p, "file.close");
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn dirty_document_is_not_replaced_or_closed() {
    let mut p = rig("unsaved");
    run(&mut p, "file.new");
    assert_eq!(p.shell.writer().unwrap().text(), "unsaved");
    run(&mut p, "file.close");
    assert!(
        p.shell
            .writer()
            .unwrap()
            .status
            .as_deref()
            .unwrap()
            .contains("Save")
    );
}
