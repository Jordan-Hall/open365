//! The Print page of the File tab: printer list, copies, page range, sides, collate,
//! orientation, paper, margins, scaling, N-up, the live preview and the hand-off. A
//! `MemoryPrintSystem` stands in for the printer: no test sends a real job.

use open365_preview::doc::WriterPane;
use open365_preview::print::{Ctl, PrintModel, PrintPage, PrintSource, SheetsSource, WriterSource};
use open365_preview::sheet::SheetModel;
use open365_print::{MemoryPrintSystem, PdfSource, PrintSystem, RangeMode, Scaling, Sides};
use open365_ui_core::harness::Harness;
use open365_ui_core::{Key, NodeId, Rect};

/// A Writer document of four pages (three page breaks).
fn four_pages() -> WriterPane {
    let mut p = WriterPane::blank();
    for _ in 0..3 {
        p.insert_page_break();
    }
    assert_eq!(p.page_count(), 4, "three page breaks make four pages");
    p
}

/// A sheet with `rows` rows of numbers, wide enough to need several pages.
fn tall_sheet(rows: u32) -> SheetModel {
    let mut m = SheetModel::blank();
    for r in 1..=rows {
        for c in 1..=3 {
            m.enter(r, c, &format!("{}", r * 10 + c))
                .expect("cell enters");
        }
    }
    m
}

fn system() -> MemoryPrintSystem {
    MemoryPrintSystem::with_printers(&["Office Laser", "Front Desk"])
}

fn job_pages(sys: &MemoryPrintSystem) -> usize {
    let job = sys.jobs.last().expect("a job was sent");
    PdfSource::load(&job.pdf)
        .expect("the job is a PDF")
        .page_count()
}

#[test]
fn writer_model_reads_the_document_and_the_default_printer() {
    let mut pane = four_pages();
    let sys = system();
    let mut src = WriterSource(&mut pane);
    let mut m = PrintModel::new(&src, &sys);
    m.refresh(&mut src);
    assert_eq!(m.total_pages(), 4);
    assert_eq!(m.settings.printer.as_deref(), Some("Office Laser"));
    assert_eq!(m.settings.copies, 1);
    assert!(m.settings.collate);
    assert_eq!(m.summary(), "4 pages on 4 sheets of paper");
    assert_eq!(m.sheet_count(), 4);
}

#[test]
fn custom_range_and_copies_decide_the_pages() {
    let mut pane = four_pages();
    let sys = system();
    let mut src = WriterSource(&mut pane);
    let mut m = PrintModel::new(&src, &sys);
    m.refresh(&mut src);

    m.set_range_mode(RangeMode::Custom);
    m.set_range_text("2-3");
    m.settings.copies = 3;
    assert_eq!(m.selected_pages(), Ok(vec![2, 3]));
    assert_eq!(m.summary(), "2 pages on 6 sheets of paper");
    // Collated: 2 3 2 3 2 3.
    assert_eq!(
        m.settings.sequence(4, 1).expect("sequence"),
        vec![1, 2, 1, 2, 1, 2]
    );
    let job = PdfSource::load(&m.job_pdf().expect("job")).expect("pdf");
    assert_eq!(job.page_count(), 6);

    m.settings.collate = false;
    assert_eq!(
        m.settings.sequence(4, 1).expect("sequence"),
        vec![1, 1, 1, 2, 2, 2]
    );

    // A page the document does not have is refused, with the reason, and nothing prints.
    m.set_range_text("9");
    assert!(
        m.summary().contains("outside the document"),
        "{}",
        m.summary()
    );
    let mut sys = system();
    assert!(m.print(&mut sys).is_err());
    assert!(sys.jobs.is_empty());
}

#[test]
fn print_current_page_uses_the_page_of_the_caret() {
    let mut pane = four_pages();
    let sys = system();
    let mut src = WriterSource(&mut pane);
    let mut m = PrintModel::new(&src, &sys);
    m.refresh(&mut src);
    m.set_range_mode(RangeMode::CurrentPage);
    assert_eq!(m.selected_pages().map(|p| p.len()), Ok(1));
    // The caret sits after the last page break.
    assert_eq!(m.selected_pages(), Ok(vec![4]));
}

