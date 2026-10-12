//! Find (Ctrl+F): a find box over the top right of the page area, all matches highlighted and
//! Next and Previous through the document, wrapping around. Matching is `open365_docx`'s
//! `find_all` (Word's rules for case and whole words), so the egui and the own-UI Writer agree.

use open365_docx::{DocMatch, DocPos, FindOptions};
use open365_gfx::{Color, DrawList, Rect as GRect, TextLayoutOptions};
use open365_ui_core::{Key, Modifiers, Ui};

use super::WriterPane;
use crate::palette::Palette;

/// State of the find box.
#[derive(Debug, Clone, Default)]
pub struct FindState {
    pub open: bool,
    pub query: String,
    pub match_case: bool,
    pub whole_word: bool,
    /// Word wildcards (`Use wildcards`).
    pub wildcards: bool,
    /// Why the query cannot be searched (an invalid wildcard expression), else `None`.
    pub error: Option<String>,
    pub(super) matches: Vec<DocMatch>,
    pub(super) current: Option<usize>,
    /// Document revision the matches were computed for.
    pub(super) revision: Option<u64>,
}

impl WriterPane {
    // ---- API ---------------------------------------------------------------------------------

    /// Opens the find box (Ctrl+F). A selection inside one paragraph becomes the query.
    pub fn find_open(&mut self) {
        let (a, b) = self.selection();
        if a.para == b.para && a.offset < b.offset && b.offset - a.offset <= 200 {
            self.find.query = self.selected_text();
        }
        self.find.open = true;
        self.find_refresh(true);
    }

    pub fn find_close(&mut self) {
        self.find.open = false;
        self.replace.open = false;
        self.find.matches.clear();
        self.find.current = None;
    }

    pub fn find_is_open(&self) -> bool {
        self.find.open
    }

    pub fn find_query(&self) -> &str {
        &self.find.query
    }

    /// Sets the search text and selects the first match at or after the caret.
    pub fn set_find_query(&mut self, query: &str) {
        self.find.query = query.to_owned();
        self.find.open = true;
        self.find_refresh(true);
    }

    pub fn set_find_match_case(&mut self, on: bool) {
        self.find.match_case = on;
        self.find_refresh(true);
    }

    pub fn set_find_wildcards(&mut self, on: bool) {
        self.find.wildcards = on;
        self.find_refresh(true);
    }

    pub fn set_find_whole_word(&mut self, on: bool) {
        self.find.whole_word = on;
        self.find_refresh(true);
    }

    /// Number of matches of the current query.
    pub fn find_count(&self) -> usize {
        self.find.matches.len()
    }

    /// Index (0-based) of the selected match.
    pub fn find_index(&self) -> Option<usize> {
        self.find.current
    }

    /// The matches as (start, end) positions.
    pub fn find_matches(&self) -> Vec<(DocPos, DocPos)> {
        self.find
            .matches
            .iter()
            .map(|m| (DocPos::new(m.para, m.start), DocPos::new(m.para, m.end)))
            .collect()
    }

    /// "2 of 5", "No results" or an empty string while the query is empty.
    pub fn find_status(&self) -> String {
        if self.find.query.is_empty() {
            return String::new();
        }
        if self.find.error.is_some() {
            return "Invalid pattern".to_owned();
        }
        match (self.find.current, self.find.matches.len()) {
            (_, 0) => "No results".to_owned(),
            (Some(i), n) => format!("{} of {n}", i + 1),
            (None, n) => format!("{n} results"),
        }
    }

    /// Selects the next match after the selection, wrapping to the first.
    pub fn find_next(&mut self) -> bool {
        self.find_step(true)
    }

    /// Selects the previous match before the selection, wrapping to the last.
    pub fn find_prev(&mut self) -> bool {
        self.find_step(false)
    }

    // ---- internals ---------------------------------------------------------------------------

    /// Recomputes the matches; with `jump`, selects the first one at or after the caret.
    pub(super) fn find_refresh(&mut self, jump: bool) {
        self.find.revision = Some(self.target.revision());
        self.find.matches.clear();
        self.find.current = None;
        if self.find.query.is_empty() {
            return;
        }
        let opts = FindOptions {
            pattern: self.find.query.clone(),
            match_case: self.find.match_case,
            whole_word: self.find.whole_word,
            wildcards: self.find.wildcards,
        };
        self.find.error = None;
        let Some(doc) = self.target.docx() else {
            return;
        };
        match doc.find_all(&opts) {
            Ok(m) => self.find.matches = m,
            Err(e) => self.find.error = Some(e.to_string()),
        }
        if jump {
            let from = self.selection().0;
            let first = self
                .find
                .matches
                .iter()
                .position(|m| (m.para, m.start) >= (from.para, from.offset));
            if !self.find.matches.is_empty() {
                self.find_select(first.unwrap_or(0));
            }
        } else {
            let (a, b) = self.selection();
            self.find.current = self.find.matches.iter().position(|m| {
                a.para == b.para && (m.para, m.start, m.end) == (a.para, a.offset, b.offset)
            });
        }
    }

    /// Called after the document changed.
    pub(super) fn find_sync(&mut self) {
        if self.find.open && self.find.revision != Some(self.target.revision()) {
            self.find_refresh(false);
        }
    }

    pub(super) fn find_select(&mut self, i: usize) {
        let Some(m) = self.find.matches.get(i).copied() else {
            return;
        };
        self.select(DocPos::new(m.para, m.start), DocPos::new(m.para, m.end));
        self.find.current = Some(i);
        self.reveal_caret();
    }

