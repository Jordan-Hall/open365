//! Insert > Icons: a curated set of the bundled Tabler outline icons (MIT, `assets/icons/tabler`).
//! Any icon in the folder could be added; the list keeps the binary small. An icon is inserted as an
//! SVG picture one inch square (96 CSS pixels), the size Office's own icon set uses.

/// `(name, svg)` pairs, sorted by name.
pub const ICONS: &[(&str, &str)] = &[
    (
        "alert-circle",
        include_str!("../../../../assets/icons/tabler/alert-circle.svg"),
    ),
    (
        "alert-triangle",
        include_str!("../../../../assets/icons/tabler/alert-triangle.svg"),
    ),
    (
        "arrow-back-up",
        include_str!("../../../../assets/icons/tabler/arrow-back-up.svg"),
    ),
    (
        "arrow-down",
        include_str!("../../../../assets/icons/tabler/arrow-down.svg"),
    ),
    (
        "arrow-forward-up",
        include_str!("../../../../assets/icons/tabler/arrow-forward-up.svg"),
    ),
    (
        "arrow-left",
        include_str!("../../../../assets/icons/tabler/arrow-left.svg"),
    ),
    (
        "bolt",
        include_str!("../../../../assets/icons/tabler/bolt.svg"),
    ),
    (
        "book",
        include_str!("../../../../assets/icons/tabler/book.svg"),
    ),
    (
        "bookmark",
        include_str!("../../../../assets/icons/tabler/bookmark.svg"),
    ),
    (
        "calculator",
        include_str!("../../../../assets/icons/tabler/calculator.svg"),
    ),
    (
        "calendar-time",
        include_str!("../../../../assets/icons/tabler/calendar-time.svg"),
    ),
    (
        "chart-bar",
        include_str!("../../../../assets/icons/tabler/chart-bar.svg"),
    ),
    (
        "chart-line",
        include_str!("../../../../assets/icons/tabler/chart-line.svg"),
    ),
    (
        "chart-pie",
        include_str!("../../../../assets/icons/tabler/chart-pie.svg"),
    ),
    (
        "check",
        include_str!("../../../../assets/icons/tabler/check.svg"),
    ),
    (
        "checklist",
        include_str!("../../../../assets/icons/tabler/checklist.svg"),
    ),
    (
        "clipboard",
        include_str!("../../../../assets/icons/tabler/clipboard.svg"),
    ),
    (
        "cube",
        include_str!("../../../../assets/icons/tabler/cube.svg"),
    ),
    (
        "currency-dollar",
        include_str!("../../../../assets/icons/tabler/currency-dollar.svg"),
    ),
    (
        "currency-pound",
        include_str!("../../../../assets/icons/tabler/currency-pound.svg"),
    ),
    (
        "database",
        include_str!("../../../../assets/icons/tabler/database.svg"),
    ),
    (
        "droplet",
        include_str!("../../../../assets/icons/tabler/droplet.svg"),
    ),
    (
        "eye",
        include_str!("../../../../assets/icons/tabler/eye.svg"),
    ),
    (
        "file-text",
        include_str!("../../../../assets/icons/tabler/file-text.svg"),
    ),
    (
        "filter",
        include_str!("../../../../assets/icons/tabler/filter.svg"),
    ),
    (
        "flask",
        include_str!("../../../../assets/icons/tabler/flask.svg"),
    ),
    (
        "folder-open",
        include_str!("../../../../assets/icons/tabler/folder-open.svg"),
    ),
    (
        "headphones",
        include_str!("../../../../assets/icons/tabler/headphones.svg"),
    ),
    (
        "help",
        include_str!("../../../../assets/icons/tabler/help.svg"),
    ),
    (
        "home",
        include_str!("../../../../assets/icons/tabler/home.svg"),
    ),
    (
        "info-circle",
        include_str!("../../../../assets/icons/tabler/info-circle.svg"),
    ),
    (
        "link",
        include_str!("../../../../assets/icons/tabler/link.svg"),
    ),
    (
        "lock",
        include_str!("../../../../assets/icons/tabler/lock.svg"),
    ),
    (
        "mail",
        include_str!("../../../../assets/icons/tabler/mail.svg"),
    ),
    (
        "map",
        include_str!("../../../../assets/icons/tabler/map.svg"),
    ),
    (
        "message",
        include_str!("../../../../assets/icons/tabler/message.svg"),
    ),
    (
        "microphone",
        include_str!("../../../../assets/icons/tabler/microphone.svg"),
    ),
    (
        "note",
        include_str!("../../../../assets/icons/tabler/note.svg"),
    ),
    (
        "palette",
        include_str!("../../../../assets/icons/tabler/palette.svg"),
    ),
    (
        "pencil",
        include_str!("../../../../assets/icons/tabler/pencil.svg"),
    ),
    (
        "photo",
        include_str!("../../../../assets/icons/tabler/photo.svg"),
    ),
    (
        "pin",
        include_str!("../../../../assets/icons/tabler/pin.svg"),
    ),
    (
        "plug-connected",
        include_str!("../../../../assets/icons/tabler/plug-connected.svg"),
    ),
    (
        "printer",
        include_str!("../../../../assets/icons/tabler/printer.svg"),
    ),
    (
        "puzzle",
        include_str!("../../../../assets/icons/tabler/puzzle.svg"),
    ),
    (
        "school",
        include_str!("../../../../assets/icons/tabler/school.svg"),
    ),
    (
        "search",
        include_str!("../../../../assets/icons/tabler/search.svg"),
    ),
    (
        "settings",
        include_str!("../../../../assets/icons/tabler/settings.svg"),
    ),
    (
        "share",
        include_str!("../../../../assets/icons/tabler/share.svg"),
    ),
    (
        "sparkles",
        include_str!("../../../../assets/icons/tabler/sparkles.svg"),
    ),
    (
        "tag",
        include_str!("../../../../assets/icons/tabler/tag.svg"),
    ),
    (
        "trash",
        include_str!("../../../../assets/icons/tabler/trash.svg"),
    ),
    (
        "users",
        include_str!("../../../../assets/icons/tabler/users.svg"),
    ),
    (
        "world",
        include_str!("../../../../assets/icons/tabler/world.svg"),
    ),
];

/// The SVG of icon `name`, scaled to 96 x 96 pixels (one inch), ready to insert as a picture.
#[must_use]
pub fn icon_svg(name: &str) -> Option<Vec<u8>> {
    let (_, svg) = ICONS.iter().find(|(n, _)| *n == name)?;
    Some(
        svg.replacen("width=\"24\"", "width=\"96\"", 1)
            .replacen("height=\"24\"", "height=\"96\"", 1)
            .into_bytes(),
    )
}

/// The label shown for an icon: the name with spaces and a capital.
#[must_use]
pub fn icon_label(name: &str) -> String {
    let spaced = name.replace('-', " ");
    let mut chars = spaced.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_are_sorted_and_scale_to_an_inch() {
        assert!(ICONS.windows(2).all(|w| w[0].0 < w[1].0));
        let svg = String::from_utf8(icon_svg("home").expect("home")).expect("utf-8");
        assert!(
            svg.contains("width=\"96\"") && svg.contains("height=\"96\""),
            "{svg}"
        );
        assert!(icon_svg("no-such-icon").is_none());
        assert_eq!(icon_label("info-circle"), "Info circle");
    }
}
