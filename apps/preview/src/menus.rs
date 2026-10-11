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
    let tail = id.rsplit('.').next().unwrap_or(id);
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
