//! Parser and compiler of the PCRE2-compatible regular expression engine behind `REGEXTEST`, `REGEXEXTRACT` and
//! `REGEXREPLACE`. Excel runs PCRE2 (10.4x) in UTF and UCP mode with the ANYCRLF newline convention; the syntax
//! implemented here follows what that build accepts, measured through the oracle (see
//! `docs/testing/calc-regex.md`). The matcher is in `textpcrevm.rs`.
//!
//! Supported: literals and escapes, classes with POSIX names and `\p{..}`, groups (capturing, named, branch
//! reset, non-capturing), alternation, greedy, lazy and possessive quantifiers, anchors, `\b`, `\K`, `\R`, `\X`,
//! lookahead, lookbehind (bounded, at most 255 characters), atomic groups, back references, conditionals,
//! recursion and subroutine calls, inline options `imsxnUJ`, and the pattern-start option verbs.

use super::textpcreprops::{Prop, fold, has_case, posix_class, prop_by_name};

/// A pattern that this engine rejects (Excel returns `#VALUE!`).
#[derive(Debug, Clone, Copy)]
pub(super) struct PcreError;

type PResult<T> = Result<T, PcreError>;

const MAX_COUNT: u32 = 65_535;
const MAX_DEPTH: usize = 250;
const MAX_LOOKBEHIND: u32 = 255;
/// Stands for "no upper bound" in a quantifier.
pub(super) const INF: u32 = u32::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RepKind {
    Greedy,
    Lazy,
    Possessive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AssertKind {
    Bol,
    BolMulti,
    Eol,
    EolMulti,
    StrStart,
    StrEnd,
    WordB,
    NotWordB,
    SearchStart,
}

#[derive(Debug, Clone)]
pub(super) enum ClassItem {
    Range(char, char),
    /// A property, negated when the flag is set.
    Prop(Prop, bool),
}

#[derive(Debug, Clone)]
pub(super) struct Class {
    pub items: Vec<ClassItem>,
    pub negated: bool,
    pub caseless: bool,
}

/// A matcher for exactly one character, which makes a quantified atom a tight loop.
#[derive(Debug, Clone, Copy)]
pub(super) enum Single {
    Char(char),
    /// Compares the folded character.
    CharCi(char),
    /// Any character (`(?s)`).
    Any,
    /// Any character except CR and LF.
    AnyNl,
    /// `\N`: like `AnyNl` regardless of `(?s)`.
    NotNl,
    Class(usize),
}

#[derive(Debug, Clone)]
enum CallTarget {
    Group(usize),
    Name(String),
}

#[derive(Debug, Clone)]
enum Cond {
    Group(usize),
    Name(String),
    Define,
    Look {
        ahead: bool,
        neg: bool,
        node: Box<Node>,
    },
}

#[derive(Debug, Clone)]
enum Node {
    Empty,
    Char(char, bool),
    Any(bool),
    NotNl,
    Class(usize),
    Seq(Vec<Node>),
    Alt(Vec<Node>),
    Group(Box<Node>, usize),
    Repeat {
        node: Box<Node>,
        min: u32,
        max: u32,
        kind: RepKind,
    },
    Assert(AssertKind),
    Look {
        ahead: bool,
        neg: bool,
        node: Box<Node>,
    },
    Atomic(Box<Node>),
    BackRef(usize, bool),
    BackRefName(String, bool),
    Call(CallTarget),
    Cond {
        cond: Cond,
        yes: Box<Node>,
        no: Box<Node>,
    },
    Keep,
    Fail,
    Accept,
    Newline,
    Grapheme,
}

/// One instruction of the compiled program.
#[derive(Debug, Clone)]
pub(super) enum Inst {
    One(Single),
    Newline,
    Grapheme,
    Assert(AssertKind),
    /// Try the first target, remember the second.
    Split(usize, usize),
    Jmp(usize),
    Save(usize),
    SetReg(usize, usize),
    /// A quantified single-character atom.
    Rep {
        single: Single,
        min: u32,
        max: u32,
        kind: RepKind,
    },
    /// Loop entry: the body is the next instruction, `exit` follows the loop.
    LoopHead {
        counter: usize,
        min: u32,
        max: u32,
        greedy: bool,
        exit: usize,
    },
    LoopBody {
        counter: usize,
        mark: usize,
    },
    /// Loop back to `head`, or leave when the iteration matched nothing and the minimum is met.
    LoopEnd {
        counter: usize,
        mark: usize,
        min: u32,
        head: usize,
    },
    /// Atomic group: the sub-program runs to `SubEnd` and is not backtracked into.
    Atomic {
        end: usize,
    },
    /// Lookaround. For a lookbehind the sub-program must end where it started, `min..=max` characters back.
    /// `else_pc` is the "no" branch of a conditional assertion.
    Look {
        ahead: bool,
        neg: bool,
        min: u32,
        max: u32,
        end: usize,
        else_pc: Option<usize>,
    },
    SubEnd,
    BackRef(Vec<usize>, bool),
    Call(usize),
    GroupEnd(usize),
    CondGroup {
        groups: Vec<usize>,
        else_pc: usize,
    },
    Fail,
    Accept,
    Match,
}

/// A compiled pattern.
#[derive(Debug)]
pub(super) struct Regex {
    pub prog: Vec<Inst>,
    pub classes: Vec<Class>,
    pub nregs: usize,
    pub ngroups: usize,
    /// The program counter of the first instruction inside each group; index 0 is the whole pattern.
    pub group_pc: Vec<usize>,
    pub names: Vec<(String, usize)>,
    /// A literal every match starts with, to skip hopeless start positions.
    pub first_char: Option<char>,
    /// Literals every match contains: a subject without one cannot match (PCRE2's "required code unit").
    pub required: Vec<char>,
}

#[derive(Debug, Clone, Copy, Default)]
struct Flags {
    i: bool,
    m: bool,
    s: bool,
    x: bool,
    /// `(?xx)`: spaces and tabs inside a class are ignored too.
    xx: bool,
    n: bool,
    ungreedy: bool,
    dupnames: bool,
}

struct Parser {
    s: Vec<char>,
    pos: usize,
    flags: Flags,
    ngroups: usize,
    total_groups: usize,
    names: Vec<(String, usize)>,
    classes: Vec<Class>,
    depth: usize,
    quote: bool,
}

/// What `parse_escape` found.
enum Esc {
    Lit(char),
    Item(ClassItem),
    Assert(AssertKind),
    Keep,
    Newline,
    Grapheme,
    NotNl,
    BackRef(usize),
    BackRefName(String),
    Call(CallTarget),
    Quote,
    EndQuote,
}

/// Counts the capturing groups of a pattern with a light scan (needed to tell `\12` the back reference from
/// `\12` the octal escape).
fn count_groups(s: &[char]) -> usize {
    let mut n = 0;
    let mut i = 0;
    let mut in_class = false;
    while i < s.len() {
        match s[i] {
            '\\' => i += 1,
            '[' if !in_class => in_class = true,
            ']' if in_class => in_class = false,
            '(' if !in_class => {
                let next = s.get(i + 1).copied();
                if next == Some('?') {
                    let a = s.get(i + 2).copied();
                    let b = s.get(i + 3).copied();
                    if a == Some('\'')
                        || (a == Some('<') && !matches!(b, Some('=' | '!')))
                        || (a == Some('P') && b == Some('<'))
                    {
                        n += 1;
                    }
                } else if next != Some('*') {
                    n += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    n
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn hex_val(c: char) -> Option<u32> {
    c.to_digit(16)
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.s.get(self.pos).copied()
    }

    fn peek_at(&self, n: usize) -> Option<char> {
        self.s.get(self.pos + n).copied()
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn starts_with(&self, text: &str) -> bool {
        text.chars()
            .enumerate()
            .all(|(i, c)| self.peek_at(i) == Some(c))
    }

    fn lit(&self, c: char) -> Node {
        Node::Char(c, self.flags.i && has_case(c))
    }

    fn skip_extended(&mut self) {
        if !self.flags.x || self.quote {
            return;
        }
        loop {
            match self.peek() {
                Some(' ' | '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | '\u{85}') => self.pos += 1,
                Some('#') => {
                    while let Some(c) = self.peek() {
                        self.pos += 1;
                        if c == '\n' || c == '\r' {
                            break;
                        }
                    }
                }
                _ => break,
            }
        }
    }

    fn parse_alts(&mut self) -> PResult<Vec<Node>> {
        let mut alts = vec![self.parse_seq()?];
        while !self.quote && self.eat('|') {
            alts.push(self.parse_seq()?);
        }
        Ok(alts)
    }

    fn parse_seq(&mut self) -> PResult<Node> {
        let mut items = Vec::new();
        loop {
            self.skip_extended();
            match self.peek() {
                None => break,
                Some('|' | ')') if !self.quote => break,
                _ => {}
            }
            if let Some((node, repeatable)) = self.parse_atom()? {
                items.push(self.parse_quant(node, repeatable)?);
            }
        }
        Ok(match items.len() {
            0 => Node::Empty,
            1 => items.pop().unwrap_or(Node::Empty),
            _ => Node::Seq(items),
        })
    }

    /// `{n}`, `{n,}`, `{n,m}` and `{,m}` at the current position: the bounds and the length, or `None` when
    /// the brace is a literal.
    fn brace_quant(&self) -> PResult<Option<(u32, u32, usize)>> {
        let mut i = 1;
        let num = |i: &mut usize| -> Option<u64> {
            let start = *i;
            let mut v: u64 = 0;
            while let Some(d) = self.peek_at(*i).and_then(|c| c.to_digit(10)) {
                v = (v * 10 + u64::from(d)).min(1 << 40);
                *i += 1;
            }
            (*i > start).then_some(v)
        };
        let lo = num(&mut i);
        let (lo, hi) = if self.peek_at(i) == Some(',') {
            i += 1;
            let hi = num(&mut i);
            if lo.is_none() && hi.is_none() {
                return Ok(None);
            }
            (lo.unwrap_or(0), hi)
        } else {
            match lo {
                None => return Ok(None),
                Some(v) => (v, Some(v)),
            }
        };
        if self.peek_at(i) != Some('}') {
            return Ok(None);
        }
        let limit = u64::from(MAX_COUNT);
        if lo > limit || hi.is_some_and(|h| h > limit) {
            return Err(PcreError);
        }
        if hi.is_some_and(|h| h < lo) {
            return Err(PcreError);
        }
        Ok(Some((lo as u32, hi.map_or(INF, |h| h as u32), i + 1)))
    }

    /// A quantifier at the current position: bounds and length in characters.
    fn quant_here(&self) -> PResult<Option<(u32, u32, usize)>> {
        Ok(match self.peek() {
            Some('*') => Some((0, INF, 1)),
            Some('+') => Some((1, INF, 1)),
            Some('?') => Some((0, 1, 1)),
            Some('{') => self.brace_quant()?,
            _ => None,
        })
    }

    fn parse_quant(&mut self, node: Node, repeatable: bool) -> PResult<Node> {
        self.skip_extended();
        let Some((min, max, len)) = self.quant_here()? else {
            return Ok(node);
        };
        if !repeatable {
            return Err(PcreError);
        }
        self.pos += len;
        let mut kind = if self.flags.ungreedy {
            RepKind::Lazy
        } else {
            RepKind::Greedy
        };
        if self.eat('?') {
            kind = if self.flags.ungreedy {
                RepKind::Greedy
            } else {
                RepKind::Lazy
            };
        } else if self.eat('+') {
            kind = RepKind::Possessive;
        }
        self.skip_extended();
        if self.quant_here()?.is_some() {
            return Err(PcreError);
        }
        Ok(Node::Repeat {
            node: Box::new(node),
            min,
            max,
            kind,
        })
    }

    /// Parses one atom. `None` is something that matches nothing and takes no quantifier (an option setting, a
    /// comment, `\E`). The flag says whether a quantifier may follow.
    fn parse_atom(&mut self) -> PResult<Option<(Node, bool)>> {
        if self.quote {
            if self.starts_with("\\E") {
                self.pos += 2;
                self.quote = false;
                return Ok(None);
            }
            let c = self.peek().ok_or(PcreError)?;
            self.pos += 1;
            return Ok(Some((self.lit(c), true)));
        }
        let c = self.peek().ok_or(PcreError)?;
        match c {
            '(' => self.parse_group(),
            '[' => {
                self.pos += 1;
                let class = self.parse_class()?;
                self.classes.push(class);
                Ok(Some((Node::Class(self.classes.len() - 1), true)))
            }
            '.' => {
                self.pos += 1;
                Ok(Some((Node::Any(self.flags.s), true)))
            }
            '^' => {
                self.pos += 1;
                let kind = if self.flags.m {
                    AssertKind::BolMulti
                } else {
                    AssertKind::Bol
                };
                Ok(Some((Node::Assert(kind), false)))
            }
            '$' => {
                self.pos += 1;
                let kind = if self.flags.m {
                    AssertKind::EolMulti
                } else {
                    AssertKind::Eol
                };
                Ok(Some((Node::Assert(kind), false)))
            }
            '*' | '+' | '?' => Err(PcreError),
            '{' => {
                if self.brace_quant()?.is_some() {
                    return Err(PcreError);
                }
                self.pos += 1;
                Ok(Some((self.lit('{'), true)))
            }
            '\\' => self.parse_escape_atom(),
            _ => {
                self.pos += 1;
                Ok(Some((self.lit(c), true)))
            }
        }
    }

    fn parse_escape_atom(&mut self) -> PResult<Option<(Node, bool)>> {
        let ci = self.flags.i;
        Ok(match self.parse_escape(false)? {
            Esc::Lit(c) => Some((self.lit(c), true)),
            Esc::Item(item) => {
                self.classes.push(Class {
                    items: vec![item],
                    negated: false,
                    caseless: false,
                });
                Some((Node::Class(self.classes.len() - 1), true))
            }
            Esc::Assert(k) => Some((Node::Assert(k), false)),
            Esc::Keep => Some((Node::Keep, false)),
            Esc::Newline => Some((Node::Newline, true)),
            Esc::Grapheme => Some((Node::Grapheme, true)),
            Esc::NotNl => Some((Node::NotNl, true)),
            Esc::BackRef(n) => Some((Node::BackRef(n, ci), true)),
            Esc::BackRefName(name) => Some((Node::BackRefName(name, ci), true)),
            Esc::Call(t) => Some((Node::Call(t), true)),
            Esc::Quote => {
                self.quote = true;
                None
            }
            Esc::EndQuote => None,
        })
    }

    fn parse_number(&mut self) -> Option<usize> {
        let start = self.pos;
        let mut v: usize = 0;
        while let Some(d) = self.peek().and_then(|c| c.to_digit(10)) {
            v = v.saturating_mul(10).saturating_add(d as usize);
            self.pos += 1;
        }
        (self.pos > start).then_some(v)
    }

    /// A name up to `end`, which is consumed.
    fn parse_name(&mut self, end: char) -> PResult<String> {
        let mut name = String::new();
        while let Some(c) = self.peek() {
            if c == end {
                break;
            }
            if !is_name_char(c) {
                return Err(PcreError);
            }
            name.push(c);
            self.pos += 1;
        }
        if !self.eat(end)
            || name.is_empty()
            || name.chars().count() > 32
            || name.starts_with(|c: char| c.is_ascii_digit())
        {
            return Err(PcreError);
        }
        Ok(name)
    }

    /// The escape at the current position (a backslash). `in_class` restricts it to what a class allows.
    fn parse_escape(&mut self, in_class: bool) -> PResult<Esc> {
        self.pos += 1;
        let c = self.peek().ok_or(PcreError)?;
        self.pos += 1;
        let item = |p: Prop, neg: bool| Ok(Esc::Item(ClassItem::Prop(p, neg)));
        match c {
            'd' => item(Prop::Digit, false),
            'D' => item(Prop::Digit, true),
            'w' => item(Prop::Word, false),
            'W' => item(Prop::Word, true),
            's' => item(Prop::Space, false),
            'S' => item(Prop::Space, true),
            'h' => item(Prop::HSpace, false),
            'H' => item(Prop::HSpace, true),
            'v' => item(Prop::VSpace, false),
            'V' => item(Prop::VSpace, true),
            'p' | 'P' => {
                let (prop, neg) = self.parse_property(c == 'P')?;
                item(prop, neg)
            }
            'Q' => Ok(Esc::Quote),
            'E' => Ok(Esc::EndQuote),
            'a' => Ok(Esc::Lit('\u{7}')),
            'e' => Ok(Esc::Lit('\u{1b}')),
            'f' => Ok(Esc::Lit('\u{c}')),
            'n' => Ok(Esc::Lit('\n')),
            'r' => Ok(Esc::Lit('\r')),
            't' => Ok(Esc::Lit('\t')),
            'b' if in_class => Ok(Esc::Lit('\u{8}')),
            'x' => self.parse_hex().map(Esc::Lit),
            'o' => {
                if !self.eat('{') {
                    return Err(PcreError);
                }
                let mut v: u32 = 0;
                let mut any = false;
                while let Some(d) = self.peek().and_then(|c| c.to_digit(8)) {
                    v = v.saturating_mul(8).saturating_add(d);
                    any = true;
                    self.pos += 1;
                }
                if !any || !self.eat('}') {
                    return Err(PcreError);
                }
                char::from_u32(v).map(Esc::Lit).ok_or(PcreError)
            }
            'c' => {
                let d = self.peek().ok_or(PcreError)?;
                if !d.is_ascii() || (d as u32) < 32 || d as u32 > 126 {
                    return Err(PcreError);
                }
                self.pos += 1;
                Ok(Esc::Lit(
                    char::from_u32(u32::from(d.to_ascii_uppercase() as u8 ^ 0x40)).unwrap_or('\0'),
                ))
            }
            '0' => {
                let mut v = 0;
                for _ in 0..2 {
                    match self.peek().and_then(|c| c.to_digit(8)) {
                        Some(d) => {
                            v = v * 8 + d;
                            self.pos += 1;
                        }
                        None => break,
                    }
                }
                Ok(Esc::Lit(char::from_u32(v).unwrap_or('\0')))
            }
            '1'..='9' => {
                self.pos -= 1;
                let start = self.pos;
                let n = self.parse_number().ok_or(PcreError)?;
                if !in_class && (n < 10 || n <= self.total_groups) {
                    return Ok(Esc::BackRef(n));
                }
                // An octal escape of up to three digits; 8 and 9 are errors.
                self.pos = start;
                let mut v = 0;
                let mut len = 0;
                while len < 3 {
                    match self.peek().and_then(|c| c.to_digit(8)) {
                        Some(d) => {
                            v = v * 8 + d;
                            self.pos += 1;
                            len += 1;
                        }
                        None => break,
                    }
                }
                if len == 0 {
                    return Err(PcreError);
                }
                Ok(Esc::Lit(char::from_u32(v).unwrap_or('\0')))
            }
            'B' | 'A' | 'z' | 'Z' | 'G' | 'K' | 'R' | 'X' if in_class => Err(PcreError),
            'B' => Ok(Esc::Assert(AssertKind::NotWordB)),
            'b' => Ok(Esc::Assert(AssertKind::WordB)),
            'A' => Ok(Esc::Assert(AssertKind::StrStart)),
            'z' => Ok(Esc::Assert(AssertKind::StrEnd)),
            'Z' => Ok(Esc::Assert(AssertKind::Eol)),
            'G' => Ok(Esc::Assert(AssertKind::SearchStart)),
            'K' => Ok(Esc::Keep),
            'R' => Ok(Esc::Newline),
            'X' => Ok(Esc::Grapheme),
            'N' => {
                if in_class {
                    return Err(PcreError);
                }
                if self.peek() == Some('{') {
                    // \N{U+hhhh}
                    if !self.starts_with("{U+") {
                        return Err(PcreError);
                    }
                    self.pos += 3;
                    let mut v: u32 = 0;
                    let mut any = false;
                    while let Some(d) = self.peek().and_then(hex_val) {
                        v = v.saturating_mul(16).saturating_add(d);
                        any = true;
                        self.pos += 1;
                    }
                    if !any || !self.eat('}') {
                        return Err(PcreError);
                    }
                    return char::from_u32(v).map(Esc::Lit).ok_or(PcreError);
                }
                Ok(Esc::NotNl)
            }
            'g' if !in_class => self.parse_g(),
            'k' if !in_class => {
                let end = match self.peek() {
                    Some('<') => '>',
                    Some('{') => '}',
                    Some('\'') => '\'',
                    _ => return Err(PcreError),
                };
                self.pos += 1;
                Ok(Esc::BackRefName(self.parse_name(end)?))
            }
            c if c.is_ascii_alphanumeric() => Err(PcreError),
            c => Ok(Esc::Lit(c)),
        }
    }

    /// `\g1`, `\g{1}`, `\g{-1}`, `\g{name}`, `\g<..>` and `\g'..'` (the last two are subroutine calls).
    fn parse_g(&mut self) -> PResult<Esc> {
        let call = matches!(self.peek(), Some('<' | '\''));
        let end = match self.peek() {
            Some('{') => Some('}'),
            Some('<') => Some('>'),
            Some('\'') => Some('\''),
            _ => None,
        };
        if end.is_some() {
            self.pos += 1;
        }
        let relative = match self.peek() {
            Some('-') => {
                self.pos += 1;
                Some(false)
            }
            Some('+') => {
                self.pos += 1;
                Some(true)
            }
            _ => None,
        };
        if let Some(n) = self.parse_number() {
            if let Some(e) = end
                && !self.eat(e)
            {
                return Err(PcreError);
            }
            let idx = match relative {
                None => n,
                Some(false) => {
                    if n == 0 || n > self.ngroups {
                        return Err(PcreError);
                    }
                    self.ngroups + 1 - n
                }
                Some(true) => self.ngroups + n,
            };
            if idx == 0 && !call {
                return Err(PcreError);
            }
            return Ok(if call {
                Esc::Call(CallTarget::Group(idx))
            } else {
                Esc::BackRef(idx)
            });
        }
        let end = end.ok_or(PcreError)?;
        if relative.is_some() {
            return Err(PcreError);
        }
        let name = self.parse_name(end)?;
        Ok(if call {
            Esc::Call(CallTarget::Name(name))
        } else {
            Esc::BackRefName(name)
        })
    }

    fn parse_hex(&mut self) -> PResult<char> {
        if self.eat('{') {
            let mut v: u32 = 0;
            let mut any = false;
            while let Some(d) = self.peek().and_then(hex_val) {
                v = v.saturating_mul(16).saturating_add(d);
                any = true;
                self.pos += 1;
            }
            if !any || !self.eat('}') {
                return Err(PcreError);
            }
            return char::from_u32(v).ok_or(PcreError);
        }
        let mut v = 0;
        for _ in 0..2 {
            match self.peek().and_then(hex_val) {
                Some(d) => {
                    v = v * 16 + d;
                    self.pos += 1;
                }
                None => break,
            }
        }
        char::from_u32(v).ok_or(PcreError)
    }

    /// The property after `\p` or `\P`.
    fn parse_property(&mut self, negated: bool) -> PResult<(Prop, bool)> {
        let mut neg = negated;
        let name = if self.eat('{') {
            if self.eat('^') {
                neg = !neg;
            }
            let mut name = String::new();
            loop {
                match self.peek() {
                    Some('}') => {
                        self.pos += 1;
                        break;
                    }
                    Some(c) => {
                        name.push(c);
                        self.pos += 1;
                    }
                    None => return Err(PcreError),
                }
            }
            name
        } else {
            let c = self.peek().ok_or(PcreError)?;
            self.pos += 1;
            c.to_string()
        };
        Ok((prop_by_name(&name).ok_or(PcreError)?, neg))
    }

    /// A character class; the `[` is already consumed.
    fn parse_class(&mut self) -> PResult<Class> {
        let negated = self.eat('^');
        let mut items = Vec::new();
        let mut first = true;
        loop {
            let c = self.peek().ok_or(PcreError)?;
            if self.flags.xx && !self.quote && (c == ' ' || c == '\t') {
                self.pos += 1;
                continue;
            }
            if c == ']' && !first && !self.quote {
                self.pos += 1;
                break;
            }
            first = false;
            // One class member: a character (range start) or a class-like item.
            let lo = if self.quote {
                if self.starts_with("\\E") {
                    self.pos += 2;
                    self.quote = false;
                    continue;
                }
                self.pos += 1;
                c
            } else if c == '[' && matches!(self.peek_at(1), Some(':' | '.' | '=')) {
                if self.peek_at(1) != Some(':') {
                    return Err(PcreError);
                }
                if let Some(end) = (self.pos + 2..self.s.len().saturating_sub(1))
                    .find(|&i| self.s[i] == ':' && self.s[i + 1] == ']')
                {
                    let mut name: String = self.s[self.pos + 2..end].iter().collect();
                    let neg = name.starts_with('^');
                    if neg {
                        name.remove(0);
                    }
                    let prop = posix_class(&name).ok_or(PcreError)?;
                    self.pos = end + 2;
                    items.push(ClassItem::Prop(prop, neg));
                    self.reject_range_after_item()?;
                    continue;
                }
                self.pos += 1;
                '['
            } else if c == '\\' {
                match self.parse_escape(true)? {
                    Esc::Lit(ch) => ch,
                    Esc::Item(item) => {
                        items.push(item);
                        self.reject_range_after_item()?;
                        continue;
                    }
                    Esc::Quote => {
                        self.quote = true;
                        continue;
                    }
                    Esc::EndQuote => continue,
                    _ => return Err(PcreError),
                }
            } else {
                self.pos += 1;
                c
            };
            // A range?
            if self.peek() == Some('-') && self.peek_at(1).is_some_and(|n| n != ']') && !self.quote
            {
                self.pos += 1;
                let hi_c = self.peek().ok_or(PcreError)?;
                let hi = if hi_c == '\\' {
                    match self.parse_escape(true)? {
                        Esc::Lit(ch) => ch,
                        _ => return Err(PcreError),
                    }
                } else if hi_c == '[' && self.peek_at(1) == Some(':') {
                    return Err(PcreError);
                } else {
                    self.pos += 1;
                    hi_c
                };
                if hi < lo {
                    return Err(PcreError);
                }
                items.push(ClassItem::Range(lo, hi));
            } else {
                items.push(ClassItem::Range(lo, lo));
            }
        }
        Ok(Class {
            items,
            negated,
            caseless: self.flags.i,
        })
    }

    /// After a class item such as `\d`, `-` must be the last character of the class.
    fn reject_range_after_item(&self) -> PResult<()> {
        if self.peek() == Some('-') && self.peek_at(1).is_some_and(|n| n != ']') {
            return Err(PcreError);
        }
        Ok(())
    }

    /// Everything after `(`.
    fn parse_group(&mut self) -> PResult<Option<(Node, bool)>> {
        self.pos += 1;
        if self.peek() == Some('*') {
            return self.parse_verb();
        }
        if self.peek() != Some('?') {
            if self.flags.n {
                let node = self.group_alts()?;
                return Ok(Some((node, true)));
            }
            return self.capture(None).map(Some);
        }
        self.pos += 1;
        let c = self.peek().ok_or(PcreError)?;
        match c {
            '#' => {
                while let Some(ch) = self.peek() {
                    self.pos += 1;
                    if ch == ')' {
                        return Ok(None);
                    }
                }
                Err(PcreError)
            }
            ':' => {
                self.pos += 1;
                Ok(Some((self.group_alts()?, true)))
            }
            '|' => {
                self.pos += 1;
                Ok(Some((self.branch_reset()?, true)))
            }
            '>' => {
                self.pos += 1;
                let inner = self.group_alts()?;
                Ok(Some((Node::Atomic(Box::new(inner)), true)))
            }
            '=' | '!' => {
                self.pos += 1;
                let inner = self.group_alts()?;
                Ok(Some((
                    Node::Look {
                        ahead: true,
                        neg: c == '!',
                        node: Box::new(inner),
                    },
                    true,
                )))
            }
            '<' if matches!(self.peek_at(1), Some('=' | '!')) => {
                let neg = self.peek_at(1) == Some('!');
                self.pos += 2;
                let inner = self.group_alts()?;
                Ok(Some((
                    Node::Look {
                        ahead: false,
                        neg,
                        node: Box::new(inner),
                    },
                    true,
                )))
            }
            '<' => {
                self.pos += 1;
                let name = self.parse_name('>')?;
                self.capture(Some(name)).map(Some)
            }
            '\'' => {
                self.pos += 1;
                let name = self.parse_name('\'')?;
                self.capture(Some(name)).map(Some)
            }
            'P' => {
                self.pos += 1;
                match self.peek() {
                    Some('<') => {
                        self.pos += 1;
                        let name = self.parse_name('>')?;
                        self.capture(Some(name)).map(Some)
                    }
                    Some('=') => {
                        self.pos += 1;
                        let name = self.parse_name(')')?;
                        Ok(Some((Node::BackRefName(name, self.flags.i), true)))
                    }
                    Some('>') => {
                        self.pos += 1;
                        let name = self.parse_name(')')?;
                        Ok(Some((Node::Call(CallTarget::Name(name)), true)))
                    }
                    _ => Err(PcreError),
                }
            }
            '&' => {
                self.pos += 1;
                let name = self.parse_name(')')?;
                Ok(Some((Node::Call(CallTarget::Name(name)), true)))
            }
            'R' => {
                self.pos += 1;
                if !self.eat(')') {
                    return Err(PcreError);
                }
                Ok(Some((Node::Call(CallTarget::Group(0)), true)))
            }
            '0'..='9' | '+' | '-'
                if c.is_ascii_digit() || self.peek_at(1).is_some_and(|d| d.is_ascii_digit()) =>
            {
                let sign = match c {
                    '+' => {
                        self.pos += 1;
                        Some(true)
                    }
                    '-' => {
                        self.pos += 1;
                        Some(false)
                    }
                    _ => None,
                };
                let n = self.parse_number().ok_or(PcreError)?;
                if !self.eat(')') {
                    return Err(PcreError);
                }
                let idx = match sign {
                    None => n,
                    Some(true) => self.ngroups + n,
                    Some(false) => {
                        if n == 0 || n > self.ngroups {
                            return Err(PcreError);
                        }
                        self.ngroups + 1 - n
                    }
                };
                Ok(Some((Node::Call(CallTarget::Group(idx)), true)))
            }
            '(' => {
                self.pos += 1;
                self.parse_conditional().map(Some)
            }
            'C' => Err(PcreError),
            _ => self.parse_flags(),
        }
    }

    /// `(?flags)` or `(?flags:...)`.
    fn parse_flags(&mut self) -> PResult<Option<(Node, bool)>> {
        let mut f = self.flags;
        let mut on = true;
        let mut seen_x = false;
        loop {
            let c = self.peek().ok_or(PcreError)?;
            self.pos += 1;
            match c {
                'i' => f.i = on,
                'm' => f.m = on,
                's' => f.s = on,
                'x' => {
                    f.x = on;
                    f.xx = on && seen_x;
                }
                'n' => f.n = on,
                'U' => f.ungreedy = on,
                'J' => f.dupnames = on,
                '-' => {
                    if !on {
                        return Err(PcreError);
                    }
                    on = false;
                }
                '^' => {
                    f.i = false;
                    f.m = false;
                    f.s = false;
                    f.x = false;
                    f.xx = false;
                    f.n = false;
                }
                'a' => {
                    // ASCII restrictions (aD, aS, aW, aP, aT) are accepted and ignored.
                    while matches!(self.peek(), Some('D' | 'S' | 'W' | 'P' | 'T')) {
                        self.pos += 1;
                    }
                }
                ')' => {
                    self.flags = f;
                    return Ok(None);
                }
                ':' => {
                    let saved = self.flags;
                    self.flags = f;
                    let node = self.group_alts();
                    self.flags = saved;
                    return Ok(Some((node?, true)));
                }
                _ => return Err(PcreError),
            }
            seen_x = c == 'x';
        }
    }

    /// `(*VERB)`: only `FAIL`, `ACCEPT` and the backtracking-control verbs (accepted, no effect).
    fn parse_verb(&mut self) -> PResult<Option<(Node, bool)>> {
        self.pos += 1;
        let mut word = String::new();
        while let Some(c) = self.peek() {
            self.pos += 1;
            if c == ')' {
                let (name, _) = word.split_once(':').unwrap_or((&word, ""));
                return match name {
                    "FAIL" | "F" => Ok(Some((Node::Fail, false))),
                    "ACCEPT" => Ok(Some((Node::Accept, false))),
                    "COMMIT" | "PRUNE" | "SKIP" | "THEN" | "" => Ok(Some((Node::Empty, false))),
                    _ => Err(PcreError),
                };
            }
            word.push(c);
        }
        Err(PcreError)
    }

    fn capture(&mut self, name: Option<String>) -> PResult<(Node, bool)> {
        self.ngroups += 1;
        let idx = self.ngroups;
        if let Some(name) = name {
            if self
                .names
                .iter()
                .any(|(n, i)| *n == name && *i != idx && !self.flags.dupnames)
            {
                return Err(PcreError);
            }
            self.names.push((name, idx));
        }
        let inner = self.group_alts()?;
        Ok((Node::Group(Box::new(inner), idx), true))
    }

    /// The alternatives of a group up to its `)`, which is consumed; the options set inside are undone.
    fn group_alts(&mut self) -> PResult<Node> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(PcreError);
        }
        let saved = self.flags;
        let mut alts = self.parse_alts()?;
        if !self.eat(')') {
            return Err(PcreError);
        }
        self.flags = saved;
        self.depth -= 1;
        Ok(if alts.len() == 1 {
            alts.pop().unwrap_or(Node::Empty)
        } else {
            Node::Alt(alts)
        })
    }

    /// `(?|...)`: every alternative numbers its groups from the same start.
    fn branch_reset(&mut self) -> PResult<Node> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(PcreError);
        }
        let saved = self.flags;
        let start = self.ngroups;
        let mut highest = start;
        let mut alts = Vec::new();
        loop {
            self.ngroups = start;
            alts.push(self.parse_seq()?);
            highest = highest.max(self.ngroups);
            if !self.eat('|') {
                break;
            }
        }
        if !self.eat(')') {
            return Err(PcreError);
        }
        self.ngroups = highest;
        self.flags = saved;
        self.depth -= 1;
        Ok(if alts.len() == 1 {
            alts.pop().unwrap_or(Node::Empty)
        } else {
            Node::Alt(alts)
        })
    }

    /// `(?(cond)yes|no)`, after the two opening characters.
    fn parse_conditional(&mut self) -> PResult<(Node, bool)> {
        let cond = if self.peek() == Some('?') {
            // An assertion as the condition.
            self.pos += 1;
            let (ahead, neg) = match (self.peek(), self.peek_at(1)) {
                (Some('='), _) => (true, false),
                (Some('!'), _) => (true, true),
                (Some('<'), Some('=')) => (false, false),
                (Some('<'), Some('!')) => (false, true),
                _ => return Err(PcreError),
            };
            self.pos += if ahead { 1 } else { 2 };
            let inner = self.group_alts()?;
            Cond::Look {
                ahead,
                neg,
                node: Box::new(inner),
            }
        } else if self.starts_with("DEFINE)") {
            self.pos += 7;
            Cond::Define
        } else if matches!(self.peek(), Some('<' | '\'')) {
            let end = if self.peek() == Some('<') { '>' } else { '\'' };
            self.pos += 1;
            let name = self.parse_name(end)?;
            if !self.eat(')') {
                return Err(PcreError);
            }
            Cond::Name(name)
        } else {
            let sign = match self.peek() {
                Some('+') => {
                    self.pos += 1;
                    Some(true)
                }
                Some('-') => {
                    self.pos += 1;
                    Some(false)
                }
                _ => None,
            };
            if let Some(n) = self.parse_number() {
                if !self.eat(')') {
                    return Err(PcreError);
                }
                let idx = match sign {
                    None => n,
                    Some(true) => self.ngroups + n,
                    Some(false) => {
                        if n == 0 || n > self.ngroups {
                            return Err(PcreError);
                        }
                        self.ngroups + 1 - n
                    }
                };
                Cond::Group(idx)
            } else {
                // A bare group name.
                let name = self.parse_name(')')?;
                Cond::Name(name)
            }
        };
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(PcreError);
        }
        let saved = self.flags;
        let mut alts = self.parse_alts()?;
        if !self.eat(')') {
            return Err(PcreError);
        }
        self.flags = saved;
        self.depth -= 1;
        if alts.len() > 2 {
            return Err(PcreError);
        }
        let no = if alts.len() == 2 {
            alts.pop().unwrap_or(Node::Empty)
        } else {
            Node::Empty
        };
        let yes = alts.pop().unwrap_or(Node::Empty);
        Ok((
            Node::Cond {
                cond,
                yes: Box::new(yes),
                no: Box::new(no),
            },
            true,
        ))
    }
}

/// The least and greatest number of characters a node matches (`INF` when unbounded).
fn width(n: &Node) -> (u32, u32) {
    let add = |a: u32, b: u32| {
        if a == INF || b == INF {
            INF
        } else {
            a.saturating_add(b)
        }
    };
    match n {
        Node::Char(..) | Node::Any(_) | Node::NotNl | Node::Class(_) => (1, 1),
        Node::Empty
        | Node::Assert(_)
        | Node::Look { .. }
        | Node::Keep
        | Node::Fail
        | Node::Accept => (0, 0),
        Node::Newline => (1, 2),
        Node::Grapheme | Node::BackRef(..) | Node::BackRefName(..) | Node::Call(_) => (0, INF),
        Node::Seq(v) => v.iter().fold((0, 0), |(a, b), x| {
            let (c, d) = width(x);
            (add(a, c), add(b, d))
        }),
        Node::Alt(v) => v
            .iter()
            .map(width)
            .fold((INF, 0), |(a, b), (c, d)| (a.min(c), b.max(d))),
        Node::Group(x, _) | Node::Atomic(x) => width(x),
        Node::Repeat { node, min, max, .. } => {
            let (a, b) = width(node);
            let lo = a.saturating_mul(*min);
            let hi = if b == 0 {
                0
            } else if b == INF || *max == INF {
                INF
            } else {
                b.saturating_mul(*max)
            };
            (lo, hi)
        }
        Node::Cond { yes, no, .. } => {
            let (a, b) = width(yes);
            let (c, d) = width(no);
            (a.min(c), b.max(d))
        }
    }
}

/// Case-sensitive literals that every match of `n` contains.
fn required_literals(n: &Node, out: &mut Vec<char>) {
    match n {
        Node::Char(c, false) if !out.contains(c) => out.push(*c),
        Node::Seq(v) => v.iter().for_each(|x| required_literals(x, out)),
        Node::Group(x, _) | Node::Atomic(x) => required_literals(x, out),
        Node::Repeat { node, min, .. } if *min >= 1 => required_literals(node, out),
        _ => {}
    }
}

struct Compiler<'a> {
    prog: Vec<Inst>,
    nregs: usize,
    group_pc: Vec<usize>,
    names: &'a [(String, usize)],
    ngroups: usize,
}

impl Compiler<'_> {
    fn push(&mut self, i: Inst) -> usize {
        self.prog.push(i);
        self.prog.len() - 1
    }

    fn reg(&mut self) -> usize {
        self.nregs += 1;
        self.nregs - 1
    }

    fn groups_named(&self, name: &str) -> PResult<Vec<usize>> {
        let v: Vec<usize> = self
            .names
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, i)| *i)
            .collect();
        if v.is_empty() { Err(PcreError) } else { Ok(v) }
    }

    fn single(&self, n: &Node) -> Option<Single> {
        Some(match n {
            Node::Char(c, true) => Single::CharCi(fold(*c)),
            Node::Char(c, false) => Single::Char(*c),
            Node::Any(true) => Single::Any,
            Node::Any(false) => Single::AnyNl,
            Node::NotNl => Single::NotNl,
            Node::Class(i) => Single::Class(*i),
            _ => return None,
        })
    }

    fn emit(&mut self, n: &Node) -> PResult<()> {
        if let Some(s) = self.single(n) {
            self.push(Inst::One(s));
            return Ok(());
        }
        match n {
            Node::Empty => {}
            Node::Seq(v) => {
                for x in v {
                    self.emit(x)?;
                }
            }
            Node::Alt(v) => {
                let mut jumps = Vec::new();
                for (i, x) in v.iter().enumerate() {
                    if i + 1 == v.len() {
                        self.emit(x)?;
                    } else {
                        let split = self.push(Inst::Split(0, 0));
                        self.emit(x)?;
                        jumps.push(self.push(Inst::Jmp(0)));
                        let next = self.prog.len();
                        self.prog[split] = Inst::Split(split + 1, next);
                    }
                }
                let end = self.prog.len();
                for j in jumps {
                    self.prog[j] = Inst::Jmp(end);
                }
            }
            Node::Group(inner, idx) => {
                self.push(Inst::Save(2 * idx));
                if self.group_pc[*idx] == 0 {
                    self.group_pc[*idx] = self.prog.len();
                }
                self.emit(inner)?;
                self.push(Inst::Save(2 * idx + 1));
                self.push(Inst::GroupEnd(*idx));
            }
            Node::Repeat {
                node,
                min,
                max,
                kind,
            } => {
                if let Some(single) = self.single(node) {
                    self.push(Inst::Rep {
                        single,
                        min: *min,
                        max: *max,
                        kind: *kind,
                    });
                } else if *kind == RepKind::Possessive {
                    let at = self.push(Inst::Atomic { end: 0 });
                    self.emit_loop(node, *min, *max, true)?;
                    self.push(Inst::SubEnd);
                    let end = self.prog.len();
                    self.prog[at] = Inst::Atomic { end };
                } else {
                    self.emit_loop(node, *min, *max, *kind == RepKind::Greedy)?;
                }
            }
            Node::Assert(k) => {
                self.push(Inst::Assert(*k));
            }
            Node::Look { ahead, neg, node } => {
                let (min, max) = self.look_bounds(*ahead, node)?;
                let at = self.push(Inst::Look {
                    ahead: *ahead,
                    neg: *neg,
                    min,
                    max,
                    end: 0,
                    else_pc: None,
                });
                self.emit(node)?;
                self.push(Inst::SubEnd);
                let end = self.prog.len();
                self.prog[at] = Inst::Look {
                    ahead: *ahead,
                    neg: *neg,
                    min,
                    max,
                    end,
                    else_pc: None,
                };
            }
            Node::Atomic(inner) => {
                let at = self.push(Inst::Atomic { end: 0 });
                self.emit(inner)?;
                self.push(Inst::SubEnd);
                let end = self.prog.len();
                self.prog[at] = Inst::Atomic { end };
            }
            Node::BackRef(n, ci) => {
                if *n == 0 || *n > self.ngroups {
                    return Err(PcreError);
                }
                self.push(Inst::BackRef(vec![*n], *ci));
            }
            Node::BackRefName(name, ci) => {
                let groups = self.groups_named(name)?;
                self.push(Inst::BackRef(groups, *ci));
            }
            Node::Call(t) => {
                let idx = match t {
                    CallTarget::Group(i) => *i,
                    CallTarget::Name(n) => self.groups_named(n)?[0],
                };
                if idx > self.ngroups {
                    return Err(PcreError);
                }
                self.push(Inst::Call(idx));
            }
            Node::Cond { cond, yes, no } => self.emit_cond(cond, yes, no)?,
            Node::Keep => {
                self.push(Inst::Save(0));
            }
            Node::Fail => {
                self.push(Inst::Fail);
            }
            Node::Accept => {
                self.push(Inst::Accept);
            }
            Node::Newline => {
                self.push(Inst::Newline);
            }
            Node::Grapheme => {
                self.push(Inst::Grapheme);
            }
            Node::Char(..) | Node::Any(_) | Node::NotNl | Node::Class(_) => {}
        }
        Ok(())
    }

    fn look_bounds(&self, ahead: bool, node: &Node) -> PResult<(u32, u32)> {
        if ahead {
            return Ok((0, 0));
        }
        let (min, max) = width(node);
        if max == INF || max > MAX_LOOKBEHIND {
            return Err(PcreError);
        }
        Ok((min, max))
    }

    fn emit_loop(&mut self, node: &Node, min: u32, max: u32, greedy: bool) -> PResult<()> {
        if max == 1 && min == 0 {
            let split = self.push(Inst::Split(0, 0));
            self.emit(node)?;
            let end = self.prog.len();
            self.prog[split] = if greedy {
                Inst::Split(split + 1, end)
            } else {
                Inst::Split(end, split + 1)
            };
            return Ok(());
        }
        if max == 1 && min == 1 {
            return self.emit(node);
        }
        let counter = self.reg();
        let mark = self.reg();
        self.push(Inst::SetReg(counter, 0));
        let head = self.push(Inst::LoopHead {
            counter,
            min,
            max,
            greedy,
            exit: 0,
        });
        self.push(Inst::LoopBody { counter, mark });
        self.emit(node)?;
        self.push(Inst::LoopEnd {
            counter,
            mark,
            min,
            head,
        });
        let exit = self.prog.len();
        self.prog[head] = Inst::LoopHead {
            counter,
            min,
            max,
            greedy,
            exit,
        };
        Ok(())
    }

    fn emit_cond(&mut self, cond: &Cond, yes: &Node, no: &Node) -> PResult<()> {
        match cond {
            Cond::Define => {
                let jmp = self.push(Inst::Jmp(0));
                self.emit(yes)?;
                let end = self.prog.len();
                self.prog[jmp] = Inst::Jmp(end);
            }
            Cond::Group(_) | Cond::Name(_) => {
                let groups = match cond {
                    Cond::Group(i) => {
                        if *i == 0 || *i > self.ngroups {
                            return Err(PcreError);
                        }
                        vec![*i]
                    }
                    Cond::Name(n) => self.groups_named(n)?,
                    _ => Vec::new(),
                };
                let at = self.push(Inst::CondGroup {
                    groups: groups.clone(),
                    else_pc: 0,
                });
                self.emit(yes)?;
                let jmp = self.push(Inst::Jmp(0));
                let else_pc = self.prog.len();
                self.emit(no)?;
                let end = self.prog.len();
                self.prog[at] = Inst::CondGroup { groups, else_pc };
                self.prog[jmp] = Inst::Jmp(end);
            }
            Cond::Look { ahead, neg, node } => {
                let (min, max) = self.look_bounds(*ahead, node)?;
                let at = self.push(Inst::Look {
                    ahead: *ahead,
                    neg: *neg,
                    min,
                    max,
                    end: 0,
                    else_pc: Some(0),
                });
                self.emit(node)?;
                self.push(Inst::SubEnd);
                let yes_pc = self.prog.len();
                self.emit(yes)?;
                let jmp = self.push(Inst::Jmp(0));
                let else_pc = self.prog.len();
                self.emit(no)?;
                let end = self.prog.len();
                self.prog[at] = Inst::Look {
                    ahead: *ahead,
                    neg: *neg,
                    min,
                    max,
                    end: yes_pc,
                    else_pc: Some(else_pc),
                };
                self.prog[jmp] = Inst::Jmp(end);
            }
        }
        Ok(())
    }
}

