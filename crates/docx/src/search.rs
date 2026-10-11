//! Find and replace with Word's semantics.
//!
//! Matching runs over the text of one paragraph, so a match spans runs (and hyperlinks and
//! other groups) freely but never a paragraph mark. A replacement keeps the formatting of the
//! run holding the first matched character, as Word does.
//!
//! Supported syntax (the rules were measured on Word, see `tools/office/find-oracle.ps1`):
//!
//! * Plain search: case-insensitive unless `match_case`; `whole_word` requires non-word
//!   characters (anything that is not a letter or digit) or the paragraph edge on both sides.
//!   `^t` is a tab, `^l` a line break, `^^` a caret.
//! * Wildcards: `?` one character, `*` any run of characters (shortest first), `[abc]`,
//!   `[a-z]`, `[!abc]` classes, `<` and `>` word start and end, `@` one or more of the previous
//!   item (Word repeats lazily: `x[0-9]@` alone matches `x1`, `x[0-9]@ ` matches `x123 `),
//!   `{n}`, `{n,}`, `{,m}`, `{n,m}` repeat counts (greedy), and `\` to make the next character
//!   literal. Wildcard searches are case-sensitive. Groups `( )`, `^p` and the other special
//!   codes are not supported and are rejected rather than guessed at. Unlike Word, `*` stops
//!   at the paragraph end (Word's star runs on through paragraph marks and merges paragraphs).
//! * Replacement text: `^&` is the found text (every copy keeps the formatting the found
//!   characters had), `^t` a tab, `^l` a line break, `^^` a caret. With a plain search that
//!   ignores case, the replacement takes the found text's capitalisation: an all-capitals
//!   match gives an all-capitals replacement, a match that starts with a capital gives a
//!   replacement that does.

use std::ops::Range;

use crate::model::{Inline, Paragraph};
use crate::props::RunProps;
use crate::{DocPos, Document, DocxError};

/// What to look for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FindOptions {
    pub pattern: String,
    pub match_case: bool,
    pub whole_word: bool,
    pub wildcards: bool,
}

/// A match: the half-open character range `start..end` of paragraph `para`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DocMatch {
    pub para: usize,
    pub start: usize,
    pub end: usize,
}

/// Why a search or replace could not run.
#[derive(Debug, thiserror::Error)]
pub enum FindError {
    #[error("nothing to find")]
    Empty,
    #[error("search pattern: {0}")]
    Pattern(String),
    #[error("replacement text: {0}")]
    Replacement(String),
    #[error(transparent)]
    Edit(#[from] DocxError),
}

/// Word treats letters and digits as word characters.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric()
}

