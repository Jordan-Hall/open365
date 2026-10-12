//! Writer ribbon: the Layout tab (`layout.*`): page setup, paragraph indent and spacing.
//!
//! A drop-down item has the id `<control>.<item>` (`layout.page-set-up.margins.narrow`); a typed
//! value follows `=` (`layout.paragraph.indent-left=0.5"`). The menus are listed by
//! [`menu_items`]. Page Setup changes apply to the sections the selection touches, paragraph
//! changes to the paragraphs it touches; each is one undo step.
//!
//! Lengths without a unit are inches for Page Setup and Indent, points for Spacing, as in
//! Word with US settings; `cm`, `mm`, `in`, `pt` and `"` are understood.

use open365_commands::layout_ops::{
    self, BreakKind, LayoutOp, LineNumbering, Restart, SectionStart,
};
use open365_ui_core::MenuItem;

use crate::doc::WriterPane;
use crate::ribbon::{Handled, RibbonCtx};

/// The control ids this module handles (ids as in the spec, without `=` or `#` suffixes).
pub const IDS: &[&str] = &[
    "layout.page-set-up.margins",
    "layout.page-set-up.orientation",
    "layout.page-set-up.size",
    "layout.page-set-up.columns",
    "layout.page-set-up.breaks",
    "layout.page-set-up.line-numbers",
    "layout.page-set-up.hyphenation",
    "layout.paragraph.indent-left",
    "layout.paragraph.indent-right",
    "layout.paragraph.spacing-before",
    "layout.paragraph.spacing-after",
];

/// A length typed in a box, in points. `default_unit` is points per unit of a bare number.
#[must_use]
pub fn parse_len(text: &str, default_unit: f64) -> Option<f64> {
    let t = text.trim().trim_start_matches('"');
    let (num, unit) = {
        let end = t
            .find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | ',' | '-' | '+')))
            .unwrap_or(t.len());
        (t[..end].trim(), t[end..].trim().to_ascii_lowercase())
    };
    let n: f64 = num.replace(',', ".").parse().ok()?;
    let per = match unit.as_str() {
        "" => default_unit,
        "\"" | "in" | "inch" | "inches" => 72.0,
        "cm" => 72.0 / 2.54,
        "mm" => 72.0 / 25.4,
        "pt" | "pts" => 1.0,
        "pc" => 12.0,
        _ => return None,
    };
    let pt = n * per;
    (pt.is_finite() && pt.abs() <= 1584.0).then_some(pt)
}

const INCH: f64 = 72.0;

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

fn numbers(text: &str, per: f64) -> Vec<f64> {
    text.split([',', ';'])
        .filter_map(|p| parse_len(p, per))
        .collect()
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
    let known = match key {
        "layout.page-set-up.margins" => margins(p, item, typed.as_deref()),
        "layout.page-set-up.orientation" => orientation(p, item),
        "layout.page-set-up.size" => size(p, item, typed.as_deref()),
        "layout.page-set-up.columns" => columns(p, item, typed.as_deref()),
        "layout.page-set-up.breaks" => breaks(p, item),
        "layout.page-set-up.line-numbers" => line_numbers(p, item, typed.as_deref()),
        "layout.page-set-up.hyphenation" => hyphenation(p, item, typed.as_deref()),
        "layout.paragraph.indent-left" | "layout.paragraph.indent-right" => {
            indent(p, key.ends_with("left"), typed.as_deref())
        }
        "layout.paragraph.spacing-before" | "layout.paragraph.spacing-after" => {
            spacing(p, key.ends_with("before"), typed.as_deref())
        }
        _ => true,
    };
    if !known {
        return Handled::No;
    }
    *ctx.refocus_doc = true;
    Handled::Yes
}

