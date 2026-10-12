//! Page Setup's Header/Footer tab: the preset lists, the `&` codes a section may hold and what
//! they print as.
//!
//! A header or footer is three strings, left, centre and right, each with field codes: `&P` the
//! page number, `&N` the page count, `&D` the date, `&T` the time, `&F` the file name, `&Z` the
//! file path, `&A` the sheet name, `&G` a picture, `&&` an ampersand, and the format codes `&B`,
//! `&I`, `&U`, `&S`, `&E`, `&X`, `&Y`, `&nn` (size), `&K` + `RRGGBB` and `&"Font,Style"`.
//!
//! The worksheet keeps them in its `<headerFooter>` element (see `SheetPage::header_footer`).

/// Left, centre and right section.
pub type Sections = [String; 3];

/// The longest text a header or footer may hold.
pub const MAX_LEN: usize = 255;

/// The four options under the preview boxes.
#[derive(Debug, Clone, PartialEq)]
pub struct HeaderFooter {
    pub header: Sections,
    pub footer: Sections,
    /// The even-page and first-page ones; the dialog does not edit them but keeps them.
    pub even_header: Sections,
    pub even_footer: Sections,
    pub first_header: Sections,
    pub first_footer: Sections,
    pub different_odd_even: bool,
    pub different_first: bool,
    pub scale_with_document: bool,
    pub align_with_margins: bool,
}

impl Default for HeaderFooter {
    fn default() -> Self {
        Self {
            header: Sections::default(),
            footer: Sections::default(),
            even_header: Sections::default(),
            even_footer: Sections::default(),
            first_header: Sections::default(),
            first_footer: Sections::default(),
            different_odd_even: false,
            different_first: false,
            scale_with_document: true,
            align_with_margins: true,
        }
    }
}

/// Joins three sections into the string Excel stores (`&LLeft&CCentre&RRight`, empty sections
/// left out).
#[must_use]
pub fn to_code(sections: &Sections) -> String {
    let mut out = String::new();
    for (mark, text) in ['L', 'C', 'R'].iter().zip(sections) {
        if !text.is_empty() {
            out.push('&');
            out.push(*mark);
            out.push_str(text);
        }
    }
    out
}

/// Splits a stored header or footer string into its sections. Text before the first section
/// mark belongs to the centre, as in Excel.
#[must_use]
pub fn from_code(code: &str) -> Sections {
    let mut out = Sections::default();
    let mut at = 1;
    let mut it = code.chars().peekable();
    while let Some(c) = it.next() {
        if c != '&' {
            out[at].push(c);
            continue;
        }
        match it.peek().copied() {
            Some('L') => {
                it.next();
                at = 0;
            }
            Some('C') => {
                it.next();
                at = 1;
            }
            Some('R') => {
                it.next();
                at = 2;
            }
            Some('"') => {
                it.next();
                out[at].push_str("&\"");
                for q in it.by_ref() {
                    out[at].push(q);
                    if q == '"' {
                        break;
                    }
                }
            }
            Some(n) => {
                it.next();
                out[at].push('&');
                out[at].push(n);
            }
            None => out[at].push('&'),
        }
    }
    out
}

impl HeaderFooter {
    /// The header and footer a worksheet holds.
    #[must_use]
    pub fn from_request(r: &open365_commands::HeaderFooterRequest) -> Self {
        Self {
            header: from_code(&r.odd_header),
            footer: from_code(&r.odd_footer),
            even_header: from_code(&r.even_header),
            even_footer: from_code(&r.even_footer),
            first_header: from_code(&r.first_header),
            first_footer: from_code(&r.first_footer),
            different_odd_even: r.different_odd_even,
            different_first: r.different_first,
            scale_with_document: r.scale_with_doc,
            align_with_margins: r.align_with_margins,
        }
    }