/// Option verbs at the start of a pattern that this engine accepts and ignores.
fn strip_start_verbs(chars: &mut Vec<char>) -> PResult<()> {
    const KNOWN: &[&str] = &[
        "UTF8",
        "UTF",
        "UCP",
        "ANYCRLF",
        "CRLF",
        "CR",
        "LF",
        "ANY",
        "NUL",
        "BSR_ANYCRLF",
        "BSR_UNICODE",
        "NO_JIT",
        "NO_START_OPT",
        "NO_AUTO_POSSESS",
        "NO_DOTSTAR_ANCHOR",
        "NOTEMPTY_ATSTART",
        "NOTEMPTY",
    ];
    loop {
        if !(chars.first() == Some(&'(') && chars.get(1) == Some(&'*')) {
            return Ok(());
        }
        let Some(end) = chars.iter().position(|&c| c == ')') else {
            return Ok(());
        };
        let word: String = chars[2..end].iter().collect();
        let known = KNOWN.contains(&word.as_str())
            || [
                "LIMIT_MATCH=",
                "LIMIT_DEPTH=",
                "LIMIT_HEAP=",
                "LIMIT_RECURSION=",
            ]
            .iter()
            .any(|p| {
                word.strip_prefix(p)
                    .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
            });
        if !known {
            return Ok(());
        }
        chars.drain(..=end);
    }
}

