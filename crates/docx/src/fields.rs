//! Word fields (ECMA-376 Part 1 section 17.16): the typed instruction model and the update
//! entry points.
//!
//! A field is either `w:fldSimple` or a complex field (`w:fldChar` begin / separate / end with
//! `w:instrText` between begin and separate), possibly nested and possibly spanning
//! paragraphs. Parsing never changes the tree: an untouched document saves byte-identical.
//! [`Document::update_fields`] rewrites the cached results the way Word's `Fields.Update`
//! does; see `docs/architecture/docx-fields.md`.

use crate::fields_date::CivilDateTime;
use crate::fields_number::NumberSymbols;
use crate::ranges::DocPos;

/// The field types this crate evaluates. Everything else is [`FieldKind::Other`] and keeps
/// its cached result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Page,
    NumPages,
    SectionPages,
    Section,
    Date,
    Time,
    CreateDate,
    SaveDate,
    PrintDate,
    EditTime,
    Author,
    LastSavedBy,
    Title,
    Subject,
    Keywords,
    Comments,
    FileName,
    DocProperty,
    Info,
    Ref,
    PageRef,
    NoteRef,
    Seq,
    StyleRef,
    If,
    Formula,
    MergeField,
    Hyperlink,
    Toc,
    Tc,
    Quote,
    Other,
}

impl FieldKind {
    fn from_name(name: &str) -> Self {
        match name {
            "PAGE" => Self::Page,
            "NUMPAGES" => Self::NumPages,
            "SECTIONPAGES" => Self::SectionPages,
            "SECTION" => Self::Section,
            "DATE" => Self::Date,
            "TIME" => Self::Time,
            "CREATEDATE" => Self::CreateDate,
            "SAVEDATE" => Self::SaveDate,
            "PRINTDATE" => Self::PrintDate,
            "EDITTIME" => Self::EditTime,
            "AUTHOR" => Self::Author,
            "LASTSAVEDBY" => Self::LastSavedBy,
            "TITLE" => Self::Title,
            "SUBJECT" => Self::Subject,
            "KEYWORDS" => Self::Keywords,
            "COMMENTS" => Self::Comments,
            "FILENAME" => Self::FileName,
            "DOCPROPERTY" => Self::DocProperty,
            "INFO" => Self::Info,
            "REF" => Self::Ref,
            "PAGEREF" => Self::PageRef,
            "NOTEREF" => Self::NoteRef,
            "SEQ" => Self::Seq,
            "STYLEREF" => Self::StyleRef,
            "IF" => Self::If,
            "=" => Self::Formula,
            "MERGEFIELD" => Self::MergeField,
            "HYPERLINK" => Self::Hyperlink,
            "TOC" => Self::Toc,
            "TC" => Self::Tc,
            "QUOTE" => Self::Quote,
            _ => Self::Other,
        }
    }
}

/// A positional argument of an instruction: a bare word or a quoted string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arg {
    pub text: String,
    pub quoted: bool,
}

/// A switch such as `\h`, `\* MERGEFORMAT` or `\@ "d MMMM yyyy"`. `name` is the text after the
/// backslash, with its case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Switch {
    pub name: String,
    pub arg: Option<String>,
}

/// A numbering or case format from a `\*` switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeneralFormat {
    Upper,
    Lower,
    FirstCap,
    Caps,
    CardText,
    OrdText,
    DollarText,
    Ordinal,
    Arabic,
    /// `\* Roman` (upper) and `\* roman` (lower; `true`).
    Roman(bool),
    /// `\* ALPHABETIC` and `\* alphabetic` (lower; `true`).
    Alphabetic(bool),
    Hex,
    MergeFormat,
    CharFormat,
}

impl GeneralFormat {
    fn parse(word: &str) -> Option<Self> {
        Some(match word {
            "Roman" | "ROMAN" => Self::Roman(false),
            "roman" => Self::Roman(true),
            "ALPHABETIC" => Self::Alphabetic(false),
            "alphabetic" => Self::Alphabetic(true),
            other => match other.to_ascii_lowercase().as_str() {
                "upper" => Self::Upper,
                "lower" => Self::Lower,
                "firstcap" => Self::FirstCap,
                "caps" => Self::Caps,
                "cardtext" => Self::CardText,
                "ordtext" => Self::OrdText,
                "dollartext" => Self::DollarText,
                "ordinal" => Self::Ordinal,
                "arabic" => Self::Arabic,
                "hex" => Self::Hex,
                "mergeformat" => Self::MergeFormat,
                "charformat" => Self::CharFormat,
                _ => return None,
            },
        })
    }
}

