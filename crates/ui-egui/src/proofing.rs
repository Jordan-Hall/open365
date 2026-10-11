//! Proofing in the views: asynchronous paragraph checks on `open365-spell`, the red wavy underline and the data of
//! the right-click menu (up to five suggestions, Ignore All, Add to Dictionary).
//!
//! The squiggle geometry is Word's, measured through the oracle (`fixtures/reference/ui/word-squiggle.json`): a
//! triangle wave with a fixed period of 6 physical pixels and a height of 4 pixel rows, the same at every zoom,
//! starting at the left edge of the word and running 2 pixels past its end, in `ff4a4a`.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;

use egui::{Color32, Painter, Pos2, Shape, Stroke};
use open365_spell::{IssueKind, LangSpan, ParagraphReport, ProofRequest, ProofWorker, Proofing};

use crate::docmodel::LangRun;

/// Colour of the wave at full coverage (dark Word theme; the light theme was not measured).
pub const SQUIGGLE: Color32 = Color32::from_rgb(0xff, 0x4a, 0x4a);
/// Wave period and the distance between crest and trough, in physical pixels.
pub const PERIOD_PX: f32 = 6.0;
pub const DEPTH_PX: f32 = 3.0;
/// The wave runs this far past the end of the word.
pub const OVERSHOOT_PX: f32 = 2.0;
/// The most suggestions the menu offers.
pub const MAX_SUGGESTIONS: usize = 5;

/// A flagged word, in characters of its paragraph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub start: usize,
    pub end: usize,
    pub word: String,
    /// Proofing language of the word.
    pub lang: String,
}

/// What the right-click menu is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpellTarget {
    pub para: usize,
    pub issue: Issue,
    pub suggestions: Vec<String>,
}

/// A menu choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpellChoice {
    Replace(String),
    IgnoreAll,
    AddToDictionary,
}

struct Entry {
    rev: u64,
    text_hash: u64,
    report: Arc<ParagraphReport>,
}

/// Checking state shared by the document and grid views.
pub struct ProofState {
    proofing: Proofing,
    worker: Option<ProofWorker>,
    reports: HashMap<u64, Entry>,
    pending: HashMap<u64, (u64, u64)>,
    /// Language of text that names none.
    pub default_lang: String,
    /// Spell checking as you type (Word's option); off draws nothing.
    pub enabled: bool,
    user_dictionary: Option<PathBuf>,
}

impl std::fmt::Debug for ProofState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProofState")
            .field("default_lang", &self.default_lang)
            .field("enabled", &self.enabled)
            .finish_non_exhaustive()
    }
}

impl Default for ProofState {
    fn default() -> Self {
        Self::new()
    }
}

fn hash2<A: Hash, B: Hash>(a: &A, b: &B) -> u64 {
    let mut h = DefaultHasher::new();
    a.hash(&mut h);
    b.hash(&mut h);
    h.finish()
}

impl ProofState {
    /// A state with the bundled dictionaries and no worker yet (see [`Self::start`]).
    #[must_use]
    pub fn new() -> Self {
        Self {
            proofing: Proofing::new(),
            worker: None,
            reports: HashMap::new(),
            pending: HashMap::new(),
            default_lang: "en-GB".to_owned(),
            enabled: true,
            user_dictionary: None,
        }
    }

    /// Starts the worker thread (once) and loads the default dictionary off the UI thread; finished checks
    /// repaint `ctx`.
    pub fn start(&mut self, ctx: &egui::Context) {
        if self.worker.is_some() {
            return;
        }
        let repaint = ctx.clone();
        self.worker = ProofWorker::spawn_with_notify(self.proofing.clone(), move || {
            repaint.request_repaint_after(std::time::Duration::from_millis(30));
        })
        .ok();
        let p = self.proofing.clone();
        let tag = self.default_lang.clone();
        let _ = std::thread::Builder::new()
            .name("open365-spell-preload".to_owned())
            .spawn(move || {
                let _ = p.preload(&tag);
            });
    }

    /// The speller, to install dictionaries or read the options.
    #[must_use]
    pub fn proofing(&self) -> &Proofing {
        &self.proofing
    }

    /// Keeps the user dictionary ("Add to Dictionary") in `path`: loaded now, saved after each addition.
    pub fn set_user_dictionary_path(&mut self, path: PathBuf) {
        let _ = self.proofing.load_user_dictionary(&path);
        self.user_dictionary = Some(path);
    }

