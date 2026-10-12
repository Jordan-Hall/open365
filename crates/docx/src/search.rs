//! Find and replace with Word's semantics.
//!
//! Matching runs over the whole body as one text: each paragraph's text followed by a mark
//! character, so a match spans runs (and hyperlinks and other groups) freely and `^p` and `*`
//! cross paragraph boundaries as they do in Word. A replacement keeps the formatting of the
//! run holding the first matched character, as Word does.
//!
//! Supported syntax (the rules were measured on Word, see `tools/office/find-oracle.ps1`):
//!
//! * Plain search: case-insensitive unless `match_case`; `whole_word` requires non-word
//!   characters (anything that is not a letter or digit) or the text edge on both sides.
//!   Special codes: `^p` paragraph mark, `^t` tab, `^l` line break, `^m` page break, `^n`
//!   column break, `^b` section break, `^~` non-breaking hyphen, `^-` optional hyphen, `^s`
//!   non-breaking space, `^?` any character, `^#` any digit, `^$` any letter, `^w` white space
//!   (one or more spaces, tabs, non-breaking spaces and paragraph marks), `^^` a caret. The
//!   codes for footnote and endnote marks and fields (`^f`, `^e`, `^d`) are rejected: the text
//!   model holds no character for them.
//! * Wildcards: `?` one character, `*` any run of characters (shortest first, across
//!   paragraph marks), `[abc]`, `[a-z]`, `[!abc]` classes, `<` and `>` word start and end, `@`
//!   one or more of the previous item (Word repeats lazily: `x[0-9]@` alone matches `x1`,
//!   `x[0-9]@ ` matches `x123 `), `{n}`, `{n,}`, `{,m}`, `{n,m}` repeat counts (greedy),
//!   `( )` groups (up to nine, not nested, no repeat count on a group), `\1` to `\9` to repeat
//!   a group, and `\` to make the next character literal. The codes `^p ^t ^l ^m ^n ^b ^s ^~
//!   ^- ^^` also work. Wildcard searches are case-sensitive.
//! * Replacement text: `^&` is the found text (every copy keeps the formatting the found
//!   characters had), `^p` a paragraph mark, `^t` a tab, `^l` a line break, `^m` a page
//!   break, `^n` a column break, `^~` a non-breaking hyphen, `^-` an optional hyphen, `^s` a
//!   non-breaking space, `^c` the clipboard text, `^^` a caret; with wildcards `\1` to `\9`
//!   are the groups. With a plain search that ignores case, the replacement takes the found
//!   text's capitalisation: an all-capitals match gives an all-capitals replacement, a match
//!   that starts with a capital gives a replacement that does.

use std::ops::Range;

use crate::model::{Break, BreakKind, Inline, Paragraph, RunContent};
use crate::props::RunProps;
use crate::xml::RawXml;
use crate::{DocPos, Document, DocxError};

/// What to look for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FindOptions {
    pub pattern: String,
    pub match_case: bool,
    pub whole_word: bool,
    pub wildcards: bool,
}

/// A match: from character `start` of paragraph `para` to character `end` of paragraph
/// `end_para` (half-open). Most matches sit in one paragraph (`end_para == para`); a match
/// that includes a paragraph mark ends in a later paragraph, at offset 0 when the mark is its
/// last character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DocMatch {
    pub para: usize,
    pub start: usize,
    pub end: usize,
    pub end_para: usize,
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

/// Paragraph mark in the search text.
const PARA: char = '\r';
/// Mark of a paragraph that ends a section.
const SECT: char = '\u{3}';
/// Manual page break.
const PAGE: char = '\u{c}';
/// Column break.
const COLUMN: char = '\u{e}';
const NB_SPACE: char = '\u{a0}';
const NB_HYPHEN: char = '\u{2011}';
const OPT_HYPHEN: char = '\u{ad}';

fn is_mark(c: char) -> bool {
    c == PARA || c == SECT
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
    Digit,
    Letter,
    White,
    Class {
        negated: bool,
        ranges: Vec<(char, char)>,
    },
    WordStart,
    WordEnd,
    Star,
    GroupStart(usize),
    GroupEnd(usize),
    BackRef(usize),
}

