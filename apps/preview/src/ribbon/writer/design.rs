//! Writer ribbon: the Design tab (`design.*`): document formatting (themes, style sets, colours,
//! fonts, paragraph spacing, effects) and the page background (watermark, colour, borders).
//!
//! A gallery item has the id `<control>.<item>` (`design.document-formatting.fonts.modern`);
//! [`menu_items`] lists them. A typed value follows `=`:
//! `design.page-background.watermark.custom=TEXT|font|size|RRGGBB|d or h|1 or 0` and
//! `design.page-background.page-colour.custom=RRGGBB`. Each change is one undo step.
//!
//! Not wired: Set as Default (it needs a per-user default template to write to).

use open365_commands::design_ops::{self, BordersData, DesignOp, SideData, WatermarkData};
use open365_ui_core::MenuItem;

use crate::doc::WriterPane;
use crate::ribbon::{Handled, RibbonCtx};

const DOC: &str = "design.document-formatting.";
const BG: &str = "design.page-background.";

/// The control ids this module handles (ids as in the spec, without `=` or `#` suffixes).
pub const IDS: &[&str] = &[
    "design.document-formatting.themes",
    "design.document-formatting.quickstylessets",
    "design.document-formatting.colours",
    "design.document-formatting.fonts",
    "design.document-formatting.paragraph-spacing",
    "design.document-formatting.effects",
    "design.page-background.watermark",
    "design.page-background.page-colour",
    "design.page-background.page-borders",
];

/// The control `id` belongs to and the item after it (`""` for the control itself).
fn split(id: &str) -> Option<(&'static str, &str)> {
    let id = crate::ribbon::base(id);
    IDS.iter().find_map(|k| {
        let rest = id.strip_prefix(k)?;
        match rest.strip_prefix('.') {
            Some(item) => Some((*k, item)),
            None if rest.is_empty() => Some((*k, "")),
            None => None,
        }
    })
}

fn note(p: &mut WriterPane, s: &str) {
    p.status = Some(s.to_owned());
}

/// Runs command `id`; [`Handled::No`] when this module does not know it.
pub fn handle(ctx: &mut RibbonCtx<'_>, id: &str) -> Handled {
    let typed = id.split_once('=').map(|(_, v)| v.trim().to_owned());
    let Some((key, item)) = split(id) else {
        return Handled::No;
    };
    let item = if item == "run" { "" } else { item };
    let Some(p) = ctx.writer.as_deref_mut() else {
        return Handled::Yes;
    };
    let known = match key.strip_prefix(DOC) {
        Some("themes") => theme(p, item),
        Some("quickstylessets") => style_set(p, item),
        Some("colours") => colours(p, item),
        Some("fonts") => fonts(p, item),
        Some("paragraph-spacing") => paragraph_spacing(p, item),
        Some("effects") => effects(p, item),
        _ => match key.strip_prefix(BG) {
            Some("watermark") => watermark(p, item, typed.as_deref()),
            Some("page-colour") => page_colour(p, item, typed.as_deref()),
            Some("page-borders") => page_borders(p, typed.as_deref()),
            _ => true,
        },
    };
    if !known {
        return Handled::No;
    }
    *ctx.refocus_doc = true;
    Handled::Yes
}

fn theme(p: &mut WriterPane, item: &str) -> bool {
    if item.is_empty() {
        return true;
    }
    match design_ops::themes()
        .into_iter()
        .find(|t| t.name.to_lowercase() == item)
    {
        Some(theme) => {
            p.design_apply(DesignOp::Theme { theme });
            true
        }
        None => false,
    }
}

fn style_set(p: &mut WriterPane, item: &str) -> bool {
    if item.is_empty() {
        return true;
    }
    match design_ops::style_set(item) {
        Some(styles) => {
            p.design_apply(DesignOp::StyleSet {
                name: item.to_owned(),
                styles,
            });
            true
        }
        None => false,
    }
}

fn colours(p: &mut WriterPane, item: &str) -> bool {
    if item.is_empty() {
        return true;
    }
    match design_ops::colour_sets().iter().find(|c| c.id == item) {
        Some(c) => {
            p.design_apply(DesignOp::ThemeColours {
                name: c.name.to_owned(),
                colours: c.colours.iter().map(|s| (*s).to_owned()).collect(),
            });
            true
        }
        None => false,
    }
}

fn fonts(p: &mut WriterPane, item: &str) -> bool {
    if item.is_empty() {
        return true;
    }
    match design_ops::font_sets().iter().find(|f| f.id == item) {
        Some(f) => {
            p.design_apply(DesignOp::ThemeFonts {
                name: f.name.to_owned(),
                major_font: f.major.to_owned(),
                minor_font: f.minor.to_owned(),
            });
            true
        }
        None => false,
    }
}

