//! Demonstration content for the ribbon's drop-down menus and lists. The chrome specs carry
//! geometry only; the real menus come from the command layer once the preview is wired to it.

use open365_ui_core::MenuItem;

fn item(id: &str, label: &str, access: char) -> MenuItem {
    MenuItem {
        access: Some(access),
        ..MenuItem::new(id, label)
    }
}

/// The menu that opens for ribbon control `id` (label `label`).
pub fn items_for(id: &str, label: &str) -> Vec<MenuItem> {
    if let Some(items) = crate::ribbon::writer::layout::menu_items(id)
        .or_else(|| crate::ribbon::writer::design::menu_items(id))
        .or_else(|| crate::ribbon::sheets::insert::menu_items(id))
        .or_else(|| crate::ribbon::writer::mailings::menu_items(id))
    {
        return items;
    }
    let tail = id.rsplit('.').next().unwrap_or(id);
    if let Some(items) = formulas_menu(id, tail)
        .or_else(|| sheets_menu(id))
        .or_else(|| writer_menu(id))
    {
        return items;
    }
    match tail {
        "paste" => vec![
            MenuItem::heading("Paste Options:"),
            item(id, "Paste", 'P'),
            item(&format!("{id}.values"), "Paste Values", 'V'),
            item(&format!("{id}.formatting"), "Paste Formatting", 'F'),
            item(&format!("{id}.link"), "Paste Link", 'L'),
            MenuItem::separator(),
            item(&format!("{id}.special"), "Paste Special...", 'S'),
        ],
        "undo" | "redo" => vec![MenuItem {
            enabled: false,
            ..MenuItem::new(&format!("{id}.none"), "Nothing to undo")
        }],
        "share" => vec![
            item("share.invite", "Share with people...", 'S'),
            item("share.link", "Copy link", 'C'),
            item("share.send", "Send a copy...", 'E'),
        ],
        "fill-colour" | "font-colour" | "highlight" => vec![
            item(&format!("{id}.auto"), "Automatic", 'A'),
            MenuItem::heading("Theme Colours"),
            item(&format!("{id}.theme"), "Theme Colours", 'T'),
            MenuItem::heading("Standard Colours"),
            item(&format!("{id}.standard"), "Standard Colours", 'S'),
            MenuItem::separator(),
            item(&format!("{id}.more"), "More Colours...", 'M'),
        ],
        "sort-filter" | "sort-and-filter" => vec![
            item(&format!("{id}.az"), "Sort A to Z", 'S'),
            item(&format!("{id}.za"), "Sort Z to A", 'O'),
            MenuItem::separator(),
            item(&format!("{id}.custom"), "Custom Sort...", 'U'),
            item(&format!("{id}.filter"), "Filter", 'F'),
        ],
        _ => vec![
            item(&format!("{id}.run"), label, 'R'),
            MenuItem::new(&format!("{id}.more-options"), "Options").with_submenu(vec![
                MenuItem::new(&format!("{id}.opt1"), "First option"),
                MenuItem::new(&format!("{id}.opt2"), "Second option"),
            ]),
            MenuItem::separator(),
            item(&format!("{id}.commands"), "More Commands...", 'M'),
        ],
    }
}

/// The Formulas tab's drop-downs: AutoSum and the function category lists.
fn formulas_menu(id: &str, tail: &str) -> Option<Vec<MenuItem>> {
    if !id.starts_with("formulas.function-library.") {
        return None;
    }
    if tail == "auto-sum" {
        return Some(vec![
            item(&format!("{id}.sum"), "Sum", 'S'),
            item(&format!("{id}.average"), "Average", 'A'),
            item(&format!("{id}.count-numbers"), "Count Numbers", 'C'),
            item(&format!("{id}.max"), "Max", 'M'),
            item(&format!("{id}.min"), "Min", 'I'),
            MenuItem::separator(),
            item(&format!("{id}.more"), "More Functions...", 'F'),
        ]);
    }
    let category = crate::dialogs::formulas_function::Category::from_control(tail)?;
    let mut out: Vec<MenuItem> = crate::dialogs::formulas_function::functions_in(category)
        .into_iter()
        .map(|f| MenuItem::new(&format!("{id}.{f}"), f))
        .collect();
    out.push(MenuItem::separator());
    out.push(item(&format!("{id}.run"), "Insert Function...", 'F'));
    Some(out)
}

/// The Sheets Page Layout and View drop-downs (ids `<control>.<item>`).
fn sheets_menu(id: &str) -> Option<Vec<MenuItem>> {
    let list = |entries: &[(&str, &str)]| -> Vec<MenuItem> {
        entries
            .iter()
            .map(|(k, l)| MenuItem::new(&format!("{id}.{k}"), l))
            .collect()
    };
    Some(match id {
        "page-layout.page-set-up.margins" => {
            list(&[("normal", "Normal"), ("wide", "Wide"), ("narrow", "Narrow")])
        }
        "page-layout.page-set-up.orientation" => {
            list(&[("portrait", "Portrait"), ("landscape", "Landscape")])
        }
        "page-layout.page-set-up.size" => list(&[
            ("letter", "Letter"),
            ("tabloid", "Tabloid"),
            ("legal", "Legal"),
            ("statement", "Statement"),
            ("executive", "Executive"),
            ("a3", "A3"),
            ("a4", "A4"),
            ("a5", "A5"),
            ("b4", "B4 (JIS)"),
            ("b5", "B5 (JIS)"),
        ]),
        "page-layout.page-set-up.print-area" => list(&[
            ("set", "Set Print Area"),
            ("add", "Add to Print Area"),
            ("clear", "Clear Print Area"),
        ]),
        "page-layout.page-set-up.breaks" => list(&[
            ("insert", "Insert Page Break"),
            ("remove", "Remove Page Break"),
            ("reset", "Reset All Page Breaks"),
        ]),
        "view.window.freeze-panes" => list(&[
            ("freeze-panes", "Freeze Panes"),
            ("freeze-top-row", "Freeze Top Row"),
            ("freeze-first-column", "Freeze First Column"),
            ("unfreeze", "Unfreeze Panes"),
        ]),
        _ => return None,
    })
}

