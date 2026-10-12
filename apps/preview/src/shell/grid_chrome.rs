//! The clickable parts of the Sheets grid that are not cells: the AutoFilter drop-down buttons
//! and their menu, the in-cell validation list, the outline gutter (level buttons and the `+`
//! and `-` of a group) and the Page Layout view. They paint in `doc/sheetchrome.rs`; this file
//! holds the presses and the menu picks. The menus are `ui-core` context menus; a pick arrives as
//! `Emit::MenuCommand(_, "gridx.…")`, which `grid_event` routes to [`Shell::gridx_command`].

use open365_commands::data_ops::{Cmp, ColumnFilter};
use open365_ui_core::geom::Point;
use open365_ui_core::{MenuItem, Ui};

use crate::dialogs::request::DialogRequest;
use crate::doc::sheetchrome::{self, OutlineHit};
use crate::sheet::GridPop;
use crate::sheet_ext::view::ViewMode;
use crate::shell::Shell;

/// The most values the filter menu lists (the menu does not scroll yet).
const FILTER_ITEMS: usize = 40;

fn numeric(s: &str) -> bool {
    s.trim().replace(',', "").parse::<f64>().is_ok()
}

impl Shell {
    fn gx_item(id: &str, label: &str) -> MenuItem {
        MenuItem::new(id, label)
    }

    /// A press at device point `(x, y)` on one of the grid's buttons. True when it was one.
    pub(super) fn chrome_press(&mut self, ui: &mut Ui, x: f32, y: f32) -> bool {
        if self.sheet.ext_view.mode == ViewMode::PageLayout {
            return false;
        }
        let g = self.grid_geom();
        let marks = self.sheet.grid_marks();
        if let Some(hit) = sheetchrome::outline_hit(&g, &marks, self.scale, x, y) {
            self.outline_press(ui, hit);
            return true;
        }
        if let Some((col, r)) = sheetchrome::filter_button_at(&g, self.scale, &marks, x, y) {
            let at = Point::new(r.x0 / self.scale, r.y1 / self.scale);
            self.open_filter_menu(ui, col, at);
            return true;
        }
        let (row, col) = self.sheet.active;
        if self.editor.is_none() {
            let items = self.sheet.data_validation_list(row, col);
            if !items.is_empty() {
                let r = sheetchrome::validation_button_rect(&g, self.scale, (row, col));
                if x >= r.x0 && x < r.x1 && y >= r.y0 && y < r.y1 {
                    let at = Point::new(
                        g.cell(row, col).0 / self.scale,
                        g.cell(row, col).3 / self.scale,
                    );
                    self.open_validation_list(ui, row, col, items, at);
                    return true;
                }
            }
        }
        false
    }

    // ---- validation list -------------------------------------------------------------------

    /// Opens the in-cell drop-down list of a validated cell under it.
    pub fn open_validation_list(
        &mut self,
        ui: &mut Ui,
        row: u32,
        col: u32,
        items: Vec<String>,
        at: Point,
    ) {
        let current = self.sheet.display(row, col);
        let menu: Vec<MenuItem> = items
            .iter()
            .enumerate()
            .map(|(i, t)| MenuItem {
                checked: *t == current,
                ..Self::gx_item(&format!("gridx.dv.{i}"), t)
            })
            .collect();
        self.sheet.ui.pop = Some(GridPop::Validation { row, col, items });
        ui.open_context_menu(menu, at);
    }

    /// The validation list of the active cell (Alt+Down in Excel).
    pub fn open_active_validation_list(&mut self, ui: &mut Ui) -> bool {
        let (row, col) = self.sheet.active;
        let items = self.sheet.data_validation_list(row, col);
        if items.is_empty() {
            return false;
        }
        let (l, _, _, b) = self.grid_geom().cell(row, col);
        self.open_validation_list(
            ui,
            row,
            col,
            items,
            Point::new(l / self.scale, b / self.scale),
        );
        true
    }

    // ---- AutoFilter ------------------------------------------------------------------------

