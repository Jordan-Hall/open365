//! Insert tab support for the Writer pane: page breaks, tables, pictures, links, bookmarks,
//! comments, headers and footers, page numbers, the date and symbols.

use std::path::Path;

use open365_commands::{Command, HfAlign, HfKind, LinkTarget, WriterInsertOp, to_hex};
use open365_docx::{CivilDateTime, DocPos, LocaleNames, format_date_picture};

use super::WriterPane;

/// Where Page Number puts the number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageNumberAt {
    TopLeft,
    TopCentre,
    TopRight,
    BottomLeft,
    BottomCentre,
    BottomRight,
}

impl PageNumberAt {
    /// `bottom-centre`, `top-right`... (also the American spelling).
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Some(
            match name
                .trim()
                .to_ascii_lowercase()
                .replace("center", "centre")
                .as_str()
            {
                "top-left" => Self::TopLeft,
                "top-centre" => Self::TopCentre,
                "top-right" => Self::TopRight,
                "bottom-left" => Self::BottomLeft,
                "bottom-centre" => Self::BottomCentre,
                "bottom-right" => Self::BottomRight,
                _ => return None,
            },
        )
    }

    fn footer(self) -> bool {
        matches!(
            self,
            Self::BottomLeft | Self::BottomCentre | Self::BottomRight
        )
    }

    fn align(self) -> HfAlign {
        match self {
            Self::TopLeft | Self::BottomLeft => HfAlign::Left,
            Self::TopCentre | Self::BottomCentre => HfAlign::Center,
            Self::TopRight | Self::BottomRight => HfAlign::Right,
        }
    }
}

/// Days since 1970-01-01 to a civil date (proleptic Gregorian).
fn civil_from_days(days: i64) -> (i32, u8, u8) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    ((y + i64::from(m <= 2)) as i32, m, d)
}

/// The current date and time (UTC) as the field engine reads it.
#[must_use]
pub fn now_utc() -> CivilDateTime {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let rem = secs.rem_euclid(86_400);
    CivilDateTime::new(
        y,
        m,
        d,
        (rem / 3600) as u8,
        (rem % 3600 / 60) as u8,
        (rem % 60) as u8,
    )
    .unwrap_or(CivilDateTime {
        year: 1970,
        month: 1,
        day: 1,
        hour: 0,
        minute: 0,
        second: 0,
    })
}

