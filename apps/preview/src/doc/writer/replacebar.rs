//! Find and Replace (Ctrl+H): a dialog over the top right of the page area with the Find what and
//! Replace with fields, Match case, Find whole words only and Use wildcards, and the Replace,
//! Replace All, Find Next and Close buttons. Matching is `open365_docx`'s (`find_all`, Word's
//! wildcard syntax); every edit goes through the command layer, so Replace All is one undo
//! step and each Replace is one step, as in Word. Find Next that reaches the end of the document
//! asks whether to continue at the beginning and then searches only the remainder.

use open365_commands::Command;
use open365_docx::{DocMatch, DocPos};
use open365_gfx::{Color, DrawList, Rect as GRect, TextLayoutOptions};
use open365_ui_core::{Key, Modifiers, Ui};

use super::WriterPane;
use crate::palette::Palette;

/// Which text field has the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReplaceField {
    #[default]
    Find,
    Replace,
}

/// State of the dialog. The search text and options are the find box's (`FindState`).
#[derive(Debug, Clone, Default)]
pub struct ReplaceState {
    pub open: bool,
    pub replacement: String,
    pub focus: ReplaceField,
    /// What Word says in the dialog after a search or Replace All.
    pub message: Option<String>,
    /// The "continue at the beginning?" question is showing; the search started at `origin`.
    prompt: Option<DocPos>,
    /// After the question was answered Yes: the search only covers the document from its start
    /// to where it began.
    wrap: Option<Wrap>,
}

/// A search that went round the end of the document.
#[derive(Debug, Clone, Copy)]
struct Wrap {
    origin: DocPos,
    /// No match was taken since the wrap: scan from the document start, not the selection.
    fresh: bool,
}

// Message texts are Word's with the product name (the app title's "Open365 Writer") in place
// of "Word". Not confirmed against a
// running Word: WRAP_QUESTION (the wording of the second sentence), the "found no matches" and
// "All done" lines in `count_message`. FINISHED and BAD_PATTERN are from Word's dialogs as
// remembered; the Yes and No buttons of the question are Word's.
fn finished() -> String {
    format!(
        "{} has finished searching the document.",
        open365_branding::WRITE_NAME
    )
}
const BAD_PATTERN: &str =
    "The Find What text contains a Pattern Match expression which is not valid.";
fn wrap_lines() -> [String; 2] {
    [
        format!(
            "{} has reached the end of the document.",
            open365_branding::WRITE_NAME
        ),
        "Do you want to continue searching at the beginning?".to_owned(),
    ]
}

/// What a point in the dialog is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Field(ReplaceField),
    Check(usize),
    Replace,
    ReplaceAll,
    FindNext,
    Close,
    Yes,
    No,
}

struct Parts {
    frame: GRect,
    find: GRect,
    repl: GRect,
    checks: [GRect; 3],
    buttons: [GRect; 4],
    message_y: f32,
}

/// Word's text for the count after Replace All.
fn count_message(n: usize, query: &str) -> String {
    match n {
        0 => format!("We searched the document and found no matches for \u{201c}{query}\u{201d}."),
        1 => "All done. We made 1 replacement.".to_owned(),
        n => format!("All done. We made {n} replacements."),
    }
}

/// The replacement text for one found string: `^&` is the found text, `^t` a tab, `^l` a line
/// break (as the command layer's Replace All reads it), `^p` a paragraph mark and
/// `^^` a caret. These are the codes the command layer's Replace All knows (plus `^p`); it
/// rejects the others, so a Replace with one reports the same invalid-replacement message.
/// With `adapt` (the search ignores case) the text takes the capitals of the found text, as
/// Word's Replace does.
fn expand(replacement: &str, found: &str, adapt: bool) -> Option<String> {
    let mut out = String::new();
    let mut it = replacement.chars();
    while let Some(c) = it.next() {
        if c != '^' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('&') => out.push_str(found),
            Some('t') => out.push('\t'),
            Some('l') => out.push(LINE_BREAK),
            Some('p') => out.push('\n'),
            Some('^') => out.push('^'),
            _ => return None,
        }
    }
    Some(if adapt { adapt_case(found, &out) } else { out })
}