    /// Opens the AutoFilter menu of column `col` at the logical point `at`.
    pub fn open_filter_menu(&mut self, ui: &mut Ui, col: u32, at: Point) {
        let Some((range, filtered)) = self.sheet.grid_marks().filter.clone() else {
            return;
        };
        let values = self.sheet.data_filter_items(col);
        let name = self.sheet.display(range.0, col);
        let is_filtered = filtered.contains(&col);
        let numbers = values
            .iter()
            .filter(|(v, _)| v != "(Blanks)")
            .all(|(v, _)| numeric(v))
            && !values.is_empty();
        let mut items = vec![
            Self::gx_item(
                "gridx.flt.sort.az",
                if numbers {
                    "Sort Smallest to Largest"
                } else {
                    "Sort A to Z"
                },
            ),
            Self::gx_item(
                "gridx.flt.sort.za",
                if numbers {
                    "Sort Largest to Smallest"
                } else {
                    "Sort Z to A"
                },
            ),
            MenuItem::separator(),
            MenuItem {
                enabled: is_filtered,
                ..Self::gx_item("gridx.flt.clear", &format!("Clear Filter From \"{name}\""))
            },
        ];
        let ops: &[(&str, &str)] = if numbers {
            &[
                ("equal", "Equals"),
                ("notequal", "Does Not Equal"),
                ("greater", "Greater Than"),
                ("greaterequal", "Greater Than Or Equal To"),
                ("less", "Less Than"),
                ("lessequal", "Less Than Or Equal To"),
                ("top10", "Top 10"),
                ("above", "Above Average"),
                ("below", "Below Average"),
            ]
        } else {
            &[
                ("equal", "Equals"),
                ("notequal", "Does Not Equal"),
                ("begins", "Begins With"),
                ("ends", "Ends With"),
                ("contains", "Contains"),
                ("notcontains", "Does Not Contain"),
            ]
        };
        items.push(MenuItem {
            submenu: ops
                .iter()
                .map(|(id, label)| Self::gx_item(&format!("gridx.flt.op.{id}"), label))
                .collect(),
            ..Self::gx_item(
                "gridx.flt.ops",
                if numbers {
                    "Number Filters"
                } else {
                    "Text Filters"
                },
            )
        });
        items.push(MenuItem::separator());
        items.push(MenuItem {
            checked: values.iter().all(|(_, on)| *on),
            ..Self::gx_item("gridx.flt.all", "(Select All)")
        });
        for (i, (label, on)) in values.iter().enumerate().take(FILTER_ITEMS) {
            items.push(MenuItem {
                checked: *on,
                ..Self::gx_item(&format!("gridx.flt.v.{i}"), label)
            });
        }
        self.sheet.ui.pop = Some(GridPop::Filter {
            col,
            anchor: (at.x as i32, at.y as i32),
        });
        ui.open_context_menu(items, at);
    }

    /// The operand a comparison filter uses until Custom AutoFilter has a dialog: the active
    /// cell when it is a data cell of the column, else the first value of the column.
    fn filter_operand(&self, col: u32) -> String {
        let (r, c) = self.sheet.active;
        if let Some((range, _)) = self.sheet.grid_marks().filter.clone()
            && c == col
            && r > range.0
            && r <= range.2
        {
            return self.sheet.display(r, c);
        }
        self.sheet
            .data_filter_items(col)
            .into_iter()
            .map(|(v, _)| v)
            .find(|v| v != "(Blanks)")
            .unwrap_or_default()
    }

    fn apply_filter(&mut self, col: u32, filter: Option<ColumnFilter>) {
        self.note = match self.sheet.data_filter_column(col, filter) {
            Ok(()) => "filtered".to_owned(),
            Err(e) => format!("filter failed: {e}"),
        };
    }

    /// Runs `f` with the selection moved to a data cell of `col` (the sort works on the column
    /// of the active cell), then puts the selection back.
    fn with_cell_in_col<R>(&mut self, col: u32, f: impl FnOnce(&mut Self) -> R) -> R {
        let saved = (self.sheet.active, self.sheet.anchor, self.sheet.focus);
        let row = self
            .sheet
            .grid_marks()
            .filter
            .as_ref()
            .map_or(1, |(r, _)| r.0 + 1);
        self.sheet.select_cell((row, col));
        let out = f(self);
        (self.sheet.active, self.sheet.anchor, self.sheet.focus) = saved;
        out
    }

    fn filter_command(&mut self, ui: &mut Ui, col: u32, anchor: (i32, i32), tail: &str) {
        let mut reopen = false;
        match tail {
            "sort.az" | "sort.za" => {
                let desc = tail == "sort.za";
                let r = self.with_cell_in_col(col, |s| s.sheet.data_sort_quick(desc));
                self.note =
                    r.map_or_else(|e| format!("sort failed: {e}"), |()| "sorted".to_owned());
            }
            "clear" | "all" => {
                self.apply_filter(col, None);
            }
            _ if tail.starts_with("v.") => {
                if let Ok(i) = tail[2..].parse::<usize>() {
                    let mut values = self.sheet.data_filter_items(col);
                    if let Some(v) = values.get_mut(i) {
                        v.1 = !v.1;
                    }
                    let all = values.iter().all(|(_, on)| *on);
                    let blank = values.iter().any(|(l, on)| l == "(Blanks)" && *on);
                    let items = values
                        .iter()
                        .filter(|(l, on)| *on && l != "(Blanks)")
                        .map(|(l, _)| l.clone())
                        .collect();
                    self.apply_filter(col, (!all).then_some(ColumnFilter::Values { items, blank }));
                    reopen = true;
                }
            }
            _ if tail.starts_with("op.") => {
                let x = self.filter_operand(col);
                const DIALOG_OPS: [&str; 10] = [
                    "equal",
                    "notequal",
                    "greater",
                    "greaterequal",
                    "less",
                    "lessequal",
                    "begins",
                    "ends",
                    "contains",
                    "notcontains",
                ];
                if let Some(i) = DIALOG_OPS.iter().position(|o| *o == &tail[3..]) {
                    // Excel's comparison items all open Custom AutoFilter with that operator.
                    self.filter_op = [0, 1, 2, 3, 4, 5, 6, 8, 10, 11][i];
                    self.open_dialog(
                        ui,
                        DialogRequest::CustomFilter {
                            column: col,
                            operand: x,
                        },
                    );
                    return;
                }
                let one = |cmp: Cmp, v: String| ColumnFilter::Custom {
                    first: (cmp, v),
                    second: None,
                    and: true,
                };
                let filter = match &tail[3..] {
                    "equal" => one(Cmp::Equal, x),
                    "notequal" => one(Cmp::NotEqual, x),
                    "greater" => one(Cmp::Greater, x),
                    "greaterequal" => one(Cmp::GreaterOrEqual, x),
                    "less" => one(Cmp::Less, x),
                    "lessequal" => one(Cmp::LessOrEqual, x),
                    "begins" => one(Cmp::Equal, format!("{x}*")),
                    "ends" => one(Cmp::Equal, format!("*{x}")),
                    "contains" => one(Cmp::Equal, format!("*{x}*")),
                    "notcontains" => one(Cmp::NotEqual, format!("*{x}*")),
                    "top10" => ColumnFilter::Top10 {
                        top: true,
                        percent: false,
                        count: 10.0,
                    },
                    "above" => ColumnFilter::AboveAverage,
                    "below" => ColumnFilter::BelowAverage,
                    _ => return,
                };
                self.apply_filter(col, Some(filter));
            }
            _ => {}
        }
        self.sync_bars(ui);
        self.build_grid(ui);
        if reopen {
            self.open_filter_menu(ui, col, Point::new(anchor.0 as f32, anchor.1 as f32));
        }
    }

