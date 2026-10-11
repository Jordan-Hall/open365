//! Word checking: case handling, affix stripping, compounding and `BREAK`.

use std::borrow::Cow;
use std::sync::Arc;

use crate::aff::{AffixRule, AffixTable, Quant, RuleItem};
use crate::casing::CaseKind;
use crate::dictionary::Dictionary;
use crate::flags::FlagSet;

/// Options for [`Dictionary::lookup_form`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct LookupOpts {
    /// Accept `KEEPCASE` entries (only when the form is spelled as the user typed it).
    pub(crate) keepcase_ok: bool,
    /// Reject `NOSUGGEST` entries (suggestion mode).
    pub(crate) suggest: bool,
    /// Allow the compound matcher.
    pub(crate) compound: bool,
}

/// One way a form decomposes into a dictionary stem plus affixes.
struct Analysis<'a> {
    root: &'a Arc<FlagSet>,
    prefix: Option<&'a AffixRule>,
    /// Outermost suffix first.
    suffixes: [Option<&'a AffixRule>; 2],
}

impl<'a> Analysis<'a> {
    fn rules(&self) -> impl Iterator<Item = &'a AffixRule> + '_ {
        self.prefix
            .into_iter()
            .chain(self.suffixes.iter().flatten().copied())
    }

    fn rule_count(&self) -> usize {
        self.rules().count()
    }

