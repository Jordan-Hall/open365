//! The proof surface: the Excel Home ribbon tab row and the File button, built from the same
//! spec JSON as `ui-shell` (`crates/ui-shell/spec`). Lengths in the specs are physical pixels at
//! the reference scale (1.25); they become logical units here and snap per scale.

use serde::Deserialize;

use crate::geom::Rect;
use crate::input::Key;
use crate::tree::{Node, NodeId, TabItem, Widget};
use crate::ui::Ui;

const SHEETS: &str = include_str!("../../ui-shell/spec/sheets.json");
const WRITER: &str = include_str!("../../ui-shell/spec/writer.json");
const SHEETS_RULES: &str = include_str!("../../ui-shell/spec/chrome-sheets.json");
const WRITER_RULES: &str = include_str!("../../ui-shell/spec/chrome-writer.json");

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct R4 {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TabSpec {
    pub id: String,
    pub label: String,
    pub w: f32,
    #[serde(default)]
    pub key: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FileSpec {
    pub label: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, Clone, Deserialize)]
struct TabRules {
    gap: f32,
}

#[derive(Debug, Clone, Deserialize)]
struct Rules {
    tabs: TabRules,
}

/// The subset of the ribbon spec the tab row needs.
#[derive(Debug, Clone, Deserialize)]
pub struct RowSpec {
    pub ref_scale: f32,
    pub ref_window: [f32; 2],
    /// Ribbon pane in window pixels at the reference scale: x, y, w, h.
    pub pane: [f32; 4],
    pub tab_strip: R4,
    pub file_tab: FileSpec,
    pub tabs: Vec<TabSpec>,
    #[serde(skip)]
    pub gap: f32,
}

impl RowSpec {
    fn parse(spec: &str, rules: &str) -> Self {
        let mut s: RowSpec = serde_json::from_str(spec).expect("embedded spec parses");
        let r: Rules = serde_json::from_str(rules).expect("embedded chrome rules parse");
        s.gap = r.tabs.gap;
        s
    }

    pub fn sheets() -> Self {
        Self::parse(SHEETS, SHEETS_RULES)
    }

    pub fn writer() -> Self {
        Self::parse(WRITER, WRITER_RULES)
    }

    /// Reference length `u` to logical units.
    pub fn logical(&self, u: f32) -> f32 {
        u / self.ref_scale
    }

    /// The window the spec was measured in, in logical units.
    pub fn window(&self) -> Rect {
        Rect::new(
            0.0,
            0.0,
            self.logical(self.ref_window[0]),
            self.logical(self.ref_window[1]),
        )
    }
}

/// Handles to the widgets of the row.
#[derive(Debug, Clone, Copy)]
pub struct TabRow {
    pub file: NodeId,
    pub tabs: NodeId,
}

/// Adds the File button and the tab strip to the root of `ui`.
pub fn build_tab_row(ui: &mut Ui, spec: &RowSpec) -> TabRow {
    let [px, py, _, _] = spec.pane;
    let l = |u: f32| spec.logical(u);
    let f = &spec.file_tab;
    let mut file = Node::new(
        Widget::Button { toggled: None },
        Rect::new(l(px + f.x), l(py + f.y), l(f.w), l(f.h)),
    )
    .with_label(&f.label);
    file.key_tip = Some("F".into());
    let root = ui.tree.root();
    let file = ui.add(root, file);
    let s = spec.tab_strip;
    let tabs: Vec<TabItem> = spec
        .tabs
        .iter()
        .map(|t| {
            let mut i = TabItem::new(&t.label, l(t.w));
            i.key_tip = t.key.clone();
            i
        })
        .collect();
    let total: f32 = tabs.iter().map(|t| t.width).sum::<f32>()
        + l(spec.gap) * tabs.len().saturating_sub(1) as f32;
    let mut strip = Node::new(
        Widget::TabStrip {
            tabs,
            selected: 0,
            gap: l(spec.gap),
        },
        Rect::new(l(px + s.x), l(py + s.y), total, l(s.h)),
    )
    .with_label("Ribbon tabs");
    strip.layout = crate::tree::Layout::Absolute;
    let strip = ui.add(root, strip);
    TabRow { file, tabs: strip }
}

/// Key used by the proof to open the File menu: Alt then F.
pub const FILE_TIP: Key = Key::Char('F');

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specs_parse_and_carry_the_ten_excel_tabs() {
        let s = RowSpec::sheets();
        assert_eq!(s.tabs.len(), 10);
        assert_eq!(s.tabs[0].label, "Home");
        assert_eq!(s.gap, 1.0);
        assert_eq!(s.ref_scale, 1.25);
        assert_eq!(
            RowSpec::writer().tabs.first().map(|t| t.label.as_str()),
            Some("Home")
        );
    }
}