    /// The command that writes this header and footer to the file.
    #[must_use]
    pub fn to_request(&self) -> open365_commands::HeaderFooterRequest {
        open365_commands::HeaderFooterRequest {
            odd_header: to_code(&self.header),
            odd_footer: to_code(&self.footer),
            even_header: to_code(&self.even_header),
            even_footer: to_code(&self.even_footer),
            first_header: to_code(&self.first_header),
            first_footer: to_code(&self.first_footer),
            different_odd_even: self.different_odd_even,
            different_first: self.different_first,
            scale_with_doc: self.scale_with_document,
            align_with_margins: self.align_with_margins,
        }
    }
}

/// What the field codes print.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Context {
    pub sheet: String,
    pub file: String,
    pub path: String,
    pub user: String,
    pub date: String,
    pub time: String,
}

/// `&P` ... `&Z` replaced; format codes dropped; `&&` is `&`. `pages` is shown for `&N`.
#[must_use]
pub fn render(text: &str, ctx: &Context, page: &str, pages: &str) -> String {
    let mut out = String::new();
    let mut it = text.chars().peekable();
    while let Some(c) = it.next() {
        if c != '&' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('&') => out.push('&'),
            Some('P') => out.push_str(page),
            Some('N') => out.push_str(pages),
            Some('D') => out.push_str(&ctx.date),
            Some('T') => out.push_str(&ctx.time),
            Some('F') => out.push_str(&ctx.file),
            Some('Z') => out.push_str(&ctx.path),
            Some('A') => out.push_str(&ctx.sheet),
            Some('K') => {
                for _ in 0..6 {
                    if it.peek().is_some_and(char::is_ascii_hexdigit) {
                        it.next();
                    }
                }
            }
            Some('"') => {
                for q in it.by_ref() {
                    if q == '"' {
                        break;
                    }
                }
            }
            Some(d) if d.is_ascii_digit() => {
                while it.peek().is_some_and(char::is_ascii_digit) {
                    it.next();
                }
            }
            // `&G` (picture) and the switches `&B &I &U &S &E &X &Y` print nothing.
            Some(_) | None => {}
        }
    }
    out
}