fn margins(p: &mut WriterPane, item: &str, typed: Option<&str>) -> bool {
    let sections = p.layout_sections();
    let landscape = p
        .caret_section()
        .is_some_and(|s| s.page.orient == open365_docx::Orientation::Landscape);
    // Landscape pages turn the presets a quarter turn, as Word does.
    let turn = |t: f64, b: f64, l: f64, r: f64| {
        if landscape {
            (l, r, b, t)
        } else {
            (t, b, l, r)
        }
    };
    if let Some(preset) = layout_ops::margin_presets().iter().find(|m| m.id == item) {
        let (top_pt, bottom_pt, left_pt, right_pt) =
            turn(preset.top, preset.bottom, preset.left, preset.right);
        p.layout_apply(LayoutOp::Margins {
            sections,
            top_pt,
            bottom_pt,
            left_pt,
            right_pt,
            gutter_pt: Some(0.0),
            mirrored: Some(preset.mirrored),
        });
        return true;
    }
    match item {
        "" => true,
        // Custom Margins: `top,bottom,left,right[,gutter]` (the dialog's fields).
        "custom" => {
            let v = typed.map(|t| numbers(t, INCH)).unwrap_or_default();
            if v.len() < 4 {
                note(
                    p,
                    "Custom Margins: type top, bottom, left and right, for example 1\",1\",1.25\",1.25\"",
                );
            } else {
                p.layout_apply(LayoutOp::Margins {
                    sections,
                    top_pt: v[0],
                    bottom_pt: v[1],
                    left_pt: v[2],
                    right_pt: v[3],
                    gutter_pt: v.get(4).copied(),
                    mirrored: None,
                });
            }
            true
        }
        _ => false,
    }
}

fn orientation(p: &mut WriterPane, item: &str) -> bool {
    let landscape = match item {
        "" => return true,
        "portrait" => false,
        "landscape" => true,
        _ => return false,
    };
    let sections = p.layout_sections();
    p.layout_apply(LayoutOp::Orientation {
        sections,
        landscape,
    });
    true
}

fn size(p: &mut WriterPane, item: &str, typed: Option<&str>) -> bool {
    let sections = p.layout_sections();
    if let Some(s) = layout_ops::paper_sizes().iter().find(|s| s.id == item) {
        p.layout_apply(LayoutOp::PageSize {
            sections,
            width_pt: s.width,
            height_pt: s.height,
            code: Some(s.code),
        });
        return true;
    }
    match item {
        "" => true,
        // More Paper Sizes: `width,height`.
        "custom" => {
            let v = typed.map(|t| numbers(t, INCH)).unwrap_or_default();
            if v.len() < 2 || v.iter().take(2).any(|x| *x < 1.0) {
                note(
                    p,
                    "Paper Size: type width and height, for example 8.5\",11\"",
                );
            } else {
                p.layout_apply(LayoutOp::PageSize {
                    sections,
                    width_pt: v[0],
                    height_pt: v[1],
                    code: None,
                });
            }
            true
        }
        _ => false,
    }
}

fn columns(p: &mut WriterPane, item: &str, typed: Option<&str>) -> bool {
    let sections = p.layout_sections();
    let width = p.text_width_pt();
    if let Some((count, space_pt, widths_pt)) = layout_ops::column_preset(item, width) {
        p.layout_apply(LayoutOp::Columns {
            sections,
            count,
            space_pt,
            separator: Some(false),
            widths_pt,
        });
        return true;
    }
    match item {
        "" => true,
        // More Columns: `count[,spacing][,line]`.
        "more" => {
            let parts: Vec<&str> = typed
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .collect();
            let count = parts.first().and_then(|c| c.parse::<u32>().ok());
            match count {
                Some(count) if (1..=45).contains(&count) => {
                    let space_pt = parts
                        .get(1)
                        .and_then(|s| parse_len(s, INCH))
                        .unwrap_or(36.0);
                    let separator = parts.get(2).map(|s| matches!(*s, "1" | "line" | "true"));
                    p.layout_apply(LayoutOp::Columns {
                        sections,
                        count,
                        space_pt,
                        separator,
                        widths_pt: None,
                    });
                }
                _ => note(p, "Columns: type a number of columns from 1 to 45"),
            }
            true
        }
        _ => false,
    }
}

