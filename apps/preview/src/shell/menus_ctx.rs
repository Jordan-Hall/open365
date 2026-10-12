//! The Sheets context menus (right click or Shift+F10) and the AutoFill Options menu. They are
//! `ui-core` context menus: the shared menu painter and keyboard navigation, the items and their
//! order are Excel's, limited to the commands that exist. A pick arrives as
//! `Emit::MenuCommand(_, "ctx.…")`, which `grid_event` routes to [`Shell::ctx_command`].

use open365_ui_core::geom::Point;
use open365_ui_core::{Content, Look, MenuItem, NodeId, Token, Ui, Widget};

use crate::sheet::{self, FillMode, MenuFor, PasteKind, Zone};
use crate::shell::{Act, PxRect, Shell};

/// Side of the AutoFill Options button, in logical units at 125 %.
const AUTOFILL_BTN: i32 = 20;

impl Shell {
    fn ctx_item(
        &self,
        id: &str,
        label: &str,
        access: char,
        icon: Option<&str>,
        enabled: bool,
    ) -> MenuItem {
        MenuItem {
            access: Some(access),
            enabled,
            icon: icon.and_then(|n| self.icon(n)),
            ..MenuItem::new(id, label)
        }
    }

    /// The items of the context menu for `target`, in Excel's order.
    pub fn context_menu_items(&self, target: MenuFor) -> Vec<MenuItem> {
        let copied = self.sheet.has_copied();
        let cut = self.ctx_item("ctx.cut", "Cut", 'T', Some("scissors"), true);
        let copy = self.ctx_item("ctx.copy", "Copy", 'C', Some("copy"), true);
        let mut items = vec![
            cut,
            copy,
            MenuItem::heading("Paste Options:"),
            self.ctx_item("ctx.paste", "Paste", 'P', Some("clipboard"), true),
            self.ctx_item("ctx.paste.formulas", "Formulas", 'F', None, copied),
            self.ctx_item("ctx.paste.transpose", "Transpose", 'T', None, copied),
            self.ctx_item("ctx.paste.values", "Values", 'V', Some("numbers"), copied),
            self.ctx_item(
                "ctx.paste.formats",
                "Formatting",
                'R',
                Some("paint"),
                copied,
            ),
            self.ctx_item("ctx.paste.special", "Paste Special...", 'S', None, copied),
            MenuItem::separator(),
        ];
        let (insert, delete) = match target {
            MenuFor::Cell | MenuFor::AutoFill => ("Insert...", "Delete..."),
            MenuFor::Row(_) | MenuFor::Col(_) => ("Insert", "Delete"),
        };
        items.push(self.ctx_item("ctx.insert", insert, 'I', Some("row-insert-bottom"), true));
        items.push(self.ctx_item("ctx.delete", delete, 'D', Some("trash"), true));
        items.push(self.ctx_item("ctx.clear", "Clear Contents", 'N', Some("eraser"), true));
        items.push(MenuItem::separator());
        items.push(self.ctx_item(
            "ctx.format",
            "Format Cells...",
            'F',
            Some("table-options"),
            true,
        ));
        items
    }

    /// The AutoFill Options menu: how the last fill is done, the current one checked.
    pub fn autofill_menu_items(&self) -> Vec<MenuItem> {
        let mode = self.sheet.autofill_button().map(|(_, m)| m);
        [
            ("ctx.af.copy", "Copy Cells", 'C', FillMode::Copy),
            ("ctx.af.series", "Fill Series", 'S', FillMode::Series),
            (
                "ctx.af.formats",
                "Fill Formatting Only",
                'F',
                FillMode::FormatsOnly,
            ),
            (
                "ctx.af.values",
                "Fill Without Formatting",
                'O',
                FillMode::WithoutFormatting,
            ),
        ]
        .into_iter()
        .map(|(id, label, key, m)| MenuItem {
            checked: mode == Some(m),
            ..self.ctx_item(id, label, key, None, true)
        })
        .collect()
    }

    /// Opens the context menu for `target` at `pos`, a logical window point (what the pointer
    /// reports). The grid's right click, Shift+F10 and tests call this.
    pub fn context_menu(&mut self, ui: &mut Ui, pos: Point, target: MenuFor) {
        let items = match target {
            MenuFor::AutoFill => self.autofill_menu_items(),
            _ => self.context_menu_items(target),
        };
        self.sheet.ui.menu_for = Some(target);
        ui.open_context_menu(items, pos);
    }

    /// A secondary press at device point `(x, y)`: selects what Excel selects (a cell or header
    /// outside the selection), then opens the menu for it.
    pub fn context_menu_at(&mut self, ui: &mut Ui, x: f32, y: f32) {
        let geo = self.grid_geom();
        let range = self.sheet.range();
        let (r1, c1, r2, c2) = range;
        let pos = Point::new(x / self.scale, y / self.scale);
        let target = match geo.zone(x, y, Some(range)) {
            Zone::Cell(r, c) => {
                if !((r1..=r2).contains(&r) && (c1..=c2).contains(&c)) {
                    self.sheet.select_cell((r, c));
                    self.selection_changed(ui);
                }
                MenuFor::Cell
            }
            Zone::FillHandle => MenuFor::Cell,
            Zone::RowHeader(r) | Zone::RowEdge(r) => {
                let whole = c1 == 1 && c2 == sheet::MAX_COL && (r1..=r2).contains(&r);
                if !whole {
                    self.sheet.select_rows(r, r, false);
                    self.selection_changed(ui);
                }
                MenuFor::Row(r)
            }
            Zone::ColHeader(c) | Zone::ColEdge(c) => {
                let whole = r1 == 1 && r2 == sheet::MAX_ROW && (c1..=c2).contains(&c);
                if !whole {
                    self.sheet.select_cols(c, c, false);
                    self.selection_changed(ui);
                }
                MenuFor::Col(c)
            }
            Zone::Corner | Zone::Outside => return,
        };
        self.build_grid(ui);
        self.context_menu(ui, pos, target);
    }

