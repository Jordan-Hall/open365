//! The Writer dialogs that had only typed `=value` forms: Page Setup (Margins and Paper), Columns,
//! Line Numbers, Hyphenation, Watermark, Borders and Shading (Page Border), Zoom and Paragraph.
//! Each answers with the Layout, Design, View or Home command forms the ribbon already has.

use open365_commands::layout_ops::paper_sizes;
use open365_docx::Section;
use open365_ui_core::Ui;
use open365_ui_core::dialog::{Dialog, Outcome, Spec, Values};

use super::View;
use crate::dialogs::request::WriterDialog;
use crate::shell::Shell;

pub(super) fn make(s: &mut Shell, which: WriterDialog) -> Option<(Spec, Box<dyn View>)> {
    s.writer.as_ref()?;
    Some(match which {
        WriterDialog::PageSetup => page_setup(s),
        WriterDialog::Columns => columns(s),
        WriterDialog::LineNumbers => line_numbers(),
        WriterDialog::Hyphenation => hyphenation(s),
        WriterDialog::Watermark => watermark(s),
        WriterDialog::PageBorders => page_borders(),
        WriterDialog::Zoom => zoom(s),
        WriterDialog::Paragraph => paragraph(s),
    })
}

fn section(s: &Shell) -> Option<Section> {
    s.writer
        .as_ref()
        .and_then(crate::doc::WriterPane::caret_section)
}

/// Twips as inches with a `"`, as Word's boxes show them.
fn inches(twips: i64) -> String {
    let v = twips as f64 / 1440.0;
    let t = format!("{v:.2}");
    let t = t.trim_end_matches('0').trim_end_matches('.');
    format!("{t}\"")
}

// ---- Page Setup -----------------------------------------------------------------------------------

#[derive(Debug)]
struct PageSetup {
    landscape: bool,
    paper: usize,
    size: (String, String),
}

fn page_setup(s: &mut Shell) -> (Spec, Box<dyn View>) {
    let sec = section(s);
    let m = sec.as_ref().map(|x| x.margins);
    let t = |f: fn(&open365_docx::PageMargins) -> i32| {
        m.as_ref()
            .map_or_else(|| "1\"".to_owned(), |m| inches(i64::from(f(m))))
    };
    let landscape = sec
        .as_ref()
        .is_some_and(|x| x.page.orient == open365_docx::Orientation::Landscape);
    let sizes = paper_sizes();
    let (w, h) = sec.as_ref().map_or((12240, 15840), |x| {
        let (w, h) = (x.page.width_twips, x.page.height_twips);
        if landscape { (h, w) } else { (w, h) }
    });
    let paper = sizes
        .iter()
        .position(|p| {
            (p.width * 20.0 - f64::from(w)).abs() < 2.0
                && (p.height * 20.0 - f64::from(h)).abs() < 2.0
        })
        .unwrap_or(sizes.len());
    let mut spec = Spec::new("Page Setup", 420.0, 300.0);
    spec.tabs = vec!["&Margins".into(), "&Paper".into()];
    spec.on_page(Some(0));
    spec.group(12.0, 40.0, 396.0, 120.0, "Margins");
    spec.label(24.0, 66.0, 50.0, "&Top:");
    spec.text("top", 80.0, 62.0, 90.0, &t(|m| m.top_twips));
    spec.label(210.0, 66.0, 70.0, "&Bottom:");
    spec.text("bottom", 290.0, 62.0, 90.0, &t(|m| m.bottom_twips));
    spec.label(24.0, 98.0, 50.0, "&Left:");
    spec.text("left", 80.0, 94.0, 90.0, &t(|m| m.left_twips));
    spec.label(210.0, 98.0, 70.0, "&Right:");
    spec.text("right", 290.0, 94.0, 90.0, &t(|m| m.right_twips));
    spec.label(24.0, 130.0, 60.0, "&Gutter:");
    spec.text("gutter", 80.0, 126.0, 90.0, &t(|m| m.gutter_twips));
    spec.group(12.0, 168.0, 396.0, 70.0, "Orientation");
    spec.radio(
        "portrait",
        "orient",
        24.0,
        194.0,
        170.0,
        "P&ortrait",
        !landscape,
    );
    spec.radio(
        "landscape",
        "orient",
        210.0,
        194.0,
        170.0,
        "Lan&dscape",
        landscape,
    );
    spec.on_page(Some(1));
    spec.label(12.0, 44.0, 120.0, "Pape&r size:");
    let mut names: Vec<&str> = sizes.iter().map(|p| p.label).collect();
    names.push("Custom size");
    spec.dropdown("paper", 12.0, 64.0, 300.0, &names, paper);
    let fmt = |pt: f64| inches((pt * 20.0).round() as i64);
    let size = (inches(i64::from(w)), inches(i64::from(h)));
    spec.label(12.0, 100.0, 60.0, "&Width:");
    spec.text("width", 80.0, 96.0, 100.0, &size.0);
    spec.label(200.0, 100.0, 60.0, "H&eight:");
    spec.text("height", 264.0, 96.0, 100.0, &size.1);
    let _ = fmt;
    spec.on_page(None);
    spec.footer.push(open365_ui_core::dialog::FooterButton::new(
        "apply",
        "&Apply",
        open365_ui_core::dialog::ButtonRole::Normal,
    ));
    (
        spec,
        Box::new(PageSetup {
            landscape,
            paper,
            size,
        }),
    )
}

