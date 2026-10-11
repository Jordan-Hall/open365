//! Parser for Hunspell `.aff` affix files, written from the public format description
//! (`man 5 hunspell`, the Hunspell/Nuspell documentation). No Hunspell source is used.

use std::collections::{HashMap, HashSet};

use crate::error::SpellError;
use crate::flags::{Flag, FlagMode, FlagSet, parse_flags, parse_single_flag};

/// One position of an affix condition.
#[derive(Clone, Debug)]
enum CondItem {
    Any,
    Lit(char),
    Set { negated: bool, chars: Vec<char> },
}

impl CondItem {
    fn matches(&self, c: char) -> bool {
        match self {
            Self::Any => true,
            Self::Lit(lit) => *lit == c,
            Self::Set { negated, chars } => chars.contains(&c) != *negated,
        }
    }
}

/// An affix condition: a tiny pattern language of literals, `.` and `[...]` / `[^...]`.
#[derive(Clone, Debug, Default)]
pub(crate) struct Condition(Vec<CondItem>);

impl Condition {
    pub(crate) fn parse(text: &str) -> Self {
        if text == "." {
            return Self::default();
        }
        let mut items = Vec::new();
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '.' => items.push(CondItem::Any),
                '[' => {
                    let negated = chars.peek() == Some(&'^');
                    if negated {
                        chars.next();
                    }
                    let mut set = Vec::new();
                    for member in chars.by_ref() {
                        if member == ']' {
                            break;
                        }
                        set.push(member);
                    }
                    items.push(CondItem::Set {
                        negated,
                        chars: set,
                    });
                }
                other => items.push(CondItem::Lit(other)),
            }
        }
        Self(items)
    }

    pub(crate) fn matches_start(&self, text: &str) -> bool {
        let mut it = text.chars();
        self.0
            .iter()
            .all(|item| it.next().is_some_and(|c| item.matches(c)))
    }

    pub(crate) fn matches_end(&self, text: &str) -> bool {
        let mut it = text.chars().rev();
        self.0
            .iter()
            .rev()
            .all(|item| it.next().is_some_and(|c| item.matches(c)))
    }
}

/// One `PFX` or `SFX` rule line.
#[derive(Clone, Debug)]
pub(crate) struct AffixRule {
    pub(crate) flag: Flag,
    pub(crate) cross: bool,
    pub(crate) strip: String,
    pub(crate) append: String,
    pub(crate) cond: Condition,
    /// Continuation flags: other affixes allowed after this one, plus special flags such as `CIRCUMFIX`.
    pub(crate) cont: FlagSet,
}

/// All rules of one kind, indexed by the string they append.
#[derive(Clone, Debug, Default)]
pub(crate) struct AffixTable {
    pub(crate) rules: Vec<AffixRule>,
    pub(crate) by_append: HashMap<String, Vec<usize>>,
    pub(crate) max_append_chars: usize,
}

impl AffixTable {
    fn push(&mut self, rule: AffixRule) {
        self.max_append_chars = self.max_append_chars.max(rule.append.chars().count());
        self.by_append
            .entry(rule.append.clone())
            .or_default()
            .push(self.rules.len());
        self.rules.push(rule);
    }
}

/// A `BREAK` pattern.
#[derive(Clone, Debug)]
pub(crate) struct BreakPattern {
    pub(crate) text: String,
    pub(crate) at_start: bool,
    pub(crate) at_end: bool,
}

/// Quantifier of a `COMPOUNDRULE` item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Quant {
    One,
    Star,
    Opt,
}

/// One item of a `COMPOUNDRULE`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RuleItem {
    pub(crate) flag: Flag,
    pub(crate) quant: Quant,
}

/// A `CHECKCOMPOUNDPATTERN` entry.
#[derive(Clone, Debug)]
pub(crate) struct CompoundPattern {
    pub(crate) end: String,
    pub(crate) end_flag: Option<Flag>,
    pub(crate) begin: String,
    pub(crate) begin_flag: Option<Flag>,
}