/// Stands for `^l` in an expanded replacement (`\n` there starts a paragraph).
const LINE_BREAK: char = '\u{b}';

/// Word's case matching: an all-capitals match gives capitals, a capitalised one a capital.
fn adapt_case(found: &str, text: &str) -> String {
    let mut letters = found
        .chars()
        .filter(|c| c.is_uppercase() || c.is_lowercase());
    if letters.clone().next().is_some() && letters.all(char::is_uppercase) {
        return text.to_uppercase();
    }
    if found.chars().next().is_some_and(char::is_uppercase) {
        let mut it = text.chars();
        if let Some(first) = it.next() {
            return first.to_uppercase().chain(it).collect();
        }
    }
    text.to_owned()
}

impl WriterPane {
    // ---- API ---------------------------------------------------------------------------------

    /// Opens Find and Replace (Ctrl+H). A selection inside one paragraph becomes the Find what
    /// text. The selection does not move until Find Next, Replace or Replace All.
    pub fn replace_open(&mut self) {
        let (a, b) = self.selection();
        if a.para == b.para && a.offset < b.offset && b.offset - a.offset <= 200 {
            self.find.query = self.selected_text();
        }
        self.replace.open = true;
        self.replace.focus = ReplaceField::Find;
        self.replace.message = None;
        self.replace_reset_wrap();
        self.find.open = true;
        self.find_refresh(false);
    }

    pub fn replace_close(&mut self) {
        self.replace_reset_wrap();
        self.replace.open = false;
        self.find_close();
    }

    pub fn replace_is_open(&self) -> bool {
        self.replace.open
    }

    /// The Replace with text.
    pub fn replacement(&self) -> &str {
        &self.replace.replacement
    }

    pub fn set_replacement(&mut self, text: &str) {
        self.replace.replacement = text.to_owned();
    }

    /// Which field has the keyboard.
    pub fn replace_focus(&self) -> ReplaceField {
        self.replace.focus
    }

    /// The message line of the dialog (the Replace All count, or that the search finished).
    pub fn replace_message(&self) -> Option<&str> {
        self.replace.message.as_deref()
    }

    /// Find Next: selects the next match after the selection. At the end of the document it asks
    /// whether to continue at the beginning (see [`Self::replace_wrap_answer`]); after Yes it
    /// only searches up to where the search began, then says it has finished.
    pub fn replace_find_next(&mut self) -> bool {
        if self.replace.prompt.is_some() {
            return false;
        }
        if self.find.revision != Some(self.target.revision()) {
            self.find_refresh(false);
        }
        let key = |m: &DocMatch| DocPos::new(m.para, m.start);
        let (a, b) = self.selection();
        let hit = match self.replace.wrap {
            None => self.find.matches.iter().position(|m| key(m) >= b),
            Some(w) => {
                let from = if w.fresh { DocPos::new(0, 0) } else { b };
                self.find
                    .matches
                    .iter()
                    .position(|m| key(m) >= from && key(m) < w.origin)
            }
        };
        if let Some(i) = hit {
            if let Some(w) = &mut self.replace.wrap {
                w.fresh = false;
            }
            self.find_select(i);
            self.replace.message = None;
            return true;
        }
        let ask = self.replace.wrap.is_none()
            && !self.find.matches.is_empty()
            && !self.find.query.is_empty()
            && a != DocPos::new(0, 0);
        if ask {
            self.replace.prompt = Some(a);
            self.replace.message = None;
        } else {
            self.replace.wrap = None;
            self.replace.message = self.search_failure(false);
        }
        false
    }

    /// True while the "continue at the beginning?" question waits for Yes or No.
    pub fn replace_wrap_prompt(&self) -> bool {
        self.replace.prompt.is_some()
    }

