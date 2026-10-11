//! Writer and Sheets share one chrome: every colour and size comes from the same theme and the
//! same measured geometry; only the accent colour (and the Share button fill derived from it)
//! depends on the application.

use open365_ui_shell::spec::AppSpec;
use open365_ui_shell::{AppKind, Mode, Shell, Theme};

/// `name: value` lines of a theme, one per field (the derived `Debug` output).
fn fields(t: &Theme) -> Vec<(String, String)> {
    format!("{t:#?}")
        .lines()
        .filter_map(|l| l.trim().strip_suffix(',')?.split_once(": "))
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
}

fn differing(a: &Theme, b: &Theme) -> Vec<String> {
    let (fa, fb) = (fields(a), fields(b));
    assert_eq!(fa.len(), fb.len());
    assert!(
        fa.len() > 30,
        "the whole theme is compared ({} fields)",
        fa.len()
    );
    fa.iter()
        .zip(&fb)
        .filter(|(x, y)| x != y)
        .map(|(x, _)| x.0.clone())
        .collect()
}

#[test]
fn only_the_accent_differs_between_the_apps() {
    for mode in [Mode::Light, Mode::Dark] {
        let w = Theme::new(AppKind::Writer, mode);
        let s = Theme::new(AppKind::Sheets, mode);
        assert_eq!(differing(&w, &s), ["accent", "share"], "{mode:?}");
        // The shells themselves hold those themes, nothing app-specific on top.
        let sw = Shell::new(AppKind::Writer, mode);
        let ss = Shell::new(AppKind::Sheets, mode);
        assert_eq!(
            differing(&sw.theme, &ss.theme),
            ["accent", "share"],
            "{mode:?}"
        );
    }
}

#[test]
fn light_and_dark_differ_in_every_surface() {
    // Guards the comparison above: a theme field that never changes with the mode would hide a
    // hard-coded colour.
    let light = Theme::new(AppKind::Writer, Mode::Light);
    let dark = Theme::new(AppKind::Writer, Mode::Dark);
    let same: Vec<_> = fields(&light)
        .into_iter()
        .zip(fields(&dark))
        .filter(|(l, d)| l.1 == d.1)
        .map(|(l, _)| l.0)
        .collect();
    // Colours Office keeps in both modes (white cells, grid lines, the close button, ...).
    for name in &same {
        assert!(
            [
                "cell_bg",
                "cell_text",
                "grid_line",
                "grid_accent",
                "tab_active",
                "tab_active_text",
                "close_hover",
            ]
            .contains(&name.as_str()),
            "{name} is the same in light and dark"
        );
    }
}

#[test]
fn chrome_sizes_are_the_same_in_both_apps() {
    let (w, s) = (AppSpec::writer(), AppSpec::sheets());
    assert_eq!(w.ref_window, s.ref_window);
    assert_eq!(w.pane[0], s.pane[0]);
    assert_eq!(w.pane[2..], s.pane[2..]);
    let rect = |r: &open365_ui_shell::spec::Rect4| (r.x, r.y, r.w, r.h);
    assert_eq!(rect(&w.tab_strip), rect(&s.tab_strip));
    assert_eq!(
        (w.lower.x, w.lower.y, w.lower.w, w.lower.h),
        (s.lower.x, s.lower.y, s.lower.w, s.lower.h)
    );
    assert_eq!(
        (w.groups_x, w.group_y, w.group_h),
        (s.groups_x, s.group_y, s.group_h)
    );
    assert_eq!(
        (w.status.y_from_bottom, w.status.h, w.status.x, w.status.w),
        (s.status.y_from_bottom, s.status.h, s.status.x, s.status.w)
    );
    let size = |c: &open365_ui_shell::spec::Chrome| (c.x, c.y, c.w, c.h, c.right);
    assert_eq!(size(&w.file_tab), size(&s.file_tab));
    assert_eq!(size(&w.search), size(&s.search));
    // Chrome both apps have: tab names, the account button, window buttons and the Share button
    // measure the same.
    let same = |a: &[open365_ui_shell::spec::Chrome],
                b: &[open365_ui_shell::spec::Chrome],
                ids: &[&str]| {
        for id in ids {
            let (x, y) = (
                a.iter().find(|c| c.id == *id).expect("in writer"),
                b.iter().find(|c| c.id == *id).expect("in sheets"),
            );
            assert_eq!(size(x), size(y), "{id}");
        }
    };
    same(
        &w.title,
        &s.title,
        &["mecontrolwidget", "caption.minimise", "caption.close"],
    );
    // Writer has an extra Editing button, so Comments sits further left there: compare the
    // buttons by size, and the two anchored at the right edge by position too.
    same(
        &w.tab_row_right,
        &s.tab_row_right,
        &["share", "ribbon-display-options"],
    );
    let comments = |a: &AppSpec| {
        let c = a
            .tab_row_right
            .iter()
            .find(|c| c.id == "comments")
            .expect("comments");
        (c.w, c.h)
    };
    assert_eq!(comments(&w), comments(&s));
    // Status items: the same size in both apps (Sheets' size grip shifts the zoom controls).
    let status = |a: &AppSpec, id: &str| {
        let c = a.status.items.iter().find(|c| c.id == id).expect("item");
        (c.w, c.h)
    };
    for id in [
        "status.accessibility",
        "status.zoom-out",
        "status.zoom",
        "status.zoom-in",
        "status.zoom-percent",
    ] {
        assert_eq!(status(&w, id), status(&s, id), "{id}");
    }
    for id in ["home", "insert", "review", "view", "help"] {
        let (x, y) = (w.tab(id).expect("tab"), s.tab(id).expect("tab"));
        assert_eq!(x.w, y.w, "tab {id}");
        assert_eq!(x.label, y.label);
    }
}