impl Atom {
    /// Atoms a repeat count (`@`, `{n,m}`) may follow.
    fn repeatable(&self) -> bool {
        !matches!(
            self,
            Self::Star
                | Self::WordStart
                | Self::WordEnd
                | Self::GroupStart(_)
                | Self::GroupEnd(_)
                | Self::BackRef(_)
        )
    }
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

impl Item {
    fn once(atom: Atom) -> Self {
        let (min, max) = if atom == Atom::Star {
            (0, UNBOUNDED)
        } else {
            (1, 1)
        };
        Self {
            atom,
            min,
            max,
            lazy: false,
        }
    }
}

struct Compiled {
    items: Vec<Item>,
    /// Literals are folded to lower case and the text is folded as it is compared.
    fold: bool,
    whole_word: bool,
    groups: usize,
}

const BAD_FIND: &str = "The Find What text contains a Pattern Match expression which is not valid.";
const BAD_REPLACE: &str =
    "The Replace With text contains a Pattern Match expression which is not valid.";

fn bad_find(detail: &str) -> FindError {
    FindError::Pattern(format!("{BAD_FIND} ({detail})"))
}

fn bad_replace(detail: &str) -> FindError {
    FindError::Replacement(format!("{BAD_REPLACE} ({detail})"))
}

/// The character a special code stands for; `None` for codes that are patterns (`^?`, `^#`,
/// `^$`, `^w`) and for unknown codes.
fn code_char(code: char) -> Option<char> {
    Some(match code {
        'p' => PARA,
        't' => '\t',
        'l' => '\n',
        'm' => PAGE,
        'n' => COLUMN,
        'b' => SECT,
        's' => NB_SPACE,
        '~' => NB_HYPHEN,
        '-' => OPT_HYPHEN,
        '^' => '^',
        _ => return None,
    })
}

fn compile_plain(opts: &FindOptions, chars: &[char]) -> Result<Compiled, FindError> {
    let fold_case = !opts.match_case;
    let lit = |c: char| Item::once(Atom::Lit(if fold_case { fold(c) } else { c }));
    let mut items = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        i += 1;
        if c != '^' {
            items.push(lit(c));
            continue;
        }
        let Some(&code) = chars.get(i) else {
            return Err(FindError::Pattern("a search cannot end with ^".to_owned()));
        };
        i += 1;
        let item = match code {
            '?' => Item::once(Atom::Any),
            '#' => Item::once(Atom::Digit),
            '$' => Item::once(Atom::Letter),
            'w' => Item {
                max: UNBOUNDED,
                ..Item::once(Atom::White)
            },
            'e' | 'f' | 'd' => {
                return Err(FindError::Pattern(format!(
                    "the special code ^{code} is not supported: this document model holds no \
                     text character for it"
                )));
            }
            other => match code_char(other) {
                Some(l) => lit(l),
                None => {
                    return Err(FindError::Pattern(format!(
                        "the special code ^{other} is not valid in a search"
                    )));
                }
            },
        };
        items.push(item);
    }
    Ok(Compiled {
        items,
        fold: fold_case,
        whole_word: opts.whole_word,
        groups: 0,
    })
}

