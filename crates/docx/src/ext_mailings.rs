//! Mail-merge field text: `ADDRESSBLOCK`, `GREETINGLINE`, `MERGEREC` and `MERGESEQ`.
//!
//! The fields read their data through [`MergeData`]. Address and greeting formats name logical
//! parts (`<<_FIRST0_>>`); the data source answers them under the key `_FIRST0_` (the merge
//! layer maps the part to a column, like Match Fields). `MERGEREC` and `MERGESEQ` ask for
//! `_MERGEREC_` and `_MERGESEQ_`.

use crate::fields::{Instruction, MergeData};

/// Word's default address block format.
pub const DEFAULT_ADDRESS_FORMAT: &str = "<<_TITLE0_>> <<_FIRST0_>> <<_LAST0_>> <<_SUFFIX0_>>\n<<_COMPANY_>>\n<<_STREET1_>>\n<<_STREET2_>>\n<<_CITY_>>, <<_STATE_>> <<_POSTAL_>>\n<<_COUNTRY_>>";

/// The default greeting format.
pub const DEFAULT_GREETING_FORMAT: &str = "Dear <<_TITLE0_>> <<_LAST0_>>,";

/// Greeting when the recipient has no name, "Dear Sir or Madam,".
pub const DEFAULT_GREETING_FALLBACK: &str = "Dear Sir or Madam,";

/// Fills `format` from `data`. Returns the text and whether any part had a value. Lines whose
/// parts are all empty are dropped; blanks are collapsed and stray separators trimmed. Word's
/// `<< _BEFORE_ text>>` and `<< _AFTER_ text>>` parts print their text only when the line has data.
fn fill(format: &str, data: &dyn MergeData, trim_commas: bool) -> (String, bool) {
    enum Piece {
        Lit(String),
        Val(String),
        Cond(String),
    }
    let mut any = false;
    let mut lines = Vec::new();
    for line in format.split(['\n', '\r']) {
        let mut pieces = Vec::new();
        let mut line_any = false;
        let mut rest = line;
        let mut has_part = false;
        while let Some(start) = rest.find("<<") {
            pieces.push(Piece::Lit(rest[..start].to_owned()));
            let after = &rest[start + 2..];
            let Some(end) = after.find(">>") else {
                pieces.push(Piece::Lit(rest[start..].to_owned()));
                rest = "";
                break;
            };
            let key = after[..end].trim();
            if let Some(text) = key
                .strip_prefix("_BEFORE_")
                .or_else(|| key.strip_prefix("_AFTER_"))
            {
                pieces.push(Piece::Cond(text.trim_start_matches(' ').to_owned()));
            } else {
                has_part = true;
                match data.value(key).filter(|v| !v.trim().is_empty()) {
                    Some(v) => {
                        line_any = true;
                        pieces.push(Piece::Val(v.trim().to_owned()));
                    }
                    None => pieces.push(Piece::Val(String::new())),
                }
            }
            rest = &after[end + 2..];
        }
        pieces.push(Piece::Lit(rest.to_owned()));
        if has_part && !line_any {
            continue;
        }
        any |= line_any;
        let out: String = pieces
            .into_iter()
            .map(|p| match p {
                Piece::Lit(t) | Piece::Val(t) => t,
                Piece::Cond(t) if line_any => t,
                Piece::Cond(_) => String::new(),
            })
            .collect();
        let squeezed = out.split_whitespace().collect::<Vec<_>>().join(" ");
        let spaced = squeezed.replace(" ,", ",");
        let cleaned = if trim_commas {
            spaced
                .trim_matches(|c: char| c == ',' || c.is_whitespace())
                .to_owned()
        } else {
            spaced
        };
        if !cleaned.is_empty() {
            lines.push(cleaned);
        }
    }
    (lines.join("\r"), any)
}

/// Result of `ADDRESSBLOCK`: paragraphs separated by `\r`; `None` without a data source.
#[must_use]
pub(crate) fn address_block(inst: &Instruction, data: Option<&dyn MergeData>) -> Option<String> {
    let data = data?;
    let format = inst.switch_arg("f").unwrap_or(DEFAULT_ADDRESS_FORMAT);
    let mut text = fill(format, data, true).0;
    // `\c n` leaves the country out unless it is not the given one; `\e` always leaves it out.
    if inst.switch("e").is_some() {
        text = drop_country(text, data);
    }
    Some(text)
}

fn drop_country(text: String, data: &dyn MergeData) -> String {
    let Some(country) = data.value("_COUNTRY_").filter(|c| !c.trim().is_empty()) else {
        return text;
    };
    let mut lines: Vec<&str> = text.split('\r').collect();
    if lines.last().is_some_and(|l| *l == country.trim()) {
        lines.pop();
    }
    lines.join("\r")
}

/// Result of `GREETINGLINE`: the formatted greeting or the `\e` fallback.
#[must_use]
pub(crate) fn greeting_line(inst: &Instruction, data: Option<&dyn MergeData>) -> Option<String> {
    let data = data?;
    let format = inst.switch_arg("f").unwrap_or(DEFAULT_GREETING_FORMAT);
    let (text, any) = fill(format, data, false);
    if any {
        Some(text.replace('\r', " "))
    } else {
        Some(
            inst.switch_arg("e")
                .unwrap_or(DEFAULT_GREETING_FALLBACK)
                .to_owned(),
        )
    }
}

/// Result of `MERGEREC` (record number) or `MERGESEQ` (count of merged records).
#[must_use]
pub(crate) fn merge_counter(key: &str, data: Option<&dyn MergeData>) -> Option<String> {
    data?.value(key)
}

/// Evaluates the four merge fields; without a data source the field shows its placeholder.
pub(crate) fn evaluate(
    data: Option<&dyn MergeData>,
    inst: &Instruction,
) -> crate::fields_eval::Outcome {
    use crate::fields::FieldKind;
    let (text, placeholder) = match inst.kind {
        FieldKind::AddressBlock => (address_block(inst, data), "\u{ab}AddressBlock\u{bb}"),
        FieldKind::GreetingLine => (greeting_line(inst, data), "\u{ab}GreetingLine\u{bb}"),
        FieldKind::MergeRec => (merge_counter("_MERGEREC_", data), "0"),
        _ => (merge_counter("_MERGESEQ_", data), "0"),
    };
    crate::fields_eval::Outcome::Text {
        text: text.unwrap_or_else(|| placeholder.to_owned()),
        merge: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Map(HashMap<&'static str, &'static str>);
    impl MergeData for Map {
        fn value(&self, field: &str) -> Option<String> {
            self.0.get(field).map(|s| (*s).to_owned())
        }
    }

    #[test]
    fn address_drops_empty_lines_and_separators() {
        let data = Map(HashMap::from([
            ("_FIRST0_", "Ada"),
            ("_LAST0_", "Lovelace"),
            ("_STREET1_", "1 Main St"),
            ("_CITY_", "London"),
            ("_POSTAL_", "N1"),
        ]));
        let inst = Instruction::parse("ADDRESSBLOCK");
        assert_eq!(
            address_block(&inst, Some(&data)).unwrap(),
            "Ada Lovelace\r1 Main St\rLondon, N1"
        );
    }

    #[test]
    fn greeting_falls_back_without_a_name() {
        let data = Map(HashMap::new());
        let inst = Instruction::parse("GREETINGLINE");
        assert_eq!(
            greeting_line(&inst, Some(&data)).unwrap(),
            "Dear Sir or Madam,"
        );
    }
}