fn fold(c: char) -> char {
    let mut it = c.to_lowercase();
    match (it.next(), it.next()) {
        (Some(l), None) => l,
        _ => c,
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Atom {
    Lit(char),
    Any,
    Class {
        negated: bool,
        ranges: Vec<(char, char)>,
    },
    WordStart,
    WordEnd,
    Star,
}

#[derive(Debug, Clone, PartialEq)]
struct Item {
    atom: Atom,
    min: usize,
    max: usize,
    /// `@` repeats lazily, as Word does: the fewest repeats that let the rest match.
    lazy: bool,
}

const UNBOUNDED: usize = usize::MAX;

enum Compiled {
    Plain(Vec<char>),
    Wild(Vec<Item>),
}

/// Reads the `^x` code after a caret in a search pattern or a replacement.
fn caret_code(next: Option<char>, what: &str) -> Result<char, String> {
    match next {
        Some('t') => Ok('\t'),
        Some('l') => Ok('\n'),
        Some('^') => Ok('^'),
        Some(other) => Err(format!(
            "the special code ^{other} is not supported in {what}"
        )),
        None => Err("a pattern cannot end with ^".to_owned()),
    }
}

fn compile(opts: &FindOptions) -> Result<Compiled, FindError> {
    if opts.pattern.is_empty() {
        return Err(FindError::Empty);
    }
    let chars: Vec<char> = opts.pattern.chars().collect();
    if !opts.wildcards {
        let mut out = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '^' {
                out.push(
                    caret_code(chars.get(i + 1).copied(), "a search")
                        .map_err(FindError::Pattern)?,
                );
                i += 2;
            } else {
                out.push(chars[i]);
                i += 1;
            }
        }
        return Ok(Compiled::Plain(out));
    }
    let bad = |m: &str| FindError::Pattern(m.to_owned());
    let mut items: Vec<Item> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        i += 1;
        let atom = match c {
            '\\' => {
                let lit = *chars
                    .get(i)
                    .ok_or_else(|| bad("a pattern cannot end with \\"))?;
                i += 1;
                Atom::Lit(lit)
            }
            '^' => {
                let lit = caret_code(chars.get(i).copied(), "a wildcard search")
                    .map_err(FindError::Pattern)?;
                i += 1;
                Atom::Lit(lit)
            }
            '?' => Atom::Any,
            '*' => Atom::Star,
            '<' => Atom::WordStart,
            '>' => Atom::WordEnd,
            '(' | ')' => return Err(bad("groups ( ) are not supported")),
            '@' => {
                let last = items
                    .last_mut()
                    .ok_or_else(|| bad("@ needs an item before it"))?;
                if last.atom == Atom::Star || last.max != 1 || last.min != 1 {
                    return Err(bad("@ cannot follow that item"));
                }
                last.max = UNBOUNDED;
                last.lazy = true;
                continue;
            }
            '{' => {
                let close = chars[i..]
                    .iter()
                    .position(|&c| c == '}')
                    .ok_or_else(|| bad("missing }"))?;
                let body: String = chars[i..i + close].iter().collect();
                i += close + 1;
                let (min, max) = parse_count(&body).ok_or_else(|| bad("invalid {n,m} count"))?;
                let last = items
                    .last_mut()
                    .ok_or_else(|| bad("{n,m} needs an item before it"))?;
                if last.atom == Atom::Star || last.max != 1 || last.min != 1 {
                    return Err(bad("{n,m} cannot follow that item"));
                }
                last.min = min;
                last.max = max;
                continue;
            }
            '[' => {
                let mut negated = false;
                if chars.get(i) == Some(&'!') {
                    negated = true;
                    i += 1;
                }
                let mut ranges = Vec::new();
                loop {
                    let Some(&ch) = chars.get(i) else {
                        return Err(bad("missing ]"));
                    };
                    i += 1;
                    if ch == ']' && !ranges.is_empty() {
                        break;
                    }
                    let lo = if ch == '\\' {
                        let esc = *chars.get(i).ok_or_else(|| bad("missing ]"))?;
                        i += 1;
                        esc
                    } else {
                        ch
                    };
                    if chars.get(i) == Some(&'-') && chars.get(i + 1).is_some_and(|&n| n != ']') {
                        let mut hi = chars[i + 1];
                        i += 2;
                        if hi == '\\' {
                            hi = *chars.get(i).ok_or_else(|| bad("missing ]"))?;
                            i += 1;
                        }
                        if hi < lo {
                            return Err(bad("a range in [ ] runs backwards"));
                        }
                        ranges.push((lo, hi));
                    } else {
                        ranges.push((lo, lo));
                    }
                }
                Atom::Class { negated, ranges }
            }
            other => Atom::Lit(other),
        };
        let (min, max) = if atom == Atom::Star {
            (0, UNBOUNDED)
        } else {
            (1, 1)
        };
        items.push(Item {
            atom,
            min,
            max,
            lazy: false,
        });
    }
    Ok(Compiled::Wild(items))
}

fn parse_count(body: &str) -> Option<(usize, usize)> {
    let num = |s: &str| s.trim().parse::<usize>().ok();
    match body.split_once(',') {
        None => num(body).map(|n| (n, n)),
        Some((lo, hi)) => {
            let min = if lo.trim().is_empty() { 0 } else { num(lo)? };
            let max = if hi.trim().is_empty() {
                UNBOUNDED
            } else {
                num(hi)?
            };
            (min <= max && max > 0).then_some((min, max))
        }
    }
}

/// Backtracking budget per paragraph, so a hostile pattern cannot hang the editor.
const STEP_BUDGET: usize = 2_000_000;

struct Matcher<'a> {
    items: &'a [Item],
    text: &'a [char],
    steps: usize,
}