    pub(super) fn find_step(&mut self, forward: bool) -> bool {
        if self.find.revision != Some(self.target.revision()) {
            self.find_refresh(false);
        }
        let n = self.find.matches.len();
        if n == 0 {
            return false;
        }
        let (a, b) = self.selection();
        let target = if forward {
            self.find
                .matches
                .iter()
                .position(|m| (m.para, m.start) >= (b.para, b.offset))
                .unwrap_or(0)
        } else {
            self.find
                .matches
                .iter()
                .rposition(|m| (m.para, m.end) <= (a.para, a.offset))
                .unwrap_or(n - 1)
        };
        self.find_select(target);
        true
    }

    /// Keys while the find box is open. Returns true when the key was taken.
    pub(super) fn find_key(&mut self, key: Key, m: Modifiers) -> bool {
        self.touch();
        if self.replace.open {
            return self.replace_key(key, m);
        }
        match key {
            Key::Escape => self.find_close(),
            Key::Enter => {
                self.find_step(!m.shift);
            }
            Key::Backspace => {
                self.find.query.pop();
                self.find_refresh(true);
            }
            _ => return false,
        }
        true
    }

    pub(super) fn find_text(&mut self, text: &str) {
        self.touch();
        let clean: String = text.chars().filter(|c| !c.is_control()).collect();
        if self.replace.open && self.replace_text(&clean) {
            return;
        }
        if !clean.is_empty() {
            self.find.query.push_str(&clean);
            self.find_refresh(true);
        }
    }

    // ---- painting ----------------------------------------------------------------------------

    /// Highlights every match on the lines of `page`.
    pub(super) fn paint_find_marks(
        &mut self,
        list: &mut DrawList,
        page: usize,
        origin: (f32, f32),
        view: (f32, f32),
    ) {
        if !self.find.open || self.find.matches.is_empty() {
            return;
        }
        let s = self.s();
        let first = self.index.page_first[page];
        let n = self.engine.layout().pages[page].lines.len();
        let mut marks = 0usize;
        for li in 0..n {
            let (top, bottom, para) = {
                let line = &self.engine.layout().pages[page].lines[li];
                (
                    origin.1 + line.top as f32 * s,
                    origin.1 + (line.top + line.height) as f32 * s,
                    line.paragraph,
                )
            };
            if bottom < view.0 || top > view.1 {
                continue;
            }
            let lo = self.find.matches.partition_point(|m| m.para < para);
            for (k, m) in self.find.matches[lo..].iter().enumerate() {
                if m.para != para {
                    break;
                }
                let sel = (DocPos::new(m.para, m.start), DocPos::new(m.para, m.end));
                if let Some((x0, x1)) = self.selection_span(first + li, sel) {
                    let color = if self.find.current == Some(lo + k) {
                        Color::rgb(255, 150, 0).with_opacity(0.5)
                    } else {
                        Color::rgb(255, 230, 0).with_opacity(0.45)
                    };
                    list.fill_rect(
                        GRect::from_edges(
                            origin.0 + x0 as f32 * s,
                            top,
                            origin.0 + x1 as f32 * s,
                            bottom,
                        ),
                        color,
                    );
                    marks += 1;
                }
            }
        }
        self.stats.find_marks += marks;
    }

    /// The find box in the top right corner of the pane.
    pub(super) fn paint_find_box(
        &mut self,
        ui: &mut Ui,
        list: &mut DrawList,
        pal: &Palette,
        caret_on: bool,
    ) {
        if !self.find.open || self.replace.open {
            return;
        }
        let u = self.scale;
        let (w, h) = (300.0 * u, 34.0 * u);
        let right = self.area.x as f32 + self.area.w as f32 - self.bar_w() - 12.0 * u;
        let top = self.area.y as f32 + self.top_inset() + 8.0 * u;
        let r = GRect::from_edges(right - w, top, right, top + h);
        list.shadow(
            r,
            4.0 * u,
            6.0 * u,
            [0.0, 2.0 * u],
            Color::rgb(0, 0, 0).with_opacity(0.25),
        );
        list.fill_rrect(r, 4.0 * u, pal.page);
        list.stroke_rrect(r, 4.0 * u, 1.0, pal.accent);
        let px = 14.0 * u;
        let base = (r.y0 + (h + px * 0.7) / 2.0).round();
        let (shown, color) = if self.find.query.is_empty() {
            (
                "Find in document".to_owned(),
                pal.page_text.with_opacity(0.5),
            )
        } else {
            (self.find.query.clone(), pal.page_text)
        };
        crate::doc::text(ui, list, &shown, (r.x0 + 10.0 * u).floor(), base, px, color);
        if caret_on {
            let typed = ui
                .assets
                .layout_text(
                    ui.font,
                    &self.find.query,
                    px,
                    0.0,
                    0.0,
                    TextLayoutOptions::default(),
                )
                .width;
            let x = (r.x0 + 10.0 * u + typed).floor();
            list.fill_rect(
                GRect::from_edges(
                    x,
                    base - px * 0.85,
                    x + u.max(1.0).round(),
                    base + px * 0.25,
                ),
                pal.page_text,
            );
        }
        let status = self.find_status();
        if !status.is_empty() {
            let wd = ui
                .assets
                .layout_text(ui.font, &status, px, 0.0, 0.0, TextLayoutOptions::default())
                .width;
            crate::doc::text(
                ui,
                list,
                &status,
                (r.x1 - wd - 10.0 * u).floor(),
                base,
                px,
                pal.page_text.with_opacity(0.7),
            );
        }
    }
}