/// A parsed field instruction (`w:instrText` / `w:fldSimple/@w:instr`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instruction {
    /// The instruction exactly as stored.
    pub text: String,
    /// Field type in upper case; `=` for a formula.
    pub name: String,
    pub kind: FieldKind,
    /// Text between the field name and the first switch, whitespace-trimmed.
    pub body: String,
    pub args: Vec<Arg>,
    pub switches: Vec<Switch>,
}

impl Instruction {
    /// Parses instruction text. Never fails: an empty instruction has an empty name.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let tokens = tokenize(text);
        let mut iter = tokens.into_iter().peekable();
        let (name, name_end) = match iter.next() {
            Some(Token::Word { text: w, end, .. }) => (w, end),
            Some(Token::Switch { .. }) | None => (String::new(), 0),
        };
        // A formula may be written `=1+2` without a blank after the sign.
        let (name, glued) = match name.strip_prefix('=') {
            Some(rest) if !name.is_empty() && name != "=" => {
                ("=".to_owned(), Some(rest.to_owned()))
            }
            _ => (name, None),
        };
        let name = name.to_uppercase();
        let kind = FieldKind::from_name(&name);
        let mut args = Vec::new();
        let mut switches: Vec<Switch> = Vec::new();
        let mut body_end: Option<usize> = None;
        if let Some(rest) = glued
            && !rest.is_empty()
        {
            args.push(Arg {
                text: rest,
                quoted: false,
            });
        }
        while let Some(token) = iter.next() {
            match token {
                Token::Word { text, quoted, .. } => args.push(Arg { text, quoted }),
                Token::Switch {
                    name: sw,
                    start,
                    glued,
                } => {
                    body_end.get_or_insert(start);
                    let takes = switch_takes_arg(&name, &sw);
                    let optional = switch_optional_arg(&name, &sw);
                    let mut arg = glued;
                    if arg.is_none() && (takes || optional) {
                        let next_ok = match iter.peek() {
                            Some(Token::Word { text, quoted, .. }) => {
                                takes || *quoted || looks_like_range(text)
                            }
                            _ => false,
                        };
                        if next_ok && let Some(Token::Word { text, .. }) = iter.next() {
                            arg = Some(text);
                        }
                    }
                    switches.push(Switch { name: sw, arg });
                }
            }
        }
        let body_to = body_end.unwrap_or(text.len());
        let body = text
            .get(name_end.min(body_to)..body_to)
            .unwrap_or_default()
            .trim()
            .to_owned();
        Self {
            text: text.to_owned(),
            name,
            kind,
            body,
            args,
            switches,
        }
    }

    /// The first switch called `name` (`h`, `*`, ...), case-sensitive.
    #[must_use]
    pub fn switch(&self, name: &str) -> Option<&Switch> {
        self.switches.iter().find(|s| s.name == name)
    }

    /// True when the switch is present.
    #[must_use]
    pub fn has_switch(&self, name: &str) -> bool {
        self.switch(name).is_some()
    }

    /// Argument of switch `name`.
    #[must_use]
    pub fn switch_arg(&self, name: &str) -> Option<&str> {
        self.switch(name).and_then(|s| s.arg.as_deref())
    }

    /// The `\*` formats in order of appearance.
    #[must_use]
    pub fn general_formats(&self) -> Vec<GeneralFormat> {
        self.switches
            .iter()
            .filter(|s| s.name == "*")
            .filter_map(|s| s.arg.as_deref().and_then(GeneralFormat::parse))
            .collect()
    }

    /// Argument of the first `\#` switch.
    #[must_use]
    pub fn number_picture(&self) -> Option<&str> {
        self.switch_arg("#")
    }

    /// Argument of the first `\@` switch.
    #[must_use]
    pub fn date_picture(&self) -> Option<&str> {
        self.switch_arg("@")
    }

    /// First positional argument.
    #[must_use]
    pub fn first_arg(&self) -> Option<&str> {
        self.args.first().map(|a| a.text.as_str())
    }
}

enum Token {
    Word {
        text: String,
        quoted: bool,
        /// Byte offset just after the token in the source.
        end: usize,
    },
    Switch {
        name: String,
        /// Byte offset of the backslash.
        start: usize,
        /// Argument glued to the switch letter (`\o"1-3"`, `\*Upper`).
        glued: Option<String>,
    },
}

