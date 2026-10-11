//! `styles.xml`: kept as a raw tree; only the number-format tables are interpreted.

use std::collections::HashMap;

use crate::XlsxError;
use crate::conditional::Dxf;
use crate::format::{CellFormat, Color, FormatTables, Rgb, Theme};
use crate::xml::{self, XmlElement};
use open365_locale::builtin_number_format_in;

/// Built-in number format codes for ids 0 to 49 (ECMA-376 Part 1, 18.8.30). Ids the
/// specification leaves to the locale (23 to 36) return `None`.
pub fn builtin_number_format(id: u32) -> Option<&'static str> {
    Some(match id {
        0 => "General",
        1 => "0",
        2 => "0.00",
        3 => "#,##0",
        4 => "#,##0.00",
        5 => "\"$\"#,##0_);\\(\"$\"#,##0\\)",
        6 => "\"$\"#,##0_);[Red]\\(\"$\"#,##0\\)",
        7 => "\"$\"#,##0.00_);\\(\"$\"#,##0.00\\)",
        8 => "\"$\"#,##0.00_);[Red]\\(\"$\"#,##0.00\\)",
        9 => "0%",
        10 => "0.00%",
        11 => "0.00E+00",
        12 => "# ?/?",
        13 => "# ??/??",
        14 => "mm-dd-yy",
        15 => "d-mmm-yy",
        16 => "d-mmm",
        17 => "mmm-yy",
        18 => "h:mm AM/PM",
        19 => "h:mm:ss AM/PM",
        20 => "h:mm",
        21 => "h:mm:ss",
        22 => "m/d/yy h:mm",
        37 => "#,##0_);(#,##0)",
        38 => "#,##0_);[Red](#,##0)",
        39 => "#,##0.00_);(#,##0.00)",
        40 => "#,##0.00_);[Red](#,##0.00)",
        41 => "_(* #,##0_);_(* \\(#,##0\\);_(* \"-\"_);_(@_)",
        42 => "_(\"$\"* #,##0_);_(\"$\"* \\(#,##0\\);_(\"$\"* \"-\"_);_(@_)",
        43 => "_(* #,##0.00_);_(* \\(#,##0.00\\);_(* \"-\"??_);_(@_)",
        44 => "_(\"$\"* #,##0.00_);_(\"$\"* \\(#,##0.00\\);_(\"$\"* \"-\"??_);_(@_)",
        45 => "mm:ss",
        46 => "[h]:mm:ss",
        47 => "mmss.0",
        48 => "##0.0E+0",
        49 => "@",
        _ => return None,
    })
}

/// The number-format lookup, the per-xf `numFmtId` list and the resolved format tables of a
/// `styleSheet` tree.
fn derive_tables(root: &XmlElement) -> (HashMap<u32, String>, Vec<u32>, FormatTables) {
    let mut num_fmts = HashMap::new();
    if let Some(fmts) = root.child_main("numFmts") {
        for fmt in fmts.elements().filter(|e| e.head.is_main("numFmt")) {
            if let (Some(id), Some(code)) = (
                fmt.head
                    .attr("numFmtId")
                    .and_then(|v| v.trim().parse().ok()),
                fmt.head.attr("formatCode"),
            ) {
                num_fmts.entry(id).or_insert_with(|| code.to_owned());
            }
        }
    }
    let xf_num_fmt = root
        .child_main("cellXfs")
        .map(|xfs| {
            xfs.elements()
                .filter(|e| e.head.is_main("xf"))
                .map(|xf| {
                    xf.head
                        .attr("numFmtId")
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(0)
                })
                .collect()
        })
        .unwrap_or_default();
    let tables = FormatTables::parse(root);
    (num_fmts, xf_num_fmt, tables)
}