    /// True when the stem or any applied affix carries the flag.
    fn any_has(&self, flag: Option<u32>) -> bool {
        flag.is_some() && (self.root.has(flag) || self.rules().any(|r| r.cont.has(flag)))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Position {
    Begin,
    Middle,
    End,
}

/// A validated compound constituent.
struct Part {
    text: String,
    root: Arc<FlagSet>,
    flag_ok: bool,
    rule_ok: bool,
}

const COMPOUND_BUDGET: usize = 20_000;

impl Dictionary {
    /// Checks a word the way a word processor would: `true` when it is spelled correctly.
    ///
    /// Applies `ICONV`/`IGNORE`, then tries the word, its case variants, affix stripping,
    /// compounding and the `BREAK` table (so `well-known` is accepted when both halves are).
    #[must_use]
    pub fn spell(&self, word: &str) -> bool {
        let normalized = self.normalize(word);
        if normalized.is_empty() {
            return true;
        }
        self.spell_normalized(&normalized, 0)
    }

    fn spell_normalized(&self, word: &str, depth: usize) -> bool {
        if is_number(word) {
            return true;
        }
        if self.spell_whole(word) {
            return true;
        }
        if word.contains('\u{2019}') && self.spell_whole(&word.replace('\u{2019}', "'")) {
            return true;
        }
        depth < 4 && self.spell_broken(word, depth)
    }

    /// `BREAK` handling: accept a word whose pieces are each correct.
    fn spell_broken(&self, word: &str, depth: usize) -> bool {
        const DEFAULT: [(&str, bool, bool); 3] =
            [("-", false, false), ("-", true, false), ("-", false, true)];
        let patterns: Vec<(&str, bool, bool)> = match &self.aff.break_table {
            Some(table) => table
                .iter()
                .map(|p| (p.text.as_str(), p.at_start, p.at_end))
                .collect(),
            None => DEFAULT.to_vec(),
        };
        for (text, at_start, at_end) in patterns {
            if at_start {
                if let Some(rest) = word.strip_prefix(text)
                    && !rest.is_empty()
                    && self.spell_normalized(rest, depth + 1)
                {
                    return true;
                }
            } else if at_end {
                if let Some(rest) = word.strip_suffix(text)
                    && !rest.is_empty()
                    && self.spell_normalized(rest, depth + 1)
                {
                    return true;
                }
            } else {
                for (at, _) in word.match_indices(text) {
                    let (left, right) = (&word[..at], &word[at + text.len()..]);
                    if !left.is_empty()
                        && !right.is_empty()
                        && self.spell_normalized(left, depth + 1)
                        && self.spell_normalized(right, depth + 1)
                    {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Case-variant lookup without `BREAK`.
    fn spell_whole(&self, word: &str) -> bool {
        let forms = self.case_forms(word);
        if forms.iter().any(|form| self.is_forbidden_entry(form)) {
            return false;
        }
        forms.iter().enumerate().any(|(i, form)| {
            self.lookup_form(
                form,
                LookupOpts {
                    keepcase_ok: i == 0,
                    suggest: false,
                    compound: true,
                },
            )
        })
    }

    /// The spellings to try for a typed word: as typed first, then case-folded variants.
    fn case_forms<'w>(&self, word: &'w str) -> Vec<Cow<'w, str>> {
        let mut forms = vec![Cow::Borrowed(word)];
        match self.caser.kind(word) {
            CaseKind::Lower | CaseKind::MixedLower | CaseKind::MixedTitle => {}
            CaseKind::Title => forms.push(Cow::Owned(self.caser.lower(word))),
            CaseKind::Upper => {
                forms.push(Cow::Owned(self.caser.title(word)));
                let lower = self.caser.lower(word);
                if let Some(at) = lower.find('\'') {
                    // `L'AMOUR` -> `l'Amour`: elided article stays lower case.
                    let (head, tail) = lower.split_at(at + 1);
                    forms.push(Cow::Owned(format!(
                        "{head}{}",
                        self.caser.capitalize_first(tail)
                    )));
                    forms.push(Cow::Owned(self.caser.capitalize_first(&lower)));
                }
                forms.push(Cow::Owned(lower));
            }
        }
        forms
    }

    pub(crate) fn is_forbidden_entry(&self, form: &str) -> bool {
        self.aff.forbidden.is_some()
            && self
                .words
                .get(form)
                .is_some_and(|entries| entries.iter().any(|e| e.has(self.aff.forbidden)))
    }

    /// True when `form`, exactly as spelled, is a dictionary word, an affixed form or a compound.
    pub(crate) fn lookup_form(&self, form: &str, opts: LookupOpts) -> bool {
        if self.visit_analyses(form, &mut |a| self.accept(a, opts)) {
            return true;
        }
        opts.compound && self.aff.has_compounding() && self.check_compound(form, opts)
    }

    /// Whether an analysis is a valid standalone word.
    fn accept(&self, a: &Analysis<'_>, opts: LookupOpts) -> bool {
        let aff = &self.aff;
        if a.any_has(aff.forbidden) || a.any_has(aff.only_in_compound) {
            return false;
        }
        if a.root.has(aff.keepcase) && !opts.keepcase_ok {
            return false;
        }
        if opts.suggest && a.any_has(aff.nosuggest) {
            return false;
        }
        if a.root.has(aff.needaffix) && a.rule_count() == 0 {
            return false;
        }
        if a.rules().any(|r| r.cont.has(aff.needaffix)) && a.rule_count() < 2 {
            return false;
        }
        true
    }

    /// Calls `visit` for every valid decomposition until it returns true.
    fn visit_analyses(&self, word: &str, visit: &mut dyn FnMut(&Analysis<'_>) -> bool) -> bool {
        if let Some(entries) = self.words.get(word) {
            for root in entries {
                let a = Analysis {
                    root,
                    prefix: None,
                    suffixes: [None, None],
                };
                if visit(&a) {
                    return true;
                }
            }
        }
        let aff = &self.aff;
        if aff.prefixes.rules.is_empty() && aff.suffixes.rules.is_empty() {
            return false;
        }
        // Every way to remove at most one prefix, then up to two suffixes.
        let mut prefix_options: Vec<(Cow<'_, str>, Option<&AffixRule>)> =
            vec![(Cow::Borrowed(word), None)];
        for_each_stripped(&aff.prefixes, word, true, |root, rule| {
            prefix_options.push((Cow::Owned(root), Some(rule)));
        });
        for (stem_word, prefix) in &prefix_options {
            let mut suffix_options: Vec<(String, [Option<&AffixRule>; 2])> = Vec::new();
            if prefix.is_some() {
                suffix_options.push((stem_word.to_string(), [None, None]));
            }
            for_each_stripped(&aff.suffixes, stem_word, false, |root1, outer| {
                suffix_options.push((root1.clone(), [Some(outer), None]));
                for_each_stripped(&aff.suffixes, &root1, false, |root2, inner| {
                    if inner.cont.contains(outer.flag) {
                        suffix_options.push((root2, [Some(outer), Some(inner)]));
                    }
                });
            });
            for (root_word, suffixes) in &suffix_options {
                let Some(entries) = self.words.get(root_word.as_str()) else {
                    continue;
                };
                for root in entries {
                    let a = Analysis {
                        root,
                        prefix: *prefix,
                        suffixes: *suffixes,
                    };
                    if self.chain_valid(&a) && visit(&a) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Checks that each applied affix is licensed by the stem or by an inner affix.
    fn chain_valid(&self, a: &Analysis<'_>) -> bool {
        let aff = &self.aff;
        match a.suffixes {
            [Some(outer), Some(inner)]
                if !a.root.contains(inner.flag) || !inner.cont.contains(outer.flag) =>
            {
                return false;
            }
            [Some(only), None] if !a.root.contains(only.flag) => return false,
            _ => {}
        }
        let has_suffix = a.suffixes[0].is_some();
        if let Some(prefix) = a.prefix {
            let licensed = a.root.contains(prefix.flag)
                || a.suffixes
                    .iter()
                    .flatten()
                    .any(|s| s.cont.contains(prefix.flag));
            if !licensed {
                return false;
            }
            if has_suffix && !(prefix.cross && a.suffixes.iter().flatten().all(|s| s.cross)) {
                return false;
            }
        }
        if aff.circumfix.is_some() {
            let prefix_circ = a.prefix.is_some_and(|p| p.cont.has(aff.circumfix));
            let suffix_circ = a
                .suffixes
                .iter()
                .flatten()
                .any(|s| s.cont.has(aff.circumfix));
            if prefix_circ != suffix_circ {
                return false;
            }
        }
        true
    }

    // ---- compounding -------------------------------------------------------------------------

    fn check_compound(&self, form: &str, opts: LookupOpts) -> bool {
        let chars: Vec<char> = form.chars().collect();
        let min = self.aff.compound_min.max(1);
        if chars.len() < min * 2 {
            return false;
        }
        let mut parts = Vec::new();
        let mut budget = COMPOUND_BUDGET;
        self.compound_from(&chars, 0, &mut parts, &mut budget, opts)
    }

    fn compound_from(
        &self,
        chars: &[char],
        start: usize,
        parts: &mut Vec<Part>,
        budget: &mut usize,
        opts: LookupOpts,
    ) -> bool {
        let aff = &self.aff;
        let min = aff.compound_min.max(1);
        let total = chars.len();
        if aff.compound_word_max.is_some_and(|max| parts.len() >= max) {
            return false;
        }
        for end in (start + min)..=total {
            let last = end == total;
            if start == 0 && last {
                break;
            }
            if !last && total - end < min {
                continue;
            }
            if *budget == 0 {
                return false;
            }
            *budget -= 1;
            if start > 0 && !self.boundary_ok(chars, start) {
                continue;
            }
            let position = if start == 0 {
                Position::Begin
            } else if last {
                Position::End
            } else {
                Position::Middle
            };
            let text: String = chars[start..end].iter().collect();
            let mut candidates: Vec<Part> = Vec::new();
            self.visit_analyses(&text, &mut |a| {
                let flag_ok = self.part_flag_ok(a, position, opts);
                let rule_ok = a.rule_count() == 0
                    && !self.compound_rule_flags.is_empty()
                    && a.root.iter().any(|f| self.compound_rule_flags.contains(&f))
                    && !a.root.has(aff.forbidden)
                    && !(opts.suggest && a.root.has(aff.nosuggest));
                if flag_ok || rule_ok {
                    candidates.push(Part {
                        text: text.clone(),
                        root: Arc::clone(a.root),
                        flag_ok,
                        rule_ok,
                    });
                }
                false
            });
            for part in candidates {
                if !self.pair_ok(parts.last(), &part) {
                    continue;
                }
                parts.push(part);
                let done = if last {
                    self.compound_complete(parts, chars)
                } else {
                    self.compound_from(chars, end, parts, budget, opts)
                };
                if done {
                    return true;
                }
                parts.pop();
            }
        }
        false
    }

    /// Checks the seam between two compound parts (`CHECKCOMPOUNDTRIPLE`, `CHECKCOMPOUNDCASE`).
    fn boundary_ok(&self, chars: &[char], seam: usize) -> bool {
        let aff = &self.aff;
        if aff.check_compound_triple {
            let triple_before =
                seam >= 2 && chars[seam - 2] == chars[seam - 1] && chars[seam - 1] == chars[seam];
            let triple_after = seam + 1 < chars.len()
                && chars[seam - 1] == chars[seam]
                && chars[seam] == chars[seam + 1];
            if triple_before || triple_after {
                return false;
            }
        }
        if aff.check_compound_case {
            let (a, b) = (chars[seam - 1], chars[seam]);
            if (a.is_uppercase() || b.is_uppercase()) && a != '-' && b != '-' {
                return false;
            }
        }
        true
    }

    /// Checks a pair of adjacent parts (`CHECKCOMPOUNDDUP`, `CHECKCOMPOUNDPATTERN`).
    fn pair_ok(&self, previous: Option<&Part>, next: &Part) -> bool {
        let Some(previous) = previous else {
            return true;
        };
        let aff = &self.aff;
        if aff.check_compound_dup && previous.text == next.text {
            return false;
        }
        for pattern in &aff.compound_patterns {
            let flags_match = pattern.end_flag.is_none_or(|f| previous.root.contains(f))
                && pattern.begin_flag.is_none_or(|f| next.root.contains(f));
            if flags_match
                && previous.text.ends_with(&pattern.end)
                && next.text.starts_with(&pattern.begin)
            {
                return false;
            }
        }
        true
    }

    /// Final acceptance of a complete decomposition.
    fn compound_complete(&self, parts: &[Part], chars: &[char]) -> bool {
        if parts.len() < 2 {
            return false;
        }
        let by_flags = parts.iter().all(|p| p.flag_ok);
        let by_rule = !by_flags
            && parts.iter().all(|p| p.rule_ok)
            && self.aff.compound_rules.iter().any(|rule| {
                let flags: Vec<&FlagSet> = parts.iter().map(|p| &*p.root).collect();
                rule_matches(rule, &flags)
            });
        if !(by_flags || by_rule) {
            return false;
        }
        if let Some(last) = parts.last()
            && last.root.has(self.aff.force_ucase)
        {
            let word: String = chars.iter().collect();
            if !matches!(self.caser.kind(&word), CaseKind::Title | CaseKind::Upper) {
                return false;
            }
        }
        true
    }

    /// Whether an analysis may serve as a compound part at `position` via compound flags.
    fn part_flag_ok(&self, a: &Analysis<'_>, position: Position, opts: LookupOpts) -> bool {
        let aff = &self.aff;
        if a.any_has(aff.forbidden) || (opts.suggest && a.any_has(aff.nosuggest)) {
            return false;
        }
        if a.root.has(aff.needaffix) && a.rule_count() == 0 {
            return false;
        }
        let positional = match position {
            Position::Begin => aff.compound_begin,
            Position::Middle => aff.compound_middle,
            Position::End => aff.compound_end,
        };
        if !(a.any_has(aff.compound_flag) || a.any_has(positional)) {
            return false;
        }
        if let Some(prefix) = a.prefix {
            let allowed = position == Position::Begin || prefix.cont.has(aff.compound_permit);
            if !allowed || prefix.cont.has(aff.compound_forbid) {
                return false;
            }
        }
        for suffix in a.suffixes.iter().flatten() {
            let allowed = position == Position::End || suffix.cont.has(aff.compound_permit);
            if !allowed || suffix.cont.has(aff.compound_forbid) {
                return false;
            }
        }
        true
    }
}

/// Calls `found(root, rule)` for every rule of `table` that can be stripped from `word`.
fn for_each_stripped<'t>(
    table: &'t AffixTable,
    word: &str,
    prefix: bool,
    mut found: impl FnMut(String, &'t AffixRule),
) {
    if table.rules.is_empty() {
        return;
    }
    let mut try_append = |append: &str| {
        let Some(indices) = table.by_append.get(append) else {
            return;
        };
        let rest = if prefix {
            word.strip_prefix(append)
        } else {
            word.strip_suffix(append)
        };
        let Some(rest) = rest else {
            return;
        };
        for &index in indices {
            let rule = &table.rules[index];
            let root = if prefix {
                format!("{}{rest}", rule.strip)
            } else {
                format!("{rest}{}", rule.strip)
            };
            if root.is_empty() {
                continue;
            }
            let cond_ok = if prefix {
                rule.cond.matches_start(&root)
            } else {
                rule.cond.matches_end(&root)
            };
            if cond_ok {
                found(root, rule);
            }
        }
    };
    try_append("");
    let boundaries: Vec<usize> = word.char_indices().map(|(i, _)| i).collect();
    for taken in 1..=table.max_append_chars.min(boundaries.len()) {
        if prefix {
            let end = boundaries.get(taken).copied().unwrap_or(word.len());
            try_append(&word[..end]);
        } else {
            try_append(&word[boundaries[boundaries.len() - taken]..]);
        }
    }
}

/// Matches a sequence of part flag sets against a `COMPOUNDRULE` pattern.
fn rule_matches(rule: &[RuleItem], parts: &[&FlagSet]) -> bool {
    let Some((item, rest)) = rule.split_first() else {
        return parts.is_empty();
    };
    let matches_item = |part: &FlagSet| part.contains(item.flag);
    match item.quant {
        Quant::One => {
            parts.first().is_some_and(|p| matches_item(p)) && rule_matches(rest, &parts[1..])
        }
        Quant::Opt => {
            rule_matches(rest, parts)
                || (parts.first().is_some_and(|p| matches_item(p))
                    && rule_matches(rest, &parts[1..]))
        }
        Quant::Star => {
            if rule_matches(rest, parts) {
                return true;
            }
            let mut taken = 0;
            while parts.get(taken).is_some_and(|p| matches_item(p)) {
                taken += 1;
                if rule_matches(rest, &parts[taken..]) {
                    return true;
                }
            }
            false
        }
    }
}

/// Numbers, with `.`, `,` and `-` separators, are always acceptable.
fn is_number(word: &str) -> bool {
    word.chars().any(|c| c.is_numeric())
        && word
            .chars()
            .all(|c| c.is_numeric() || matches!(c, '.' | ',' | '-' | '\u{2212}'))
}
