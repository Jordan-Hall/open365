//! The Share dialog calls the share commands of the command layer (ADR-013 Stage 0).

use std::path::PathBuf;

use open365_preview::doc::WriterPane;
use open365_preview::{AppKind, Preview};
use open365_ui_core::NodeId;

fn find_by(p: &Preview, pred: impl Fn(&str) -> bool) -> Option<NodeId> {
    p.ui.tree
        .paint_order()
        .into_iter()
        .find(|id| p.ui.tree.get(*id).is_some_and(|n| pred(&n.label)))
}

fn click(p: &mut Preview, id: NodeId) {
    let r = p.ui.tree.abs_rect(id).center();
    let s = p.scale();
    p.click(r.x * s, r.y * s);
}

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/docx/cmt-basic.docx")
}

#[test]
fn inspect_then_send_a_clean_copy_and_a_pdf() {
    let mut p = Preview::new(AppKind::Writer, true, 1600, 1000, 1.25);
    let pane = WriterPane::open(&fixture()).expect("fixture opens");
    p.shell.set_writer(&mut p.ui, pane);
    p.shell.open_share(&mut p.ui);
    assert!(p.shell.share_open());

    // Link and people sections are disabled, not hidden.
    for label in ["Share with people", "Copy link"] {
        let id = find_by(&p, |l| l == label).expect(label);
        assert!(
            !p.ui.tree.get(id).expect("node").enabled,
            "{label} is disabled"
        );
    }
    assert!(find_by(&p, |l| l == "Connect a server").is_some());

    let inspect = find_by(&p, |l| l == "Inspect for hidden content").expect("inspect");
    click(&mut p, inspect);
    let labels: Vec<String> =
        p.ui.tree
            .paint_order()
            .into_iter()
            .filter_map(|id| p.ui.tree.get(id).map(|n| n.label.clone()))
            .filter(|l| !l.is_empty())
            .collect();
    assert!(
        find_by(&p, |l| l.starts_with("Comments (")).is_some(),
        "labels {labels:?}",
    );
    let send = find_by(&p, |l| l == "Remove selected and send a copy").expect("sanitize");
    click(&mut p, send);
    let status =
        p.ui.tree
            .paint_order()
            .into_iter()
            .filter_map(|id| p.ui.tree.get(id).map(|n| n.label.clone()))
            .find(|l| l.starts_with("Clean copy written to "))
            .expect("status line");
    let path = status
        .trim_start_matches("Clean copy written to ")
        .split(".docx")
        .next()
        .map(|s| PathBuf::from(format!("{s}.docx")))
        .expect("path");
    assert!(path.exists(), "{}", path.display());
    let bytes = std::fs::read(&path).expect("read copy");
    let zip = open365_opc::Package::read(&bytes, &open365_opc::Limits::default()).expect("package");
    assert!(
        zip.part("word/comments.xml").is_none(),
        "comments removed from the copy"
    );
    open365_commands::remove_share_temp(&path);

    let pdf = find_by(&p, |l| l == "Send as PDF").expect("pdf");
    click(&mut p, pdf);
    let status =
        p.ui.tree
            .paint_order()
            .into_iter()
            .filter_map(|id| p.ui.tree.get(id).map(|n| n.label.clone()))
            .find(|l| l.starts_with("PDF written to "))
            .expect("pdf status");
    let pdf_path = PathBuf::from(
        status
            .trim_start_matches("PDF written to ")
            .split(".pdf")
            .next()
            .map(|s| format!("{s}.pdf"))
            .expect("path"),
    );
    assert!(std::fs::read(&pdf_path).expect("pdf").starts_with(b"%PDF"));
    open365_commands::remove_share_temp(&pdf_path);
    // The document itself is untouched.
    assert!(!p.shell.writer().expect("pane").is_dirty());
}