    /// Answers the question: Yes searches the rest of the document from its start, No leaves
    /// the selection where it is. Returns true when a match was selected.
    pub fn replace_wrap_answer(&mut self, yes: bool) -> bool {
        let Some(origin) = self.replace.prompt.take() else {
            return false;
        };
        if !yes {
            return false;
        }
        self.replace.wrap = Some(Wrap {
            origin,
            fresh: true,
        });
        self.replace_find_next()
    }

    /// Forgets a wrapped search and its question (the text, options or document changed).
    fn replace_reset_wrap(&mut self) {
        self.replace.prompt = None;
        self.replace.wrap = None;
    }

    fn search_failure(&self, found: bool) -> Option<String> {
        if found || self.find.query.is_empty() {
            None
        } else if self.find.error.is_some() {
            Some(BAD_PATTERN.to_owned())
        } else {
            Some(finished())
        }
    }

    /// Replace: when the selection is a match it becomes the replacement text and the next
    /// match is selected; otherwise it only finds the next match. True when text was replaced.
    pub fn replace_one(&mut self) -> bool {
        if self.find.query.is_empty() || self.replace.prompt.is_some() {
            return false;
        }
        if self.find.revision != Some(self.target.revision()) {
            self.find_refresh(false);
        }
        let (a, b) = self.selection();
        let is_match = a.para == b.para
            && self
                .find
                .matches
                .iter()
                .any(|m| (m.para, m.start, m.end) == (a.para, a.offset, b.offset));
        if !is_match {
            self.replace_find_next();
            return false;
        }
        let found = self.selected_text();
        let adapt = !self.find.match_case && !self.find.wildcards;
        let Some(text) = expand(&self.replace.replacement, &found, adapt) else {
            self.replace.message = Some(BAD_PATTERN.to_owned());
            return false;
        };
        let applied = if text.contains(LINE_BREAK) {
            // A line break is inserted text (`\n` in a plain insert); a range replace would
            // start a paragraph with it. Two steps: the command layer has no one-step form.
            let text = text.replace(LINE_BREAK, "\n");
            self.run(Command::DocxDeleteRange { start: a, end: b })
                .is_some()
                && self
                    .run(Command::DocxInsertText {
                        para: a.para,
                        offset: a.offset,
                        text,
                    })
                    .is_some()
        } else if text.is_empty() {
            self.run(Command::DocxDeleteRange { start: a, end: b })
                .is_some()
        } else {
            self.run(Command::DocxReplaceRange {
                start: a,
                end: b,
                text: text.clone(),
            })
            .is_some()
        };
        if !applied {
            return false;
        }
        let after = match text.rsplit_once('\n') {
            Some((_, last)) if !text.contains(LINE_BREAK) => {
                DocPos::new(a.para + text.matches('\n').count(), last.chars().count())
            }
            _ => DocPos::new(a.para, a.offset + text.chars().count()),
        };
        self.set_caret(after);
        self.find_refresh(false);
        self.replace_find_next();
        self.reveal_caret();
        true
    }

    /// Replace All: one command, so one undo step. Returns how many replacements were made and
    /// leaves Word's count in [`Self::replace_message`].
    pub fn replace_all(&mut self) -> usize {
        if self.find.query.is_empty() {
            return 0;
        }
        if self.find.error.is_some() {
            self.replace.message = Some(BAD_PATTERN.to_owned());
            return 0;
        }
        if self.find.revision != Some(self.target.revision()) {
            self.find_refresh(false);
        }
        if self.find.matches.is_empty() {
            self.replace.message = Some(count_message(0, &self.find.query));
            return 0;
        }
        let command = Command::DocxReplaceAll {
            find: self.find.query.clone(),
            replacement: self.replace.replacement.clone(),
            match_case: self.find.match_case,
            whole_word: self.find.whole_word,
            wildcards: self.find.wildcards,
            clipboard: self.clipboard.payload().text,
        };
        let Some(outcome) = self.run(command) else {
            self.replace.message = Some(BAD_PATTERN.to_owned());
            return 0;
        };
        self.find_refresh(false);
        self.replace.message = Some(count_message(outcome.replaced, &self.find.query));
        outcome.replaced
    }

