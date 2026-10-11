//! Table model and structured references (`Table[Column]`, `Table[[#This Row],[Column]]`, `Table[#Totals]`).
//!
//! A structured reference is kept as written (table name plus the bracket text) in the syntax tree and
//! resolved to a rectangle when it is evaluated, against the tables the engine was given by
//! [`crate::Engine::define_table`].

use crate::value::ErrorKind;

/// A table as the workbook defines it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableDef {
    /// The table's name (`displayName`), unique in the workbook, compared without regard to case.
    pub name: String,
    /// The whole table `(row1, col1, row2, col2)`, 1-based, headers and totals included.
    pub rect: (u32, u32, u32, u32),
    /// Header rows at the top of `rect` (0 or 1).
    pub header_rows: u32,
    /// Totals rows at the bottom of `rect` (0 or 1).
    pub totals_rows: u32,
    /// Column names, left to right.
    pub columns: Vec<String>,
}

/// A table with the name of the sheet it sits on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacedTable {
    /// Sheet name.
    pub sheet: String,
    /// The table.
    pub def: TableDef,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Special {
    All,
    Data,
    Headers,
    Totals,
    ThisRow,
}

#[derive(Debug, Default)]
struct Spec {
    specials: Vec<Special>,
    /// Column selections: one name, or the two ends of `[A]:[B]`.
    columns: Vec<(String, Option<String>)>,
}

/// Content of one `[...]` group starting at `chars[start]` (which is `[`): the text inside, with `'` escapes
/// resolved, and the index after the closing `]`.
fn group(chars: &[char], start: usize) -> Option<(String, usize)> {
    let mut depth = 0usize;
    let mut out = String::new();
    let mut i = start;
    while i < chars.len() {
        match chars[i] {
            '\'' if i + 1 < chars.len() => {
                if depth > 0 {
                    out.push(chars[i + 1]);
                }
                i += 2;
                continue;
            }
            '[' => {
                depth += 1;
                if depth > 1 {
                    out.push('[');
                }
            }
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some((out, i + 1));
                }
                out.push(']');
            }
            c => {
                if depth > 0 {
                    out.push(c);
                }
            }
        }
        i += 1;
    }
    None
}

fn special_of(name: &str) -> Option<Special> {
    match name.trim().to_ascii_lowercase().as_str() {
        "#all" => Some(Special::All),
        "#data" => Some(Special::Data),
        "#headers" => Some(Special::Headers),
        "#totals" => Some(Special::Totals),
        "#this row" => Some(Special::ThisRow),
        _ => None,
    }
}

/// One item: a special item name or a column name (`@Name` is `#This Row` plus the column).
fn add_item(spec: &mut Spec, item: &str, upper: Option<&str>) -> Option<()> {
    if let Some(rest) = item.strip_prefix('@') {
        spec.specials.push(Special::ThisRow);
        let rest = rest.trim();
        if !rest.is_empty() {
            spec.columns
                .push((rest.to_owned(), upper.map(str::to_owned)));
        }
        return Some(());
    }
    if item.starts_with('#') {
        spec.specials.push(special_of(item)?);
        return Some(());
    }
    spec.columns
        .push((item.to_owned(), upper.map(str::to_owned)));
    Some(())
}

fn parse_spec(bracket: &str) -> Option<Spec> {
    let chars: Vec<char> = bracket.chars().collect();
    let (inner, end) = group(&chars, 0)?;
    if end != chars.len() {
        return None;
    }
    let mut spec = Spec::default();
    let inner_chars: Vec<char> = inner.chars().collect();
    if inner_chars.first() != Some(&'[') {
        // `[Name]`, `[#Data]`, `[@Name]`, `[@[Name]]`
        let text = inner.as_str();
        if text.trim().is_empty() {
            // `Table[]`: the whole data body.
            return Some(spec);
        }
        if let Some(rest) = text.strip_prefix('@')
            && rest.trim_start().starts_with('[')
        {
            let rest: Vec<char> = rest.trim_start().chars().collect();
            let (name, _) = group(&rest, 0)?;
            spec.specials.push(Special::ThisRow);
            spec.columns.push((name, None));
            return Some(spec);
        }
        add_item(&mut spec, text.trim_start(), None)?;
        return Some(spec);
    }
    let mut i = 0;
    while i < inner_chars.len() {
        match inner_chars[i] {
            c if c.is_whitespace() || c == ',' => i += 1,
            '[' => {
                let (first, next) = group(&inner_chars, i)?;
                i = next;
                let mut j = i;
                while inner_chars.get(j).is_some_and(|c| c.is_whitespace()) {
                    j += 1;
                }
                if inner_chars.get(j) == Some(&':') {
                    j += 1;
                    while inner_chars.get(j).is_some_and(|c| c.is_whitespace()) {
                        j += 1;
                    }
                    let (second, next) = group(&inner_chars, j)?;
                    i = next;
                    spec.columns.push((first, Some(second)));
                } else {
                    add_item(&mut spec, &first, None)?;
                }
            }
            _ => return None,
        }
    }
    Some(spec)
}