impl View for PageSetup {
    fn event(&mut self, _s: &mut Shell, ui: &mut Ui, d: &mut Dialog, o: &Outcome) {
        if let Outcome::Changed(id) = o
            && id == "paper"
            && let Some(i) = d.values(ui).index("paper")
            && let Some(p) = paper_sizes().get(i)
        {
            d.set_text(ui, "width", &inches((p.width * 20.0).round() as i64));
            d.set_text(ui, "height", &inches((p.height * 20.0).round() as i64));
        }
    }

    fn commit(
        &mut self,
        s: &mut Shell,
        ui: &mut Ui,
        _d: &mut Dialog,
        v: &Values,
    ) -> Result<(), String> {
        let field = |id: &str| -> Result<String, String> {
            let t = v.text(id).trim().to_owned();
            if crate::ribbon::writer::layout::parse_len(&t, 72.0).is_none() {
                return Err("Enter a measurement such as 1\" or 2.54 cm.".to_owned());
            }
            Ok(t)
        };
        let margins = format!(
            "{},{},{},{},{}",
            field("top")?,
            field("bottom")?,
            field("left")?,
            field("right")?,
            field("gutter")?
        );
        let landscape = v.radio("orient") == "landscape";
        let mut cmds = Vec::new();
        if landscape != self.landscape {
            cmds.push(format!(
                "layout.page-set-up.orientation.{}",
                if landscape { "landscape" } else { "portrait" }
            ));
        }
        let (w, h) = (field("width")?, field("height")?);
        let paper = v.index("paper").unwrap_or(self.paper);
        if (w.clone(), h.clone()) != self.size {
            // A size gallery entry keeps its paper code; typed sizes are custom.
            match paper_sizes().get(paper) {
                Some(p) if paper != self.paper => {
                    cmds.push(format!("layout.page-set-up.size.{}", p.id))
                }
                _ => cmds.push(format!("layout.page-set-up.size.custom={w},{h}")),
            }
        }
        cmds.push(format!("layout.page-set-up.margins.custom={margins}"));
        s.run_ids(ui, &cmds);
        self.landscape = landscape;
        self.paper = paper;
        self.size = (w, h);
        Ok(())
    }
}

// ---- Columns ---------------------------------------------------------------------------------------

#[derive(Debug)]
struct Columns;

fn columns(s: &mut Shell) -> (Spec, Box<dyn View>) {
    let cols = section(s).map(|x| x.columns);
    let count = cols.as_ref().map_or(1, |c| c.count);
    let space = cols.as_ref().map_or(720, |c| i64::from(c.space_twips));
    let line = cols.as_ref().is_some_and(|c| c.separator);
    let mut spec = Spec::new("Columns", 380.0, 220.0);
    spec.group(12.0, 8.0, 356.0, 66.0, "Presets");
    for (i, (id, label)) in [
        ("one", "&One"),
        ("two", "&Two"),
        ("three", "T&hree"),
        ("left", "L&eft"),
        ("right", "&Right"),
    ]
    .iter()
    .enumerate()
    {
        let x = 24.0 + 66.0 * i as f32;
        spec.radio(id, "preset", x, 36.0, 64.0, label, false);
    }
    spec.label(12.0, 92.0, 160.0, "&Number of columns:");
    spec.spin(
        "count",
        180.0,
        88.0,
        80.0,
        (f64::from(count), 1.0, 45.0, 1.0),
    );
    spec.check("line", 12.0, 122.0, 200.0, "&Line between", line);
    spec.label(12.0, 154.0, 160.0, "Spacin&g:");
    spec.text("space", 180.0, 150.0, 80.0, &inches(space));
    (spec, Box::new(Columns))
}