    // ---- input -------------------------------------------------------------------------------

    /// Keys while the dialog is open. Returns true when the key was taken.
    pub(super) fn replace_key(&mut self, key: Key, _m: Modifiers) -> bool {
        if self.replace.prompt.is_some() {
            match key {
                Key::Enter => {
                    self.replace_wrap_answer(true);
                }
                Key::Escape => {
                    self.replace_wrap_answer(false);
                }
                _ => {}
            }
            return true;
        }
        match key {
            Key::Escape => self.replace_close(),
            Key::Enter => {
                self.replace_find_next();
            }
            Key::Tab => {
                self.replace.focus = match self.replace.focus {
                    ReplaceField::Find => ReplaceField::Replace,
                    ReplaceField::Replace => ReplaceField::Find,
                };
            }
            Key::Backspace => {
                self.replace.message = None;
                self.replace_reset_wrap();
                match self.replace.focus {
                    ReplaceField::Find => {
                        self.find.query.pop();
                        self.find_refresh(false);
                    }
                    ReplaceField::Replace => {
                        self.replace.replacement.pop();
                    }
                }
            }
            _ => return false,
        }
        true
    }

    /// Typed text goes to the focused field. True when the dialog took it.
    pub(super) fn replace_text(&mut self, clean: &str) -> bool {
        if self.replace.prompt.is_some() {
            match clean.to_lowercase().as_str() {
                "y" => {
                    self.replace_wrap_answer(true);
                }
                "n" => {
                    self.replace_wrap_answer(false);
                }
                _ => {}
            }
            return true;
        }
        if !clean.is_empty() {
            self.replace.message = None;
            self.replace_reset_wrap();
            match self.replace.focus {
                ReplaceField::Find => {
                    self.find.query.push_str(clean);
                    self.find_refresh(false);
                }
                ReplaceField::Replace => self.replace.replacement.push_str(clean),
            }
        }
        true
    }

    // ---- geometry ----------------------------------------------------------------------------

    fn replace_parts(&self) -> Parts {
        let u = self.scale;
        let (w, h) = (420.0 * u, 236.0 * u);
        let right = self.area.x as f32 + self.area.w as f32 - self.bar_w() - 12.0 * u;
        let top = self.area.y as f32 + self.top_inset() + 8.0 * u;
        let frame = GRect::from_edges(right - w, top, right, top + h);
        let (x0, x1) = (frame.x0 + 12.0 * u, frame.x1 - 12.0 * u);
        let field =
            |y: f32| GRect::from_edges(x0 + 104.0 * u, top + y * u, x1, top + (y + 26.0) * u);
        let check = |y: f32| GRect::from_edges(x0, top + y * u, x1, top + (y + 20.0) * u);
        let button = |i: f32| {
            let bw = 90.0 * u;
            let bx = x0 + i * (bw + 12.0 * u);
            GRect::from_edges(bx, top + 156.0 * u, bx + bw, top + 182.0 * u)
        };
        Parts {
            frame,
            find: field(40.0),
            repl: field(74.0),
            checks: [check(106.0), check(128.0), check(150.0)].map(|r| {
                // Checkbox rows are narrow; only the left part toggles.
                GRect::from_edges(r.x0, r.y0, r.x0 + 220.0 * u, r.y1)
            }),
            buttons: [button(0.0), button(1.0), button(2.0), button(3.0)],
            message_y: top + 210.0 * u,
        }
    }