    /// Takes in finished checks. True when any report changed.
    pub fn pump(&mut self) -> bool {
        let Some(worker) = &self.worker else {
            return false;
        };
        let mut changed = false;
        for r in worker.drain() {
            if let Some(&(rev, text_hash)) = self.pending.get(&r.id)
                && rev == r.revision
            {
                self.pending.remove(&r.id);
                self.reports.insert(
                    r.id,
                    Entry {
                        rev,
                        text_hash,
                        report: r.report,
                    },
                );
                changed = true;
            }
        }
        changed
    }

    /// The flagged words of paragraph `id` (any caller-chosen key). A changed text is queued for checking; until the
    /// answer is back the previous answer is used when the text did not change (for example after Ignore All).
    pub fn issues(&mut self, id: u64, text: &str, langs: &[LangRun]) -> Vec<Issue> {
        if !self.enabled || text.trim().is_empty() {
            return Vec::new();
        }
        let generation = self.proofing.generation();
        let lang_key: Vec<(usize, usize, &Option<String>)> =
            langs.iter().map(|l| (l.start, l.end, &l.lang)).collect();
        let text_hash = hash2(&text, &lang_key);
        let rev = hash2(&text_hash, &generation);
        let current = self.reports.get(&id).map(|e| e.rev);
        if current != Some(rev) && self.pending.get(&id).map(|p| p.0) != Some(rev) {
            let spans = self.spans(text, langs);
            if let Some(w) = &self.worker
                && w.submit(ProofRequest {
                    id,
                    revision: rev,
                    text: text.to_owned(),
                    spans,
                })
            {
                self.pending.insert(id, (rev, text_hash));
            }
        }
        match self.reports.get(&id) {
            Some(e) if e.text_hash == text_hash => {
                let report = Arc::clone(&e.report);
                self.issues_of(&report, langs)
            }
            _ => Vec::new(),
        }
    }

    /// Language spans in bytes for the checker: the document default everywhere, the runs on top.
    fn spans(&self, text: &str, langs: &[LangRun]) -> Vec<LangSpan> {
        let total = text.chars().count();
        let byte = |c: usize| text.char_indices().nth(c).map_or(text.len(), |(b, _)| b);
        let mut out = Vec::new();
        let mut at = 0;
        for l in langs {
            if l.start > at {
                out.push(LangSpan::new(
                    byte(at)..byte(l.start),
                    self.default_lang.clone(),
                ));
            }
            let range = byte(l.start)..byte(l.end.min(total));
            out.push(match &l.lang {
                Some(tag) => LangSpan::new(range, tag.clone()),
                None => LangSpan::no_proof(range),
            });
            at = l.end;
        }
        if at < total {
            out.push(LangSpan::new(
                byte(at)..text.len(),
                self.default_lang.clone(),
            ));
        }
        out
    }

    fn issues_of(&self, report: &ParagraphReport, langs: &[LangRun]) -> Vec<Issue> {
        report
            .issues
            .iter()
            .filter(|i| i.kind == IssueKind::Misspelled)
            .map(|i| Issue {
                start: i.char_range.start,
                end: i.char_range.end,
                word: i.word.clone(),
                lang: self.lang_at(i.char_range.start, langs),
            })
            .collect()
    }

    fn lang_at(&self, at: usize, langs: &[LangRun]) -> String {
        langs
            .iter()
            .find(|l| l.start <= at && at < l.end)
            .and_then(|l| l.lang.clone())
            .unwrap_or_else(|| self.default_lang.clone())
    }

    /// The menu content for the word that holds character `at` of the paragraph, if it is flagged.
    pub fn target_at(
        &mut self,
        para: usize,
        text: &str,
        langs: &[LangRun],
        at: usize,
    ) -> Option<SpellTarget> {
        self.target_for(para as u64, para, text, langs, at)
    }

    /// [`Self::target_at`] for a text that is not a document paragraph (a cell editor): `key` names its cache slot.
    pub fn target_for(
        &mut self,
        key: u64,
        para: usize,
        text: &str,
        langs: &[LangRun],
        at: usize,
    ) -> Option<SpellTarget> {
        let issue = self
            .issues(key, text, langs)
            .into_iter()
            .find(|i| i.start <= at && at <= i.end)?;
        let suggestions = self.suggest(&issue.word, &issue.lang);
        Some(SpellTarget {
            para,
            issue,
            suggestions,
        })
    }

    /// At most five suggestions, best first.
    #[must_use]
    pub fn suggest(&self, word: &str, lang: &str) -> Vec<String> {
        let mut s = self.proofing.suggest(word, lang);
        s.truncate(MAX_SUGGESTIONS);
        s
    }