impl View for Columns {
    fn event(&mut self, _s: &mut Shell, ui: &mut Ui, d: &mut Dialog, o: &Outcome) {
        // Choosing a preset sets the count; editing the count leaves the presets.
        match o {
            Outcome::Changed(id) if id == "count" || id == "space" || id == "line" => {
                for p in ["one", "two", "three", "left", "right"] {
                    d.set_checked(ui, p, false);
                }
            }
            Outcome::Changed(id)
                if ["one", "two", "three", "left", "right"].contains(&id.as_str()) =>
            {
                let n = match id.as_str() {
                    "one" => 1.0,
                    "two" | "left" | "right" => 2.0,
                    _ => 3.0,
                };
                d.set_number(ui, "count", n);
            }
            _ => {}
        }
    }

    fn commit(
        &mut self,
        s: &mut Shell,
        ui: &mut Ui,
        _d: &mut Dialog,
        v: &Values,
    ) -> Result<(), String> {
        let preset = v.radio("preset");
        if !preset.is_empty() {
            s.run_id(ui, &format!("layout.page-set-up.columns.{preset}"));
            return Ok(());
        }
        let count = v.num("count");
        if !(1.0..=45.0).contains(&count) {
            return Err("Enter a number of columns from 1 to 45.".to_owned());
        }
        let space = v.text("space").trim().to_owned();
        if crate::ribbon::writer::layout::parse_len(&space, 72.0).is_none() {
            return Err("Enter a measurement such as 0.5\" or 1.27 cm.".to_owned());
        }
        s.run_id(
            ui,
            &format!(
                "layout.page-set-up.columns.more={},{space},{}",
                count as u32,
                u8::from(v.flag("line"))
            ),
        );
        Ok(())
    }
}

// ---- Line Numbers ------------------------------------------------------------------------------------

#[derive(Debug)]
struct LineNumbers;

fn line_numbers() -> (Spec, Box<dyn View>) {
    let mut spec = Spec::new("Line Numbers", 320.0, 244.0);
    spec.check("add", 12.0, 8.0, 280.0, "&Add line numbering", true);
    spec.label(12.0, 40.0, 120.0, "&Start at:");
    spec.spin("start", 150.0, 36.0, 80.0, (1.0, 1.0, 32767.0, 1.0));
    spec.label(12.0, 72.0, 120.0, "From &text:");
    spec.text("from", 150.0, 68.0, 80.0, "Auto");
    spec.label(12.0, 104.0, 120.0, "Count &by:");
    spec.spin("by", 150.0, 100.0, 80.0, (1.0, 1.0, 100.0, 1.0));
    spec.group(12.0, 136.0, 296.0, 94.0, "Numbering");
    spec.radio(
        "page",
        "restart",
        24.0,
        158.0,
        260.0,
        "&Restart each page",
        true,
    );
    spec.radio(
        "section",
        "restart",
        24.0,
        180.0,
        260.0,
        "Restart each sectio&n",
        false,
    );
    spec.radio(
        "continuous",
        "restart",
        24.0,
        202.0,
        260.0,
        "C&ontinuous",
        false,
    );
    (spec, Box::new(LineNumbers))
}

impl View for LineNumbers {
    fn event(&mut self, _s: &mut Shell, ui: &mut Ui, d: &mut Dialog, o: &Outcome) {
        if let Outcome::Changed(id) = o
            && id == "add"
        {
            let on = d.values(ui).flag("add");
            for c in ["start", "from", "by", "page", "section", "continuous"] {
                d.set_enabled(ui, c, on);
            }
        }
    }

    fn commit(
        &mut self,
        s: &mut Shell,
        ui: &mut Ui,
        _d: &mut Dialog,
        v: &Values,
    ) -> Result<(), String> {
        if !v.flag("add") {
            s.run_id(ui, "layout.page-set-up.line-numbers.none");
            return Ok(());
        }
        let from = v.text("from").trim();
        let from = if from.eq_ignore_ascii_case("auto") || from.is_empty() {
            "0.1\""
        } else {
            from
        };
        if crate::ribbon::writer::layout::parse_len(from, 1.0).is_none() {
            return Err("Enter a distance such as 0.2\".".to_owned());
        }
        let restart = match v.radio("restart") {
            "section" => "s",
            "continuous" => "c",
            _ => "p",
        };
        s.run_id(
            ui,
            &format!(
                "layout.page-set-up.line-numbers.options={},{},{from},{restart}",
                v.num("by") as u32,
                v.num("start") as u32
            ),
        );
        Ok(())
    }
}