/// The rectangle `bracket` selects in `def`; `row` is the formula's row for `#This Row`.
///
/// # Errors
/// `#REF!` for a column, header or totals row the table lacks, or text that is not a structured reference;
/// `#VALUE!` for `#This Row` outside the table's data rows.
pub fn resolve(
    def: &TableDef,
    bracket: &str,
    row: Option<u32>,
) -> Result<(u32, u32, u32, u32), ErrorKind> {
    let spec = parse_spec(bracket).ok_or(ErrorKind::Ref)?;
    let (top, left, bottom, right) = def.rect;
    let data_top = top + def.header_rows;
    let data_bottom = bottom.saturating_sub(def.totals_rows);
    let mut lo = u32::MAX;
    let mut hi = 0u32;
    let mut take = |a: u32, b: u32| {
        lo = lo.min(a);
        hi = hi.max(b);
    };
    if spec.specials.is_empty() {
        take(data_top, data_bottom);
    }
    for special in &spec.specials {
        match special {
            Special::All => take(top, bottom),
            Special::Data => take(data_top, data_bottom),
            Special::Headers => {
                if def.header_rows == 0 {
                    return Err(ErrorKind::Ref);
                }
                take(top, data_top - 1);
            }
            Special::Totals => {
                if def.totals_rows == 0 {
                    return Err(ErrorKind::Ref);
                }
                take(data_bottom + 1, bottom);
            }
            Special::ThisRow => {
                let r = row.ok_or(ErrorKind::Value)?;
                if r < data_top || r > data_bottom {
                    return Err(ErrorKind::Value);
                }
                take(r, r);
            }
        }
    }
    if lo > hi {
        return Err(ErrorKind::Ref);
    }
    let width = u32::try_from(def.columns.len()).unwrap_or(0);
    let (mut c_lo, mut c_hi) = (u32::MAX, 0u32);
    let find = |name: &str| -> Result<u32, ErrorKind> {
        // Names compare as written first (a column may end in a space), then trimmed.
        def.columns
            .iter()
            .position(|c| c.eq_ignore_ascii_case(name))
            .or_else(|| {
                def.columns
                    .iter()
                    .position(|c| c.trim().eq_ignore_ascii_case(name.trim()))
            })
            .and_then(|i| u32::try_from(i).ok())
            .ok_or(ErrorKind::Ref)
    };
    for (a, b) in &spec.columns {
        let first = find(a)?;
        let last = match b {
            Some(b) => find(b)?,
            None => first,
        };
        c_lo = c_lo.min(first.min(last));
        c_hi = c_hi.max(first.max(last));
    }
    if spec.columns.is_empty() {
        c_lo = 0;
        c_hi = width.max(1) - 1;
    }
    Ok((
        lo,
        left + c_lo,
        hi,
        left + c_hi.min(right.saturating_sub(left)),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sales() -> TableDef {
        // A2:C7: header row 2, data rows 3 to 6, totals row 7.
        TableDef {
            name: "SalesTbl".into(),
            rect: (2, 1, 7, 3),
            header_rows: 1,
            totals_rows: 1,
            columns: vec!["Item".into(), "Qty".into(), "Unit price".into()],
        }
    }

    #[test]
    fn columns_and_specials() {
        let t = sales();
        assert_eq!(resolve(&t, "[Qty]", None), Ok((3, 2, 6, 2)));
        assert_eq!(resolve(&t, "[[Qty]]", None), Ok((3, 2, 6, 2)));
        assert_eq!(resolve(&t, "[Unit price]", None), Ok((3, 3, 6, 3)));
        assert_eq!(resolve(&t, "[#All]", None), Ok((2, 1, 7, 3)));
        assert_eq!(resolve(&t, "[#Headers]", None), Ok((2, 1, 2, 3)));
        assert_eq!(resolve(&t, "[#Data]", None), Ok((3, 1, 6, 3)));
        assert_eq!(resolve(&t, "[[#Totals],[Qty]]", None), Ok((7, 2, 7, 2)));
        assert_eq!(
            resolve(&t, "[[#Headers],[#Data],[Qty]]", None),
            Ok((2, 2, 6, 2))
        );
        assert_eq!(resolve(&t, "[[Item]:[Qty]]", None), Ok((3, 1, 6, 2)));
    }

    #[test]
    fn this_row() {
        let t = sales();
        assert_eq!(
            resolve(&t, "[[#This Row],[Qty]]", Some(4)),
            Ok((4, 2, 4, 2))
        );
        assert_eq!(resolve(&t, "[@Qty]", Some(5)), Ok((5, 2, 5, 2)));
        assert_eq!(resolve(&t, "[@[Unit price]]", Some(5)), Ok((5, 3, 5, 3)));
        assert_eq!(
            resolve(&t, "[[#This Row],[Qty]]", Some(2)),
            Err(ErrorKind::Value)
        );
        assert_eq!(
            resolve(&t, "[[#This Row],[Qty]]", Some(7)),
            Err(ErrorKind::Value)
        );
    }

    #[test]
    fn missing_pieces_are_ref_errors() {
        let mut t = sales();
        assert_eq!(resolve(&t, "[Nope]", None), Err(ErrorKind::Ref));
        t.totals_rows = 0;
        t.rect.2 = 6;
        assert_eq!(resolve(&t, "[#Totals]", None), Err(ErrorKind::Ref));
        assert_eq!(resolve(&t, "[Qty]", None), Ok((3, 2, 6, 2)));
    }

    #[test]
    fn escaped_names() {
        let mut t = sales();
        t.columns[1] = "Qty [kg]".into();
        assert_eq!(resolve(&t, "[Qty '[kg']]", None), Ok((3, 2, 6, 2)));
    }
}
