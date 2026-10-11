//! Number-format code helpers for the formatting buttons: how Excel normalises a typed custom
//! code, and what Increase and Decrease Decimal do to a code.

/// Excel's stored form of a typed custom format: literal spaces, hyphens and parentheses (and
/// the dot between date parts) get a backslash. Quoted text, `\x`, `_x`, `*x` pairs and
/// `[...]` groups are copied as they are.
pub(crate) fn normalise_code(code: &str) -> String {
    let chars: Vec<char> = code.chars().collect();
    let is_date = {
        let mut in_quote = false;
        let mut found = false;
        let mut i = 0;
        while i < chars.len() {
            match chars[i] {
                '"' => in_quote = !in_quote,
                '\\' | '_' | '*' => i += 1,
                '[' if !in_quote => {
                    while i < chars.len() && chars[i] != ']' {
                        i += 1;
                    }
                }
                'd' | 'm' | 'y' | 'h' | 's' | 'D' | 'M' | 'Y' | 'H' | 'S' if !in_quote => {
                    found = true;
                }
                _ => {}
            }
            i += 1;
        }
        found
    };
    let mut out = String::with_capacity(code.len() + 4);
    let mut in_quote = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if in_quote {
            out.push(c);
            in_quote = c != '"';
            i += 1;
            continue;
        }
        match c {
            '"' => {
                in_quote = true;
                out.push(c);
            }
            '\\' | '_' | '*' => {
                out.push(c);
                if let Some(n) = chars.get(i + 1) {
                    out.push(*n);
                    i += 1;
                }
            }
            '[' => {
                while i < chars.len() {
                    out.push(chars[i]);
                    if chars[i] == ']' {
                        break;
                    }
                    i += 1;
                }
            }
            ' ' | '-' | '(' | ')' => {
                out.push('\\');
                out.push(c);
            }
            '.' if is_date && !matches!(i.checked_sub(1).map(|p| chars[p]), Some('s' | 'S')) => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
        i += 1;
    }
    out
}

fn is_placeholder(c: char) -> bool {
    matches!(c, '0' | '#' | '?')
}

/// One section of a format code with one more (`delta` > 0) or one fewer decimal place. `None`
/// when the section shows no number.
fn adjust_section(section: &str, delta: i32) -> Option<String> {
    let chars: Vec<char> = section.chars().collect();
    // Positions of the unquoted, unescaped characters that matter.
    let mut point: Option<usize> = None;
    let mut last_placeholder: Option<usize> = None;
    let mut i = 0;
    let mut in_quote = false;
    while i < chars.len() {
        let c = chars[i];
        if in_quote {
            in_quote = c != '"';
        } else {
            match c {
                '"' => in_quote = true,
                '\\' | '_' | '*' => i += 1,
                '[' => {
                    while i < chars.len() && chars[i] != ']' {
                        i += 1;
                    }
                }
                'E' | 'e' if matches!(chars.get(i + 1), Some('+' | '-')) => break,
                '.' if point.is_none() => point = Some(i),
                c if is_placeholder(c) => last_placeholder = Some(i),
                _ => {}
            }
        }
        i += 1;
    }
    let last = last_placeholder?;
    let mut out: Vec<char> = chars.clone();
    match (point, delta > 0) {
        (Some(p), true) => {
            // End of the run of placeholders after the point.
            let mut end = p;
            while end + 1 < chars.len() && is_placeholder(chars[end + 1]) {
                end += 1;
            }
            out.insert(end + 1, '0');
        }
        (None, true) => {
            out.insert(last + 1, '0');
            out.insert(last + 1, '.');
        }
        (Some(p), false) => {
            let mut end = p;
            while end + 1 < chars.len() && is_placeholder(chars[end + 1]) {
                end += 1;
            }
            if end == p {
                return Some(section.to_owned());
            }
            out.remove(end);
            if end == p + 1 {
                out.remove(p);
            }
        }
        (None, false) => return Some(section.to_owned()),
    }
    Some(out.into_iter().collect())
}

/// The code after Increase (`delta` > 0) or Decrease Decimal; `None` when it shows no number.
pub(crate) fn adjust_decimals(code: &str, delta: i32) -> Option<String> {
    let mut sections: Vec<String> = Vec::new();
    let mut start = 0;
    let mut in_quote = false;
    let bytes: Vec<(usize, char)> = code.char_indices().collect();
    let mut skip = false;
    for &(i, c) in &bytes {
        if skip {
            skip = false;
            continue;
        }
        match c {
            '"' => in_quote = !in_quote,
            '\\' | '_' | '*' if !in_quote => skip = true,
            ';' if !in_quote => {
                sections.push(code[start..i].to_owned());
                start = i + 1;
            }
            _ => {}
        }
    }
    sections.push(code[start..].to_owned());
    let mut any = false;
    let adjusted: Vec<String> = sections
        .iter()
        .map(|s| {
            if let Some(a) = adjust_section(s, delta) {
                any = true;
                a
            } else {
                s.clone()
            }
        })
        .collect();
    any.then(|| adjusted.join(";"))
}

/// Number of decimal places General shows for `value` in `width` characters.
pub(crate) fn general_decimals(value: f64, width: usize) -> usize {
    if !value.is_finite() || value == 0.0 {
        return 0;
    }
    let mut best = 0;
    for d in 0..=15usize {
        let text = format!("{value:.d$}");
        if text.len() > width.max(1) && d > 0 {
            break;
        }
        best = d;
    }
    // Trailing zeros are not shown.
    let text = format!("{value:.best$}");
    if let Some((_, frac)) = text.split_once('.') {
        frac.trim_end_matches('0').len()
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_codes_get_excels_backslashes() {
        assert_eq!(normalise_code("000-00-0000"), "000\\-00\\-0000");
        assert_eq!(normalise_code("dd.mm.yyyy"), "dd\\.mm\\.yyyy");
        assert_eq!(normalise_code("hh:mm:ss.000"), "hh:mm:ss.000");
        assert_eq!(normalise_code("0_);(0)"), "0_);\\(0\\)");
        assert_eq!(normalise_code("0 \"a b\""), "0\\ \"a b\"");
    }

    #[test]
    fn decimals_follow_the_buttons() {
        assert_eq!(adjust_decimals("0.00", -1).as_deref(), Some("0.0"));
        assert_eq!(adjust_decimals("0", 1).as_deref(), Some("0.0"));
        assert_eq!(adjust_decimals("#,##0.00", -1).as_deref(), Some("#,##0.0"));
        assert_eq!(adjust_decimals("0%", 1).as_deref(), Some("0.0%"));
        assert_eq!(adjust_decimals("0.0", -1).as_deref(), Some("0"));
        assert_eq!(adjust_decimals("0.0E+00", 1).as_deref(), Some("0.00E+00"));
        assert_eq!(adjust_decimals("@", 1), None);
    }

    #[test]
    fn general_shows_what_fits() {
        assert_eq!(general_decimals(1.5, 8), 1);
        assert_eq!(general_decimals(100.0, 8), 0);
        assert_eq!(general_decimals(1.23456789, 8), 6);
        assert_eq!(general_decimals(0.1, 8), 1);
    }
}