fn breaks(p: &mut WriterPane, item: &str) -> bool {
    match item {
        "" => {}
        "page" => {
            p.layout_break(
                |pos| LayoutOp::Break {
                    pos,
                    kind: BreakKind::Page,
                },
                true,
            );
        }
        "column" => {
            p.layout_break(
                |pos| LayoutOp::Break {
                    pos,
                    kind: BreakKind::Column,
                },
                true,
            );
        }
        "text-wrapping" => {
            p.layout_break(
                |pos| LayoutOp::Break {
                    pos,
                    kind: BreakKind::TextWrapping,
                },
                false,
            );
        }
        "next-page" | "continuous" | "even-page" | "odd-page" => {
            let start = match item {
                "next-page" => SectionStart::NextPage,
                "continuous" => SectionStart::Continuous,
                "even-page" => SectionStart::EvenPage,
                _ => SectionStart::OddPage,
            };
            p.layout_break(|pos| LayoutOp::SectionBreak { pos, start }, true);
        }
        _ => return false,
    }
    true
}

fn line_numbers(p: &mut WriterPane, item: &str, typed: Option<&str>) -> bool {
    let sections = p.layout_sections();
    let numbering = |restart| {
        Some(LineNumbering {
            restart,
            ..LineNumbering::default()
        })
    };
    match item {
        "" => {}
        "none" => {
            p.layout_apply(LayoutOp::LineNumbers {
                sections,
                numbering: None,
            });
        }
        "continuous" => {
            p.layout_apply(LayoutOp::LineNumbers {
                sections,
                numbering: numbering(Restart::Continuous),
            });
        }
        "restart-page" => {
            p.layout_apply(LayoutOp::LineNumbers {
                sections,
                numbering: numbering(Restart::EachPage),
            });
        }
        "restart-section" => {
            p.layout_apply(LayoutOp::LineNumbers {
                sections,
                numbering: numbering(Restart::EachSection),
            });
        }
        "suppress" => {
            // Toggles like the menu item: off when every selected paragraph is suppressed.
            let paras = p.layout_paras();
            let on = !p.all_suppressed(&paras);
            p.layout_apply(LayoutOp::SuppressLineNumbers { paras, on });
        }
        // Line Numbering Options: `count by, start at, from text, restart (c, p or s)`.
        "options" => {
            let parts: Vec<&str> = typed
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .collect();
            let count_by = parts.first().and_then(|s| s.parse().ok()).unwrap_or(1);
            let start_at = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);
            let from_text_pt = parts.get(2).and_then(|s| parse_len(s, 1.0));
            let restart = match parts.get(3).copied().unwrap_or("c") {
                "p" | "page" => Restart::EachPage,
                "s" | "section" => Restart::EachSection,
                _ => Restart::Continuous,
            };
            p.layout_apply(LayoutOp::LineNumbers {
                sections,
                numbering: Some(LineNumbering {
                    count_by,
                    start_at,
                    from_text_pt,
                    restart,
                }),
            });
        }
        _ => return false,
    }
    true
}

fn hyphenation(p: &mut WriterPane, item: &str, typed: Option<&str>) -> bool {
    match item {
        "" => {}
        "none" => {
            p.layout_apply(LayoutOp::Hyphenation {
                auto: false,
                zone_pt: None,
                limit: None,
                hyphenate_caps: None,
            });
        }
        "automatic" => {
            p.layout_apply(LayoutOp::Hyphenation {
                auto: true,
                zone_pt: None,
                limit: None,
                hyphenate_caps: None,
            });
        }
        "manual" => note(
            p,
            "Manual hyphenation needs the word-by-word prompt, which is not available yet",
        ),
        // Hyphenation Options: `zone, limit, caps (1 or 0)`; the document keeps automatic on.
        "options" => {
            let parts: Vec<&str> = typed
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .collect();
            let zone_pt = parts.first().and_then(|s| parse_len(s, INCH));
            let limit = parts.get(1).and_then(|s| s.parse().ok());
            let hyphenate_caps = parts.get(2).map(|s| matches!(*s, "1" | "true" | "yes"));
            let auto = p.hyphenation_on();
            p.layout_apply(LayoutOp::Hyphenation {
                auto,
                zone_pt,
                limit,
                hyphenate_caps,
            });
        }
        _ => return false,
    }
    true
}

