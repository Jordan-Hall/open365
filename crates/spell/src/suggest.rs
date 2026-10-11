//! Suggestion generation: `REP`, `MAP`, `KEY`/`TRY` edits, word splitting and an edit-distance
//! fallback over dictionary stems. Written from the public descriptions of those directives.

use std::collections::{HashMap, HashSet};

use crate::casing::CaseKind;
use crate::check::LookupOpts;
use crate::dictionary::Dictionary;

/// Limits for [`Dictionary::suggest`].
#[derive(Clone, Debug)]
pub struct SuggestOptions {
    /// Maximum number of suggestions returned.
    pub max_suggestions: usize,
}

impl Default for SuggestOptions {
    fn default() -> Self {
        Self {
            max_suggestions: 10,
        }
    }
}

/// Words longer than this only get the cheap stages.
const LONG_WORD: usize = 40;

struct Collector<'d> {
    dict: &'d Dictionary,
    original: String,
    out: Vec<String>,
    seen: HashSet<String>,
    limit: usize,
    compounds: usize,
}

impl Collector<'_> {
    fn full(&self) -> bool {
        self.out.len() >= self.limit
    }

    /// Proposes a candidate; it is kept only if the dictionary accepts it.
    fn offer(&mut self, candidate: &str) {
        if self.full() || candidate.is_empty() || candidate == self.original {
            return;
        }
        if !self.seen.insert(candidate.to_owned()) {
            return;
        }
        if candidate.contains(' ') {
            let parts: Vec<&str> = candidate.split(' ').collect();
            if parts.iter().any(|p| p.is_empty()) {
                return;
            }
            let forms: Option<Vec<String>> = parts.iter().map(|p| self.valid_form(p)).collect();
            if let Some(forms) = forms {
                self.out.push(forms.join(" "));
            }
            return;
        }
        if let Some(form) = self.valid_form(candidate) {
            self.out.push(form);
        }
    }

    /// The correctly cased spelling of `candidate` if it is a valid word.
    fn valid_form(&mut self, candidate: &str) -> Option<String> {
        let dict = self.dict;
        let forms = [candidate.to_owned(), dict.caser.title(candidate)];
        let plain = LookupOpts {
            keepcase_ok: true,
            suggest: true,
            compound: false,
        };
        for form in &forms {
            if !dict.is_forbidden_entry(form) && dict.lookup_form(form, plain) {
                return Some(form.clone());
            }
        }
        if dict.aff.has_compounding() && self.compounds < dict.aff.max_cpd_sugs {
            let compound = LookupOpts {
                compound: true,
                ..plain
            };
            for form in &forms {
                if !dict.is_forbidden_entry(form) && dict.lookup_form(form, compound) {
                    self.compounds += 1;
                    return Some(form.clone());
                }
            }
        }
        None
    }
}

impl Dictionary {
    /// Suggests corrections for a misspelled word, best first. Returns nothing for a correct word.
    ///
    /// The result is cased like the input (`teh` -> `the`, `Teh` -> `The`, `TEH` -> `THE`) and
    /// passed through `OCONV`.
    #[must_use]
    pub fn suggest(&self, word: &str, options: &SuggestOptions) -> Vec<String> {
        let typed = self.normalize(&word.replace('\u{2019}', "'"));
        if typed.is_empty() || options.max_suggestions == 0 || self.spell(&typed) {
            return Vec::new();
        }
        let kind = self.caser.kind(&typed);
        let working = match kind {
            CaseKind::Title | CaseKind::Upper => self.caser.lower(&typed),
            _ => typed.clone(),
        };
        let mut col = Collector {
            dict: self,
            original: working.clone(),
            out: Vec::new(),
            seen: HashSet::new(),
            limit: options.max_suggestions.saturating_mul(2),
            compounds: 0,
        };
        let chars: Vec<char> = working.chars().collect();

        for variant in [
            self.caser.lower(&typed),
            self.caser.title(&typed),
            self.caser.upper(&typed),
        ] {
            if variant != typed {
                col.offer(&variant);
            }
        }
        self.rep_stage(&working, &mut col);
        if chars.len() <= LONG_WORD {
            self.map_stage(&working, &mut col);
            self.edit_stage(&chars, &mut col);
            if !self.aff.no_split_sugs {
                self.split_stage(&chars, &mut col);
            }
        } else {
            self.delete_stage(&chars, &mut col);
        }
        if col.out.len() < 3 && chars.len() <= LONG_WORD {
            self.ngram_stage(&chars, &mut col);
        }
        self.finish(col.out, kind, word, options.max_suggestions)
    }