/// The text of the three sections as one line, as the preset list shows a choice.
#[must_use]
pub fn one_line(sections: &Sections, ctx: &Context, pages: &str) -> String {
    sections
        .iter()
        .map(|s| render(s, ctx, "1", pages))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

/// The preset lists of Header and Footer: the sections each stands for.
#[must_use]
pub fn presets(ctx: &Context) -> Vec<Sections> {
    let s = |l: &str, c: &str, r: &str| [l.to_owned(), c.to_owned(), r.to_owned()];
    let _ = ctx;
    vec![
        s("", "", ""),
        s("", "&P", ""),
        s("", "Page &P of &N", ""),
        s("", "&A", ""),
        s("Confidential", "&D", "Page &P"),
        s("", "&F", ""),
        s("", "&F", "Page &P"),
        s("", "&A", "Page &P"),
        s("", "Prepared by &[User]", ""),
        s("Page &P", "&A", "&D"),
    ]
    .into_iter()
    .map(|[l, c, r]| {
        let fix = |t: String| t.replace("&[User]", &ctx.user.replace('&', "&&"));
        [fix(l), fix(c), fix(r)]
    })
    .collect()
}

/// The list entries of a Header or Footer drop-down, `(none)` first. When `current` is not one
/// of the presets it is added first (as the box shows custom text) and selected.
#[must_use]
pub fn choices(current: &Sections, ctx: &Context) -> (Vec<Sections>, Vec<String>, usize) {
    let mut sets = presets(ctx);
    let mut selected = sets.iter().position(|p| p == current);
    if selected.is_none() {
        sets.insert(0, current.clone());
        selected = Some(0);
    }
    let labels = sets
        .iter()
        .map(|p| {
            if p.iter().all(String::is_empty) {
                "(none)".to_owned()
            } else {
                one_line(p, ctx, "?")
            }
        })
        .collect();
    (sets, labels, selected.unwrap_or(0))
}

/// The date as `m/d/yyyy` for `unix` seconds (UTC), what `&D` prints for the preview.
#[must_use]
pub fn date_text(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{m}/{d}/{y}")
}

/// The time as `h:mm AM/PM` for `unix` seconds (UTC).
#[must_use]
pub fn time_text(unix: i64) -> String {
    let secs = unix.rem_euclid(86_400);
    let (h, m) = (secs / 3600, (secs % 3600) / 60);
    let (h12, ampm) = match h {
        0 => (12, "AM"),
        1..=11 => (h, "AM"),
        12 => (12, "PM"),
        _ => (h - 12, "PM"),
    };
    format!("{h12}:{m:02} {ampm}")
}

/// Checks a section the way Excel does: the whole header may not be longer than 255 characters.
///
/// # Errors
/// Excel's message for a text that is too long.
pub fn check_length(sections: &Sections) -> Result<(), String> {
    let n: usize = sections.iter().map(|s| s.chars().count()).sum();
    if n > MAX_LEN {
        Err("The header or footer is too long. Headers and footers can hold up to 255 characters, including the codes.".to_owned())
    } else {
        Ok(())
    }
}

/// Inserts `code` into `text` at byte offset `at`, replacing the selection `(from, to)`.
#[must_use]
pub fn insert_code(text: &str, selection: (usize, usize), code: &str) -> (String, usize) {
    let (a, b) = (selection.0.min(text.len()), selection.1.min(text.len()));
    let (a, b) = (a.min(b), a.max(b));
    let mut out = String::with_capacity(text.len() + code.len());
    out.push_str(&text[..a]);
    out.push_str(code);
    out.push_str(&text[b..]);
    (out, a + code.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Context {
        Context {
            sheet: "Sheet1".into(),
            file: "Book1.xlsx".into(),
            path: "C:\\Book1.xlsx".into(),
            user: "Ann".into(),
            date: "10/12/2026".into(),
            time: "9:05 AM".into(),
        }
    }

    #[test]
    fn codes_print_their_fields_and_format_codes_vanish() {
        let c = ctx();
        assert_eq!(render("Page &P of &N", &c, "3", "9"), "Page 3 of 9");
        assert_eq!(render("&A - &F", &c, "1", "1"), "Sheet1 - Book1.xlsx");
        assert_eq!(render("R&&D", &c, "1", "1"), "R&D");
        assert_eq!(
            render("&B&14&\"Arial,Bold\"&KFF0000Hi&G", &c, "1", "1"),
            "Hi"
        );
        assert_eq!(render("&D &T", &c, "1", "1"), "10/12/2026 9:05 AM");
    }

    #[test]
    fn presets_start_with_none_and_a_custom_text_is_added_and_selected() {
        let c = ctx();
        let (sets, labels, sel) = choices(&Sections::default(), &c);
        assert_eq!(labels[0], "(none)");
        assert_eq!(sel, 0);
        assert_eq!(sets.len(), presets(&c).len());
        assert!(labels.contains(&"Page 1 of ?".to_owned()));
        let custom: Sections = ["x".into(), String::new(), "&P".into()];
        let (sets, labels, sel) = choices(&custom, &c);
        assert_eq!(sel, 0);
        assert_eq!(sets[0], custom);
        assert_eq!(labels[0], "x, 1");
        assert_eq!(sets.len(), presets(&c).len() + 1);
    }

    #[test]
    fn dates_and_times_are_written_like_the_us_boxes() {
        assert_eq!(date_text(0), "1/1/1970");
        assert_eq!(date_text(1_791_763_200), "10/12/2026");
        assert_eq!(time_text(0), "12:00 AM");
        assert_eq!(time_text(13 * 3600 + 5 * 60), "1:05 PM");
    }

    #[test]
    fn codes_are_inserted_at_the_caret_over_the_selection() {
        assert_eq!(insert_code("ab", (1, 1), "&P"), ("a&Pb".to_owned(), 3));
        assert_eq!(insert_code("abc", (0, 2), "&D"), ("&Dc".to_owned(), 2));
    }

    #[test]
    fn long_texts_are_refused() {
        let long: Sections = ["a".repeat(200), "b".repeat(100), String::new()];
        assert!(check_length(&long).is_err());
        assert!(check_length(&Sections::default()).is_ok());
    }
}