/// Parsed affix file.
#[derive(Clone, Debug, Default)]
pub(crate) struct Aff {
    pub(crate) lang: String,
    pub(crate) flag_mode: FlagMode,
    pub(crate) try_chars: Vec<char>,
    pub(crate) key_rows: Vec<Vec<char>>,
    pub(crate) word_chars: Vec<char>,
    pub(crate) ignore: Vec<char>,
    pub(crate) iconv: Vec<(String, String)>,
    pub(crate) oconv: Vec<(String, String)>,
    pub(crate) rep: Vec<(String, String)>,
    pub(crate) map: Vec<Vec<String>>,
    pub(crate) break_table: Option<Vec<BreakPattern>>,
    pub(crate) af: Vec<Vec<Flag>>,

    pub(crate) nosuggest: Option<Flag>,
    pub(crate) forbidden: Option<Flag>,
    pub(crate) keepcase: Option<Flag>,
    pub(crate) needaffix: Option<Flag>,
    pub(crate) only_in_compound: Option<Flag>,
    pub(crate) circumfix: Option<Flag>,
    pub(crate) force_ucase: Option<Flag>,

    pub(crate) compound_flag: Option<Flag>,
    pub(crate) compound_begin: Option<Flag>,
    pub(crate) compound_middle: Option<Flag>,
    pub(crate) compound_end: Option<Flag>,
    pub(crate) compound_permit: Option<Flag>,
    pub(crate) compound_forbid: Option<Flag>,
    pub(crate) compound_min: usize,
    pub(crate) compound_word_max: Option<usize>,
    pub(crate) check_compound_dup: bool,
    pub(crate) check_compound_case: bool,
    pub(crate) check_compound_triple: bool,
    pub(crate) compound_rules: Vec<Vec<RuleItem>>,
    pub(crate) compound_patterns: Vec<CompoundPattern>,

    pub(crate) prefixes: AffixTable,
    pub(crate) suffixes: AffixTable,

    pub(crate) max_ngram_sugs: usize,
    pub(crate) max_cpd_sugs: usize,
    pub(crate) no_split_sugs: bool,
    pub(crate) full_strip: bool,
    /// `COMPLEXPREFIXES` is recorded but not supported by the matcher.
    pub(crate) complex_prefixes: bool,
}