    /// Restores the input's capitalisation, applies `OCONV` and removes duplicates.
    fn finish(&self, raw: Vec<String>, kind: CaseKind, original: &str, max: usize) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for suggestion in raw {
            let cased = match kind {
                CaseKind::Upper => self.caser.upper(&suggestion),
                CaseKind::Title if suggestion.chars().next().is_some_and(char::is_lowercase) => {
                    self.caser.capitalize_first(&suggestion)
                }
                _ => suggestion,
            };
            let converted = self.output_convert(&cased);
            if converted != original && seen.insert(converted.clone()) {
                out.push(converted);
                if out.len() == max {
                    break;
                }
            }
        }
        out
    }

    fn rep_stage(&self, word: &str, col: &mut Collector<'_>) {
        for (from, to) in &self.aff.rep {
            let from = from.replace('_', " ");
            let to = to.replace('_', " ");
            let at_start = from.starts_with('^');
            let body = from.strip_prefix('^').unwrap_or(&from);
            let at_end = body.ends_with('$');
            let body = body.strip_suffix('$').unwrap_or(body);
            if body.is_empty() {
                continue;
            }
            for (at, _) in word.match_indices(body) {
                if (at_start && at != 0) || (at_end && at + body.len() != word.len()) {
                    continue;
                }
                col.offer(&format!("{}{to}{}", &word[..at], &word[at + body.len()..]));
                if col.full() {
                    return;
                }
            }
        }
    }

    fn map_stage(&self, word: &str, col: &mut Collector<'_>) {
        if self.aff.map.is_empty() {
            return;
        }
        let first = self.map_variants(word);
        for candidate in &first {
            col.offer(candidate);
        }
        for candidate in first.iter().take(60) {
            for second in self.map_variants(candidate).into_iter().take(60) {
                col.offer(&second);
            }
        }
    }

    /// Every single substitution of a character by a `MAP`-equivalent one.
    fn map_variants(&self, word: &str) -> Vec<String> {
        let mut out = Vec::new();
        for group in &self.aff.map {
            for (at, _) in word.char_indices() {
                let rest = &word[at..];
                for unit in group {
                    if rest.starts_with(unit.as_str()) {
                        for other in group.iter().filter(|o| *o != unit) {
                            out.push(format!("{}{other}{}", &word[..at], &rest[unit.len()..]));
                        }
                    }
                }
            }
        }
        out
    }

    fn delete_stage(&self, w: &[char], col: &mut Collector<'_>) {
        for i in 0..w.len() {
            let mut c = w.to_vec();
            c.remove(i);
            col.offer(&c.iter().collect::<String>());
        }
    }

    fn edit_stage(&self, w: &[char], col: &mut Collector<'_>) {
        let n = w.len();
        let offer = |col: &mut Collector<'_>, c: &[char]| col.offer(&c.iter().collect::<String>());

        // Adjacent transpositions, and the double swaps of 4 and 5 letter words.
        for i in 0..n.saturating_sub(1) {
            let mut c = w.to_vec();
            c.swap(i, i + 1);
            offer(col, &c);
        }
        let double_swaps: &[(usize, usize)] = match n {
            4 => &[(0, 2)],
            5 => &[(0, 3), (1, 3)],
            _ => &[],
        };
        for &(a, b) in double_swaps {
            let mut c = w.to_vec();
            c.swap(a, a + 1);
            c.swap(b, b + 1);
            offer(col, &c);
        }
        // Longer-range transpositions.
        for i in 0..n {
            for j in (i + 2)..n.min(i + 5) {
                let mut c = w.to_vec();
                c.swap(i, j);
                offer(col, &c);
            }
        }
        // Neighbouring keys and wrong-case letters.
        let neighbours = self.key_neighbours();
        for i in 0..n {
            for &other in neighbours.get(&w[i]).map_or(&[][..], Vec::as_slice) {
                let mut c = w.to_vec();
                c[i] = other;
                offer(col, &c);
            }
            let mut upper = w[i].to_uppercase();
            if let (Some(u), None) = (upper.next(), upper.next())
                && u != w[i]
            {
                let mut c = w.to_vec();
                c[i] = u;
                offer(col, &c);
            }
        }
        // Extra letter.
        self.delete_stage(w, col);
        // Missing letter.
        for i in 0..=n {
            for &t in self.try_chars() {
                let mut c = w.to_vec();
                c.insert(i, t);
                offer(col, &c);
            }
            if col.full() {
                return;
            }
        }
        // Letter in the wrong place.
        for i in 0..n {
            for j in 0..n {
                if j == i || j.abs_diff(i) < 2 || j.abs_diff(i) > 10 {
                    continue;
                }
                let mut c = w.to_vec();
                let ch = c.remove(i);
                c.insert(j, ch);
                offer(col, &c);
            }
        }
        // Wrong letter.
        for i in 0..n {
            for &t in self.try_chars() {
                if t != w[i] {
                    let mut c = w.to_vec();
                    c[i] = t;
                    offer(col, &c);
                }
            }
            if col.full() {
                return;
            }
        }
        // A doubled pair of letters (`vacacation`).
        for i in 0..n.saturating_sub(4) {
            if w[i] == w[i + 2] && w[i + 1] == w[i + 3] && w[i] == w[i + 4] {
                let mut c = w.to_vec();
                c.drain(i + 3..i + 5);
                offer(col, &c);
            }
        }
    }

    fn key_neighbours(&self) -> HashMap<char, Vec<char>> {
        let mut map: HashMap<char, Vec<char>> = HashMap::new();
        for row in &self.aff.key_rows {
            for (i, &c) in row.iter().enumerate() {
                let entry = map.entry(c).or_default();
                if i > 0 {
                    entry.push(row[i - 1]);
                }
                if let Some(&next) = row.get(i + 1) {
                    entry.push(next);
                }
            }
        }
        map
    }

    /// `alot` -> `a lot`.
    fn split_stage(&self, w: &[char], col: &mut Collector<'_>) {
        for at in 1..w.len() {
            let left: String = w[..at].iter().collect();
            let right: String = w[at..].iter().collect();
            col.offer(&format!("{left} {right}"));
            if self.try_chars().contains(&'-')
                && left.chars().count() > 1
                && right.chars().count() > 1
                && let (Some(l), Some(r)) = (col.valid_form(&left), col.valid_form(&right))
            {
                let hyphenated = format!("{l}-{r}");
                if col.seen.insert(hyphenated.clone()) && !col.full() {
                    col.out.push(hyphenated);
                }
            }
        }
    }

    /// Edit-distance fallback over dictionary stems for words with several mistakes.
    fn ngram_stage(&self, w: &[char], col: &mut Collector<'_>) {
        let n = w.len();
        let max_dist = (n / 3).clamp(1, 3);
        let aff = &self.aff;
        let mut found: Vec<(usize, usize, &str)> = Vec::new();
        for (stem, entries) in &self.words {
            let usable = entries.iter().any(|e| {
                !(e.has(aff.nosuggest)
                    || e.has(aff.forbidden)
                    || e.has(aff.only_in_compound)
                    || e.has(aff.needaffix))
            });
            if !usable || stem.chars().count().abs_diff(n) > max_dist {
                continue;
            }
            let lowered: Vec<char> = self.caser.lower(stem).chars().collect();
            if let Some(distance) = bounded_distance(w, &lowered, max_dist) {
                found.push((distance, 1000 - bigram_dice(w, &lowered), stem));
            }
        }
        found.sort_unstable();
        let wanted = col.out.len() + aff.max_ngram_sugs;
        for (_, _, stem) in found {
            if col.out.len() >= wanted || col.full() {
                break;
            }
            col.offer(stem);
        }
    }
}