fn paragraph_spacing(p: &mut WriterPane, item: &str) -> bool {
    let (before_pt, after_pt, line) = match item {
        "" => return true,
        "default" => (0.0, 8.0, 259.0 / 240.0),
        _ => match design_ops::spacing_presets().iter().find(|s| s.id == item) {
            Some(s) => (s.before, s.after, s.line),
            None => return false,
        },
    };
    p.design_apply(DesignOp::ParagraphSpacing {
        before_pt,
        after_pt,
        line,
    });
    true
}

fn effects(p: &mut WriterPane, item: &str) -> bool {
    if item.is_empty() {
        return true;
    }
    match design_ops::effect_names()
        .iter()
        .position(|n| n.to_lowercase() == item)
    {
        Some(level) => {
            p.design_apply(DesignOp::ThemeEffects { level: level as u8 });
            true
        }
        None => false,
    }
}

const MARK_TEXTS: [(&str, &str); 6] = [
    ("asap", "ASAP"),
    ("confidential", "CONFIDENTIAL"),
    ("do-not-copy", "DO NOT COPY"),
    ("draft", "DRAFT"),
    ("sample", "SAMPLE"),
    ("urgent", "URGENT"),
];

fn watermark(p: &mut WriterPane, item: &str, typed: Option<&str>) -> bool {
    let preset = |text: &str, diagonal: bool| WatermarkData {
        text: text.to_owned(),
        font: "Calibri".to_owned(),
        size_pt: None,
        colour: "C0C0C0".to_owned(),
        semi_transparent: true,
        diagonal,
    };
    match item {
        "" => {}
        "remove" => {
            p.design_apply(DesignOp::Watermark { mark: None });
        }
        "custom" => {
            // `text|font|size|colour|d or h|semi (1 or 0)`; missing parts take Word's defaults.
            let parts: Vec<&str> = typed
                .unwrap_or_default()
                .split('|')
                .map(str::trim)
                .collect();
            let text = parts.first().copied().unwrap_or_default();
            if text.is_empty() {
                note(p, "Watermark: type the text, for example DRAFT");
            } else {
                let mut m = preset(text, true);
                if let Some(f) = parts.get(1).filter(|s| !s.is_empty()) {
                    m.font = (*f).to_owned();
                }
                m.size_pt = parts.get(2).and_then(|s| s.parse().ok());
                if let Some(c) = parts.get(3).filter(|s| !s.is_empty()) {
                    m.colour = c.trim_start_matches('#').to_owned();
                }
                m.diagonal = parts
                    .get(4)
                    .is_none_or(|s| !matches!(*s, "h" | "horizontal"));
                m.semi_transparent = parts.get(5).is_none_or(|s| !matches!(*s, "0" | "false"));
                p.design_apply(DesignOp::Watermark { mark: Some(m) });
            }
        }
        _ => {
            let Some((id, text, diagonal)) = MARK_TEXTS.iter().find_map(|(id, text)| {
                if item == format!("{id}-diagonal") {
                    Some((*id, *text, true))
                } else if item == format!("{id}-horizontal") {
                    Some((*id, *text, false))
                } else {
                    None
                }
            }) else {
                return false;
            };
            let _ = id;
            p.design_apply(DesignOp::Watermark {
                mark: Some(preset(text, diagonal)),
            });
        }
    }
    true
}

/// The theme colour slots of the Page Colour palette, left to right.
const PALETTE_SLOTS: [(&str, usize, &str); 10] = [
    ("lt1", 1, "bg1"),
    ("dk1", 0, "tx1"),
    ("lt2", 3, "bg2"),
    ("dk2", 2, "tx2"),
    ("accent1", 4, "accent1"),
    ("accent2", 5, "accent2"),
    ("accent3", 6, "accent3"),
    ("accent4", 7, "accent4"),
    ("accent5", 8, "accent5"),
    ("accent6", 9, "accent6"),
];

fn page_colour(p: &mut WriterPane, item: &str, typed: Option<&str>) -> bool {
    match item {
        "" => {}
        "none" => {
            p.design_apply(DesignOp::PageColour {
                colour: None,
                theme: None,
            });
        }
        "custom" => match typed.map(|t| t.trim().trim_start_matches('#').to_owned()) {
            Some(c) if c.len() == 6 && c.bytes().all(|b| b.is_ascii_hexdigit()) => {
                p.design_apply(DesignOp::PageColour {
                    colour: Some(c),
                    theme: None,
                });
            }
            _ => note(
                p,
                "Page Colour: type a colour as RRGGBB, for example FFF2CC",
            ),
        },
        _ => {
            if let Some(slot) = item.strip_prefix("theme-") {
                let Some((theme, index, _)) = PALETTE_SLOTS.iter().find(|(s, _, _)| *s == slot)
                else {
                    return false;
                };
                let colours = p
                    .theme_colours()
                    .unwrap_or_else(|| design_ops::base_theme().colours);
                p.design_apply(DesignOp::PageColour {
                    colour: colours.get(*index).cloned(),
                    theme: Some((*theme).to_owned()),
                });
            } else if let Some(name) = item.strip_prefix("standard-") {
                let Some((_, hex)) = design_ops::standard_colours()
                    .into_iter()
                    .find(|(n, _)| *n == name)
                else {
                    return false;
                };
                p.design_apply(DesignOp::PageColour {
                    colour: Some(hex.to_owned()),
                    theme: None,
                });
            } else {
                return false;
            }
        }
    }
    true
}