// ---- Hyphenation --------------------------------------------------------------------------------------

#[derive(Debug)]
struct Hyphenation;

fn hyphenation(s: &mut Shell) -> (Spec, Box<dyn View>) {
    let on = s
        .writer
        .as_ref()
        .is_some_and(crate::doc::WriterPane::hyphenation_on);
    let mut spec = Spec::new("Hyphenation", 340.0, 176.0);
    spec.check(
        "auto",
        12.0,
        8.0,
        310.0,
        "&Automatically hyphenate document",
        on,
    );
    spec.check("caps", 12.0, 34.0, 310.0, "Hyphenate words in &CAPS", true);
    spec.label(12.0, 70.0, 180.0, "Hyphenation &zone:");
    spec.text("zone", 200.0, 66.0, 100.0, "0.25\"");
    spec.label(12.0, 102.0, 180.0, "Limit consecutive &hyphens to:");
    spec.spin("limit", 200.0, 98.0, 100.0, (0.0, 0.0, 100.0, 1.0));
    spec.label(12.0, 128.0, 300.0, "A limit of 0 means no limit.");
    (spec, Box::new(Hyphenation))
}

impl View for Hyphenation {
    fn commit(
        &mut self,
        s: &mut Shell,
        ui: &mut Ui,
        _d: &mut Dialog,
        v: &Values,
    ) -> Result<(), String> {
        let zone = v.text("zone").trim().to_owned();
        if crate::ribbon::writer::layout::parse_len(&zone, 72.0).is_none() {
            return Err("Enter a measurement such as 0.25\".".to_owned());
        }
        let auto = if v.flag("auto") { "automatic" } else { "none" };
        s.run_id(ui, &format!("layout.page-set-up.hyphenation.{auto}"));
        s.run_id(
            ui,
            &format!(
                "layout.page-set-up.hyphenation.options={zone},{},{}",
                v.num("limit") as u32,
                u8::from(v.flag("caps"))
            ),
        );
        Ok(())
    }
}

// ---- Watermark ------------------------------------------------------------------------------------------

const MARKS: [&str; 7] = [
    "ASAP",
    "CONFIDENTIAL",
    "COPY",
    "DO NOT COPY",
    "DRAFT",
    "SAMPLE",
    "URGENT",
];
const WM_COLOURS: [(&str, &str); 6] = [
    ("Silver", "C0C0C0"),
    ("Gray", "808080"),
    ("Red", "FF0000"),
    ("Blue", "0070C0"),
    ("Green", "00B050"),
    ("Black", "000000"),
];
const WM_FONTS: [&str; 5] = ["Calibri", "Arial", "Times New Roman", "Cambria", "Verdana"];

#[derive(Debug)]
struct Watermark;

fn watermark(s: &mut Shell) -> (Spec, Box<dyn View>) {
    let cur = s
        .writer
        .as_ref()
        .and_then(crate::doc::WriterPane::watermark_text);
    let mut spec = Spec::new("Printed Watermark", 400.0, 270.0);
    spec.radio(
        "none",
        "mode",
        12.0,
        8.0,
        360.0,
        "&No watermark",
        cur.is_none(),
    );
    spec.radio(
        "picture",
        "mode",
        12.0,
        32.0,
        360.0,
        "&Picture watermark",
        false,
    )
    .disabled();
    spec.radio(
        "text",
        "mode",
        12.0,
        56.0,
        360.0,
        "&Text watermark",
        cur.is_some(),
    );
    spec.label(36.0, 86.0, 80.0, "Te&xt:");
    spec.combo(
        "wm_text",
        130.0,
        82.0,
        240.0,
        &MARKS,
        cur.as_deref().unwrap_or("DRAFT"),
    );
    spec.label(36.0, 116.0, 80.0, "&Font:");
    spec.combo("wm_font", 130.0, 112.0, 240.0, &WM_FONTS, "Calibri");
    spec.label(36.0, 146.0, 80.0, "Si&ze:");
    spec.text("wm_size", 130.0, 142.0, 100.0, "Auto");
    spec.label(36.0, 176.0, 80.0, "&Colour:");
    let names: Vec<&str> = WM_COLOURS.iter().map(|c| c.0).collect();
    spec.dropdown("wm_colour", 130.0, 172.0, 140.0, &names, 0);
    spec.check("wm_semi", 280.0, 174.0, 100.0, "Semitran&sparent", true);
    spec.label(36.0, 206.0, 80.0, "Layout:");
    spec.radio("wm_diag", "layout", 130.0, 204.0, 120.0, "&Diagonal", true);
    spec.radio(
        "wm_horiz",
        "layout",
        256.0,
        204.0,
        120.0,
        "&Horizontal",
        false,
    );
    spec.footer.push(open365_ui_core::dialog::FooterButton::new(
        "apply",
        "&Apply",
        open365_ui_core::dialog::ButtonRole::Normal,
    ));
    (spec, Box::new(Watermark))
}