/// Splits instruction text into words, quoted strings and switches.
fn tokenize(text: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let bytes: Vec<(usize, char)> = text.char_indices().collect();
    let mut i = 0usize;
    let at = |i: usize| bytes.get(i).map(|(_, c)| *c);
    let offset = |i: usize| bytes.get(i).map_or(text.len(), |(o, _)| *o);
    while i < bytes.len() {
        let c = bytes[i].1;
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '\\' {
            let start = offset(i);
            i += 1;
            let mut name = String::new();
            if let Some(first) = at(i) {
                name.push(first);
                i += 1;
            }
            let mut glued = None;
            if matches!(name.as_str(), "*" | "#" | "@")
                && let Some(next) = at(i)
                && !next.is_whitespace()
            {
                if next == '"' {
                    let (word, ni) = read_quoted(&bytes, i);
                    glued = Some(word);
                    i = ni;
                } else {
                    let mut word = String::new();
                    while let Some(ch) = at(i) {
                        if ch.is_whitespace() || ch == '\\' {
                            break;
                        }
                        word.push(ch);
                        i += 1;
                    }
                    glued = Some(word);
                }
            } else if name.chars().all(|ch| ch.is_ascii_alphabetic()) && at(i) == Some('"') {
                let (word, ni) = read_quoted(&bytes, i);
                glued = Some(word);
                i = ni;
            }
            out.push(Token::Switch { name, start, glued });
            continue;
        }
        if c == '"' {
            let (word, ni) = read_quoted(&bytes, i);
            i = ni;
            out.push(Token::Word {
                text: word,
                quoted: true,
                end: offset(i),
            });
            continue;
        }
        let mut word = String::new();
        while let Some(ch) = at(i) {
            if ch.is_whitespace() {
                break;
            }
            word.push(ch);
            i += 1;
        }
        out.push(Token::Word {
            text: word,
            quoted: false,
            end: offset(i),
        });
    }
    out
}

/// Reads a quoted string starting at the opening quote `bytes[i]`; `\"` and `\\` are escapes.
fn read_quoted(bytes: &[(usize, char)], start: usize) -> (String, usize) {
    let mut i = start + 1;
    let mut word = String::new();
    while let Some((_, ch)) = bytes.get(i) {
        match ch {
            '\\' => {
                if let Some((_, next)) = bytes.get(i + 1)
                    && matches!(next, '"' | '\\')
                {
                    word.push(*next);
                    i += 2;
                    continue;
                }
                word.push('\\');
                i += 1;
            }
            '"' => {
                i += 1;
                return (word, i);
            }
            other => {
                word.push(*other);
                i += 1;
            }
        }
    }
    (word, i)
}

fn looks_like_range(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_ascii_digit() || c == '-')
}

/// Switches that always take the next word or quoted string as their argument.
fn switch_takes_arg(field: &str, switch: &str) -> bool {
    if matches!(switch, "*" | "#" | "@") {
        return true;
    }
    match field {
        "TC" => matches!(switch, "f" | "l"),
        "TOC" | "TOA" | "TOF" => matches!(
            switch,
            "a" | "b" | "c" | "d" | "f" | "l" | "o" | "p" | "s" | "t"
        ),
        "HYPERLINK" => matches!(switch, "l" | "o" | "t"),
        "REF" => switch == "d",
        "SEQ" => matches!(switch, "r" | "s"),
        "MERGEFIELD" => matches!(switch, "b" | "f"),
        "INDEX" | "XE" => matches!(
            switch,
            "b" | "c" | "d" | "e" | "f" | "g" | "h" | "k" | "l" | "p" | "r" | "s" | "y"
        ),
        _ => false,
    }
}

/// Switches whose argument may be left out (`\n` in TOC takes a level range or nothing).
fn switch_optional_arg(field: &str, switch: &str) -> bool {
    matches!((field, switch), ("TOC", "n"))
}

// ---------------------------------------------------------------------------
// Caller-supplied context
// ---------------------------------------------------------------------------

/// What a layout engine knows about one position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageInfo {
    /// Physical page, 1-based.
    pub page: u32,
    /// Number shown on the page after restarts and offsets.
    pub number: u32,
    /// The page label when it is not the decimal `number` (`iv`, `B`).
    pub label: Option<String>,
    /// Index of the section the page belongs to.
    pub section: usize,
}

/// Page numbers come from layout, which lives elsewhere.
pub trait PageProvider {
    /// Page holding the character at `pos` (paragraph index and character offset).
    fn page_at(&self, pos: DocPos) -> Option<PageInfo>;
    /// Number of pages in the document.
    fn total_pages(&self) -> Option<u32>;
    /// Number of pages in section `section`.
    fn section_pages(&self, section: usize) -> Option<u32>;
}