#[test]
fn print_sends_the_composed_pdf_to_the_chosen_printer() {
    let mut pane = four_pages();
    let mut sys = system();
    let mut src = WriterSource(&mut pane);
    let mut m = PrintModel::new(&src, &sys);
    m.refresh(&mut src);
    m.settings.printer = Some("Front Desk".to_owned());
    m.settings.sides = Sides::TwoSidedLongEdge;
    m.settings.copies = 2;
    m.print(&mut sys).expect("sent");
    let job = sys.jobs.last().expect("job");
    assert_eq!(job.printer, "Front Desk");
    assert_eq!(job.options.sides, Sides::TwoSidedLongEdge);
    assert_eq!(job.options.name, "Document1");
    assert_eq!(job_pages(&sys), 8, "four pages, two copies");
    assert_eq!(m.message.as_deref(), Some("Sent to Front Desk."));
}

#[test]
fn no_printer_chosen_is_a_message_not_a_job() {
    let mut pane = four_pages();
    let mut sys = MemoryPrintSystem::default();
    let mut src = WriterSource(&mut pane);
    let mut m = PrintModel::new(&src, &sys);
    m.refresh(&mut src);
    assert_eq!(m.settings.printer, None);
    assert_eq!(
        m.print(&mut sys).err().as_deref(),
        Some("Choose a printer first.")
    );
}

#[test]
fn pages_per_sheet_and_two_sided_change_the_sheet_count() {
    let mut pane = four_pages();
    let mut sys = system();
    let mut src = WriterSource(&mut pane);
    let mut m = PrintModel::new(&src, &sys);
    m.refresh(&mut src);
    m.set_per_sheet(4);
    assert_eq!(m.sheet_count(), 1);
    assert_eq!(m.summary(), "4 pages on 1 sheet of paper");
    m.set_per_sheet(2);
    m.settings.sides = Sides::TwoSidedShortEdge;
    assert_eq!(m.sheet_count(), 2);
    assert_eq!(m.summary(), "4 pages on 1 sheet of paper");
    m.print(&mut sys).expect("sent");
    assert_eq!(job_pages(&sys), 2, "two faces of two pages each");
}

#[test]
fn orientation_paper_and_margins_reach_the_document_and_the_render() {
    let mut pane = four_pages();
    let sys = system();
    {
        let mut src = WriterSource(&mut pane);
        let mut m = PrintModel::new(&src, &sys);
        m.refresh(&mut src);
        let (w, h) = m.pdf().and_then(|p| p.page_size(0)).expect("size");
        assert!(w < h, "portrait first");

        m.set_landscape(&mut src, true);
        m.refresh(&mut src);
        let (lw, lh) = m.pdf().and_then(|p| p.page_size(0)).expect("size");
        assert!(lw > lh, "landscape after the change: {lw} x {lh}");
        assert!(m.sheet_size().0 > m.sheet_size().1);

        m.set_paper(&mut src, "letter");
        m.refresh(&mut src);
        let (pw, ph) = m.pdf().and_then(|p| p.page_size(0)).expect("size");
        assert!(
            (pw - 792.0).abs() < 1.0 && (ph - 612.0).abs() < 1.0,
            "{pw} x {ph}"
        );

        m.set_margins(&mut src, "narrow");
        assert_eq!(m.settings.margins_id, "narrow");
        m.refresh(&mut src);
    }
    // The document itself changed, as Word's Print page does.
    let src = WriterSource(&mut pane);
    let setup = src.page_setup();
    assert!(setup.landscape);
    assert_eq!(setup.margins_pt, (36.0, 36.0, 36.0, 36.0));
}

#[test]
fn sheets_scaling_fits_the_sheet_on_one_page() {
    let mut sheet = tall_sheet(120);
    let sys = system();
    let mut src = SheetsSource(&mut sheet);
    let mut m = PrintModel::new(&src, &sys);
    m.refresh(&mut src);
    let unscaled = m.total_pages();
    assert!(unscaled >= 2, "120 rows need several pages, got {unscaled}");

    m.set_scaling(&mut src, Scaling::FitSheet);
    m.refresh(&mut src);
    assert_eq!(m.total_pages(), 1, "Fit Sheet on One Page");

    m.set_scaling(&mut src, Scaling::FitColumns);
    m.refresh(&mut src);
    assert!(m.total_pages() >= 1);

    m.set_scaling(&mut src, Scaling::Custom(50));
    m.refresh(&mut src);
    assert!(
        m.total_pages() < unscaled,
        "50 % needs fewer pages than 100 %"
    );

    m.set_scaling(&mut src, Scaling::None);
    m.refresh(&mut src);
    assert_eq!(m.total_pages(), unscaled);
}