impl View for Watermark {
    fn event(&mut self, _s: &mut Shell, ui: &mut Ui, d: &mut Dialog, o: &Outcome) {
        // Typing text chooses "Text watermark".
        if let Outcome::Changed(id) = o
            && id.starts_with("wm_")
        {
            d.set_radio(ui, "text");
        }
    }

    fn commit(
        &mut self,
        s: &mut Shell,
        ui: &mut Ui,
        _d: &mut Dialog,
        v: &Values,
    ) -> Result<(), String> {
        if v.radio("mode") != "text" {
            s.run_id(ui, "design.page-background.watermark.remove");
            return Ok(());
        }
        let text = v.text("wm_text").trim().to_owned();
        if text.is_empty() {
            return Err("Type the watermark text.".to_owned());
        }
        if text.contains(['|', '=']) {
            return Err("The text cannot contain | or =.".to_owned());
        }
        let size = v.text("wm_size").trim();
        let size = if size.eq_ignore_ascii_case("auto") {
            String::new()
        } else {
            size.to_owned()
        };
        if !size.is_empty() && size.parse::<f64>().is_err() {
            return Err("Enter a size in points, or Auto.".to_owned());
        }
        let colour = WM_COLOURS[v.index("wm_colour").unwrap_or(0).min(WM_COLOURS.len() - 1)].1;
        s.run_id(
            ui,
            &format!(
                "design.page-background.watermark.custom={text}|{}|{size}|{colour}|{}|{}",
                v.text("wm_font").trim(),
                if v.radio("layout") == "wm_horiz" {
                    "h"
                } else {
                    "d"
                },
                u8::from(v.flag("wm_semi"))
            ),
        );
        Ok(())
    }
}

// ---- Page Borders ------------------------------------------------------------------------------------------

const BORDER_STYLES: [(&str, &str); 7] = [
    ("Single line", "single"),
    ("Double line", "double"),
    ("Dotted", "dotted"),
    ("Dashed", "dashed"),
    ("Dot dash", "dotDash"),
    ("Wavy line", "wave"),
    ("Thick-thin", "thickThinSmallGap"),
];
const BORDER_WIDTHS: [&str; 8] = ["0.5", "0.75", "1", "1.5", "2.25", "3", "4.5", "6"];
const BORDER_APPLY: [(&str, &str); 4] = [
    ("Whole document", "whole"),
    ("This section", "section"),
    ("First page only", "first"),
    ("All except first page", "notfirst"),
];

#[derive(Debug)]
struct PageBorders;

fn page_borders() -> (Spec, Box<dyn View>) {
    let mut spec = Spec::new("Borders and Shading", 420.0, 280.0);
    spec.tabs = vec!["Page &Border".into()];
    spec.on_page(Some(0));
    spec.label(12.0, 44.0, 80.0, "Setting:");
    spec.radio("none", "setting", 12.0, 62.0, 90.0, "N&one", true);
    spec.radio("box", "setting", 12.0, 84.0, 90.0, "&Box", false);
    spec.radio("shadow", "setting", 12.0, 106.0, 90.0, "&Shadow", false);
    spec.radio("3d", "setting", 12.0, 128.0, 90.0, "3-&D", false);
    spec.label(130.0, 44.0, 80.0, "St&yle:");
    let styles: Vec<&str> = BORDER_STYLES.iter().map(|b| b.0).collect();
    spec.dropdown("style", 130.0, 62.0, 160.0, &styles, 0);
    spec.label(130.0, 94.0, 80.0, "&Colour:");
    spec.dropdown(
        "colour",
        130.0,
        112.0,
        160.0,
        &["Automatic", "Black", "Red", "Blue", "Green"],
        0,
    );
    spec.label(130.0, 144.0, 80.0, "&Width (pt):");
    spec.combo("width", 130.0, 162.0, 160.0, &BORDER_WIDTHS, "0.5");
    spec.label(304.0, 44.0, 100.0, "Apply &to:");
    let apply: Vec<&str> = BORDER_APPLY.iter().map(|b| b.0).collect();
    spec.dropdown("apply", 304.0, 62.0, 104.0, &apply, 0);
    spec.label(304.0, 94.0, 100.0, "Measure &from:");
    spec.dropdown("from", 304.0, 112.0, 104.0, &["Edge of page", "Text"], 0);
    spec.label(304.0, 144.0, 100.0, "&Margin (pt):");
    spec.spin("space", 304.0, 162.0, 104.0, (24.0, 0.0, 31.0, 1.0));
    spec.on_page(None);
    (spec, Box::new(PageBorders))
}

