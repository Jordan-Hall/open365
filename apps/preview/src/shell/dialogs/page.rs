//! The Sheets Page Setup (Page, Margins, Header/Footer and Sheet tabs) and Zoom dialogs.

use open365_ui_core::Ui;
use open365_ui_core::dialog::{Dialog, Outcome, Spec, Values};

use super::{View, header_footer as custom, parse_ref, pick_selection, request_child};
use crate::dialogs::header_footer::{self as hf, Context, HeaderFooter, Sections};
use crate::dialogs::request::DialogRequest;
use crate::sheet_ext::page_layout::Paper;
use crate::shell::Shell;

pub(super) fn make(s: &mut Shell, req: &DialogRequest) -> Option<(Spec, Box<dyn View>)> {
    Some(match req {
        DialogRequest::PageSetup { tab } => page_setup(s, tab.index()),
        DialogRequest::Zoom => zoom(s),
        _ => return None,
    })
}

const PAPERS: [(&str, &str); 10] = [
    ("Letter", "letter"),
    ("Tabloid", "tabloid"),
    ("Legal", "legal"),
    ("Statement", "statement"),
    ("Executive", "executive"),
    ("A3", "a3"),
    ("A4", "a4"),
    ("A5", "a5"),
    ("B4 (JIS)", "b4"),
    ("B5 (JIS)", "b5"),
];

fn paper_index(p: Paper) -> usize {
    match p {
        Paper::Letter => 0,
        Paper::Tabloid => 1,
        Paper::Legal => 2,
        Paper::Statement => 3,
        Paper::Executive => 4,
        Paper::A3 => 5,
        Paper::A4 => 6,
        Paper::A5 => 7,
        Paper::B4 => 8,
        Paper::B5 => 9,
    }
}

#[derive(Debug)]
struct PageSetup {
    had_titles: bool,
    /// The header and footer being edited.
    hf: HeaderFooter,
    ctx: Context,
    sheet: usize,
    /// The sections behind the entries of the Header and Footer drop-downs.
    sets: [Vec<Sections>; 2],
    /// The custom dialog open over this one: true for the footer.
    custom_footer: bool,
}

const HF_IDS: [(&str, [&str; 3]); 2] = [
    ("hf_header", ["hf_hl", "hf_hc", "hf_hr"]),
    ("hf_footer", ["hf_fl", "hf_fc", "hf_fr"]),
];