#[test]
fn sheets_orientation_and_margins_follow_the_page_setup() {
    let mut sheet = tall_sheet(10);
    let sys = system();
    let mut src = SheetsSource(&mut sheet);
    let mut m = PrintModel::new(&src, &sys);
    assert_eq!(m.settings.margins_id, "normal");
    m.refresh(&mut src);
    m.set_landscape(&mut src, true);
    m.set_margins(&mut src, "wide");
    m.refresh(&mut src);
    let (w, h) = m.pdf().and_then(|p| p.page_size(0)).expect("size");
    assert!(w > h);
    assert_eq!(src.page_setup().margins_pt.0, 72.0);
}

// ---- the page --------------------------------------------------------------------------------

struct Rig {
    h: Harness,
    page: PrintPage,
    sys: MemoryPrintSystem,
}

impl Rig {
    fn new(src: &mut dyn PrintSource) -> Self {
        let sys = system();
        let mut h = Harness::new(1100.0, 800.0, 1.0);
        let mut page = PrintPage::open(src, &sys);
        let root = h.ui.tree.root();
        page.build(&mut h.ui, root, Rect::new(0.0, 0.0, 1100.0, 800.0), src);
        Self { h, page, sys }
    }

    fn pump(&mut self, src: &mut dyn PrintSource) {
        for e in self.h.ui.take_events() {
            self.page.handle(&mut self.h.ui, &e, src, &mut self.sys);
        }
    }

    fn node(&self, ctl: Ctl) -> NodeId {
        self.page
            .node_of(ctl)
            .unwrap_or_else(|| panic!("{ctl:?} is on the page"))
    }

    fn choose(&mut self, ctl: Ctl, index: usize, src: &mut dyn PrintSource) {
        let n = self.node(ctl);
        self.h.ui.set_focus(Some(n));
        self.h.key(Key::Home);
        for _ in 0..index {
            self.h.key(Key::Down);
        }
        self.pump(src);
    }

    fn click(&mut self, ctl: Ctl, src: &mut dyn PrintSource) {
        let n = self.node(ctl);
        self.h.click_node(n);
        self.pump(src);
    }

    fn labels(&self) -> Vec<String> {
        self.h
            .ui
            .tree
            .paint_order()
            .into_iter()
            .filter_map(|id| self.h.ui.tree.get(id))
            .map(|n| n.label.clone())
            .filter(|l| !l.is_empty())
            .collect()
    }

    fn sheet_children(&self) -> usize {
        self.h
            .ui
            .tree
            .paint_order()
            .into_iter()
            .filter_map(|id| self.h.ui.tree.get(id))
            .find(|n| n.label.starts_with("Page ") && n.label.contains(" of "))
            .map_or(0, |n| n.children.len())
    }
}

#[test]
fn the_page_shows_every_setting_of_the_print_tab() {
    let mut pane = four_pages();
    let mut src = WriterSource(&mut pane);
    let rig = Rig::new(&mut src);
    let labels = rig.labels();
    for want in [
        "Print",
        "Copies:",
        "Printer",
        "Settings",
        "Office Laser (default)",
        "Front Desk",
        "Print All Pages",
        "Pages:",
        "Print One Sided",
        "Collated",
        "Portrait Orientation",
        "1 Page Per Sheet",
        "4 pages on 4 sheets of paper",
        "1 of 4",
    ] {
        // Dropdown labels are their accessible names; the shown text is the selected item.
        let shown = rig.h.ui.tree.paint_order().into_iter().any(|id| {
            rig.h.ui.tree.get(id).is_some_and(|n| {
                n.label == want
                    || matches!(&n.widget, open365_ui_core::Widget::Dropdown { items, .. } if items.iter().any(|i| i == want))
            })
        });
        assert!(shown, "{want} is on the page; labels: {labels:?}");
    }
    assert!(
        !labels
            .iter()
            .any(|l| l.to_ascii_lowercase().contains("word")),
        "no product names: {labels:?}"
    );
    // The Sheets-only scaling row is not on the Writer page, and the Pages box starts disabled.
    assert!(rig.page.node_of(Ctl::Scaling).is_none());
    let pages = rig.node(Ctl::Pages);
    assert!(!rig.h.ui.tree.get(pages).expect("node").enabled);
    // The preview draws the first page's ink.
    assert!(rig.sheet_children() > 0, "the preview has marks");
}