/// A provider that knows nothing; page fields keep their cached results.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoPages;

impl PageProvider for NoPages {
    fn page_at(&self, _pos: DocPos) -> Option<PageInfo> {
        None
    }
    fn total_pages(&self) -> Option<u32> {
        None
    }
    fn section_pages(&self, _section: usize) -> Option<u32> {
        None
    }
}

/// Mail-merge data for `MERGEFIELD`; without it the field shows its placeholder.
pub trait MergeData {
    fn value(&self, field: &str) -> Option<String>;
}

/// Month, day and AM/PM names for a language, to override the built-in table.
pub trait DateNames {
    /// Names for the BCP 47 tag, when known.
    fn names(&self, tag: &str) -> Option<crate::fields_locale::LocaleNames>;
}

/// Everything an update needs from outside the document.
pub struct FieldContext<'a> {
    /// Local time for `DATE` and `TIME`.
    pub now: CivilDateTime,
    /// Minutes east of UTC applied to document properties stored in UTC.
    pub utc_offset_minutes: i32,
    pub file_name: Option<String>,
    /// Full path for `FILENAME \p`.
    pub file_path: Option<String>,
    pub user_name: Option<String>,
    pub user_initials: Option<String>,
    pub user_address: Option<String>,
    pub pages: &'a dyn PageProvider,
    pub merge: Option<&'a dyn MergeData>,
    pub date_names: Option<&'a dyn DateNames>,
    pub number_symbols: NumberSymbols,
    /// Language used when the field's run, style and document defaults name none.
    pub default_language: String,
}

impl std::fmt::Debug for FieldContext<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FieldContext")
            .field("now", &self.now)
            .field("utc_offset_minutes", &self.utc_offset_minutes)
            .field("file_name", &self.file_name)
            .finish_non_exhaustive()
    }
}

static NO_PAGES: NoPages = NoPages;

impl FieldContext<'static> {
    /// A context with the given clock, no layout, English and no merge data.
    #[must_use]
    pub fn new(now: CivilDateTime) -> Self {
        Self {
            now,
            utc_offset_minutes: 0,
            file_name: None,
            file_path: None,
            user_name: None,
            user_initials: None,
            user_address: None,
            pages: &NO_PAGES,
            merge: None,
            date_names: None,
            number_symbols: NumberSymbols::default(),
            default_language: "en-US".to_owned(),
        }
    }
}

impl<'a> FieldContext<'a> {
    /// The same context reading page numbers from `pages`.
    #[must_use]
    pub fn with_pages<'b>(self, pages: &'b dyn PageProvider) -> FieldContext<'b>
    where
        'a: 'b,
    {
        FieldContext {
            now: self.now,
            utc_offset_minutes: self.utc_offset_minutes,
            file_name: self.file_name,
            file_path: self.file_path,
            user_name: self.user_name,
            user_initials: self.user_initials,
            user_address: self.user_address,
            pages,
            merge: self.merge,
            date_names: self.date_names,
            number_symbols: self.number_symbols,
            default_language: self.default_language,
        }
    }
}

/// A field found in the main story.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// Position among all fields in begin-marker order (nested fields follow their parent).
    pub index: usize,
    pub instruction: Instruction,
    /// 0 for a top-level field.
    pub depth: usize,
    /// Index of the enclosing field.
    pub parent: Option<usize>,
    /// `w:fldSimple` rather than `w:fldChar`.
    pub simple: bool,
    /// Index of the paragraph holding the begin marker.
    pub paragraph: usize,
    /// The cached result as Word's `Range.Text` shows it: nested results included, nested
    /// codes excluded, `\t` for tabs, `\u{b}` for line breaks, `\r` between paragraphs.
    pub result_text: String,
    /// `w:fldLock`: the field never updates.
    pub locked: bool,
    /// `w:dirty`: the file asks for an update on open.
    pub dirty: bool,
}

/// What [`Document::update_fields`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FieldUpdateReport {
    /// Fields whose result was rewritten (the text may be unchanged).
    pub updated: usize,
    /// Fields left as they were: locked, unsupported type or missing input.
    pub skipped: Vec<SkippedField>,
}

/// A field `update_fields` left alone, with the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedField {
    pub index: usize,
    pub name: String,
    pub reason: SkipReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    Locked,
    UnsupportedType,
    /// The layout provider (or another input) could not answer.
    MissingInput,
}
