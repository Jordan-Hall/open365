//! Citations and bibliography: a source record and three citation styles (APA, MLA, Chicago
//! author-date), written from the public style descriptions. Nothing here touches a document;
//! the command layer turns the text into `CITATION` and `BIBLIOGRAPHY` fields.

use serde::{Deserialize, Serialize};

/// A person named as an author.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Author {
    pub last: String,
    #[serde(default)]
    pub first: String,
}

impl Author {
    #[must_use]
    pub fn new(last: &str, first: &str) -> Self {
        Self {
            last: last.to_owned(),
            first: first.to_owned(),
        }
    }

    /// `F. M.` from the given names.
    fn initials(&self) -> String {
        self.first
            .split_whitespace()
            .filter_map(|w| w.chars().next())
            .map(|c| format!("{}.", c.to_uppercase()))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// `Last, F. M.` (APA).
    fn inverted_initials(&self) -> String {
        let i = self.initials();
        if i.is_empty() {
            self.last.clone()
        } else {
            format!("{}, {i}", self.last)
        }
    }

    /// `Last, First` (MLA, Chicago first author).
    fn inverted(&self) -> String {
        if self.first.is_empty() {
            self.last.clone()
        } else {
            format!("{}, {}", self.last, self.first)
        }
    }

    /// `First Last` (later authors in MLA and Chicago).
    fn natural(&self) -> String {
        if self.first.is_empty() {
            self.last.clone()
        } else {
            format!("{} {}", self.first, self.last)
        }
    }
}

/// The type of a source (the common subset of the source types Word offers).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Book,
    JournalArticle,
    Website,
}

/// One bibliographic source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    /// The tag a `CITATION` field refers to (`Smith2020`).
    pub tag: String,
    pub kind: SourceKind,
    pub authors: Vec<Author>,
    pub title: String,
    pub year: String,
    /// Month name and day (websites).
    #[serde(default)]
    pub month: String,
    #[serde(default)]
    pub day: String,
    /// The publisher, or the site name for a website.
    #[serde(default)]
    pub publisher: String,
    #[serde(default)]
    pub city: String,
    /// The journal title.
    #[serde(default)]
    pub journal: String,
    #[serde(default)]
    pub volume: String,
    #[serde(default)]
    pub issue: String,
    #[serde(default)]
    pub pages: String,
    #[serde(default)]
    pub url: String,
}

impl Source {
    /// A source with only the required parts; fill the rest through the fields.
    #[must_use]
    pub fn new(tag: &str, kind: SourceKind, authors: Vec<Author>, title: &str, year: &str) -> Self {
        Self {
            tag: tag.to_owned(),
            kind,
            authors,
            title: title.to_owned(),
            year: year.to_owned(),
            month: String::new(),
            day: String::new(),
            publisher: String::new(),
            city: String::new(),
            journal: String::new(),
            volume: String::new(),
            issue: String::new(),
            pages: String::new(),
            url: String::new(),
        }
    }
}

/// A bibliography style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BibStyle {
    #[default]
    Apa,
    Mla,
    Chicago,
}

impl BibStyle {
    /// Every style, in the order of the Style box.
    pub const ALL: [BibStyle; 3] = [BibStyle::Apa, BibStyle::Chicago, BibStyle::Mla];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Apa => "APA",
            Self::Mla => "MLA",
            Self::Chicago => "Chicago",
        }
    }

    /// The style a Style box entry names (`APA`, `MLA`, `Chicago`, any case or edition suffix).
    #[must_use]
    pub fn from_label(text: &str) -> Option<Self> {
        let t = text.trim().to_ascii_lowercase();
        if t.starts_with("apa") {
            Some(Self::Apa)
        } else if t.starts_with("mla") {
            Some(Self::Mla)
        } else if t.starts_with("chicago") {
            Some(Self::Chicago)
        } else {
            None
        }
    }

    /// The heading the style uses by default.
    #[must_use]
    pub fn heading(self) -> &'static str {
        match self {
            Self::Apa => "References",
            Self::Mla => "Works Cited",
            Self::Chicago => "Bibliography",
        }
    }
}