#[test]
fn choosing_settings_on_the_page_updates_the_model_and_the_preview() {
    let mut pane = four_pages();
    let mut src = WriterSource(&mut pane);
    let mut rig = Rig::new(&mut src);

    rig.choose(Ctl::Range, 2, &mut src);
    assert_eq!(rig.page.model.settings.range_mode, RangeMode::Custom);
    let pages = rig.node(Ctl::Pages);
    assert!(
        rig.h.ui.tree.get(pages).expect("node").enabled,
        "Custom Print enables the Pages box"
    );

    rig.h.ui.set_focus(Some(pages));
    rig.h.type_text("2-3");
    rig.pump(&mut src);
    assert_eq!(rig.page.model.settings.range_text, "2-3");
    assert!(
        rig.labels().iter().any(|l| l == "1 of 2"),
        "{:?}",
        rig.labels()
    );
    assert!(
        rig.labels()
            .iter()
            .any(|l| l == "2 pages on 2 sheets of paper")
    );

    rig.choose(Ctl::Sides, 1, &mut src);
    assert_eq!(rig.page.model.settings.sides, Sides::TwoSidedLongEdge);
    assert!(
        rig.labels()
            .iter()
            .any(|l| l == "2 pages on 1 sheet of paper")
    );

    rig.choose(Ctl::Collate, 1, &mut src);
    assert!(!rig.page.model.settings.collate);

    rig.choose(Ctl::Orientation, 1, &mut src);
    assert!(rig.page.model.settings.landscape);
    let (w, h) = rig.page.model.sheet_size();
    assert!(w > h);

    // Next page moves the preview.
    rig.click(Ctl::Next, &mut src);
    assert_eq!(rig.page.model.viewing, 1);
    assert!(rig.labels().iter().any(|l| l == "2 of 2"));
}

#[test]
fn the_print_button_sends_the_job_once() {
    let mut pane = four_pages();
    let mut src = WriterSource(&mut pane);
    let mut rig = Rig::new(&mut src);
    rig.choose(Ctl::Printer, 1, &mut src);
    rig.choose(Ctl::PerSheet, 2, &mut src);
    assert_eq!(rig.sys.jobs.len(), 0, "choosing sends nothing");
    rig.click(Ctl::Print, &mut src);
    assert_eq!(rig.sys.jobs.len(), 1);
    assert_eq!(rig.sys.jobs[0].printer, "Front Desk");
    assert_eq!(job_pages(&rig.sys), 1, "four pages, four to a sheet");
    assert!(
        rig.labels().iter().any(|l| l == "Sent to Front Desk."),
        "{:?}",
        rig.labels()
    );
}

#[test]
fn a_bad_range_disables_print_and_says_why() {
    let mut pane = four_pages();
    let mut src = WriterSource(&mut pane);
    let mut rig = Rig::new(&mut src);
    rig.choose(Ctl::Range, 2, &mut src);
    let pages = rig.node(Ctl::Pages);
    rig.h.ui.set_focus(Some(pages));
    rig.h.type_text("7");
    rig.pump(&mut src);
    let print = rig.node(Ctl::Print);
    assert!(!rig.h.ui.tree.get(print).expect("node").enabled);
    assert!(
        rig.labels()
            .iter()
            .any(|l| l.starts_with("Page 7 is outside the document")),
        "{:?}",
        rig.labels()
    );
}

#[test]
fn the_sheets_page_has_scaling_and_no_pages_per_sheet() {
    let mut sheet = tall_sheet(120);
    let mut src = SheetsSource(&mut sheet);
    let mut rig = Rig::new(&mut src);
    assert!(rig.page.node_of(Ctl::PerSheet).is_none());
    let before = rig.page.model.total_pages();
    assert!(before >= 2);
    rig.choose(Ctl::Scaling, 1, &mut src);
    assert_eq!(rig.page.model.settings.scaling, Scaling::FitSheet);
    assert_eq!(rig.page.model.total_pages(), 1);
    assert!(
        rig.labels()
            .iter()
            .any(|l| l == "1 page on 1 sheet of paper"),
        "{:?}",
        rig.labels()
    );

    // Custom Scaling Options adds the percentage box.
    rig.choose(Ctl::Scaling, 4, &mut src);
    assert!(rig.page.node_of(Ctl::ScalePercent).is_some());
    rig.click(Ctl::Print, &mut src);
    assert_eq!(rig.sys.jobs.len(), 1);
    assert_eq!(rig.sys.jobs[0].options.name, "Book1");
}

#[test]
fn closing_removes_the_page_from_the_tree() {
    let mut pane = four_pages();
    let mut src = WriterSource(&mut pane);
    let mut rig = Rig::new(&mut src);
    assert!(!rig.labels().is_empty());
    rig.page.close(&mut rig.h.ui);
    assert!(rig.labels().is_empty());
}

#[test]
fn the_os_print_system_lists_printers_without_printing() {
    // Listing only: it asks the spooler for names and sends nothing.
    let printers = open365_print::OsPrintSystem.printers();
    for p in &printers {
        assert!(!p.system_name.is_empty());
    }
}
