//! Icon registry: the Tabler SVGs `ui-shell` already vendors (`icon_table`), plus a few thin
//! glyphs of our own (window controls, back chevron, dialog launcher) that Tabler's 2 px strokes
//! cannot match at 10 px.

use std::collections::HashMap;

use open365_gfx::{Assets, IconId};

use crate::icon_table::ICONS;

fn thin(body: &str) -> String {
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"10\" height=\"10\" viewBox=\"0 0 10 10\" \
         fill=\"none\" stroke=\"currentColor\" stroke-width=\"1\">{body}</svg>"
    )
}

/// Name and SVG of the glyphs drawn here.
pub fn custom() -> Vec<(&'static str, String)> {
    vec![
        ("cap-min", thin("<path d=\"M0 5.5H10\"/>")),
        (
            "cap-max",
            thin("<rect x=\"0.5\" y=\"0.5\" width=\"9\" height=\"9\"/>"),
        ),
        (
            "cap-restore",
            thin(
                "<path d=\"M2.5 2.5V0.5H9.5V7.5H7.5\"/><rect x=\"0.5\" y=\"2.5\" width=\"7\" height=\"7\"/>",
            ),
        ),
        ("cap-close", thin("<path d=\"M0 0L10 10M10 0L0 10\"/>")),
        ("chevron-left", thin("<path d=\"M6.5 1.5L3 5L6.5 8.5\"/>")),
        (
            "chevron-right-thin",
            thin("<path d=\"M3.5 1.5L7 5L3.5 8.5\"/>"),
        ),
        (
            "launcher",
            thin("<path d=\"M2 5V1.5H8.5V8H5\"/><path d=\"M1 9L6.5 3.5M3.5 3.5H6.5V6.5\"/>"),
        ),
    ]
}

/// Registers every icon on `assets` and returns the ids by name.
pub fn register(assets: &mut Assets) -> HashMap<String, IconId> {
    let mut out = HashMap::new();
    for (name, svg) in ICONS {
        if let Ok(id) = assets.add_icon(svg.as_bytes()) {
            out.insert((*name).to_owned(), id);
        }
    }
    for (name, svg) in custom() {
        if let Ok(id) = assets.add_icon(svg.as_bytes()) {
            out.insert(name.to_owned(), id);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_the_specs_name_is_available() {
        let mut assets = Assets::new(256);
        let icons = register(&mut assets);
        for spec in [
            crate::spec::AppSpec::sheets(),
            crate::spec::AppSpec::writer(),
        ] {
            let mut names: Vec<String> = Vec::new();
            for t in &spec.tabs {
                for g in &t.groups {
                    names.extend(g.icon.clone());
                    names.extend(g.controls().filter_map(|c| c.icon.clone()));
                }
            }
            for c in spec
                .qat
                .items
                .iter()
                .chain(&spec.title)
                .chain(&spec.tab_row_right)
                .chain(&spec.status.items)
            {
                names.extend(c.icon.clone());
            }
            for n in names {
                assert!(icons.contains_key(&n), "icon {n} is not registered");
            }
        }
        for (tail, n) in crate::shell::MENU_ICONS {
            assert!(
                icons.contains_key(*n),
                "menu icon {n} (for {tail}) is not registered"
            );
        }
        for n in [
            "cap-close",
            "chevron-left",
            "launcher",
            "user-circle",
            "search",
        ] {
            assert!(icons.contains_key(n), "{n}");
        }
    }
}