    /// "Ignore All": the word is accepted until the session ends.
    pub fn ignore_all(&mut self, word: &str) {
        self.proofing.ignore_all(word);
    }

    /// "Add to Dictionary".
    pub fn add_to_dictionary(&mut self, word: &str) {
        self.proofing.add_to_dictionary(word);
        if let Some(path) = &self.user_dictionary {
            let _ = self.proofing.save_user_dictionary(path);
        }
    }

    /// Runs the two choices that need no document change; false for a replacement.
    pub fn apply_local(&mut self, target: &SpellTarget, choice: &SpellChoice) -> bool {
        match choice {
            SpellChoice::IgnoreAll => self.ignore_all(&target.issue.word),
            SpellChoice::AddToDictionary => self.add_to_dictionary(&target.issue.word),
            SpellChoice::Replace(_) => return false,
        }
        true
    }

    /// Waits until paragraph `id` has been checked (tests).
    #[cfg(test)]
    pub fn settle(&mut self, id: u64, text: &str, langs: &[LangRun]) -> Vec<Issue> {
        for _ in 0..500 {
            self.pump();
            let issues = self.issues(id, text, langs);
            if self.pending.is_empty() {
                return issues;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        Vec::new()
    }
}

/// Vertices of the wave from `x0` to `x1` (points) with its crest at `y_top` (points), snapped to physical pixels.
#[must_use]
pub fn squiggle_points(x0: f32, x1: f32, y_top: f32, ppp: f32) -> Vec<Pos2> {
    let left = (x0 * ppp).round();
    let right = (x1 * ppp).round() + OVERSHOOT_PX;
    let top = (y_top * ppp).round();
    let at = |i: u32| {
        let x = left + 0.5 + i as f32 * PERIOD_PX / 2.0;
        let y = top + 0.5 + if i.is_multiple_of(2) { 0.0 } else { DEPTH_PX };
        (x, y)
    };
    let mut out = Vec::new();
    let mut i = 0u32;
    loop {
        let (x, y) = at(i);
        if x >= right {
            // Cut the last segment where the wave ends.
            let (px, py) = if i == 0 { (left, top + 0.5) } else { at(i - 1) };
            let t = ((right - px) / (x - px).max(1e-3)).clamp(0.0, 1.0);
            out.push(Pos2::new(
                (px + (x - px) * t) / ppp,
                (py + (y - py) * t) / ppp,
            ));
            return out;
        }
        out.push(Pos2::new(x / ppp, y / ppp));
        i += 1;
    }
}

/// Paints the squiggle under `x0..x1`.
pub fn paint_squiggle(painter: &Painter, x0: f32, x1: f32, y_top: f32, ppp: f32) {
    if x1 <= x0 {
        return;
    }
    let pts = squiggle_points(x0, x1, y_top, ppp);
    painter.add(Shape::line(pts, Stroke::new(1.0 / ppp, SQUIGGLE)));
}

/// Key of the cell editor's slot in the check cache (document paragraphs use their index).
const EDITOR_KEY: u64 = u64::MAX - 1;

/// Squiggles and the right-click menu for a single text box (the Sheets cell editor): formulas are not checked,
/// like Excel. `out` is what `TextEdit::show` returned for `text`.
pub fn edit_spelling(
    ui: &egui::Ui,
    proof: &mut ProofState,
    menu: &mut Option<SpellTarget>,
    text: &mut String,
    out: &egui::text_edit::TextEditOutput,
) {
    proof.start(ui.ctx());
    proof.pump();
    if text.starts_with('=') {
        *menu = None;
        return;
    }
    let ppp = ui.ctx().pixels_per_point();
    let painter = ui.painter_at(out.response.rect);
    let galley = &out.galley;
    let pos = out.galley_pos;
    let issues = proof.issues(EDITOR_KEY, text, &[]);
    for issue in &issues {
        // One wave per galley row the word touches.
        let mut run: Option<(f32, f32, f32)> = None;
        for c in issue.start..=issue.end {
            let r = galley.pos_from_cursor(egui::text::CCursor::new(c));
            let (x, bottom) = (pos.x + r.min.x, pos.y + r.max.y);
            match &mut run {
                Some((x0, x1, y)) if (bottom - *y).abs() < 1.0 => {
                    *x0 = x0.min(x);
                    *x1 = x1.max(x);
                }
                _ => {
                    if let Some((x0, x1, y)) = run.take() {
                        paint_squiggle(&painter, x0, x1, y - 3.0, ppp);
                    }
                    run = Some((x, x, bottom));
                }
            }
        }
        if let Some((x0, x1, y)) = run {
            paint_squiggle(&painter, x0, x1, y - 3.0, ppp);
        }
    }
    if out.response.secondary_clicked()
        && let Some(p) = ui.ctx().input(|i| i.pointer.interact_pos())
    {
        let at = galley.cursor_from_pos(p - pos).index;
        *menu = proof.target_for(EDITOR_KEY, 0, text, &[], at.0);
    }
    let Some(target) = menu.clone() else {
        return;
    };
    let mut choice = None;
    let shown = out.response.context_menu(|ui| {
        for s in &target.suggestions {
            if ui.button(egui::RichText::new(s).strong()).clicked() {
                choice = Some(SpellChoice::Replace(s.clone()));
            }
        }
        if target.suggestions.is_empty() {
            ui.add_enabled(false, egui::Button::new("(no spelling suggestions)"));
        }
        ui.separator();
        if ui.button("Ignore All").clicked() {
            choice = Some(SpellChoice::IgnoreAll);
        }
        if ui.button("Add to Dictionary").clicked() {
            choice = Some(SpellChoice::AddToDictionary);
        }
        if choice.is_some() {
            ui.close();
        }
    });
    if shown.is_none() || choice.is_some() {
        *menu = None;
    }
    if let Some(choice) = choice
        && !proof.apply_local(&target, &choice)
        && let SpellChoice::Replace(word) = choice
    {
        replace_chars(
            text,
            target.issue.start,
            target.issue.end,
            &target.issue.word,
            &word,
        );
    }
}

/// Replaces characters `start..end` of `text` with `with` when they still read `expected` (fail closed).
pub fn replace_chars(
    text: &mut String,
    start: usize,
    end: usize,
    expected: &str,
    with: &str,
) -> bool {
    let found: String = text
        .chars()
        .skip(start)
        .take(end.saturating_sub(start))
        .collect();
    if found != expected || with.contains(['\n', '\r']) {
        return false;
    }
    let head: String = text.chars().take(start).collect();
    let tail: String = text.chars().skip(end).collect();
    *text = format!("{head}{with}{tail}");
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wave_has_the_measured_period_and_depth() {
        let p = squiggle_points(10.0, 40.0, 20.0, 1.0);
        // Crest every 6 px, trough 3 px lower in between.
        assert_eq!(p[0].y, 20.5);
        assert_eq!(p[1].y, 23.5);
        assert_eq!(p[2].y, 20.5);
        assert!((p[2].x - p[0].x - 6.0).abs() < 1e-4);
        assert!(p.last().unwrap().x <= 42.0 + 1e-3);
    }

    #[test]
    fn wave_size_does_not_scale_with_pixels_per_point() {
        let a = squiggle_points(0.0, 30.0, 10.0, 1.0);
        let b = squiggle_points(0.0, 15.0, 5.0, 2.0);
        assert_eq!(a.len(), b.len());
        for (p, q) in a.iter().zip(&b) {
            assert!((p.x - q.x * 2.0).abs() < 1e-3 && (p.y - q.y * 2.0).abs() < 1e-3);
        }
    }

    #[test]
    fn finds_misspellings_and_suggests_with_a_five_cap() {
        let mut state = ProofState::new();
        state.start(&egui::Context::default());
        let issues = state.settle(1, "the quik fox", &[]);
        assert_eq!(issues.len(), 1);
        assert_eq!((issues[0].start, issues[0].end), (4, 8));
        let target = state.target_at(1, "the quik fox", &[], 5).expect("flagged");
        assert!(target.suggestions.len() <= MAX_SUGGESTIONS);
        assert!(target.suggestions.iter().any(|s| s == "quick"));
        state.ignore_all("quik");
        assert!(state.settle(1, "the quik fox", &[]).is_empty());
    }

    #[test]
    fn replace_chars_fails_closed() {
        let mut t = "a quik fox".to_owned();
        assert!(!replace_chars(&mut t, 2, 6, "quok", "quick"));
        assert_eq!(t, "a quik fox");
        assert!(replace_chars(&mut t, 2, 6, "quik", "quick"));
        assert_eq!(t, "a quick fox");
    }

    #[test]
    fn no_proof_runs_are_skipped() {
        let mut state = ProofState::new();
        state.start(&egui::Context::default());
        let langs = [LangRun {
            start: 4,
            end: 8,
            lang: None,
        }];
        assert!(state.settle(2, "the quik fox", &langs).is_empty());
    }
}