fn compile(opts: &FindOptions) -> Result<Compiled, FindError> {
    if opts.pattern.is_empty() {
        return Err(FindError::Empty);
    }
    let chars: Vec<char> = opts.pattern.chars().collect();
    if !opts.wildcards {
        return compile_plain(opts, &chars);
    }
    let mut items: Vec<Item> = Vec::new();
    let mut groups = 0usize;
    let mut open: Option<usize> = None;
    let mut closed = 0usize;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        i += 1;
        let atom = match c {
            '\\' => {
                let lit = *chars
                    .get(i)
                    .ok_or_else(|| bad_find("a pattern cannot end with \\"))?;
                i += 1;
                match lit.to_digit(10) {
                    Some(n @ 1..=9) => {
                        let n = n as usize;
                        if n > closed {
                            return Err(bad_find(&format!(
                                "\\{n} refers to a group that is not closed yet"
                            )));
                        }
                        Atom::BackRef(n)
                    }
                    _ => Atom::Lit(lit),
                }
            }
            '^' => {
                let code = *chars
                    .get(i)
                    .ok_or_else(|| bad_find("a pattern cannot end with ^"))?;
                i += 1;
                let lit = code_char(code).ok_or_else(|| {
                    bad_find(&format!(
                        "the special code ^{code} cannot be used with wildcards"
                    ))
                })?;
                Atom::Lit(lit)
            }
            '?' => Atom::Any,
            '*' => Atom::Star,
            '<' => Atom::WordStart,
            '>' => Atom::WordEnd,
            '(' => {
                if open.is_some() {
                    return Err(bad_find("groups cannot be nested"));
                }
                groups += 1;
                if groups > 9 {
                    return Err(bad_find("more than nine groups"));
                }
                open = Some(groups);
                Atom::GroupStart(groups)
            }
            ')' => {
                let n = open
                    .take()
                    .ok_or_else(|| bad_find("a ) has no ( before it"))?;
                if matches!(items.last(), Some(Item { atom: Atom::GroupStart(g), .. }) if *g == n) {
                    return Err(bad_find("an empty group"));
                }
                closed = n;
                Atom::GroupEnd(n)
            }
            '@' => {
                let last = items
                    .last_mut()
                    .ok_or_else(|| bad_find("@ needs an item before it"))?;
                if !last.atom.repeatable() || last.max != 1 || last.min != 1 {
                    return Err(bad_find("@ cannot follow that item"));
                }
                last.max = UNBOUNDED;
                last.lazy = true;
                continue;
            }
            '{' => {
                let close = chars[i..]
                    .iter()
                    .position(|&c| c == '}')
                    .ok_or_else(|| bad_find("missing }"))?;
                let body: String = chars[i..i + close].iter().collect();
                i += close + 1;
                let (min, max) =
                    parse_count(&body).ok_or_else(|| bad_find("invalid {n,m} count"))?;
                let last = items
                    .last_mut()
                    .ok_or_else(|| bad_find("{n,m} needs an item before it"))?;
                if !last.atom.repeatable() || last.max != 1 || last.min != 1 {
                    return Err(bad_find("{n,m} cannot follow that item"));
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
                        return Err(bad_find("missing ]"));
                    };
                    i += 1;
                    if ch == ']' && !ranges.is_empty() {
                        break;
                    }
                    let lo = if ch == '\\' {
                        let esc = *chars.get(i).ok_or_else(|| bad_find("missing ]"))?;
                        i += 1;
                        esc
                    } else {
                        ch
                    };
                    if chars.get(i) == Some(&'-') && chars.get(i + 1).is_some_and(|&n| n != ']') {
                        let mut hi = chars[i + 1];
                        i += 2;
                        if hi == '\\' {
                            hi = *chars.get(i).ok_or_else(|| bad_find("missing ]"))?;
                            i += 1;
                        }
                        if hi < lo {
                            return Err(bad_find("a range in [ ] runs backwards"));
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
        items.push(Item::once(atom));
    }
    if open.is_some() {
        return Err(bad_find("a ( has no ) after it"));
    }
    Ok(Compiled {
        items,
        fold: false,
        whole_word: false,
        groups,
    })
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

/// Backtracking budget per start position, so a hostile pattern cannot hang the editor.
const STEP_BUDGET: usize = 2_000_000;
/// Budget for a whole search; a search that runs out stops with the matches found so far.
const TOTAL_BUDGET: usize = 60_000_000;

type Span = Option<(usize, usize)>;

struct Matcher<'a> {
    items: &'a [Item],
    text: &'a [char],
    fold: bool,
    steps: usize,
    caps: [Span; 10],
    open: [usize; 10],
}

impl<'a> Matcher<'a> {
    fn new(compiled: &'a Compiled, text: &'a [char]) -> Self {
        Self {
            items: &compiled.items,
            text,
            fold: compiled.fold,
            steps: 0,
            caps: [None; 10],
            open: [0; 10],
        }
    }

    fn reset(&mut self) {
        self.steps = 0;
        self.caps = [None; 10];
    }

    fn one(&self, atom: &Atom, pos: usize) -> bool {
        let Some(&c) = self.text.get(pos) else {
            return false;
        };
        match atom {
            Atom::Lit(l) => {
                if self.fold {
                    fold(c) == *l
                } else {
                    c == *l
                }
            }
            Atom::Any => !is_mark(c),
            Atom::Digit => c.is_ascii_digit(),
            Atom::Letter => c.is_alphabetic(),
            Atom::White => matches!(c, ' ' | '\t' | NB_SPACE) || is_mark(c),
            Atom::Class { negated, ranges } => {
                let inside = ranges.iter().any(|&(lo, hi)| (lo..=hi).contains(&c));
                if *negated {
                    !inside && !is_mark(c)
                } else {
                    inside
                }
            }
            Atom::WordStart
            | Atom::WordEnd
            | Atom::Star
            | Atom::GroupStart(_)
            | Atom::GroupEnd(_)
            | Atom::BackRef(_) => false,
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
            Atom::GroupStart(n) => {
                let old = self.open[*n];
                self.open[*n] = pos;
                let end = self.run(idx + 1, pos);
                if end.is_none() {
                    self.open[*n] = old;
                }
                end
            }
            Atom::GroupEnd(n) => {
                let old = self.caps[*n];
                self.caps[*n] = Some((self.open[*n], pos));
                let end = self.run(idx + 1, pos);
                if end.is_none() {
                    self.caps[*n] = old;
                }
                end
            }
            Atom::BackRef(n) => {
                let (s, e) = self.caps[*n]?;
                let len = e - s;
                let same = self.text.get(pos..pos + len)? == &self.text[s..e];
                same.then(|| self.run(idx + 1, pos + len))?
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

/// A match in the search text: character range and the span of each group.
#[derive(Debug, Clone, Copy)]
struct Hit {
    start: usize,
    end: usize,
    groups: [Span; 10],
}

fn bounded(text: &[char], start: usize, end: usize) -> bool {
    (start == 0 || !is_word_char(text[start - 1]))
        && (end == text.len() || !is_word_char(text[end]))
}

/// The match that starts exactly at `start`, if any.
fn match_at(compiled: &Compiled, text: &[char], start: usize) -> Option<Hit> {
    let mut matcher = Matcher::new(compiled, text);
    let end = matcher.run(0, start)?;
    (end > start && (!compiled.whole_word || bounded(text, start, end))).then_some(Hit {
        start,
        end,
        groups: matcher.caps,
    })
}

/// All non-overlapping matches in `text`, left to right. Empty matches are skipped.
fn find_compiled(compiled: &Compiled, text: &[char]) -> Vec<Hit> {
    let mut out = Vec::new();
    let mut matcher = Matcher::new(compiled, text);
    let mut start = 0;
    let mut total = 0usize;
    while start < text.len() {
        matcher.reset();
        let found = matcher.run(0, start);
        total += matcher.steps;
        match found {
            Some(end) if end > start && (!compiled.whole_word || bounded(text, start, end)) => {
                out.push(Hit {
                    start,
                    end,
                    groups: matcher.caps,
                });
                start = end;
            }
            _ => start += 1,
        }
        if total > TOTAL_BUDGET {
            break;
        }
    }
    out
}

/// All non-overlapping matches in `text`, left to right, as character ranges. Empty matches
/// are skipped.
///
/// # Errors
/// The pattern is empty or invalid.
pub fn find_in_text(text: &str, opts: &FindOptions) -> Result<Vec<(usize, usize)>, FindError> {
    let compiled = compile(opts)?;
    Ok(find_compiled(&compiled, &text.chars().collect::<Vec<_>>())
        .into_iter()
        .map(|h| (h.start, h.end))
        .collect())
}

/// One piece of a replacement: literal text, a copy of the found text, or a group's text.
#[derive(Debug, PartialEq, Eq)]
enum Part {
    Lit(String),
    Found,
    Group(usize),
}

/// The clipboard text as replacement text: line breaks become paragraph marks, control
/// characters go.
fn clipboard_text(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push(PARA);
            }
            '\n' => out.push(PARA),
            '\t' => out.push('\t'),
            c if (c as u32) < 0x20 || matches!(c, '\u{fffe}' | '\u{ffff}') => {}
            c => out.push(c),
        }
    }
    out
}

/// Splits replacement text at its `^&` and `\n` codes and resolves the other codes.
fn parse_replacement(
    replacement: &str,
    wildcards: bool,
    groups: usize,
    clipboard: Option<&str>,
) -> Result<Vec<Part>, FindError> {
    let mut parts = Vec::new();
    let mut lit = String::new();
    let mut chars = replacement.chars().peekable();
    let flush = |lit: &mut String, parts: &mut Vec<Part>| {
        if !lit.is_empty() {
            parts.push(Part::Lit(std::mem::take(lit)));
        }
    };
    while let Some(c) = chars.next() {
        if c == '\\' && wildcards {
            match chars.peek().and_then(|d| d.to_digit(10)) {
                Some(n @ 1..=9) => {
                    chars.next();
                    let n = n as usize;
                    if n > groups {
                        return Err(bad_replace(&format!(
                            "\\{n} refers to a group the search lacks"
                        )));
                    }
                    flush(&mut lit, &mut parts);
                    parts.push(Part::Group(n));
                }
                _ => {
                    if chars.peek() == Some(&'\\') {
                        chars.next();
                    }
                    lit.push('\\');
                }
            }
            continue;
        }
        if c != '^' {
            lit.push(c);
            continue;
        }
        let Some(code) = chars.next() else {
            return Err(FindError::Replacement(
                "a replacement cannot end with ^".to_owned(),
            ));
        };
        match code {
            '&' => {
                flush(&mut lit, &mut parts);
                parts.push(Part::Found);
            }
            'c' => {
                let clip = clipboard.ok_or_else(|| {
                    FindError::Replacement("the clipboard holds no text for ^c".to_owned())
                })?;
                lit.push_str(&clipboard_text(clip));
            }
            'p' | 't' | 'l' | 'm' | 'n' | 's' | '~' | '-' | '^' => {
                if let Some(l) = code_char(code) {
                    lit.push(l);
                }
            }
            other => {
                return Err(FindError::Replacement(format!(
                    "the special code ^{other} is not valid in a replacement"
                )));
            }
        }
    }
    flush(&mut lit, &mut parts);
    Ok(parts)
}

/// A replacement for one match: the new text and where the copies of the found text sit in it
/// (character offset, length). Paragraph marks, page breaks and column breaks are the mark
/// characters [`PARA`], [`PAGE`] and [`COLUMN`].
struct Expansion {
    text: String,
    copies: Vec<(usize, usize)>,
}

fn expand(parts: &[Part], found: &str, groups: &[String], adapt: bool) -> Expansion {
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
            Part::Group(n) => {
                let g = groups.get(n - 1).map_or("", String::as_str);
                text.push_str(g);
                at += g.chars().count();
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

/// A piece of replacement text to put into the document.
#[derive(Debug, PartialEq, Eq)]
enum Seg {
    Text(String),
    Para,
    Page,
    Column,
}

fn segments(text: &str) -> Vec<Seg> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in text.chars() {
        let seg = match c {
            PARA | SECT => Seg::Para,
            PAGE => Seg::Page,
            COLUMN => Seg::Column,
            c => {
                cur.push(c);
                continue;
            }
        };
        if !cur.is_empty() {
            out.push(Seg::Text(std::mem::take(&mut cur)));
        }
        out.push(seg);
    }
    if !cur.is_empty() {
        out.push(Seg::Text(cur));
    }
    out
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

fn push_chars(content: &[Inline], out: &mut Vec<char>) {
    for inline in content {
        match inline {
            Inline::Run(run) => {
                for item in &run.content {
                    match item {
                        RunContent::Text { text, .. } => out.extend(text.chars()),
                        RunContent::Tab => out.push('\t'),
                        RunContent::Break(b) => out.push(match b.kind() {
                            BreakKind::TextWrapping => '\n',
                            BreakKind::Page => PAGE,
                            BreakKind::Column => COLUMN,
                        }),
                        RunContent::Raw(_) => {}
                    }
                }
            }
            Inline::Group(group) => push_chars(&group.content, out),
            Inline::Raw(_) => {}
        }
    }
}

fn ends_section(p: &Paragraph) -> bool {
    p.props.as_ref().is_some_and(|ppr| {
        ppr.children
            .iter()
            .filter_map(RawXml::as_element)
            .any(|e| e.head.is_w("sectPr"))
    })
}

/// The body as one search text: every paragraph, then its mark.
struct Corpus {
    chars: Vec<char>,
    /// Index in `chars` of each paragraph's first character.
    bases: Vec<usize>,
    /// Characters of each paragraph, its mark not counted.
    lens: Vec<usize>,
}

impl Corpus {
    fn of(doc: &Document) -> Self {
        let mut chars = Vec::new();
        let mut bases = Vec::new();
        let mut lens = Vec::new();
        for p in doc.body().paragraphs() {
            let base = chars.len();
            bases.push(base);
            push_chars(&p.content, &mut chars);
            lens.push(chars.len() - base);
            chars.push(if ends_section(p) { SECT } else { PARA });
        }
        Self { chars, bases, lens }
    }

    /// The document position of search-text index `idx`. The index just past the last mark
    /// is the end of the last paragraph.
    fn pos(&self, idx: usize) -> DocPos {
        if self.bases.is_empty() {
            return DocPos::new(0, 0);
        }
        let k = self.bases.partition_point(|&b| b <= idx).saturating_sub(1);
        DocPos::new(k, (idx - self.bases[k]).min(self.lens[k]))
    }

    fn index(&self, pos: DocPos) -> Option<usize> {
        let base = *self.bases.get(pos.para)?;
        (pos.offset <= self.lens[pos.para]).then_some(base + pos.offset)
    }

    fn text(&self, range: Range<usize>) -> String {
        self.chars[range].iter().collect()
    }
}

/// Turns the break at character `target` of the paragraph into a page or column break.
/// `None`: not reached yet; `Some(done)`: the walk is over.
fn set_break_kind(
    content: &mut [Inline],
    target: usize,
    kind: BreakKind,
    acc: &mut usize,
) -> Option<bool> {
    for inline in content {
        match inline {
            Inline::Run(run) => {
                let owner = run.head.clone();
                for item in &mut run.content {
                    match item {
                        RunContent::Text { text, .. } => {
                            let n = text.chars().count();
                            if (*acc..*acc + n).contains(&target) {
                                return Some(false);
                            }
                            *acc += n;
                        }
                        RunContent::Tab => {
                            if *acc == target {
                                return Some(false);
                            }
                            *acc += 1;
                        }
                        RunContent::Break(b) => {
                            if *acc == target {
                                *b = Break::new(kind, &owner);
                                return Some(true);
                            }
                            *acc += 1;
                        }
                        RunContent::Raw(_) => {}
                    }
                }
            }
            Inline::Group(group) => {
                if let Some(done) = set_break_kind(&mut group.content, target, kind, acc) {
                    return Some(done);
                }
            }
            Inline::Raw(_) => {}
        }
    }
    None
}

fn corpus_expansion(corpus: &Corpus, hit: &Hit, parts: &[Part], adapt: bool) -> Expansion {
    let found = corpus.text(hit.start..hit.end);
    let groups: Vec<String> = (1..=9)
        .map(|n| hit.groups[n].map_or_else(String::new, |(s, e)| corpus.text(s..e)))
        .collect();
    expand(parts, &found, &groups, adapt)
}

fn doc_match(corpus: &Corpus, start: usize, end: usize) -> DocMatch {
    let (s, e) = (corpus.pos(start), corpus.pos(end));
    DocMatch {
        para: s.para,
        start: s.offset,
        end: e.offset,
        end_para: e.para,
    }
}

impl Document {
    /// Every match in document order (paragraphs in tables included). A match may span
    /// paragraphs (`^p`, `*`); see [`DocMatch`].
    ///
    /// # Errors
    /// The pattern is empty or invalid.
    pub fn find_all(&self, opts: &FindOptions) -> Result<Vec<DocMatch>, FindError> {
        let compiled = compile(opts)?;
        let corpus = Corpus::of(self);
        Ok(find_compiled(&compiled, &corpus.chars)
            .iter()
            .map(|h| doc_match(&corpus, h.start, h.end))
            .collect())
    }

    /// Replaces every match and returns how many were replaced. The replacement takes the
    /// formatting of the run holding the first matched character. A match whose edit the
    /// document refuses (it would cut a field in two, or span table cells) is left alone and
    /// not counted.
    ///
    /// # Errors
    /// The pattern or the replacement text is invalid; nothing changes then.
    pub fn replace_all(
        &mut self,
        opts: &FindOptions,
        replacement: &str,
    ) -> Result<usize, FindError> {
        self.replace_all_with_clipboard(opts, replacement, None)
    }

    /// [`Document::replace_all`] where `^c` in the replacement is `clipboard`.
    ///
    /// # Errors
    /// The pattern or the replacement text is invalid (`^c` without a clipboard text too);
    /// nothing changes then.
    pub fn replace_all_with_clipboard(
        &mut self,
        opts: &FindOptions,
        replacement: &str,
        clipboard: Option<&str>,
    ) -> Result<usize, FindError> {
        // Validate the replacement before touching anything.
        let compiled = compile(opts)?;
        let parts = parse_replacement(replacement, opts.wildcards, compiled.groups, clipboard)?;
        let adapt = !opts.wildcards && !opts.match_case;
        let corpus = Corpus::of(self);
        let hits = find_compiled(&compiled, &corpus.chars);
        let mut replaced = 0;
        // Right to left, so earlier offsets stay valid.
        for hit in hits.iter().rev() {
            let (s, e) = (corpus.pos(hit.start), corpus.pos(hit.end));
            let Ok(snapshot) = self.capture(s.para, e.para) else {
                continue;
            };
            if self.apply_hit(&corpus, hit, &parts, adapt).is_ok() {
                replaced += 1;
            } else {
                self.restore(&snapshot)?;
            }
        }
        Ok(replaced)
    }

    /// Replaces the one match `m` (as [`Document::find_all`] gave it) and returns the position
    /// just after the new text. The text is checked first: when `m` no longer matches the
    /// pattern nothing changes. `^p` in the replacement splits the paragraph.
    ///
    /// # Errors
    /// The pattern or the replacement is invalid, or `m` is stale, or the document refuses the
    /// edit (the match spans table cells); the document is unchanged then.
    pub fn replace_match(
        &mut self,
        opts: &FindOptions,
        replacement: &str,
        clipboard: Option<&str>,
        m: &DocMatch,
    ) -> Result<DocPos, FindError> {
        let (parts, corpus, hit) = self.resolve_match(opts, replacement, clipboard, m)?;
        let first = corpus.pos(hit.start);
        let last = corpus.pos(hit.end);
        let snapshot = self.capture(first.para, last.para)?;
        let adapt = !opts.wildcards && !opts.match_case;
        match self.apply_hit(&corpus, &hit, &parts, adapt) {
            Ok(caret) => Ok(caret),
            Err(e) => {
                self.restore(&snapshot)?;
                Err(e.into())
            }
        }
    }

    /// The text [`Document::replace_match`] would put in place of `m`: a paragraph mark is
    /// `\r`, a page break `\u{c}`, a column break `\u{e}`, a line break `\n`, a tab `\t`.
    ///
    /// # Errors
    /// As [`Document::replace_match`].
    pub fn replacement_text(
        &self,
        opts: &FindOptions,
        replacement: &str,
        clipboard: Option<&str>,
        m: &DocMatch,
    ) -> Result<String, FindError> {
        let (parts, corpus, hit) = self.resolve_match(opts, replacement, clipboard, m)?;
        let adapt = !opts.wildcards && !opts.match_case;
        Ok(corpus_expansion(&corpus, &hit, &parts, adapt).text)
    }

    /// Checks `m` still matches; gives the parsed replacement, the search text and the hit.
    fn resolve_match(
        &self,
        opts: &FindOptions,
        replacement: &str,
        clipboard: Option<&str>,
        m: &DocMatch,
    ) -> Result<(Vec<Part>, Corpus, Hit), FindError> {
        let compiled = compile(opts)?;
        let parts = parse_replacement(replacement, opts.wildcards, compiled.groups, clipboard)?;
        let corpus = Corpus::of(self);
        let stale = || {
            FindError::Edit(DocxError::UnsupportedRange(
                "the text at that place no longer matches".to_owned(),
            ))
        };
        let start = corpus
            .index(DocPos::new(m.para, m.start))
            .ok_or_else(stale)?;
        let hit = match_at(&compiled, &corpus.chars, start).ok_or_else(stale)?;
        if corpus.pos(hit.end) != DocPos::new(m.end_para, m.end) {
            return Err(stale());
        }
        Ok((parts, corpus, hit))
    }

    /// Replaces `hit`; gives the position after the new text.
    fn apply_hit(
        &mut self,
        corpus: &Corpus,
        hit: &Hit,
        parts: &[Part],
        adapt: bool,
    ) -> Result<DocPos, DocxError> {
        if hit.end == corpus.chars.len() {
            return Err(DocxError::UnsupportedRange(
                "the last paragraph mark of the document cannot be replaced".to_owned(),
            ));
        }
        let new = corpus_expansion(corpus, hit, parts, adapt);
        let s = corpus.pos(hit.start);
        let e = corpus.pos(hit.end);
        let simple = s.para == e.para && !new.text.contains([PARA, SECT]);
        if new.text.is_empty() {
            return self.delete_range(s, e).map(|()| s);
        }
        if simple {
            self.replace_in_paragraph(s.para, s.offset..e.offset, &new)?;
            return Ok(DocPos::new(s.para, s.offset + new.text.chars().count()));
        }
        self.replace_across(s, e, corpus.lens[s.para], &new.text)
    }

    /// A match inside one paragraph: the new text goes into the first matched character's run
    /// and keeps its formatting; copies of the found text keep theirs.
    fn replace_in_paragraph(
        &mut self,
        para: usize,
        range: Range<usize>,
        new: &Expansion,
    ) -> Result<(), DocxError> {
        let at = |offset| DocPos::new(para, offset);
        // What the found characters looked like, for the copies of them in the replacement.
        let originals: Vec<RunProps> = if new.copies.is_empty() {
            Vec::new()
        } else {
            let count = self.paragraph_count();
            let p = self
                .paragraph(para)
                .ok_or(DocxError::ParagraphOutOfRange { index: para, count })?;
            range.clone().map(|i| p.run_props_at(i + 1)).collect()
        };
        // Insert inside the first matched character's run, then drop the matched text around
        // the insertion: the new text keeps that run's formatting.
        if range.end > range.start + 1 {
            self.delete_range(at(range.start + 1), at(range.end))?;
        }
        let plain: String = new
            .text
            .chars()
            .map(|c| if c == PAGE || c == COLUMN { '\n' } else { c })
            .collect();
        self.insert_text(para, range.start + 1, &plain)?;
        for (k, c) in new.text.chars().enumerate() {
            if c == PAGE || c == COLUMN {
                let kind = if c == PAGE {
                    BreakKind::Page
                } else {
                    BreakKind::Column
                };
                let p = self.paragraph_mut(para)?;
                set_break_kind(&mut p.content, range.start + 1 + k, kind, &mut 0);
            }
        }
        self.delete_range(at(range.start), at(range.start + 1))?;
        if !new.copies.is_empty() {
            let p = self.paragraph_mut(para)?;
            for &(offset, len) in &new.copies {
                let mut k = 0;
                while k < len {
                    // Characters that shared formatting are set in one go.
                    let mut j = k + 1;
                    while j < len && originals[j] == originals[k] {
                        j += 1;
                    }
                    let from = range.start + offset + k;
                    set_exact_props(p, from..from + (j - k), &originals[k]);
                    k = j;
                }
            }
        }
        self.edited();
        Ok(())
    }

    /// A match that includes paragraph marks, or a replacement that makes some: delete the
    /// match, then put the new text in piece by piece (`^p` splits the paragraph).
    fn replace_across(
        &mut self,
        s: DocPos,
        e: DocPos,
        start_len: usize,
        text: &str,
    ) -> Result<DocPos, DocxError> {
        let segs = segments(text);
        let mut cur: DocPos;
        if s.offset < start_len {
            // The new text goes in after the first matched character (so it takes that
            // character's formatting), which is then dropped.
            let after = DocPos::new(s.para, s.offset + 1);
            if e > after {
                self.delete_range(after, e)?;
            }
            cur = after;
            self.insert_segs(&mut cur, &segs)?;
            self.delete_range(s, after)?;
            if cur.para == s.para {
                cur.offset -= 1;
            }
        } else {
            if e > s {
                self.delete_range(s, e)?;
            }
            cur = s;
            self.insert_segs(&mut cur, &segs)?;
        }
        Ok(cur)
    }

    fn insert_segs(&mut self, cur: &mut DocPos, segs: &[Seg]) -> Result<(), DocxError> {
        for seg in segs {
            match seg {
                Seg::Text(t) => {
                    self.insert_text(cur.para, cur.offset, t)?;
                    cur.offset += t.chars().count();
                }
                Seg::Para => {
                    self.split_paragraph(*cur)?;
                    *cur = DocPos::new(cur.para + 1, 0);
                }
                Seg::Page | Seg::Column => {
                    self.insert_text(cur.para, cur.offset, "\n")?;
                    let kind = if *seg == Seg::Page {
                        BreakKind::Page
                    } else {
                        BreakKind::Column
                    };
                    let p = self.paragraph_mut(cur.para)?;
                    set_break_kind(&mut p.content, cur.offset, kind, &mut 0);
                    cur.offset += 1;
                }
            }
        }
        self.edited();
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

    fn wild(pattern: &str) -> FindOptions {
        FindOptions {
            wildcards: true,
            ..opts(pattern)
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
        let w = wild;
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
    fn invalid_syntax_is_rejected() {
        for bad in [
            "((a))", "(a", "a)", "()", "\\1", "(a)\\2", "(a){2}", "[a", "a{2", "@a", "^q", "^?",
        ] {
            assert!(
                matches!(find_in_text("a", &wild(bad)), Err(FindError::Pattern(_))),
                "{bad}"
            );
        }
        assert!(matches!(
            find_in_text("a", &opts("")),
            Err(FindError::Empty)
        ));
        for bad in ["^q", "^f", "^e", "^d", "a^"] {
            assert!(
                matches!(find_in_text("a", &opts(bad)), Err(FindError::Pattern(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn groups_and_back_references() {
        assert_eq!(texts("abab abba", &wild("(ab)\\1")), ["abab"]);
        assert_eq!(texts("xyyx xyxy", &wild("(x)(y)\\2\\1")), ["xyyx"]);
    }

    #[test]
    fn special_codes_in_a_search() {
        let t = "a\tb 1 \u{a0}\r";
        assert_eq!(texts(t, &opts("^t")).len(), 1);
        assert_eq!(texts(t, &opts("^p")).len(), 1);
        assert_eq!(texts(t, &opts("^#")), ["1"]);
        assert_eq!(texts(t, &opts("^$")), ["a", "b"]);
        assert_eq!(texts(t, &opts("^s")).len(), 1);
        assert_eq!(texts("a \t\u{a0}b", &opts("^w")).len(), 1);
        assert_eq!(texts(t, &opts("a^?b")).len(), 1);
    }

    #[test]
    fn replacement_codes() {
        let parts = parse_replacement("[^&]^t", false, 0, None).unwrap();
        let e = expand(&parts, "x", &[], false);
        assert_eq!(e.text, "[x]\t");
        assert_eq!(e.copies, [(1, 1)]);
        assert!(parse_replacement("^q", false, 0, None).is_err());
        assert!(parse_replacement("^c", false, 0, None).is_err());
        assert!(parse_replacement("\\1", true, 0, None).is_err());
        let p = parse_replacement("<\\2\\1>", true, 2, None).unwrap();
        let e = expand(&p, "ab", &["a".to_owned(), "b".to_owned()], false);
        assert_eq!(e.text, "<ba>");
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