/// Optimal-string-alignment distance (insert, delete, replace, adjacent swap), `None` above `max`.
pub(crate) fn bounded_distance(a: &[char], b: &[char], max: usize) -> Option<usize> {
    if a.len().abs_diff(b.len()) > max {
        return None;
    }
    let mut prev2: Vec<usize> = vec![0; b.len() + 1];
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur: Vec<usize> = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        let mut row_min = cur[0];
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut best = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                best = best.min(prev2[j - 2] + 1);
            }
            cur[j] = best;
            row_min = row_min.min(best);
        }
        if row_min > max {
            return None;
        }
        std::mem::swap(&mut prev2, &mut prev);
        std::mem::swap(&mut prev, &mut cur);
    }
    let distance = prev[b.len()];
    (distance <= max).then_some(distance)
}

/// Dice coefficient over padded bigrams, scaled to 0..=1000.
fn bigram_dice(a: &[char], b: &[char]) -> usize {
    fn grams(text: &[char]) -> Vec<(char, char)> {
        let mut padded = vec!['^'];
        padded.extend_from_slice(text);
        padded.push('$');
        let mut grams: Vec<(char, char)> = padded.windows(2).map(|p| (p[0], p[1])).collect();
        grams.sort_unstable();
        grams
    }
    let (ga, gb) = (grams(a), grams(b));
    let (mut i, mut j, mut common) = (0, 0, 0);
    while i < ga.len() && j < gb.len() {
        match ga[i].cmp(&gb[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                common += 1;
                i += 1;
                j += 1;
            }
        }
    }
    2000 * common / (ga.len() + gb.len()).max(1)
}