fn page_setup(s: &mut Shell, tab: usize) -> (Spec, Box<dyn View>) {
    let page = s.sheet.page();
    let fit = page.fit_width > 0 || page.fit_height > 0;
    let mut spec = Spec::new("Page Setup", 440.0, 312.0);
    spec.tabs = vec![
        "Pa&ge".into(),
        "Ma&rgins".into(),
        "&Header/Footer".into(),
        "S&heet".into(),
    ];

    spec.on_page(Some(0));
    spec.group(12.0, 40.0, 416.0, 52.0, "Orientation");
    spec.radio(
        "portrait",
        "orient",
        24.0,
        62.0,
        180.0,
        "P&ortrait",
        !page.landscape,
    );
    spec.radio(
        "landscape",
        "orient",
        220.0,
        62.0,
        180.0,
        "Lan&dscape",
        page.landscape,
    );
    spec.group(12.0, 100.0, 416.0, 100.0, "Scaling");
    spec.radio("adjust", "scaling", 24.0, 122.0, 100.0, "&Adjust to:", !fit);
    spec.spin(
        "scale",
        128.0,
        118.0,
        70.0,
        (f64::from(page.scale), 10.0, 400.0, 1.0),
    );
    spec.label(204.0, 122.0, 130.0, "% normal size");
    spec.radio("fit", "scaling", 24.0, 156.0, 70.0, "&Fit to:", fit);
    spec.spin(
        "wide",
        98.0,
        152.0,
        60.0,
        (f64::from(page.fit_width), 0.0, 9.0, 1.0),
    );
    spec.label(164.0, 156.0, 100.0, "page(s) wide by");
    spec.spin(
        "tall",
        266.0,
        152.0,
        60.0,
        (f64::from(page.fit_height), 0.0, 9.0, 1.0),
    );
    spec.label(332.0, 156.0, 80.0, "tall (0 is automatic)");
    spec.label(12.0, 214.0, 100.0, "Pa&per size:");
    let papers: Vec<&str> = PAPERS.iter().map(|p| p.0).collect();
    spec.dropdown(
        "paper",
        112.0,
        210.0,
        200.0,
        &papers,
        paper_index(page.paper),
    );

    spec.on_page(Some(1));
    let m = page.margins;
    let rows = [
        ("top", "&Top:", m.top, 12.0, 44.0),
        ("bottom", "&Bottom:", m.bottom, 12.0, 80.0),
        ("left", "&Left:", m.left, 12.0, 116.0),
        ("right", "&Right:", m.right, 12.0, 152.0),
        ("header", "H&eader:", m.header, 224.0, 44.0),
        ("footer", "F&ooter:", m.footer, 224.0, 80.0),
    ];
    for (id, label, v, x, y) in rows {
        spec.label(x, y + 4.0, 70.0, label);
        spec.spin(id, x + 72.0, y, 90.0, (v, 0.0, 50.0, 0.05));
    }
    spec.label(12.0, 192.0, 400.0, "Margins are in inches.");
    spec.group(12.0, 214.0, 416.0, 56.0, "Center on page");
    spec.check("horizontally", 24.0, 236.0, 180.0, "Hori&zontally", false)
        .disabled();
    spec.check("vertically", 220.0, 236.0, 180.0, "&Vertically", false)
        .disabled();

    let ctx = custom::context(s);
    let sheet = s.sheet.active_sheet_index();
    let hfs = s.sheet.header_footer_of(sheet);
    let (hsets, hlabels, hsel) = hf::choices(&hfs.header, &ctx);
    let (fsets, flabels, fsel) = hf::choices(&hfs.footer, &ctx);
    let hl: Vec<&str> = hlabels.iter().map(String::as_str).collect();
    let fl: Vec<&str> = flabels.iter().map(String::as_str).collect();
    spec.on_page(Some(2));
    spec.label(12.0, 44.0, 200.0, "Hea&der:");
    spec.dropdown("hf_header", 12.0, 62.0, 416.0, &hl, hsel);
    spec.button("hf_custom_header", 12.0, 90.0, 140.0, "C&ustom Header...");
    spec.button("hf_custom_footer", 160.0, 90.0, 140.0, "Custom Foo&ter...");
    spec.group(12.0, 120.0, 416.0, 40.0, "Header preview");
    spec.label(24.0, 138.0, 125.0, "").id = "hf_hl".to_owned();
    spec.label(160.0, 138.0, 125.0, "").id = "hf_hc".to_owned();
    spec.label(296.0, 138.0, 125.0, "").id = "hf_hr".to_owned();
    spec.label(12.0, 168.0, 200.0, "&Footer:");
    spec.dropdown("hf_footer", 12.0, 186.0, 416.0, &fl, fsel);
    spec.group(12.0, 214.0, 416.0, 40.0, "Footer preview");
    spec.label(24.0, 232.0, 125.0, "").id = "hf_fl".to_owned();
    spec.label(160.0, 232.0, 125.0, "").id = "hf_fc".to_owned();
    spec.label(296.0, 232.0, 125.0, "").id = "hf_fr".to_owned();
    spec.check(
        "hf_oddeven",
        12.0,
        262.0,
        200.0,
        "Different &odd and even pages",
        hfs.different_odd_even,
    );
    spec.check(
        "hf_first",
        220.0,
        262.0,
        200.0,
        "Different first pa&ge",
        hfs.different_first,
    );
    spec.check(
        "hf_scale",
        12.0,
        284.0,
        200.0,
        "&Scale with document",
        hfs.scale_with_document,
    );
    spec.check(
        "hf_align",
        220.0,
        284.0,
        200.0,
        "&Align with page margins",
        hfs.align_with_margins,
    );

    spec.on_page(Some(3));
    let titles = |rows: bool| -> String {
        let t = if rows {
            page.title_rows.map(|(a, b)| format!("${a}:${b}"))
        } else {
            page.title_cols
                .map(|(a, b)| format!("${}:${}", super::col_letters(a), super::col_letters(b)))
        };
        t.unwrap_or_default()
    };
    spec.label(12.0, 44.0, 200.0, "&Rows to repeat at top:");
    spec.reference("rows", 12.0, 64.0, 410.0, &titles(true));
    spec.label(12.0, 98.0, 200.0, "&Columns to repeat at left:");
    spec.reference("cols", 12.0, 118.0, 410.0, &titles(false));
    spec.group(12.0, 156.0, 416.0, 80.0, "Print");
    spec.check(
        "gridlines",
        24.0,
        178.0,
        200.0,
        "&Gridlines",
        page.print_gridlines,
    );
    spec.check(
        "headings",
        24.0,
        202.0,
        300.0,
        "Row and column &headings",
        page.print_headings,
    );
    spec.on_page(None);
    spec.start_page = tab.min(3);
    (
        spec,
        Box::new(PageSetup {
            had_titles: page.title_rows.is_some() || page.title_cols.is_some(),
            hf: hfs,
            ctx,
            sheet,
            sets: [hsets, fsets],
            custom_footer: false,
        }),
    )
}