impl View for PageBorders {
    fn event(&mut self, _s: &mut Shell, ui: &mut Ui, d: &mut Dialog, o: &Outcome) {
        // Any style choice means a border: leave Setting "None" for "Box".
        if let Outcome::Changed(id) = o
            && ["style", "colour", "width", "space"].contains(&id.as_str())
            && d.values(ui).radio("setting") == "none"
        {
            d.set_radio(ui, "box");
        }
    }

    fn commit(
        &mut self,
        s: &mut Shell,
        ui: &mut Ui,
        _d: &mut Dialog,
        v: &Values,
    ) -> Result<(), String> {
        let setting = v.radio("setting");
        if setting == "none" || setting.is_empty() {
            s.run_id(ui, "design.page-background.page-borders=none");
            return Ok(());
        }
        let width: f64 = v
            .text("width")
            .trim()
            .parse()
            .map_err(|_| "Enter a width in points from 0.25 to 6.".to_owned())?;
        if !(0.25..=6.0).contains(&width) {
            return Err("Enter a width in points from 0.25 to 6.".to_owned());
        }
        let style = BORDER_STYLES[v.index("style").unwrap_or(0).min(BORDER_STYLES.len() - 1)].1;
        let colour =
            ["auto", "000000", "FF0000", "0070C0", "00B050"][v.index("colour").unwrap_or(0).min(4)];
        let apply = BORDER_APPLY[v.index("apply").unwrap_or(0).min(BORDER_APPLY.len() - 1)].1;
        let from = if v.index("from") == Some(1) {
            "text"
        } else {
            "page"
        };
        s.run_id(
            ui,
            &format!(
                "design.page-background.page-borders={setting},{style},{width},{colour},{},{from},{apply}",
                v.num("space") as u32
            ),
        );
        Ok(())
    }
}

// ---- Zoom --------------------------------------------------------------------------------------------------------

#[derive(Debug)]
struct WriterZoom;

fn zoom(s: &mut Shell) -> (Spec, Box<dyn View>) {
    let z = s.writer.as_ref().map_or(100, |p| p.zoom_percent() as u32);
    let presets = [200_u32, 100, 75];
    let mut spec = Spec::new("Zoom", 300.0, 252.0);
    spec.group(12.0, 8.0, 276.0, 190.0, "Zoom to");
    for (i, p) in presets.iter().enumerate() {
        spec.radio(
            &format!("z{p}"),
            "zoom",
            24.0,
            28.0 + 22.0 * i as f32,
            120.0,
            &format!("{p}%"),
            z == *p,
        );
    }
    spec.radio(
        "pagewidth",
        "zoom",
        150.0,
        28.0,
        130.0,
        "Pa&ge width",
        false,
    );
    spec.radio("onepage", "zoom", 150.0, 50.0, 130.0, "&Whole page", false);
    spec.radio(
        "manypages",
        "zoom",
        150.0,
        72.0,
        130.0,
        "&Many pages",
        false,
    );
    let custom = !presets.contains(&z);
    spec.radio("custom", "zoom", 24.0, 108.0, 90.0, "&Percent:", custom);
    spec.spin(
        "percent",
        118.0,
        104.0,
        80.0,
        (f64::from(z), 10.0, 500.0, 1.0),
    );
    spec.label(202.0, 108.0, 20.0, "%");
    spec.focus = Some(if custom {
        "custom".to_owned()
    } else {
        format!("z{z}")
    });
    (spec, Box::new(WriterZoom))
}