impl Matcher<'_> {
    fn one(&self, atom: &Atom, pos: usize) -> bool {
        let Some(&c) = self.text.get(pos) else {
            return false;
        };
        match atom {
            Atom::Lit(l) => c == *l,
            Atom::Any => true,
            Atom::Class { negated, ranges } => {
                ranges.iter().any(|&(lo, hi)| (lo..=hi).contains(&c)) != *negated
            }
            Atom::WordStart | Atom::WordEnd | Atom::Star => false,
        }
    }

    fn at_word_start(&self, pos: usize) -> bool {
        self.text.get(pos).is_some_and(|&c| is_word_char(c))
            && (pos == 0 || !is_word_char(self.text[pos - 1]))
    }

    fn at_word_end(&self, pos: usize) -> bool {
        pos > 0
            && is_word_char(self.text[pos - 1])
            && self.text.get(pos).is_none_or(|&c| !is_word_char(c))
    }

    /// End of a match of `items[idx..]` that starts at `pos`.
    fn run(&mut self, idx: usize, pos: usize) -> Option<usize> {
        self.steps += 1;
        if self.steps > STEP_BUDGET {
            return None;
        }
        let items = self.items;
        let Some(item) = items.get(idx) else {
            return Some(pos);
        };
        match &item.atom {
            Atom::WordStart => self.at_word_start(pos).then(|| self.run(idx + 1, pos))?,
            Atom::WordEnd => self.at_word_end(pos).then(|| self.run(idx + 1, pos))?,
            Atom::Star => {
                // Shortest first.
                (pos..=self.text.len()).find_map(|end| self.run(idx + 1, end))
            }
            atom => {
                let mut count = 0;
                while count < item.max && self.one(atom, pos + count) {
                    count += 1;
                }
                if count < item.min {
                    return None;
                }
                if item.lazy {
                    (item.min..=count).find_map(|n| self.run(idx + 1, pos + n))
                } else {
                    (item.min..=count)
                        .rev()
                        .find_map(|n| self.run(idx + 1, pos + n))
                }
            }
        }
    }
}

/// All non-overlapping matches in `text`, left to right, as character ranges. Empty matches
/// are skipped.
///
/// # Errors
/// The pattern is empty or uses unsupported syntax.
pub fn find_in_text(text: &str, opts: &FindOptions) -> Result<Vec<(usize, usize)>, FindError> {
    let compiled = compile(opts)?;
    Ok(find_compiled(
        &compiled,
        &text.chars().collect::<Vec<_>>(),
        opts,
    ))
}

fn find_compiled(compiled: &Compiled, text: &[char], opts: &FindOptions) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0;
    match compiled {
        Compiled::Plain(pat) => {
            let folded: Vec<char> = if opts.match_case {
                pat.clone()
            } else {
                pat.iter().map(|&c| fold(c)).collect()
            };
            while start + folded.len() <= text.len() {
                let hit = folded.iter().enumerate().all(|(k, &p)| {
                    let c = text[start + k];
                    p == if opts.match_case { c } else { fold(c) }
                });
                let end = start + folded.len();
                let bounded = !opts.whole_word
                    || ((start == 0 || !is_word_char(text[start - 1]))
                        && (end == text.len() || !is_word_char(text[end])));
                if hit && bounded {
                    out.push((start, end));
                    start = end;
                } else {
                    start += 1;
                }
            }
        }
        Compiled::Wild(items) => {
            let mut matcher = Matcher {
                items,
                text,
                steps: 0,
            };
            while start < text.len() {
                match matcher.run(0, start) {
                    Some(end) if end > start => {
                        out.push((start, end));
                        start = end;
                    }
                    _ => start += 1,
                }
            }
        }
    }
    out
}

/// One piece of a replacement: literal text, or a copy of the found text.
#[derive(Debug, PartialEq, Eq)]
enum Part {
    Lit(String),
    Found,
}

/// Splits replacement text at its `^&` codes and resolves `^t`, `^l` and `^^`.
fn parse_replacement(replacement: &str) -> Result<Vec<Part>, FindError> {
    let mut parts = Vec::new();
    let mut lit = String::new();
    let mut chars = replacement.chars();
    while let Some(c) = chars.next() {
        if c != '^' {
            lit.push(c);
            continue;
        }
        match chars.next() {
            Some('&') => {
                if !lit.is_empty() {
                    parts.push(Part::Lit(std::mem::take(&mut lit)));
                }
                parts.push(Part::Found);
            }
            next => lit.push(caret_code(next, "a replacement").map_err(FindError::Replacement)?),
        }
    }
    if !lit.is_empty() {
        parts.push(Part::Lit(lit));
    }
    Ok(parts)
}

/// Word's case matching for a replacement when the search ignores case.
fn adapt_case(found: &str, replacement: &str) -> String {
    let mut letters = found
        .chars()
        .filter(|c| c.is_uppercase() || c.is_lowercase());
    let all_upper = letters.clone().next().is_some() && letters.all(char::is_uppercase);
    if all_upper {
        return replacement.to_uppercase();
    }
    if found.chars().next().is_some_and(char::is_uppercase) {
        let mut it = replacement.chars();
        if let Some(first) = it.next() {
            return first.to_uppercase().chain(it).collect();
        }
    }
    replacement.to_owned()
}

/// A replacement for one match: the new text and where the copies of the found text sit in it
/// (character offset, length).
struct Expansion {
    text: String,
    copies: Vec<(usize, usize)>,
}