    fn replace_part_at(&self, x: f32, y: f32) -> Option<Part> {
        let p = self.replace_parts();
        let inside = |r: GRect| x >= r.x0 && x <= r.x1 && y >= r.y0 && y <= r.y1;
        if !inside(p.frame) {
            return None;
        }
        if self.replace.prompt.is_some() {
            return p
                .buttons
                .iter()
                .zip([Part::Yes, Part::No])
                .find_map(|(r, part)| inside(*r).then_some(part));
        }
        let order = [Part::Replace, Part::ReplaceAll, Part::FindNext, Part::Close];
        for (r, part) in p.buttons.iter().zip(order) {
            if inside(*r) {
                return Some(part);
            }
        }
        for (i, r) in p.checks.iter().enumerate() {
            if inside(*r) {
                return Some(Part::Check(i));
            }
        }
        if inside(p.find) {
            return Some(Part::Field(ReplaceField::Find));
        }
        if inside(p.repl) {
            return Some(Part::Field(ReplaceField::Replace));
        }
        None
    }

    /// A press at canvas-local device point (`x`, `y`). True when the dialog took it (anything
    /// inside its frame); the dialog does not block the document outside it.
    pub(super) fn replace_press(&mut self, x: f32, y: f32) -> bool {
        if !self.replace.open {
            return false;
        }
        let (ox, oy) = (self.area.x as f32, self.area.y as f32);
        let p = self.replace_parts();
        if x + ox < p.frame.x0 || x + ox > p.frame.x1 || y + oy < p.frame.y0 || y + oy > p.frame.y1
        {
            return false;
        }
        match self.replace_part_at(x + ox, y + oy) {
            Some(Part::Field(f)) => {
                self.replace.focus = f;
                self.touch();
            }
            Some(Part::Yes) => {
                self.replace_wrap_answer(true);
            }
            Some(Part::No) => {
                self.replace_wrap_answer(false);
            }
            Some(Part::Check(0)) => {
                self.find.match_case = !self.find.match_case;
                self.replace_reset_wrap();
                self.find_refresh(false);
            }
            Some(Part::Check(1)) => {
                self.find.whole_word = !self.find.whole_word;
                self.replace_reset_wrap();
                self.find_refresh(false);
            }
            Some(Part::Check(_)) => {
                self.find.wildcards = !self.find.wildcards;
                self.replace_reset_wrap();
                self.find_refresh(false);
            }
            Some(Part::Replace) => {
                self.replace_one();
            }
            Some(Part::ReplaceAll) => {
                self.replace_all();
            }
            Some(Part::FindNext) => {
                self.replace_find_next();
            }
            Some(Part::Close) => self.replace_close(),
            None => {}
        }
        true
    }

    // ---- painting ----------------------------------------------------------------------------