impl View for WriterZoom {
    fn event(&mut self, _s: &mut Shell, ui: &mut Ui, d: &mut Dialog, o: &Outcome) {
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
        match v.radio("zoom") {
            "pagewidth" => s.run_id(ui, "view.zoom.page-width"),
            "onepage" => s.run_id(ui, "view.zoom.one-page"),
            "manypages" => s.run_id(ui, "view.zoom.multiple-pages"),
            other => {
                let percent = match other.strip_prefix('z').and_then(|n| n.parse::<u32>().ok()) {
                    Some(p) => p,
                    None => {
                        let p = v.num("percent");
                        if !(10.0..=500.0).contains(&p) {
                            return Err("Enter a number between 10 and 500.".to_owned());
                        }
                        p as u32
                    }
                };
                s.run_id(ui, &format!("view.zoom.zoom={percent}"))
            }
        };
        Ok(())
    }
}

// ---- Paragraph ------------------------------------------------------------------------------------------------------

#[derive(Debug)]
struct Paragraph {
    align: usize,
    /// Left, right and first-line (negative: hanging) indent when the dialog opened, in points.
    indents: (f64, f64, f64),
    /// The outline level when the dialog opened (0 is Body Text).
    outline: usize,
}

const OUTLINE_LEVELS: [&str; 10] = [
    "Body Text",
    "Level 1",
    "Level 2",
    "Level 3",
    "Level 4",
    "Level 5",
    "Level 6",
    "Level 7",
    "Level 8",
    "Level 9",
];

/// Points as a box shows them: inches with a `"`.
fn pt_inches(pt: f64) -> String {
    inches((pt * 20.0).round() as i64)
}

fn paragraph(s: &mut Shell) -> (Spec, Box<dyn View>) {
    use open365_docx::Alignment as A;
    let pane = s.writer.as_ref();
    let align = match pane.and_then(crate::doc::WriterPane::alignment_state) {
        Some(A::Center) => 1,
        Some(A::Right) => 2,
        Some(A::Justify) => 3,
        _ => 0,
    };
    let (left, right, first) = pane
        .and_then(crate::doc::WriterPane::ruler_info)
        .map_or((0.0, 0.0, 0.0), |r| {
            (r.indent_left_pt, r.indent_right_pt, r.first_line_pt)
        });
    let special = if first > 0.01 {
        1
    } else if first < -0.01 {
        2
    } else {
        0
    };
    let outline = pane.map_or(0, crate::doc::WriterPane::outline_level);
    let mut spec = Spec::new("Paragraph", 400.0, 346.0);
    spec.tabs = vec!["Indents and &Spacing".into()];
    spec.on_page(Some(0));
    spec.group(12.0, 40.0, 376.0, 88.0, "General");
    spec.label(24.0, 66.0, 90.0, "Align&ment:");
    spec.dropdown(
        "align",
        120.0,
        62.0,
        130.0,
        &["Left", "Centered", "Right", "Justified"],
        align,
    );
    spec.label(24.0, 96.0, 90.0, "&Outline level:");
    spec.dropdown("outline", 120.0, 92.0, 130.0, &OUTLINE_LEVELS, outline);
    spec.group(12.0, 134.0, 376.0, 94.0, "Indentation");
    spec.label(24.0, 158.0, 50.0, "&Left:");
    spec.text("left", 70.0, 154.0, 90.0, &pt_inches(left));
    spec.label(210.0, 158.0, 50.0, "&Right:");
    spec.text("right", 262.0, 154.0, 90.0, &pt_inches(right));
    spec.label(24.0, 190.0, 50.0, "&Special:");
    spec.dropdown(
        "special",
        70.0,
        186.0,
        110.0,
        &["(none)", "First line", "Hanging"],
        special,
    );
    spec.label(210.0, 190.0, 50.0, "B&y:");
    spec.text(
        "by",
        262.0,
        186.0,
        90.0,
        &if special == 0 {
            String::new()
        } else {
            pt_inches(first.abs())
        },
    )
    .enabled = special != 0;
    spec.group(12.0, 234.0, 376.0, 104.0, "Spacing");
    spec.label(24.0, 258.0, 60.0, "&Before:");
    spec.text("before", 90.0, 254.0, 90.0, "0 pt");
    spec.label(210.0, 258.0, 60.0, "A&fter:");
    spec.text("after", 262.0, 254.0, 90.0, "8 pt");
    spec.label(24.0, 290.0, 90.0, "Li&ne spacing:");
    spec.dropdown(
        "spacing",
        110.0,
        286.0,
        140.0,
        &["Keep as is", "Single", "1.5 lines", "Double"],
        0,
    );
    spec.on_page(None);
    (
        spec,
        Box::new(Paragraph {
            align,
            indents: (left, right, first),
            outline,
        }),
    )
}