fn expand(parts: &[Part], found: &str, adapt: bool) -> Expansion {
    let found_len = found.chars().count();
    let mut text = String::new();
    let mut copies = Vec::new();
    let mut at = 0;
    for part in parts {
        match part {
            Part::Lit(l) => {
                text.push_str(l);
                at += l.chars().count();
            }
            Part::Found => {
                copies.push((at, found_len));
                text.push_str(found);
                at += found_len;
            }
        }
    }
    if adapt {
        // Capitalisation only touches letters, and a copy of the found text already has the
        // capitals that triggered it, so the copy offsets stay valid.
        text = adapt_case(found, &text);
    }
    Expansion { text, copies }
}

fn split_runs_at(content: &mut Vec<Inline>, offset: usize, acc: &mut usize) -> bool {
    let mut i = 0;
    while i < content.len() {
        match &mut content[i] {
            Inline::Run(run) => {
                let len = run.char_len();
                if len > 0 && offset > *acc && offset < *acc + len {
                    let right = run.split_at(offset - *acc);
                    content.insert(i + 1, Inline::Run(right));
                    return true;
                }
                *acc += len;
            }
            Inline::Group(group) => {
                if split_runs_at(&mut group.content, offset, acc) {
                    return true;
                }
            }
            Inline::Raw(_) => {}
        }
        i += 1;
    }
    false
}

fn assign_props(content: &mut [Inline], range: &Range<usize>, props: &RunProps, acc: &mut usize) {
    for inline in content {
        match inline {
            Inline::Run(run) => {
                let len = run.char_len();
                if len > 0 && *acc >= range.start && *acc + len <= range.end {
                    run.props = props.clone();
                }
                *acc += len;
            }
            Inline::Group(group) => assign_props(&mut group.content, range, props, acc),
            Inline::Raw(_) => {}
        }
    }
}

/// Gives the characters of `range` exactly `props`, splitting runs at the ends.
fn set_exact_props(para: &mut Paragraph, range: Range<usize>, props: &RunProps) {
    for boundary in [range.start, range.end] {
        let mut acc = 0;
        split_runs_at(&mut para.content, boundary, &mut acc);
    }
    let mut acc = 0;
    assign_props(&mut para.content, &range, props, &mut acc);
}

impl Document {
    /// Every match in document order (paragraphs in tables included).
    ///
    /// # Errors
    /// The pattern is empty or unsupported.
    pub fn find_all(&self, opts: &FindOptions) -> Result<Vec<DocMatch>, FindError> {
        let compiled = compile(opts)?;
        let mut out = Vec::new();
        for para in 0..self.paragraph_count() {
            let Some(text) = self.paragraph_text(para) else {
                continue;
            };
            let chars: Vec<char> = text.chars().collect();
            out.extend(
                find_compiled(&compiled, &chars, opts)
                    .into_iter()
                    .map(|(start, end)| DocMatch { para, start, end }),
            );
        }
        Ok(out)
    }

    /// Replaces every match and returns how many were replaced. The replacement takes the
    /// formatting of the run holding the first matched character. A match whose edit the
    /// document refuses (it would cut a field in two) is left alone and not counted.
    ///
    /// # Errors
    /// The pattern or the replacement text is invalid; nothing changes then.
    pub fn replace_all(
        &mut self,
        opts: &FindOptions,
        replacement: &str,
    ) -> Result<usize, FindError> {
        // Validate the replacement before touching anything.
        let parts = parse_replacement(replacement)?;
        let adapt = !opts.wildcards && !opts.match_case;
        let matches = self.find_all(opts)?;
        let mut replaced = 0;
        // Right to left, so earlier offsets stay valid.
        for m in matches.iter().rev() {
            let found: String = self
                .paragraph_text(m.para)
                .unwrap_or_default()
                .chars()
                .skip(m.start)
                .take(m.end - m.start)
                .collect();
            let expansion = expand(&parts, &found, adapt);
            let snapshot = self.capture(m.para, m.para)?;
            if self.replace_one(m, &expansion).is_ok() {
                replaced += 1;
            } else {
                self.restore(&snapshot)?;
            }
        }
        Ok(replaced)
    }