/// Compiles a pattern; `caseless` is the initial `(?i)` state.
pub(super) fn compile(pattern: &str, caseless: bool) -> PResult<Regex> {
    let mut chars: Vec<char> = pattern.chars().collect();
    strip_start_verbs(&mut chars)?;
    let total_groups = count_groups(&chars);
    let mut p = Parser {
        s: chars,
        pos: 0,
        flags: Flags {
            i: caseless,
            ..Flags::default()
        },
        ngroups: 0,
        total_groups,
        names: Vec::new(),
        classes: Vec::new(),
        depth: 0,
        quote: false,
    };
    let alts = p.parse_alts()?;
    if p.pos != p.s.len() {
        // An unmatched `)`.
        return Err(PcreError);
    }
    let root = if alts.len() == 1 {
        alts.into_iter().next().unwrap_or(Node::Empty)
    } else {
        Node::Alt(alts)
    };
    let ngroups = p.ngroups;
    let mut c = Compiler {
        prog: Vec::new(),
        nregs: 2 * (ngroups + 1),
        group_pc: vec![0; ngroups + 1],
        names: &p.names,
        ngroups,
    };
    c.push(Inst::Save(0));
    c.group_pc[0] = 1;
    c.emit(&root)?;
    c.push(Inst::Match);
    let mut required = Vec::new();
    required_literals(&root, &mut required);
    let first_char = match c.prog.get(1) {
        Some(Inst::One(Single::Char(ch))) => Some(*ch),
        _ => None,
    };
    let Compiler {
        prog,
        nregs,
        group_pc,
        ..
    } = c;
    Ok(Regex {
        prog,
        classes: p.classes,
        nregs,
        ngroups,
        group_pc,
        names: p.names,
        first_char,
        required,
    })
}