/// The style sheet: the raw tree plus the number-format lookup tables.
#[derive(Debug, Clone, Default)]
pub struct Styles {
    pub(crate) parsed: Option<(String, XmlElement, String)>,
    pub(crate) num_fmts: HashMap<u32, String>,
    /// `numFmtId` of each `cellXfs/xf`, by style index.
    pub(crate) xf_num_fmt: Vec<u32>,
    pub(crate) tables: FormatTables,
    pub(crate) theme: Theme,
    /// `dxfs/dxf` entries, by `dxfId`.
    pub(crate) dxfs: Vec<Dxf>,
    /// The Windows locale that built-in ids resolve under; `None` keeps the specification's codes.
    pub(crate) num_fmt_locale: Option<u32>,
    /// Built-in ids 0 to 81 resolved under `num_fmt_locale`.
    pub(crate) builtin_resolved: HashMap<u32, String>,
}

impl Styles {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, XlsxError> {
        let parsed = xml::parse(bytes)?;
        if !parsed.root.head.is_main("styleSheet") {
            return Err(XlsxError::NotSpreadsheet(
                "styles part is not a styleSheet".to_owned(),
            ));
        }
        let (num_fmts, xf_num_fmt, tables) = derive_tables(&parsed.root);
        let lookup = |id: u32| {
            num_fmts
                .get(&id)
                .cloned()
                .or_else(|| builtin_number_format(id).map(str::to_owned))
        };
        let dxfs = parsed
            .root
            .child_main("dxfs")
            .map(|d| {
                d.elements()
                    .filter(|e| e.head.is_main("dxf"))
                    .map(|e| Dxf::parse(e, &lookup))
                    .collect()
            })
            .unwrap_or_default();
        Ok(Self {
            parsed: Some((parsed.prolog, parsed.root, parsed.epilogue)),
            num_fmts,
            xf_num_fmt,
            tables,
            theme: Theme::default(),
            dxfs,
            num_fmt_locale: None,
            builtin_resolved: HashMap::new(),
        })
    }

    /// Recomputes the lookup tables after the tree was edited.
    pub(crate) fn reindex(&mut self) {
        if let Some((_, root, _)) = &self.parsed {
            let (num_fmts, xf_num_fmt, tables) = derive_tables(root);
            self.num_fmts = num_fmts;
            self.xf_num_fmt = xf_num_fmt;
            self.tables = tables;
        }
    }

    /// Number of `cellXfs` entries.
    pub fn cell_xf_count(&self) -> usize {
        self.xf_num_fmt.len()
    }

    /// `numFmtId` of cell style `xf` (0 when the index is unknown).
    pub fn num_fmt_id(&self, xf: u32) -> u32 {
        usize::try_from(xf)
            .ok()
            .and_then(|i| self.xf_num_fmt.get(i))
            .copied()
            .unwrap_or(0)
    }

    /// Makes the built-in number format ids resolve as Excel does under the Windows locale `lcid`
    /// (see [`crate::builtin_number_format_in`]); `None` returns to the specification's codes
    /// ([`builtin_number_format`]). The file does not record the locale, so the caller supplies the
    /// one of the machine or user that opens it.
    pub fn set_number_format_locale(&mut self, lcid: Option<u32>) {
        self.num_fmt_locale = lcid;
        self.builtin_resolved = match lcid {
            Some(lcid) => (0..=81)
                .filter_map(|id| builtin_number_format_in(id, lcid).map(|code| (id, code)))
                .collect(),
            None => HashMap::new(),
        };
    }

    /// The Windows locale built-in ids resolve under, when one was set.
    pub fn number_format_locale(&self) -> Option<u32> {
        self.num_fmt_locale
    }

    /// Format code for a number format id: custom `numFmts` first, then the built-in
    /// table (locale-resolved when a locale was set); `General` when unknown.
    pub fn format_code(&self, num_fmt_id: u32) -> &str {
        if let Some(code) = self.num_fmts.get(&num_fmt_id) {
            return code;
        }
        if let Some(code) = self.builtin_resolved.get(&num_fmt_id) {
            return code;
        }
        builtin_number_format(num_fmt_id).unwrap_or("General")
    }

    /// Format code of cell style `xf`.
    pub fn number_format(&self, xf: u32) -> &str {
        self.format_code(self.num_fmt_id(xf))
    }