impl WriterPane {
    /// Asks for a picture file (the `rfd` dialog). A cargo test binary (it lives in a `deps`
    /// folder) never shows it: the ribbon test runs every plain control id.
    #[must_use]
    pub fn choose_picture_path() -> Option<std::path::PathBuf> {
        let in_test_binary = std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().and_then(|d| d.file_name()).map(|n| n == "deps"))
            .unwrap_or(false);
        if in_test_binary {
            return None;
        }
        rfd::FileDialog::new()
            .add_filter(
                "Pictures",
                &["png", "jpg", "jpeg", "gif", "bmp", "tif", "tiff"],
            )
            .set_title("Insert Picture")
            .pick_file()
    }

    fn insert_op(&mut self, op: WriterInsertOp) -> bool {
        self.run(Command::DocxInsert { op }).is_some()
    }

    /// Page Break (Ctrl+Enter): the text after the caret starts the next page.
    pub fn insert_page_break(&mut self) {
        if self.models.is_empty() {
            return;
        }
        self.delete_selection();
        let pos = self.caret;
        if self.insert_op(WriterInsertOp::PageBreak { pos }) {
            self.set_caret(DocPos::new(pos.para + 1, 0));
            self.reveal_caret();
        }
    }

    /// Blank Page: an empty page at the caret.
    pub fn insert_blank_page(&mut self) {
        if self.models.is_empty() {
            return;
        }
        self.delete_selection();
        let pos = self.caret;
        if self.insert_op(WriterInsertOp::BlankPage { pos }) {
            self.set_caret(DocPos::new(pos.para + 1, 0));
            self.reveal_caret();
        }
    }

    /// Insert Table: `rows` by `cols` at the caret; the caret goes to the first cell.
    pub fn insert_table(&mut self, rows: usize, cols: usize) {
        if self.models.is_empty() {
            return;
        }
        self.delete_selection();
        let pos = self.caret;
        let Some(text_width_pt) = self.target.docx().map(|d| d.text_width_pt(pos.para)) else {
            return;
        };
        if self.insert_op(WriterInsertOp::Table {
            pos,
            rows,
            cols,
            text_width_pt,
        }) {
            let first_cell = if pos.offset > 0 {
                pos.para + 1
            } else {
                pos.para
            };
            self.set_caret(DocPos::new(first_cell, 0));
            self.reveal_caret();
        }
    }

    /// Pictures: inserts the image file at the caret, scaled down to fit the text width.
    pub fn insert_picture(&mut self, path: &Path) -> bool {
        if self.models.is_empty() {
            return false;
        }
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                self.status = Some(format!("Cannot read {}: {e}", path.display()));
                return false;
            }
        };
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map_or_else(String::new, str::to_ascii_lowercase);
        let size = image::ImageReader::new(std::io::Cursor::new(&bytes))
            .with_guessed_format()
            .ok()
            .and_then(|r| r.into_dimensions().ok());
        let Some((w_px, h_px)) = size.filter(|(w, h)| *w > 0 && *h > 0) else {
            self.status = Some("The file is not a picture".to_owned());
            return false;
        };
        // 96 pixels to the inch.
        let (mut w, mut h) = (f64::from(w_px) * 0.75, f64::from(h_px) * 0.75);
        self.delete_selection();
        let pos = self.caret;
        let max = self
            .target
            .docx()
            .map_or(468.0, |d| d.text_width_pt(pos.para));
        if w > max {
            h *= max / w;
            w = max;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Picture")
            .to_owned();
        self.insert_op(WriterInsertOp::Picture {
            pos,
            data_hex: to_hex(&bytes),
            ext,
            width_pt: w,
            height_pt: h,
            name: name.clone(),
            alt: name,
        })
    }

    /// Link: the selected text becomes a link to `url`; with nothing selected `text` (default
    /// the address) is inserted as the link.
    pub fn insert_link(&mut self, url: &str, text: Option<&str>) -> bool {
        self.link_to(
            LinkTarget::Url {
                url: url.trim().to_owned(),
            },
            text,
            url.trim(),
        )
    }

    /// A link to a bookmark of this document.
    pub fn insert_bookmark_link(&mut self, name: &str) -> bool {
        self.link_to(
            LinkTarget::Bookmark {
                name: name.to_owned(),
            },
            None,
            name,
        )
    }

    fn link_to(&mut self, target: LinkTarget, text: Option<&str>, fallback: &str) -> bool {
        if self.models.is_empty() {
            return false;
        }
        let (start, end) = self.selection();
        let text = (start == end).then(|| text.unwrap_or(fallback).to_owned());
        let len = text.as_ref().map_or(0, |t| t.chars().count());
        let ok = self.insert_op(WriterInsertOp::Hyperlink {
            start,
            end,
            target,
            tip: None,
            text,
        });
        if ok && start == end {
            self.set_caret(DocPos::new(start.para, start.offset + len));
        }
        ok
    }

    /// Bookmark: marks the selection (or the caret) with `name`.
    pub fn insert_bookmark(&mut self, name: &str) -> bool {
        let (start, end) = self.selection();
        self.insert_op(WriterInsertOp::Bookmark {
            start,
            end,
            name: name.to_owned(),
        })
    }

    /// Comment on the selection (or at the caret).
    pub fn insert_comment(&mut self, text: &str) -> bool {
        let author = std::env::var("USERNAME")
            .or_else(|_| std::env::var("USER"))
            .unwrap_or_else(|_| "Author".to_owned());
        let (start, end) = self.selection();
        self.insert_op(WriterInsertOp::Comment {
            start,
            end,
            author,
            text: text.to_owned(),
        })
    }

    /// Header or Footer: `Some(text)` sets it (blank when empty), `None` removes it.
    pub fn insert_header_footer(&mut self, footer: bool, text: Option<&str>) -> bool {
        let para = self.caret.para;
        self.insert_op(WriterInsertOp::HeaderFooter {
            para,
            footer,
            kind: HfKind::Default,
            text: text.unwrap_or_default().to_owned(),
            page_number: None,
            append: false,
            remove: text.is_none(),
        })
    }

    /// Page Number: a PAGE field in the header or footer at the chosen side.
    pub fn insert_page_number(&mut self, at: PageNumberAt) -> bool {
        let para = self.caret.para;
        self.insert_op(WriterInsertOp::HeaderFooter {
            para,
            footer: at.footer(),
            kind: HfKind::Default,
            text: String::new(),
            page_number: Some(at.align()),
            append: true,
            remove: false,
        })
    }

    /// Page Number at a named place (`bottom-centre`...); false for an unknown place.
    pub fn insert_page_number_at(&mut self, place: &str) -> bool {
        match PageNumberAt::parse(place) {
            Some(at) => self.insert_page_number(at),
            None => {
                self.status = Some(format!("{place} is not a page number place"));
                false
            }
        }
    }

    /// Date & Time with the clock (UTC) and a date picture.
    pub fn insert_date_time_now(&mut self, picture: &str) {
        self.insert_date_time(now_utc(), picture);
    }

    /// Date & Time: inserts `when` formatted with the date picture (`dd/MM/yyyy`) as text.
    pub fn insert_date_time(&mut self, when: CivilDateTime, picture: &str) {
        let names = LocaleNames::for_tag("en-GB").unwrap_or_else(LocaleNames::english);
        let text = format_date_picture(when, picture, &names);
        self.insert_text(&text);
    }

    /// Symbol: inserts the character (or `U+XXXX`).
    pub fn insert_symbol(&mut self, symbol: &str) -> bool {
        let s = symbol.trim();
        let ch = match s.strip_prefix("U+").or_else(|| s.strip_prefix("u+")) {
            Some(hex) => u32::from_str_radix(hex, 16).ok().and_then(char::from_u32),
            None => {
                let mut it = s.chars();
                match (it.next(), it.next()) {
                    (Some(c), None) => Some(c),
                    _ => None,
                }
            }
        };
        match ch.filter(|c| !c.is_control()) {
            Some(c) => {
                self.insert_text(&c.to_string());
                true
            }
            None => {
                self.status = Some(format!("{symbol} is not a symbol"));
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_calendar_conversion_is_right() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
    }

    #[test]
    fn page_number_places_parse() {
        assert_eq!(
            PageNumberAt::parse("bottom-center"),
            Some(PageNumberAt::BottomCentre)
        );
        assert_eq!(
            PageNumberAt::parse("top-right"),
            Some(PageNumberAt::TopRight)
        );
        assert_eq!(PageNumberAt::parse("middle"), None);
    }
}