    /// Runs a pick of a grid pop-up menu (`gridx.…`).
    pub(super) fn gridx_command(&mut self, ui: &mut Ui, id: &str) {
        let Some(pop) = self.sheet.ui.pop.clone() else {
            return;
        };
        let tail = id.strip_prefix("gridx.").unwrap_or(id);
        match pop {
            GridPop::Filter { col, anchor } => {
                if let Some(t) = tail.strip_prefix("flt.") {
                    self.filter_command(ui, col, anchor, t);
                }
                if ui.focus().is_none() {
                    ui.set_focus(self.canvas);
                }
            }
            GridPop::Validation { row, col, items } => {
                self.sheet.ui.pop = None;
                if let Some(text) = tail
                    .strip_prefix("dv.")
                    .and_then(|i| i.parse::<usize>().ok())
                    .and_then(|i| items.get(i))
                {
                    let text = text.clone();
                    self.enter_checked(ui, row, col, &text);
                }
                self.sync_bars(ui);
                self.build_grid(ui);
                ui.set_focus(self.canvas);
            }
        }
    }

    // ---- outline ---------------------------------------------------------------------------

    fn outline_press(&mut self, ui: &mut Ui, hit: OutlineHit) {
        let marks = self.sheet.grid_marks();
        match hit {
            OutlineHit::RowGroup(i) => {
                if let Some(grp) = marks.row_groups.get(i).copied() {
                    self.toggle_group(true, grp.last + 1, grp.collapsed);
                }
            }
            OutlineHit::ColGroup(i) => {
                if let Some(grp) = marks.col_groups.get(i).copied() {
                    self.toggle_group(false, grp.last + 1, grp.collapsed);
                }
            }
            OutlineHit::RowLevel(n) => self.show_levels(true, n),
            OutlineHit::ColLevel(n) => self.show_levels(false, n),
        }
        self.sync_bars(ui);
        self.build_grid(ui);
    }

    /// Hides or shows the detail of the group whose summary row (column) is `summary`, without
    /// moving the selection.
    fn toggle_group(&mut self, rows: bool, summary: u32, show: bool) {
        let saved = (self.sheet.active, self.sheet.anchor, self.sheet.focus);
        self.sheet
            .select_cell(if rows { (summary, 1) } else { (1, summary) });
        let r = if rows {
            self.sheet.data_detail(show)
        } else {
            self.sheet.data_detail_cols(show)
        };
        (self.sheet.active, self.sheet.anchor, self.sheet.focus) = saved;
        self.note = r.map_or_else(
            |e| e,
            |()| {
                if show {
                    "detail shown"
                } else {
                    "detail hidden"
                }
                .to_owned()
            },
        );
    }

    /// Level button `n`: groups below level `n` show their detail, groups at level `n` hide it.
    fn show_levels(&mut self, rows: bool, n: u32) {
        let marks = self.sheet.grid_marks();
        let groups = if rows {
            &marks.row_groups
        } else {
            &marks.col_groups
        };
        let mut sorted: Vec<_> = groups.clone();
        sorted.sort_by_key(|g| g.level);
        for g in sorted.iter().filter(|g| g.level < n && g.collapsed) {
            self.toggle_group(rows, g.last + 1, true);
        }
        for g in sorted.iter().filter(|g| g.level == n && !g.collapsed) {
            self.toggle_group(rows, g.last + 1, false);
        }
    }
}