/// A piece of an entry; `italic` pieces are titles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seg {
    pub text: String,
    pub italic: bool,
}

#[derive(Default)]
struct Entry(Vec<Seg>);

impl Entry {
    fn t(&mut self, s: &str) -> &mut Self {
        if !s.is_empty() {
            self.0.push(Seg {
                text: s.to_owned(),
                italic: false,
            });
        }
        self
    }

    fn i(&mut self, s: &str) -> &mut Self {
        if !s.is_empty() {
            self.0.push(Seg {
                text: s.to_owned(),
                italic: true,
            });
        }
        self
    }

    /// Ends the text so far with `.` unless it already ends with `.`, `?` or `!`.
    fn stop(&mut self) -> &mut Self {
        if let Some(last) = self.0.last_mut()
            && !last.text.ends_with(['.', '?', '!'])
        {
            last.text.push('.');
        }
        self
    }
}

/// The plain text of an entry.
#[must_use]
pub fn plain(segs: &[Seg]) -> String {
    segs.iter().map(|s| s.text.as_str()).collect()
}

fn authors_apa(s: &Source) -> String {
    let names: Vec<String> = s.authors.iter().map(Author::inverted_initials).collect();
    match names.split_last() {
        None => String::new(),
        Some((only, [])) => only.clone(),
        Some((last, rest)) => format!("{}, & {last}", rest.join(", ")),
    }
}

fn authors_full(s: &Source) -> String {
    let names: Vec<String> = s
        .authors
        .iter()
        .enumerate()
        .map(|(i, a)| if i == 0 { a.inverted() } else { a.natural() })
        .collect();
    match names.split_last() {
        None => String::new(),
        Some((only, [])) => only.clone(),
        Some((last, [one])) => format!("{one}, and {last}"),
        Some((last, rest)) => format!("{}, and {last}", rest.join(", ")),
    }
}

fn authors_mla(s: &Source) -> String {
    match s.authors.as_slice() {
        [first, _, _, ..] => format!("{}, et al", first.inverted()),
        _ => authors_full(s),
    }
}

/// The bibliography entry for `s` in `style`.
#[must_use]
pub fn bibliography_entry(s: &Source, style: BibStyle) -> Vec<Seg> {
    let mut e = Entry::default();
    match style {
        BibStyle::Apa => entry_apa(s, &mut e),
        BibStyle::Mla => entry_mla(s, &mut e),
        BibStyle::Chicago => entry_chicago(s, &mut e),
    }
    e.0
}

fn quoted(title: &str) -> String {
    format!("\u{201C}{}.\u{201D} ", title.trim_end_matches('.'))
}

fn entry_apa(s: &Source, e: &mut Entry) {
    let a = authors_apa(s);
    let has_author = !a.is_empty();
    if has_author {
        e.t(&a).stop().t(" ");
    } else if s.kind == SourceKind::JournalArticle {
        // Without an author the title leads.
        e.t(&s.title).stop().t(" ");
    } else {
        e.i(&s.title).stop().t(" ");
    }
    let mut year = if s.year.is_empty() {
        "n.d.".to_owned()
    } else {
        s.year.clone()
    };
    if s.kind == SourceKind::Website && !s.month.is_empty() {
        year = format!("{year}, {} {}", s.month, s.day)
            .trim_end()
            .to_owned();
    }
    e.t(&format!("({year}).")).t(" ");
    match s.kind {
        SourceKind::Book => {
            if has_author {
                e.i(&s.title).stop();
            }
            if !s.publisher.is_empty() {
                e.t(" ").t(&s.publisher).stop();
            }
        }
        SourceKind::JournalArticle => {
            if has_author {
                e.t(&s.title).stop().t(" ");
            }
            e.i(&s.journal);
            if !s.volume.is_empty() {
                e.t(", ").i(&s.volume);
            }
            if !s.issue.is_empty() {
                e.t(&format!("({})", s.issue));
            }
            if !s.pages.is_empty() {
                e.t(&format!(", {}", s.pages.replace('-', "\u{2013}")));
            }
            e.stop();
        }
        SourceKind::Website => {
            if has_author {
                e.i(&s.title).stop();
            }
            if !s.publisher.is_empty() {
                e.t(" ").t(&s.publisher).stop();
            }
            if !s.url.is_empty() {
                e.t(" ").t(&s.url);
            }
        }
    }
    trim_end(e);
}