/// The Writer Home and Insert drop-downs; the choice ids are those of `ribbon/writer/home.rs` and `insert.rs`.
fn writer_menu(id: &str) -> Option<Vec<MenuItem>> {
    let list = |entries: &[(&str, &str)]| -> Vec<MenuItem> {
        entries
            .iter()
            .map(|(k, l)| MenuItem::new(&format!("{id}.{k}"), l))
            .collect()
    };
    let colours = || {
        list(&[
            ("auto", "Automatic"),
            ("000000", "Black"),
            ("FF0000", "Red"),
            ("FFC000", "Orange"),
            ("FFFF00", "Yellow"),
            ("00B050", "Green"),
            ("0070C0", "Blue"),
            ("7030A0", "Purple"),
            ("FFFFFF", "White"),
        ])
    };
    Some(match id {
        "home.font.change-case" => list(&[
            ("sentence-case", "Sentence case."),
            ("lowercase", "lowercase"),
            ("uppercase", "UPPERCASE"),
            ("capitalize-each-word", "Capitalize Each Word"),
            ("toggle-case", "tOGGLE cASE"),
        ]),
        "home.font.underline" => list(&[
            ("single", "Single"),
            ("double", "Double"),
            ("thick", "Thick"),
            ("dotted", "Dotted"),
            ("dash", "Dashed"),
            ("wave", "Wavy"),
            ("none", "No Underline"),
        ]),
        "home.font.font-colour" | "home.font.text-highlight-colour" | "home.paragraph.shading" => {
            colours()
        }
        "home.paragraph.line-and-paragraph-spacing" => list(&[
            ("1.0", "1.0"),
            ("1.15", "1.15"),
            ("1.5", "1.5"),
            ("2.0", "2.0"),
            ("2.5", "2.5"),
            ("3.0", "3.0"),
            ("add-space-before", "Add Space Before Paragraph"),
            ("remove-space-before", "Remove Space Before Paragraph"),
            ("add-space-after", "Add Space After Paragraph"),
            ("remove-space-after", "Remove Space After Paragraph"),
        ]),
        "home.paragraph.borders" => list(&[
            ("bottom", "Bottom Border"),
            ("top", "Top Border"),
            ("left", "Left Border"),
            ("right", "Right Border"),
            ("none", "No Border"),
            ("all", "All Borders"),
            ("outside", "Outside Borders"),
        ]),
        "home.editing.select" => list(&[("select-all", "Select All")]),
        "insert.tables.table" => list(&[
            ("2x2", "2 x 2 Table"),
            ("3x3", "3 x 3 Table"),
            ("4x4", "4 x 4 Table"),
            ("2x5", "2 x 5 Table"),
        ]),
        "insert.header-footer.header" | "insert.header-footer.footer" => {
            list(&[("blank", "Blank"), ("remove", "Remove")])
        }
        "insert.header-footer.page-number" => list(&[
            ("top-left", "Top of Page: Left"),
            ("top-centre", "Top of Page: Centre"),
            ("top-right", "Top of Page: Right"),
            ("bottom-left", "Bottom of Page: Left"),
            ("bottom-centre", "Bottom of Page: Centre"),
            ("bottom-right", "Bottom of Page: Right"),
        ]),
        "insert.symbols.symbol" => list(&[
            ("U+00A9", "Copyright"),
            ("U+00AE", "Registered"),
            ("U+2122", "Trademark"),
            ("U+00B0", "Degree"),
            ("U+00B1", "Plus-Minus"),
            ("U+20AC", "Euro"),
        ]),
        _ => return None,
    })
}

/// Entries of the list behind a ribbon combo box.
pub fn combo_items(id: &str) -> Vec<String> {
    let tail = id.rsplit('.').next().unwrap_or(id);
    match tail {
        "font" => [
            "Aptos",
            "Arial",
            "Calibri",
            "Cambria",
            "Consolas",
            "Segoe UI",
            "Times New Roman",
        ]
        .map(String::from)
        .to_vec(),
        "font-size" => [
            "8", "9", "10", "11", "12", "14", "16", "18", "20", "24", "28", "36", "48", "72",
        ]
        .map(String::from)
        .to_vec(),
        "number-format" => crate::sheet::sheet_ext::home::NUMBER_FORMAT_NAMES
            .iter()
            .map(|s| (*s).to_owned())
            .collect(),
        "editing" => ["Editing", "Reviewing", "Viewing"]
            .map(String::from)
            .to_vec(),
        _ => vec!["Option 1".into(), "Option 2".into(), "Option 3".into()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_and_unknown_controls_get_a_menu() {
        assert_eq!(items_for("home.clipboard.paste", "Paste").len(), 7);
        let generic = items_for("home.cells.format", "Format");
        assert_eq!(generic.len(), 4);
        assert!(generic[1].submenu.len() == 2);
        assert!(items_for("home.clipboard.paste", "Paste")[0].heading);
        assert!(combo_items("home.font.font").contains(&"Calibri".to_string()));
    }
}