    pub(super) fn paint_replace_box(
        &mut self,
        ui: &mut Ui,
        list: &mut DrawList,
        pal: &Palette,
        caret_on: bool,
    ) {
        if !self.replace.open {
            return;
        }
        let u = self.scale;
        let p = self.replace_parts();
        let px = 13.0 * u;
        let ink = pal.page_text;
        list.shadow(
            p.frame,
            4.0 * u,
            6.0 * u,
            [0.0, 2.0 * u],
            Color::rgb(0, 0, 0).with_opacity(0.25),
        );
        list.fill_rrect(p.frame, 4.0 * u, pal.page);
        list.stroke_rrect(p.frame, 4.0 * u, 1.0, pal.accent);
        let base = |r: GRect| (r.y0 + (r.y1 - r.y0 + px * 0.7) / 2.0).round();
        crate::doc::text(
            ui,
            list,
            "Find and Replace",
            (p.frame.x0 + 12.0 * u).floor(),
            (p.frame.y0 + 24.0 * u).round(),
            px,
            ink,
        );
        let fields = [
            (
                "Find what:",
                p.find,
                self.find.query.as_str(),
                ReplaceField::Find,
            ),
            (
                "Replace with:",
                p.repl,
                self.replace.replacement.as_str(),
                ReplaceField::Replace,
            ),
        ];
        for (label, r, value, which) in fields {
            crate::doc::text(
                ui,
                list,
                label,
                (p.frame.x0 + 12.0 * u).floor(),
                base(r),
                px,
                ink,
            );
            list.fill_rect(r, pal.canvas.with_opacity(0.4));
            let focused = self.replace.focus == which;
            list.rect_outline(
                r,
                if focused { 2.0 } else { 1.0 },
                if focused {
                    pal.accent
                } else {
                    ink.with_opacity(0.5)
                },
            );
            crate::doc::text(ui, list, value, (r.x0 + 6.0 * u).floor(), base(r), px, ink);
            if focused && caret_on && self.replace.prompt.is_none() {
                let typed = ui
                    .assets
                    .layout_text(ui.font, value, px, 0.0, 0.0, TextLayoutOptions::default())
                    .width;
                let x = (r.x0 + 6.0 * u + typed).floor();
                list.fill_rect(
                    GRect::from_edges(
                        x,
                        base(r) - px * 0.85,
                        x + u.max(1.0).round(),
                        base(r) + px * 0.25,
                    ),
                    ink,
                );
            }
        }
        if self.replace.prompt.is_some() {
            // Word's question box: the message over Yes and No; the rest of the dialog waits.
            let body = GRect::from_edges(
                p.frame.x0 + 2.0 * u,
                p.frame.y0 + 30.0 * u,
                p.frame.x1 - 2.0 * u,
                p.frame.y1 - 2.0 * u,
            );
            list.fill_rect(body, pal.page);
            for (i, line) in wrap_lines().iter().enumerate() {
                crate::doc::text(
                    ui,
                    list,
                    line,
                    (p.frame.x0 + 12.0 * u).floor(),
                    (p.frame.y0 + (64.0 + 24.0 * i as f32) * u).round(),
                    px,
                    ink,
                );
            }
            for (r, label) in p.buttons.iter().take(2).zip(["Yes", "No"]) {
                list.stroke_rrect(*r, 3.0 * u, 1.0, ink.with_opacity(0.6));
                let wd = ui
                    .assets
                    .layout_text(ui.font, label, px, 0.0, 0.0, TextLayoutOptions::default())
                    .width;
                crate::doc::text(
                    ui,
                    list,
                    label,
                    (r.x0 + (r.x1 - r.x0 - wd) / 2.0).floor(),
                    base(*r),
                    px,
                    ink,
                );
            }
            return;
        }
        let checks = [
            ("Match case", self.find.match_case),
            ("Find whole words only", self.find.whole_word),
            ("Use wildcards", self.find.wildcards),
        ];
        for (r, (label, on)) in p.checks.iter().zip(checks) {
            let b = 14.0 * u;
            let bx = GRect::from_edges(r.x0, r.y0 + 3.0 * u, r.x0 + b, r.y0 + 3.0 * u + b);
            list.rect_outline(bx, 1.0, ink.with_opacity(0.7));
            if on {
                list.fill_rect(
                    GRect::from_edges(
                        bx.x0 + 3.0 * u,
                        bx.y0 + 3.0 * u,
                        bx.x1 - 3.0 * u,
                        bx.y1 - 3.0 * u,
                    ),
                    pal.accent,
                );
            }
            crate::doc::text(
                ui,
                list,
                label,
                (r.x0 + b + 8.0 * u).floor(),
                base(*r),
                px,
                ink,
            );
        }
        for (r, label) in p
            .buttons
            .iter()
            .zip(["Replace", "Replace All", "Find Next", "Cancel"])
        {
            list.stroke_rrect(*r, 3.0 * u, 1.0, ink.with_opacity(0.6));
            let wd = ui
                .assets
                .layout_text(ui.font, label, px, 0.0, 0.0, TextLayoutOptions::default())
                .width;
            crate::doc::text(
                ui,
                list,
                label,
                (r.x0 + (r.x1 - r.x0 - wd) / 2.0).floor(),
                base(*r),
                px,
                ink,
            );
        }
        let count = self.find_status();
        let message = self.replace.message.clone().unwrap_or(count);
        if !message.is_empty() {
            crate::doc::text(
                ui,
                list,
                &message,
                (p.frame.x0 + 12.0 * u).floor(),
                p.message_y.round(),
                12.0 * u,
                ink.with_opacity(0.8),
            );
        }
    }
}