/// Page Borders: `setting,style,width pt,colour,space pt,from (page or text),apply to (whole,
/// section, first or notfirst)`. Setting is `none`, `box`, `shadow` or `3d`. A bare click
/// applies the dialog's defaults for Box (single line, 0.5 pt, automatic colour, 24 pt from the
/// page edge, whole document), since there is no dialog host yet.
fn page_borders(p: &mut WriterPane, typed: Option<&str>) -> bool {
    let parts: Vec<&str> = typed
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .collect();
    let part = |i: usize| parts.get(i).copied().filter(|s| !s.is_empty());
    let setting = part(0).unwrap_or("box");
    let all: Vec<usize> = (0..p.section_count()).collect();
    let apply = part(6).unwrap_or("whole");
    let sections = if apply == "section" {
        p.layout_sections()
    } else {
        all
    };
    if setting == "none" {
        p.design_apply(DesignOp::PageBorders {
            sections,
            borders: None,
        });
        return true;
    }
    let style = match (setting, part(1)) {
        ("3d", None) => "threeDEmboss",
        (_, s) => s.unwrap_or("single"),
    };
    let width = part(2).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.5);
    let side = Some(SideData {
        style: style.to_owned(),
        width_eighths: (width * 8.0).round() as u32,
        colour: part(3).unwrap_or("auto").to_owned(),
        space_pt: part(4).and_then(|s| s.parse().ok()).unwrap_or(24),
    });
    let display = match apply {
        "first" => "firstPage",
        "notfirst" => "notFirstPage",
        _ => "allPages",
    };
    p.design_apply(DesignOp::PageBorders {
        sections,
        borders: Some(BordersData {
            top: side.clone(),
            left: side.clone(),
            bottom: side.clone(),
            right: side,
            from_text: part(5) == Some("text"),
            shadow: setting == "shadow",
            display: display.to_owned(),
        }),
    });
    true
}

fn item(control: &str, id: &str, label: &str) -> MenuItem {
    MenuItem::new(&format!("{control}.{id}"), label)
}

/// The drop-down or gallery of Design control `control`, `None` for a control without one.
#[must_use]
pub fn menu_items(control: &str) -> Option<Vec<MenuItem>> {
    let c = control;
    let out = match c {
        "design.document-formatting.themes" => design_ops::themes()
            .iter()
            .map(|t| item(c, &t.name.to_lowercase(), &t.name))
            .collect(),
        "design.document-formatting.quickstylessets" => design_ops::style_set_names()
            .iter()
            .map(|(id, label)| item(c, id, label))
            .collect(),
        "design.document-formatting.colours" => {
            let mut v = vec![MenuItem::heading("Built-In")];
            v.extend(
                design_ops::colour_sets()
                    .iter()
                    .map(|s| item(c, s.id, s.name)),
            );
            v
        }
        "design.document-formatting.fonts" => {
            let mut v = vec![MenuItem::heading("Built-In")];
            v.extend(
                design_ops::font_sets()
                    .iter()
                    .map(|s| item(c, s.id, s.name)),
            );
            v
        }
        "design.document-formatting.paragraph-spacing" => {
            let mut v = vec![MenuItem::heading("Built-In"), item(c, "default", "Default")];
            v.extend(
                design_ops::spacing_presets()
                    .iter()
                    .map(|s| item(c, s.id, s.label)),
            );
            v
        }
        "design.document-formatting.effects" => design_ops::effect_names()
            .iter()
            .map(|n| item(c, &n.to_lowercase(), n))
            .collect(),
        "design.page-background.watermark" => {
            let mut v = vec![MenuItem::heading("Disclaimers")];
            for (id, text) in MARK_TEXTS {
                v.push(item(c, &format!("{id}-diagonal"), &format!("{text} 1")));
                v.push(item(c, &format!("{id}-horizontal"), &format!("{text} 2")));
            }
            v.push(MenuItem::separator());
            v.push(item(c, "custom", "Custom Watermark..."));
            v.push(item(c, "remove", "Remove Watermark"));
            v
        }
        "design.page-background.page-colour" => {
            let mut v = vec![MenuItem::heading("Theme Colours")];
            v.extend(
                PALETTE_SLOTS
                    .iter()
                    .map(|(slot, _, label)| item(c, &format!("theme-{slot}"), label)),
            );
            v.push(MenuItem::heading("Standard Colours"));
            v.extend(
                design_ops::standard_colours()
                    .into_iter()
                    .map(|(n, _)| item(c, &format!("standard-{n}"), n)),
            );
            v.push(MenuItem::separator());
            v.push(item(c, "none", "No Colour"));
            v.push(item(c, "custom", "More Colours..."));
            v
        }
        _ => return None,
    };
    Some(out)
}