    /// The default font (`fonts/font[0]`, the Normal style's font) as `(name, size in points)`.
    pub fn default_font(&self) -> Option<(String, f64)> {
        let (_, root, _) = self.parsed.as_ref()?;
        let font = root
            .child_main("fonts")?
            .elements()
            .find(|e| e.head.is_main("font"))?;
        let attr = |local: &str| {
            font.elements()
                .find(|e| e.head.is_main(local))
                .and_then(|e| e.head.attr("val"))
        };
        let size = attr("sz")?.trim().parse::<f64>().ok()?;
        Some((attr("name")?.to_owned(), size))
    }

    pub(crate) fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
    }

    /// The workbook theme (the Office default when the package has none).
    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    /// Font, fill, border and alignment of cell format `xf`, resolved through
    /// `cellStyleXfs`. An unknown index gives the default format (the first font, no fill,
    /// no borders, general alignment).
    pub fn cell_format(&self, xf: u32) -> &CellFormat {
        self.tables.format(xf)
    }

    /// A colour with theme, palette and tint applied. Automatic colours are black.
    pub fn resolve_color(&self, color: &Color) -> Rgb {
        crate::format::resolve(color, &self.theme, &self.tables.palette)
    }

    /// The differential format `dxfId` (conditional formatting), when the sheet has it.
    pub fn dxf(&self, id: u32) -> Option<&Dxf> {
        self.dxfs.get(usize::try_from(id).ok()?)
    }

    /// Custom number formats declared in the style sheet, by id.
    pub fn custom_formats(&self) -> &HashMap<u32, String> {
        &self.num_fmts
    }

    /// The part re-serialised from the tree; `None` when the workbook has no styles part.
    pub(crate) fn serialize(&self) -> Option<Vec<u8>> {
        let (prolog, root, epilogue) = self.parsed.as_ref()?;
        let mut out = String::new();
        out.push_str(prolog);
        root.write(&mut out);
        out.push_str(epilogue);
        Some(out.into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_table_has_the_documented_entries() {
        assert_eq!(builtin_number_format(0), Some("General"));
        assert_eq!(builtin_number_format(14), Some("mm-dd-yy"));
        assert_eq!(builtin_number_format(49), Some("@"));
        assert_eq!(builtin_number_format(30), None);
        assert_eq!(builtin_number_format(50), None);
    }

    #[test]
    fn custom_formats_override_and_xf_lookup_works() {
        let styles = Styles::parse(
            br#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><numFmts count="1"><numFmt numFmtId="164" formatCode="0.0%"/></numFmts><cellXfs count="3"><xf numFmtId="0"/><xf numFmtId="164"/><xf numFmtId="14"/></cellXfs></styleSheet>"#,
        )
        .unwrap();
        assert_eq!(styles.number_format(0), "General");
        assert_eq!(styles.number_format(1), "0.0%");
        assert_eq!(styles.number_format(2), "mm-dd-yy");
        assert_eq!(styles.number_format(99), "General");
    }

    #[test]
    fn built_in_ids_follow_the_set_locale() {
        let mut styles = Styles::parse(
            br#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><numFmts count="1"><numFmt numFmtId="164" formatCode="0.0%"/></numFmts><cellXfs count="4"><xf numFmtId="0"/><xf numFmtId="164"/><xf numFmtId="14"/><xf numFmtId="5"/></cellXfs></styleSheet>"#,
        )
        .unwrap();
        assert_eq!(styles.number_format_locale(), None);
        styles.set_number_format_locale(Some(0x0809));
        assert_eq!(styles.number_format_locale(), Some(0x0809));
        assert_eq!(styles.number_format(2), "dd/mm/yyyy");
        assert_eq!(styles.number_format(3), "\u{a3}#,##0;-\u{a3}#,##0");
        assert_eq!(styles.number_format(1), "0.0%");
        styles.set_number_format_locale(Some(0x0407));
        assert_eq!(styles.number_format(2), "dd.mm.yyyy");
        styles.set_number_format_locale(None);
        assert_eq!(styles.number_format(2), "mm-dd-yy");
    }
}