impl Aff {
    pub(crate) fn parse(text: &str) -> Result<Self, SpellError> {
        let mut aff = Self {
            compound_min: 3,
            max_ngram_sugs: 4,
            max_cpd_sugs: 3,
            ..Self::default()
        };
        let mut seen: HashSet<&'static str> = HashSet::new();
        let mut cross: HashMap<(bool, Flag), bool> = HashMap::new();
        let mut break_table: Option<Vec<BreakPattern>> = None;

        for (index, raw) in text.lines().enumerate() {
            let line_no = index + 1;
            let line = raw.trim_start_matches('\u{feff}').trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let tokens: Vec<&str> = line.split_whitespace().collect();
            let Some(&name) = tokens.first() else {
                continue;
            };
            let arg = tokens.get(1).copied().unwrap_or_default();
            let err = |message: String| SpellError::Aff {
                line: line_no,
                message,
            };
            let mode = aff.flag_mode;
            let single = |value: &str| -> Result<Option<Flag>, SpellError> {
                parse_single_flag(value, mode).map(Some).map_err(&err)
            };
            // The first line of a counted directive carries only the entry count.
            let mut header = |directive: &'static str| -> bool {
                if !seen.contains(directive) && arg.parse::<usize>().is_ok() {
                    seen.insert(directive);
                    true
                } else {
                    false
                }
            };

            match name {
                "FLAG" => {
                    aff.flag_mode = match arg.to_ascii_lowercase().as_str() {
                        "long" => FlagMode::Long,
                        "num" => FlagMode::Num,
                        _ => FlagMode::Char,
                    }
                }
                "LANG" => aff.lang = arg.to_owned(),
                "TRY" => aff.try_chars = arg.chars().collect(),
                "KEY" => {
                    aff.key_rows = arg.split('|').map(|row| row.chars().collect()).collect();
                }
                "WORDCHARS" => aff.word_chars = arg.chars().collect(),
                "IGNORE" => aff.ignore = arg.chars().collect(),
                "ICONV" | "OCONV" => {
                    let directive = if name == "ICONV" { "ICONV" } else { "OCONV" };
                    if header(directive) {
                        continue;
                    }
                    if let (Some(from), Some(to)) = (tokens.get(1), tokens.get(2)) {
                        let pair = ((*from).to_owned(), (*to).to_owned());
                        if name == "ICONV" {
                            aff.iconv.push(pair);
                        } else {
                            aff.oconv.push(pair);
                        }
                    }
                }
                "REP" => {
                    if header("REP") {
                        continue;
                    }
                    if let (Some(from), Some(to)) = (tokens.get(1), tokens.get(2)) {
                        aff.rep.push(((*from).to_owned(), (*to).to_owned()));
                    }
                }
                "MAP" => {
                    if header("MAP") {
                        continue;
                    }
                    let group = parse_map_units(arg);
                    if group.len() > 1 {
                        aff.map.push(group);
                    }
                }
                "BREAK" => {
                    let table = break_table.get_or_insert_with(Vec::new);
                    if header("BREAK") {
                        continue;
                    }
                    let at_start = arg.starts_with('^');
                    let body = arg.strip_prefix('^').unwrap_or(arg);
                    let (text, at_end) = match body.strip_suffix('$') {
                        Some(stripped) if !stripped.is_empty() => (stripped, true),
                        _ => (body, false),
                    };
                    if !text.is_empty() {
                        table.push(BreakPattern {
                            text: text.to_owned(),
                            at_start,
                            at_end,
                        });
                    }
                }
                "AF" => {
                    if header("AF") {
                        continue;
                    }
                    let flags = parse_flags(arg, aff.flag_mode).map_err(&err)?;
                    aff.af.push(flags);
                }
                "NOSUGGEST" => aff.nosuggest = single(arg)?,
                "FORBIDDENWORD" => aff.forbidden = single(arg)?,
                "KEEPCASE" => aff.keepcase = single(arg)?,
                "NEEDAFFIX" | "PSEUDOROOT" => aff.needaffix = single(arg)?,
                "ONLYINCOMPOUND" => aff.only_in_compound = single(arg)?,
                "CIRCUMFIX" => aff.circumfix = single(arg)?,
                "FORCEUCASE" => aff.force_ucase = single(arg)?,
                "COMPOUNDFLAG" => aff.compound_flag = single(arg)?,
                "COMPOUNDBEGIN" => aff.compound_begin = single(arg)?,
                "COMPOUNDMIDDLE" => aff.compound_middle = single(arg)?,
                "COMPOUNDEND" => aff.compound_end = single(arg)?,
                "COMPOUNDPERMITFLAG" => aff.compound_permit = single(arg)?,
                "COMPOUNDFORBIDFLAG" => aff.compound_forbid = single(arg)?,
                "COMPOUNDMIN" => {
                    aff.compound_min = arg.parse().map_err(|_| err("bad COMPOUNDMIN".into()))?;
                }
                "COMPOUNDWORDMAX" => aff.compound_word_max = arg.parse().ok(),
                "CHECKCOMPOUNDDUP" => aff.check_compound_dup = true,
                "CHECKCOMPOUNDCASE" => aff.check_compound_case = true,
                "CHECKCOMPOUNDTRIPLE" => aff.check_compound_triple = true,
                "COMPOUNDRULE" => {
                    if header("COMPOUNDRULE") {
                        continue;
                    }
                    let rule = parse_compound_rule(arg, aff.flag_mode).map_err(&err)?;
                    if !rule.is_empty() {
                        aff.compound_rules.push(rule);
                    }
                }
                "CHECKCOMPOUNDPATTERN" => {
                    if header("CHECKCOMPOUNDPATTERN") {
                        continue;
                    }
                    if let (Some(end), Some(begin)) = (tokens.get(1), tokens.get(2)) {
                        let (end, end_flag) = split_pattern_flag(end, aff.flag_mode);
                        let (begin, begin_flag) = split_pattern_flag(begin, aff.flag_mode);
                        aff.compound_patterns.push(CompoundPattern {
                            end,
                            end_flag,
                            begin,
                            begin_flag,
                        });
                    }
                }
                "MAXNGRAMSUGS" => aff.max_ngram_sugs = arg.parse().unwrap_or(4),
                "MAXCPDSUGS" => aff.max_cpd_sugs = arg.parse().unwrap_or(3),
                "NOSPLITSUGS" => aff.no_split_sugs = true,
                "FULLSTRIP" => aff.full_strip = true,
                "COMPLEXPREFIXES" => aff.complex_prefixes = true,
                "PFX" | "SFX" => {
                    let is_prefix = name == "PFX";
                    if tokens.len() == 4
                        && matches!(tokens[2], "Y" | "N")
                        && tokens[3].parse::<usize>().is_ok()
                    {
                        let flag = parse_single_flag(tokens[1], aff.flag_mode).map_err(&err)?;
                        cross.insert((is_prefix, flag), tokens[2] == "Y");
                        continue;
                    }
                    if tokens.len() < 4 {
                        return Err(err(format!("incomplete {name} rule")));
                    }
                    let flag = parse_single_flag(tokens[1], aff.flag_mode).map_err(&err)?;
                    let strip = if tokens[2] == "0" { "" } else { tokens[2] };
                    let (append, cont) = match tokens[3].split_once('/') {
                        Some((append, cont)) => (append, cont),
                        None => (tokens[3], ""),
                    };
                    let append = if append == "0" { "" } else { append };
                    let cond = Condition::parse(tokens.get(4).copied().unwrap_or("."));
                    let cont = FlagSet::new(parse_flags(cont, aff.flag_mode).map_err(&err)?);
                    let rule = AffixRule {
                        flag,
                        cross: cross.get(&(is_prefix, flag)).copied().unwrap_or(true),
                        strip: strip.to_owned(),
                        append: append.to_owned(),
                        cond,
                        cont,
                    };
                    if is_prefix {
                        aff.prefixes.push(rule);
                    } else {
                        aff.suffixes.push(rule);
                    }
                }
                _ => {}
            }
        }
        aff.break_table = break_table;
        Ok(aff)
    }

    /// True when any compounding directive is present.
    pub(crate) fn has_compounding(&self) -> bool {
        self.compound_flag.is_some()
            || self.compound_begin.is_some()
            || self.compound_middle.is_some()
            || self.compound_end.is_some()
            || !self.compound_rules.is_empty()
    }
}