impl View for Paragraph {
    fn event(&mut self, _s: &mut Shell, ui: &mut Ui, d: &mut Dialog, o: &Outcome) {
        // Word fills By with half an inch when a special indent is chosen, and empties it for
        // none.
        if let Outcome::Changed(id) = o
            && id == "special"
        {
            let special = d.values(ui).index("special").unwrap_or(0);
            d.set_enabled(ui, "by", special != 0);
            let by = d.values(ui).text("by").trim().to_owned();
            if special == 0 {
                d.set_text(ui, "by", "");
            } else if by.is_empty() || by.starts_with("0\"") || by == "0" {
                d.set_text(ui, "by", "0.5\"");
            }
        }
    }

    fn commit(
        &mut self,
        s: &mut Shell,
        ui: &mut Ui,
        _d: &mut Dialog,
        v: &Values,
    ) -> Result<(), String> {
        use crate::doc::ruler::RulerEdit;
        use crate::ribbon::writer::layout::parse_len;
        const INDENT_MSG: &str = "Enter an indent such as 0.5\" or 1.27 cm.";
        let len = |id: &str, unit: f64, msg: &str| -> Result<f64, String> {
            parse_len(v.text(id).trim(), unit).ok_or_else(|| msg.to_owned())
        };
        let left = len("left", 72.0, INDENT_MSG)?;
        let right = len("right", 72.0, INDENT_MSG)?;
        let special = v.index("special").unwrap_or(0);
        let by = if special == 0 {
            0.0
        } else {
            let by = len("by", 72.0, INDENT_MSG)?;
            if by < 0.0 {
                return Err(INDENT_MSG.to_owned());
            }
            by
        };
        let first = match special {
            1 => by,
            2 => -by,
            _ => 0.0,
        };
        let before = len("before", 1.0, "Enter spacing from 0 pt to 1584 pt.")?;
        let after = len("after", 1.0, "Enter spacing from 0 pt to 1584 pt.")?;
        let mut cmds = Vec::new();
        let align = v.index("align").unwrap_or(self.align);
        if align != self.align {
            cmds.push(
                [
                    "home.paragraph.align-left",
                    "home.paragraph.centre",
                    "home.paragraph.align-right",
                    "home.paragraph.justify",
                ][align.min(3)]
                .to_owned(),
            );
        }
        // Indents and spacing are written only when they differ from what the dialog opened with.
        let moved = |a: f64, b: f64| (a - b).abs() > 0.01;
        if moved(left, self.indents.0)
            || moved(right, self.indents.1)
            || moved(first, self.indents.2)
        {
            if let Some(w) = s.writer.as_mut() {
                let para = w.ruler_info().map_or(0, |r| r.para);
                w.apply_ruler_edit(&RulerEdit::SetIndents {
                    para,
                    left_pt: left,
                    first_line_pt: first,
                    right_pt: right,
                });
            }
            self.indents = (left, right, first);
        }
        if moved(before, 0.0) {
            cmds.push(format!(
                "layout.paragraph.spacing-before={}",
                v.text("before").trim()
            ));
        }
        if moved(after, 8.0) {
            cmds.push(format!(
                "layout.paragraph.spacing-after={}",
                v.text("after").trim()
            ));
        }
        match v.index("spacing") {
            Some(1) => cmds.push("home.paragraph.line-and-paragraph-spacing=1.0".to_owned()),
            Some(2) => cmds.push("home.paragraph.line-and-paragraph-spacing=1.5".to_owned()),
            Some(3) => cmds.push("home.paragraph.line-and-paragraph-spacing=2.0".to_owned()),
            _ => {}
        }
        // Only a changed level is written, so a heading style's own level stays as it is.
        let outline = v.index("outline").unwrap_or(self.outline);
        if outline != self.outline {
            if let Some(w) = s.writer.as_mut() {
                w.set_outline_level(outline);
            }
            self.outline = outline;
        }
        s.run_ids(ui, &cmds);
        self.align = align;
        Ok(())
    }
}