fn indent(p: &mut WriterPane, left: bool, typed: Option<&str>) -> bool {
    let Some(text) = typed else { return true };
    match parse_len(text, INCH) {
        Some(v) if v >= 0.0 || left => {
            let paras = p.layout_paras();
            let (left_pt, right_pt) = if left {
                (Some(v), None)
            } else {
                (None, Some(v))
            };
            p.layout_apply(LayoutOp::Indent {
                paras,
                left_pt,
                right_pt,
            });
        }
        _ => note(p, "Indent: type a length such as 0.5\" or 1.27 cm"),
    }
    true
}

fn spacing(p: &mut WriterPane, before: bool, typed: Option<&str>) -> bool {
    let Some(text) = typed else { return true };
    match parse_len(text, 1.0) {
        Some(v) if (0.0..=1584.0).contains(&v) => {
            let paras = p.layout_paras();
            let (before_pt, after_pt) = if before {
                (Some(v), None)
            } else {
                (None, Some(v))
            };
            p.layout_apply(LayoutOp::Spacing {
                paras,
                before_pt,
                after_pt,
            });
        }
        _ => note(p, "Spacing: type a length from 0 pt to 1584 pt"),
    }
    true
}

fn item(control: &str, id: &str, label: &str) -> MenuItem {
    MenuItem::new(&format!("{control}.{id}"), label)
}

/// The drop-down menu of Layout control `control`, `None` for a control without one.
#[must_use]
pub fn menu_items(control: &str) -> Option<Vec<MenuItem>> {
    let c = control;
    let out = match c {
        "layout.page-set-up.margins" => {
            let mut v: Vec<MenuItem> = layout_ops::margin_presets()
                .iter()
                .map(|m| MenuItem {
                    detail: Some(format!(
                        "Top: {}  Bottom: {}  Left: {}  Right: {}",
                        inches(m.top),
                        inches(m.bottom),
                        inches(m.left),
                        inches(m.right)
                    )),
                    ..item(c, m.id, m.label)
                })
                .collect();
            v.push(MenuItem::separator());
            v.push(item(c, "custom", "Custom Margins..."));
            v
        }
        "layout.page-set-up.orientation" => {
            vec![
                item(c, "portrait", "Portrait"),
                item(c, "landscape", "Landscape"),
            ]
        }
        "layout.page-set-up.size" => {
            let mut v: Vec<MenuItem> = layout_ops::paper_sizes()
                .iter()
                .map(|s| MenuItem {
                    detail: Some(format!("{} x {}", inches(s.width), inches(s.height))),
                    ..item(c, s.id, s.label)
                })
                .collect();
            v.push(MenuItem::separator());
            v.push(item(c, "custom", "More Paper Sizes..."));
            v
        }
        "layout.page-set-up.columns" => vec![
            item(c, "one", "One"),
            item(c, "two", "Two"),
            item(c, "three", "Three"),
            item(c, "left", "Left"),
            item(c, "right", "Right"),
            MenuItem::separator(),
            item(c, "more", "More Columns..."),
        ],
        "layout.page-set-up.breaks" => vec![
            MenuItem::heading("Page Breaks"),
            item(c, "page", "Page"),
            item(c, "column", "Column"),
            item(c, "text-wrapping", "Text Wrapping"),
            MenuItem::heading("Section Breaks"),
            item(c, "next-page", "Next Page"),
            item(c, "continuous", "Continuous"),
            item(c, "even-page", "Even Page"),
            item(c, "odd-page", "Odd Page"),
        ],
        "layout.page-set-up.line-numbers" => vec![
            item(c, "none", "None"),
            item(c, "continuous", "Continuous"),
            item(c, "restart-page", "Restart Each Page"),
            item(c, "restart-section", "Restart Each Section"),
            item(c, "suppress", "Suppress for Current Paragraph"),
            MenuItem::separator(),
            item(c, "options", "Line Numbering Options..."),
        ],
        "layout.page-set-up.hyphenation" => vec![
            item(c, "none", "None"),
            item(c, "automatic", "Automatic"),
            item(c, "manual", "Manual"),
            MenuItem::separator(),
            item(c, "options", "Hyphenation Options..."),
        ],
        _ => return None,
    };
    Some(out)
}

fn inches(pt: f64) -> String {
    format!("{:.2}\"", pt / 72.0)
}