fn parse_map_units(text: &str) -> Vec<String> {
    let mut units = Vec::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '(' {
            let mut unit = String::new();
            for member in chars.by_ref() {
                if member == ')' {
                    break;
                }
                unit.push(member);
            }
            if !unit.is_empty() {
                units.push(unit);
            }
        } else {
            units.push(c.to_string());
        }
    }
    units
}

fn parse_compound_rule(text: &str, mode: FlagMode) -> Result<Vec<RuleItem>, String> {
    let mut items = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let flag = if c == '(' {
            let mut inner = String::new();
            for member in chars.by_ref() {
                if member == ')' {
                    break;
                }
                inner.push(member);
            }
            parse_single_flag(&inner, mode)?
        } else if c == '*' || c == '?' {
            return Err(format!("misplaced `{c}` in COMPOUNDRULE `{text}`"));
        } else if mode == FlagMode::Char {
            c as u32
        } else {
            return Err(format!("COMPOUNDRULE `{text}` needs parenthesised flags"));
        };
        let quant = match chars.peek() {
            Some('*') => {
                chars.next();
                Quant::Star
            }
            Some('?') => {
                chars.next();
                Quant::Opt
            }
            _ => Quant::One,
        };
        items.push(RuleItem { flag, quant });
    }
    Ok(items)
}

fn split_pattern_flag(text: &str, mode: FlagMode) -> (String, Option<Flag>) {
    let (word, flag) = match text.split_once('/') {
        Some((word, flag)) => (word, parse_single_flag(flag, mode).ok()),
        None => (text, None),
    };
    let word = if word == "0" { "" } else { word };
    (word.to_owned(), flag)
}