    /// Shift+F10 (the Menu key): the cell menu beside the active cell.
    pub(super) fn context_menu_from_keyboard(&mut self, ui: &mut Ui) {
        let (l, _, _, b) = self
            .grid_geom()
            .cell(self.sheet.active.0, self.sheet.active.1);
        let pos = Point::new(l / self.scale, b / self.scale);
        let target = {
            let (r1, c1, r2, c2) = self.sheet.range();
            if c1 == 1 && c2 == sheet::MAX_COL {
                MenuFor::Row(r1)
            } else if r1 == 1 && r2 == sheet::MAX_ROW {
                MenuFor::Col(c1)
            } else {
                MenuFor::Cell
            }
        };
        self.context_menu(ui, pos, target);
    }

    /// Runs a pick of the context or AutoFill menu.
    pub(super) fn ctx_command(&mut self, ui: &mut Ui, id: &str) {
        let target = self.sheet.ui.menu_for.take().unwrap_or(MenuFor::Cell);
        ui.set_focus(self.canvas);
        let rows = matches!(target, MenuFor::Row(_));
        match id {
            "ctx.cut" => self.copy_cells(ui, true),
            "ctx.copy" => self.copy_cells(ui, false),
            "ctx.paste" => self.paste_cells(ui),
            "ctx.paste.formulas" => self.paste_special(ui, PasteKind::Formulas),
            "ctx.paste.transpose" => self.paste_special(ui, PasteKind::Transpose),
            "ctx.paste.values" => self.paste_special(ui, PasteKind::Values),
            "ctx.paste.formats" => self.paste_special(ui, PasteKind::Formats),
            "ctx.paste.special" => self.open_paste_special_dialog(ui),
            "ctx.insert" | "ctx.delete" => {
                let insert = id == "ctx.insert";
                match target {
                    MenuFor::Row(_) | MenuFor::Col(_) => self.insert_rows_cols(ui, rows, insert),
                    _ if insert => self.open_insert_dialog(ui),
                    _ => self.open_delete_dialog(ui),
                }
            }
            "ctx.clear" => {
                let _ = self.sheet.clear_range();
                self.sync_bars(ui);
                self.build_grid(ui);
            }
            "ctx.format" => {
                let _ = self.run_command(ui, "home.number.launcher");
            }
            _ => {
                if let Some(mode) = match id {
                    "ctx.af.copy" => Some(FillMode::Copy),
                    "ctx.af.series" => Some(FillMode::Series),
                    "ctx.af.formats" => Some(FillMode::FormatsOnly),
                    "ctx.af.values" => Some(FillMode::WithoutFormatting),
                    _ => None,
                } {
                    self.note = match self.sheet.refill(mode) {
                        Ok(true) => "filled".to_owned(),
                        Ok(false) => "nothing to fill".to_owned(),
                        Err(e) => format!("fill failed: {e}"),
                    };
                    self.after_nav(ui);
                }
            }
        }
    }

    /// The floating AutoFill Options button at the corner of the last filled range; built with
    /// the grid while `SheetModel::autofill_button` is set.
    pub(super) fn build_autofill_button(&mut self, ui: &mut Ui, root: NodeId) {
        let Some((range, _)) = self.sheet.autofill_button() else {
            return;
        };
        if self.sheet.gesture.is_some() || self.editor.is_some() {
            return;
        }
        let (_, _, r, b) = self.grid_geom().range_rect(range);
        let side = self.px(AUTOFILL_BTN);
        let (x, y) = (r as i32 + self.px(1), b as i32 + self.px(1));
        let a = self.content;
        // Hidden when the corner is scrolled out of view (the bars take the right and bottom).
        if x + side > a.right() - self.px(14) || y + side > a.bottom() - self.px(14) {
            return;
        }
        let id = self.add(
            ui,
            root,
            (0, 0),
            PxRect {
                x,
                y,
                w: side,
                h: side,
            },
            Widget::Button { toggled: None },
            "AutoFill Options",
            Look {
                icon: self.icon("clipboard"),
                content: Content::IconOnly,
                icon_px: 14.0,
                fill: Some(Token::Field),
                border: Some(Token::FieldBorder),
                ..Look::default()
            },
            Act::Inert,
        );
        self.grid_ids.push(id);
        self.sheet.ui.nodes.push((id, "autofill".to_owned()));
    }

    /// A click on the AutoFill Options button: its menu opens under it.
    pub(super) fn open_autofill_menu(&mut self, ui: &mut Ui, button: NodeId) {
        let r = ui.tree.abs_rect(button);
        self.context_menu(ui, Point::new(r.x, r.bottom()), MenuFor::AutoFill);
    }
}