/// `$1:$3` rows or `$A:$C` columns typed in a titles box; `None` when it is empty.
fn titles_text(text: &str, rows: bool) -> Result<Option<String>, String> {
    let t = text.trim();
    if t.is_empty() {
        return Ok(None);
    }
    let ok = parse_ref(t).is_some() || t.replace('$', "").split_once(':').is_some();
    if !ok {
        return Err(if rows {
            "Rows to repeat at top: the reference is not valid.".to_owned()
        } else {
            "Columns to repeat at left: the reference is not valid.".to_owned()
        });
    }
    Ok(Some(t.to_owned()))
}

impl PageSetup {
    fn sections(&self, footer: bool) -> &Sections {
        if footer {
            &self.hf.footer
        } else {
            &self.hf.header
        }
    }

    /// Shows the three sections of the header or footer in its preview box.
    fn preview(&self, ui: &mut Ui, d: &mut Dialog, footer: bool) {
        let ids = HF_IDS[usize::from(footer)].1;
        for (i, id) in ids.iter().enumerate() {
            let text = hf::render(&self.sections(footer)[i], &self.ctx, "1", "1");
            d.set_text(ui, id, &text);
        }
    }

    /// Rebuilds a drop-down after its sections changed (a custom text is listed first).
    fn refresh(&mut self, ui: &mut Ui, d: &mut Dialog, footer: bool) {
        let (sets, labels, sel) = hf::choices(self.sections(footer), &self.ctx);
        self.sets[usize::from(footer)] = sets;
        d.set_items(ui, HF_IDS[usize::from(footer)].0, labels, Some(sel));
        self.preview(ui, d, footer);
    }
}

impl View for PageSetup {
    fn opened(&mut self, _s: &mut Shell, ui: &mut Ui, d: &mut Dialog) {
        self.preview(ui, d, false);
        self.preview(ui, d, true);
    }

    fn child_done(
        &mut self,
        _s: &mut Shell,
        ui: &mut Ui,
        d: &mut Dialog,
        applied: bool,
    ) -> Result<(), String> {
        if applied && let Some(sections) = custom::take_result() {
            if self.custom_footer {
                self.hf.footer = sections;
            } else {
                self.hf.header = sections;
            }
            self.refresh(ui, d, self.custom_footer);
        }
        Ok(())
    }

    fn event(&mut self, s: &mut Shell, ui: &mut Ui, d: &mut Dialog, o: &Outcome) {
        match o {
            Outcome::Changed(id) if id == "hf_header" || id == "hf_footer" => {
                let footer = id == "hf_footer";
                if let Some(set) = d
                    .values(ui)
                    .index(id)
                    .and_then(|i| self.sets[usize::from(footer)].get(i).cloned())
                {
                    if footer {
                        self.hf.footer = set;
                    } else {
                        self.hf.header = set;
                    }
                    self.preview(ui, d, footer);
                }
            }
            Outcome::Pressed(id) if id == "hf_custom_header" || id == "hf_custom_footer" => {
                let footer = id == "hf_custom_footer";
                self.custom_footer = footer;
                request_child(DialogRequest::HeaderFooterCustom {
                    footer,
                    sections: self.sections(footer).clone(),
                });
            }
            _ => {}
        }
        pick_selection(s, ui, d, o);
        // A reference box shows entire rows or columns: `$1:$3`.
        if let Outcome::Ref {
            id,
            collapsed: false,
        } = o
        {
            let (r1, c1, r2, c2) = s.sheet.range();
            let text = if id == "rows" {
                format!("${r1}:${r2}")
            } else {
                format!("${}:${}", super::col_letters(c1), super::col_letters(c2))
            };
            d.set_text(ui, id, &text);
        }
    }