fn trim_end(e: &mut Entry) {
    if let Some(l) = e.0.last_mut() {
        l.text = l.text.trim_end().to_owned();
    }
}

fn entry_mla(s: &Source, e: &mut Entry) {
    let a = authors_mla(s);
    if !a.is_empty() {
        e.t(&a).stop().t(" ");
    }
    match s.kind {
        SourceKind::Book => {
            e.i(&s.title).stop();
            let tail: Vec<&str> = [s.publisher.as_str(), s.year.as_str()]
                .into_iter()
                .filter(|p| !p.is_empty())
                .collect();
            if !tail.is_empty() {
                e.t(&format!(" {}", tail.join(", "))).stop();
            }
        }
        SourceKind::JournalArticle => {
            e.t(&quoted(&s.title)).i(&s.journal);
            let mut tail = Vec::new();
            if !s.volume.is_empty() {
                tail.push(format!("vol. {}", s.volume));
            }
            if !s.issue.is_empty() {
                tail.push(format!("no. {}", s.issue));
            }
            if !s.year.is_empty() {
                tail.push(s.year.clone());
            }
            if !s.pages.is_empty() {
                tail.push(format!("pp. {}", s.pages.replace('-', "\u{2013}")));
            }
            if !tail.is_empty() {
                e.t(&format!(", {}", tail.join(", ")));
            }
            e.stop();
        }
        SourceKind::Website => {
            e.t(&quoted(&s.title)).i(&s.publisher);
            let date = [s.day.as_str(), s.month.as_str(), s.year.as_str()]
                .into_iter()
                .filter(|p| !p.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            if !date.is_empty() {
                e.t(&format!(", {date}"));
            }
            if !s.url.is_empty() {
                e.t(&format!(", {}", s.url));
            }
            e.stop();
        }
    }
    trim_end(e);
}

fn entry_chicago(s: &Source, e: &mut Entry) {
    let a = authors_full(s);
    if !a.is_empty() {
        e.t(&a).stop().t(" ");
    }
    if !s.year.is_empty() {
        e.t(&s.year).stop().t(" ");
    }
    match s.kind {
        SourceKind::Book => {
            e.i(&s.title).stop();
            match (s.city.is_empty(), s.publisher.is_empty()) {
                (false, false) => e.t(&format!(" {}: {}", s.city, s.publisher)).stop(),
                (true, false) => e.t(&format!(" {}", s.publisher)).stop(),
                (false, true) => e.t(&format!(" {}", s.city)).stop(),
                (true, true) => &mut *e,
            };
        }
        SourceKind::JournalArticle => {
            e.t(&quoted(&s.title)).i(&s.journal);
            if !s.volume.is_empty() {
                e.t(&format!(" {}", s.volume));
            }
            if !s.issue.is_empty() {
                e.t(&format!(" ({})", s.issue));
            }
            if !s.pages.is_empty() {
                e.t(&format!(": {}", s.pages.replace('-', "\u{2013}")));
            }
            e.stop();
        }
        SourceKind::Website => {
            e.t(&quoted(&s.title));
            if !s.publisher.is_empty() {
                e.t(&s.publisher).stop().t(" ");
            }
            if !s.month.is_empty() {
                e.t(&format!("{} {}, {}", s.month, s.day, s.year).replace("  ", " "))
                    .stop();
            }
            if !s.url.is_empty() {
                e.t(" ").t(&s.url).stop();
            }
        }
    }
    trim_end(e);
}

/// The in-text citation text, with the optional page(s): `(Smith, 2020, p. 5)`.
#[must_use]
pub fn citation_text(s: &Source, style: BibStyle, pages: &str) -> String {
    let names: Vec<&str> = s.authors.iter().map(|a| a.last.as_str()).collect();
    let who = match names.as_slice() {
        [] => s.title.clone(),
        [a] => (*a).to_owned(),
        [a, b] if style == BibStyle::Apa => format!("{a} & {b}"),
        [a, b] => format!("{a} and {b}"),
        [a, ..] => format!("{a} et al."),
    };
    let year = if s.year.is_empty() {
        "n.d."
    } else {
        s.year.as_str()
    };
    let pages = pages.trim().replace('-', "\u{2013}");
    match style {
        BibStyle::Apa if pages.is_empty() => format!("({who}, {year})"),
        BibStyle::Apa => {
            let label = if pages.contains(['\u{2013}', ',']) {
                "pp."
            } else {
                "p."
            };
            format!("({who}, {year}, {label} {pages})")
        }
        BibStyle::Mla if pages.is_empty() => format!("({who})"),
        BibStyle::Mla => format!("({who} {pages})"),
        BibStyle::Chicago if pages.is_empty() => format!("({who} {year})"),
        BibStyle::Chicago => format!("({who} {year}, {pages})"),
    }
}

/// The sources ordered as a bibliography lists them: by first author's last name (the title when
/// there is no author), then year.
#[must_use]
pub fn sorted_sources(sources: &[Source]) -> Vec<Source> {
    let mut v = sources.to_vec();
    v.sort_by_key(|s| {
        (
            s.authors
                .first()
                .map_or_else(|| s.title.to_lowercase(), |a| a.last.to_lowercase()),
            s.year.clone(),
        )
    });
    v
}

/// The `CITATION` field instruction for `tag` and optional `pages`.
#[must_use]
pub fn citation_instruction(tag: &str, pages: &str) -> String {
    let mut i = format!("CITATION {tag} \\l 1033");
    if !pages.trim().is_empty() {
        i.push_str(&format!(" \\p {}", pages.trim().replace(' ', "")));
    }
    i
}

/// The tag a `CITATION` instruction cites, and its pages.
#[must_use]
pub fn parse_citation(instruction: &str) -> Option<(String, String)> {
    let mut words = instruction.split_whitespace();
    if !words.next()?.eq_ignore_ascii_case("CITATION") {
        return None;
    }
    let tag = words.next()?.to_owned();
    let mut pages = String::new();
    while let Some(w) = words.next() {
        if w.eq_ignore_ascii_case("\\p") {
            pages = words.next().unwrap_or_default().to_owned();
        }
    }
    Some((tag, pages))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book() -> Source {
        let mut s = Source::new(
            "Doe2020",
            SourceKind::Book,
            vec![Author::new("Doe", "Jane Q"), Author::new("Roe", "Richard")],
            "Learning Rust",
            "2020",
        );
        s.publisher = "Acme Press".into();
        s.city = "Boston".into();
        s
    }

    fn article() -> Source {
        let mut s = Source::new(
            "Lee2019",
            SourceKind::JournalArticle,
            vec![Author::new("Lee", "Ann")],
            "On memory safety",
            "2019",
        );
        s.journal = "Journal of Systems".into();
        s.volume = "12".into();
        s.issue = "3".into();
        s.pages = "45-67".into();
        s
    }

    fn site() -> Source {
        let mut s = Source::new(
            "Web1",
            SourceKind::Website,
            vec![Author::new("Kim", "Sam")],
            "A page",
            "2021",
        );
        s.publisher = "Example Site".into();
        s.month = "March".into();
        s.day = "4".into();
        s.url = "https://example.org/a".into();
        s
    }

    fn text(s: &Source, st: BibStyle) -> String {
        plain(&bibliography_entry(s, st))
    }

    #[test]
    fn apa() {
        assert_eq!(
            text(&book(), BibStyle::Apa),
            "Doe, J. Q., & Roe, R. (2020). Learning Rust. Acme Press."
        );
        assert_eq!(
            text(&article(), BibStyle::Apa),
            "Lee, A. (2019). On memory safety. Journal of Systems, 12(3), 45\u{2013}67."
        );
        assert_eq!(
            text(&site(), BibStyle::Apa),
            "Kim, S. (2021, March 4). A page. Example Site. https://example.org/a"
        );
    }

    #[test]
    fn mla() {
        assert_eq!(
            text(&book(), BibStyle::Mla),
            "Doe, Jane Q, and Richard Roe. Learning Rust. Acme Press, 2020."
        );
        assert_eq!(
            text(&article(), BibStyle::Mla),
            "Lee, Ann. \u{201C}On memory safety.\u{201D} Journal of Systems, vol. 12, no. 3, 2019, pp. 45\u{2013}67."
        );
        assert_eq!(
            text(&site(), BibStyle::Mla),
            "Kim, Sam. \u{201C}A page.\u{201D} Example Site, 4 March 2021, https://example.org/a."
        );
    }

    #[test]
    fn chicago() {
        assert_eq!(
            text(&book(), BibStyle::Chicago),
            "Doe, Jane Q, and Richard Roe. 2020. Learning Rust. Boston: Acme Press."
        );
        assert_eq!(
            text(&article(), BibStyle::Chicago),
            "Lee, Ann. 2019. \u{201C}On memory safety.\u{201D} Journal of Systems 12 (3): 45\u{2013}67."
        );
    }

    #[test]
    fn titles_are_italic_in_books() {
        let segs = bibliography_entry(&book(), BibStyle::Apa);
        assert!(segs.iter().any(|s| s.italic && s.text == "Learning Rust."));
    }

    #[test]
    fn in_text() {
        assert_eq!(
            citation_text(&book(), BibStyle::Apa, ""),
            "(Doe & Roe, 2020)"
        );
        assert_eq!(
            citation_text(&book(), BibStyle::Apa, "5"),
            "(Doe & Roe, 2020, p. 5)"
        );
        assert_eq!(
            citation_text(&article(), BibStyle::Apa, "5-7"),
            "(Lee, 2019, pp. 5\u{2013}7)"
        );
        assert_eq!(
            citation_text(&book(), BibStyle::Mla, "5"),
            "(Doe and Roe 5)"
        );
        assert_eq!(citation_text(&article(), BibStyle::Mla, ""), "(Lee)");
        assert_eq!(
            citation_text(&article(), BibStyle::Chicago, "9"),
            "(Lee 2019, 9)"
        );
        let mut three = book();
        three.authors.push(Author::new("Poe", "P"));
        assert_eq!(
            citation_text(&three, BibStyle::Apa, ""),
            "(Doe et al., 2020)"
        );
    }

    #[test]
    fn sorting() {
        let v = sorted_sources(&[site(), book(), article()]);
        let tags: Vec<_> = v.iter().map(|s| s.tag.as_str()).collect();
        assert_eq!(tags, ["Doe2020", "Web1", "Lee2019"]);
    }

    #[test]
    fn instruction_round_trip() {
        let i = citation_instruction("Doe2020", "5 - 7");
        assert_eq!(i, "CITATION Doe2020 \\l 1033 \\p 5-7");
        assert_eq!(parse_citation(&i), Some(("Doe2020".into(), "5-7".into())));
        assert_eq!(parse_citation("TOC"), None);
    }

    #[test]
    fn style_labels() {
        assert_eq!(
            BibStyle::from_label("Chicago Seventeenth Edition"),
            Some(BibStyle::Chicago)
        );
        assert_eq!(BibStyle::from_label("apa"), Some(BibStyle::Apa));
        assert_eq!(BibStyle::from_label("ISO 690"), None);
    }
}
