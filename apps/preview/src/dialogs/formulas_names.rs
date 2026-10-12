//! Name Manager (read side): the defined names of a workbook as the dialog lists them.

use open365_xlsx::Workbook;

/// One row of the Name Manager list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameRow {
    pub name: String,
    /// What the name refers to, with a leading `=`.
    pub refers_to: String,
    /// `Workbook` or the sheet name.
    pub scope: String,
    pub comment: String,
}

/// Every visible defined name, sorted by name as the dialog sorts them. Hidden names (print
/// areas and titles, filter databases) are listed by Excel too, except the hidden ones.
#[must_use]
pub fn rows(wb: &Workbook) -> Vec<NameRow> {
    let mut out: Vec<NameRow> = wb
        .defined_names()
        .iter()
        .filter(|n| !n.hidden)
        .map(|n| NameRow {
            name: n.name.clone(),
            refers_to: format!("={}", n.formula),
            scope: n
                .local_sheet_id
                .and_then(|i| wb.sheets().get(i as usize))
                .map_or_else(|| "Workbook".to_owned(), |s| s.name.clone()),
            comment: n.comment.clone().unwrap_or_default(),
        })
        .collect();
    out.sort_by_key(|r| r.name.to_lowercase());
    out
}