    fn commit(
        &mut self,
        s: &mut Shell,
        ui: &mut Ui,
        _d: &mut Dialog,
        v: &Values,
    ) -> Result<(), String> {
        let rows = titles_text(v.text("rows"), true)?;
        let cols = titles_text(v.text("cols"), false)?;
        let mut cmds: Vec<String> = Vec::new();
        cmds.push(format!(
            "page-layout.page-set-up.orientation.{}",
            if v.radio("orient") == "landscape" {
                "landscape"
            } else {
                "portrait"
            }
        ));
        let paper = PAPERS[v.index("paper").unwrap_or(6).min(PAPERS.len() - 1)].1;
        cmds.push(format!("page-layout.page-set-up.size.{paper}"));
        if v.radio("scaling") == "fit" {
            let pages = |n: f64| {
                if n < 1.0 {
                    "Automatic".to_owned()
                } else {
                    format!("{} page", n as u32)
                }
            };
            cmds.push(format!(
                "page-layout.scale-to-fit.width={}",
                pages(v.num("wide"))
            ));
            cmds.push(format!(
                "page-layout.scale-to-fit.height={}",
                pages(v.num("tall"))
            ));
        } else {
            cmds.push(format!(
                "page-layout.scale-to-fit.scale={}",
                v.num("scale") as u32
            ));
        }
        cmds.push(format!(
            "page-layout.page-set-up.margins.custom={},{},{},{},{},{}",
            v.num("top"),
            v.num("bottom"),
            v.num("left"),
            v.num("right"),
            v.num("header"),
            v.num("footer"),
        ));
        let want_gridlines = if v.flag("gridlines") { "on" } else { "off" };
        let want_headings = if v.flag("headings") { "on" } else { "off" };
        cmds.push(format!("page-layout.sheet-options.print={want_gridlines}"));
        cmds.push(format!("page-layout.sheet-options.print-2={want_headings}"));
        if rows.is_none() && cols.is_none() {
            if self.had_titles {
                cmds.push("page-layout.page-set-up.print-titles=clear".to_owned());
            }
        } else {
            if self.had_titles {
                cmds.push("page-layout.page-set-up.print-titles=clear".to_owned());
            }
            for t in rows.iter().chain(cols.iter()) {
                cmds.push(format!("page-layout.page-set-up.print-titles={t}"));
            }
        }
        s.run_ids(ui, &cmds);
        self.had_titles = rows.is_some() || cols.is_some();
        self.hf.different_odd_even = v.flag("hf_oddeven");
        self.hf.different_first = v.flag("hf_first");
        self.hf.scale_with_document = v.flag("hf_scale");
        self.hf.align_with_margins = v.flag("hf_align");
        s.sheet.set_header_footer(self.sheet, &self.hf)?;
        Ok(())
    }
}

// ---- Zoom -----------------------------------------------------------------------------------------------

#[derive(Debug)]
pub(super) struct Zoom {
    /// The commands of the "fit" radios, by radio id.
    pub fits: Vec<(&'static str, &'static str)>,
}

fn zoom(s: &mut Shell) -> (Spec, Box<dyn View>) {
    let z = s.sheet.ext_view.zoom;
    let presets = [200_u32, 100, 75, 50, 25];
    let mut spec = Spec::new("Zoom", 220.0, 232.0);
    spec.group(12.0, 8.0, 196.0, 176.0, "Magnification");
    for (i, p) in presets.iter().enumerate() {
        spec.radio(
            &format!("z{p}"),
            "zoom",
            24.0,
            28.0 + 22.0 * i as f32,
            150.0,
            &format!("{p}%"),
            z == *p,
        );
    }
    spec.radio(
        "fitsel",
        "zoom",
        24.0,
        138.0,
        170.0,
        "&Fit selection",
        false,
    );
    let custom = !presets.contains(&z);
    spec.radio("custom", "zoom", 24.0, 160.0, 80.0, "&Custom:", custom);
    spec.spin(
        "percent",
        108.0,
        156.0,
        70.0,
        (f64::from(z), 10.0, 400.0, 1.0),
    );
    spec.label(182.0, 160.0, 20.0, "%");
    spec.focus = Some(if custom {
        "custom".to_owned()
    } else {
        format!("z{z}")
    });
    (
        spec,
        Box::new(Zoom {
            fits: vec![("fitsel", "view.zoom.zoom-to-selection")],
        }),
    )
}

impl View for Zoom {
    fn event(&mut self, _s: &mut Shell, ui: &mut Ui, d: &mut Dialog, o: &Outcome) {
        // Typing in the box chooses Custom.
        if let Outcome::Changed(id) = o
            && id == "percent"
        {
            d.set_radio(ui, "custom");
        }
    }

    fn commit(
        &mut self,
        s: &mut Shell,
        ui: &mut Ui,
        _d: &mut Dialog,
        v: &Values,
    ) -> Result<(), String> {
        let choice = v.radio("zoom");
        if let Some((_, cmd)) = self.fits.iter().find(|(id, _)| *id == choice) {
            s.run_id(ui, cmd);
            return Ok(());
        }
        let percent = match choice.strip_prefix('z').and_then(|n| n.parse::<u32>().ok()) {
            Some(p) => p,
            None => {
                let p = v.num("percent");
                if !(10.0..=400.0).contains(&p) {
                    return Err("Enter a number between 10 and 400.".to_owned());
                }
                p as u32
            }
        };
        s.run_id(ui, &format!("view.zoom.zoom={percent}"));
        Ok(())
    }
}