    fn replace_one(&mut self, m: &DocMatch, new: &Expansion) -> Result<(), DocxError> {
        let at = |offset| DocPos::new(m.para, offset);
        if new.text.is_empty() {
            return self.delete_range(at(m.start), at(m.end));
        }
        // What the found characters looked like, for the copies of them in the replacement.
        let originals: Vec<RunProps> = if new.copies.is_empty() {
            Vec::new()
        } else {
            let count = self.paragraph_count();
            let para = self
                .paragraph(m.para)
                .ok_or(DocxError::ParagraphOutOfRange {
                    index: m.para,
                    count,
                })?;
            (m.start..m.end).map(|i| para.run_props_at(i + 1)).collect()
        };
        // Insert inside the first matched character's run, then drop the matched text around
        // the insertion: the new text keeps that run's formatting.
        if m.end > m.start + 1 {
            self.delete_range(at(m.start + 1), at(m.end))?;
        }
        self.insert_text(m.para, m.start + 1, &new.text)?;
        self.delete_range(at(m.start), at(m.start + 1))?;
        if !new.copies.is_empty() {
            let para = self.paragraph_mut(m.para)?;
            for &(offset, len) in &new.copies {
                let mut k = 0;
                while k < len {
                    // Characters that shared formatting are set in one go.
                    let mut j = k + 1;
                    while j < len && originals[j] == originals[k] {
                        j += 1;
                    }
                    let from = m.start + offset + k;
                    set_exact_props(para, from..from + (j - k), &originals[k]);
                    k = j;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(pattern: &str) -> FindOptions {
        FindOptions {
            pattern: pattern.to_owned(),
            ..FindOptions::default()
        }
    }

    fn texts(text: &str, o: &FindOptions) -> Vec<String> {
        find_in_text(text, o)
            .unwrap()
            .into_iter()
            .map(|(s, e)| text.chars().skip(s).take(e - s).collect())
            .collect()
    }

    #[test]
    fn plain_search_ignores_case_unless_asked() {
        let o = opts("cat");
        assert_eq!(texts("Cat cat CAT", &o).len(), 3);
        let o = FindOptions {
            match_case: true,
            ..o
        };
        assert_eq!(texts("Cat cat CAT", &o), ["cat"]);
    }

    #[test]
    fn whole_word_needs_non_word_neighbours() {
        let o = FindOptions {
            whole_word: true,
            ..opts("cat")
        };
        assert_eq!(texts("cat concat cat. cats (cat)", &o).len(), 3);
    }

    #[test]
    fn matches_do_not_overlap() {
        assert_eq!(texts("aaaa", &opts("aa")).len(), 2);
    }

    #[test]
    fn wildcards() {
        let w = |p: &str| FindOptions {
            wildcards: true,
            ..opts(p)
        };
        assert_eq!(texts("abc axc a c", &w("a?c")), ["abc", "axc", "a c"]);
        assert_eq!(texts("ab abab", &w("a*b")), ["ab", "ab", "ab"]);
        assert_eq!(texts("a1 b22 c333", &w("[a-c][0-9]{2,}")), ["b22", "c333"]);
        assert_eq!(texts("cat concat", &w("<cat>")), ["cat"]);
        assert_eq!(texts("ab abb a", &w("ab@")), ["ab", "ab"]);
        assert_eq!(texts("ab abb a", &w("ab@ ")), ["ab ", "abb "]);
        assert_eq!(texts("Ab ab", &w("[!A]b")), ["ab"]);
        assert_eq!(texts("a*b", &w("a\\*b")), ["a*b"]);
        assert_eq!(texts("Cat cat", &w("cat")), ["cat"]);
    }

    #[test]
    fn unsupported_syntax_is_rejected() {
        let w = FindOptions {
            wildcards: true,
            ..opts("(a)")
        };
        assert!(matches!(find_in_text("a", &w), Err(FindError::Pattern(_))));
        assert!(matches!(
            find_in_text("a", &opts("")),
            Err(FindError::Empty)
        ));
        assert!(matches!(
            find_in_text("a", &opts("^p")),
            Err(FindError::Pattern(_))
        ));
    }

    #[test]
    fn replacement_codes() {
        let parts = parse_replacement("[^&]^t").unwrap();
        let e = expand(&parts, "x", false);
        assert_eq!(e.text, "[x]\t");
        assert_eq!(e.copies, [(1, 1)]);
        assert!(parse_replacement("^p").is_err());
    }

    #[test]
    fn replacement_follows_the_found_text_capitalisation() {
        assert_eq!(adapt_case("cat", "dog"), "dog");
        assert_eq!(adapt_case("Cat", "dog"), "Dog");
        assert_eq!(adapt_case("CAT", "dog cat"), "DOG CAT");
        assert_eq!(adapt_case("A", "x"), "X");
        assert_eq!(adapt_case("WOrD", "dOg"), "DOg");
        assert_eq!(adapt_case("Cat", "1dog"), "1dog");
        assert_eq!(adapt_case("wOrd", "dog"), "dog");
    }
}
