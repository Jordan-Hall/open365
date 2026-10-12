//! Compare / Combine support: a word-level text diff and a paragraph alignment. Pure functions;
//! the front end applies the edits as tracked changes.

/// One replacement in the original text: `delete` characters at char offset `at`, then `insert`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEdit {
    pub at: usize,
    pub delete: usize,
    pub insert: String,
}

/// Splits into words, runs of spaces and single punctuation marks, so edits fall on word edges.
fn tokens(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut kind = 0u8;
    for c in s.chars() {
        let k = if c.is_alphanumeric() {
            1
        } else if c.is_whitespace() {
            2
        } else {
            3
        };
        if k == kind
            && k != 3
            && let Some(last) = out.last_mut()
        {
            last.push(c);
            continue;
        }
        kind = k;
        out.push(c.to_string());
    }
    out
}

/// Longest common subsequence as matched index pairs.
fn lcs<T: PartialEq>(a: &[T], b: &[T]) -> Vec<(usize, usize)> {
    let (n, m) = (a.len(), b.len());
    let mut t = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            t[i][j] = if a[i] == b[j] {
                t[i + 1][j + 1] + 1
            } else {
                t[i + 1][j].max(t[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::new();
    while i < n && j < m {
        if a[i] == b[j] {
            out.push((i, j));
            i += 1;
            j += 1;
        } else if t[i + 1][j] >= t[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

/// The edits that turn `original` into `revised`, in ascending order of offset.
#[must_use]
pub fn diff_text(original: &str, revised: &str) -> Vec<TextEdit> {
    if original == revised {
        return Vec::new();
    }
    let (a, b) = (tokens(original), tokens(revised));
    let mut edits = Vec::new();
    let (mut ai, mut bi, mut at) = (0usize, 0usize, 0usize);
    let mut flush = |ai: &mut usize, bi: &mut usize, at: &mut usize, to_a: usize, to_b: usize| {
        let del: usize = a[*ai..to_a].iter().map(|t| t.chars().count()).sum();
        let ins: String = b[*bi..to_b].concat();
        if del > 0 || !ins.is_empty() {
            edits.push(TextEdit {
                at: *at,
                delete: del,
                insert: ins,
            });
        }
        *at += del;
        *ai = to_a;
        *bi = to_b;
    };
    for (i, j) in lcs(&a, &b) {
        flush(&mut ai, &mut bi, &mut at, i, j);
        at += a[i].chars().count();
        ai = i + 1;
        bi = j + 1;
    }
    flush(&mut ai, &mut bi, &mut at, a.len(), b.len());
    edits
}

/// Aligns paragraphs: `(Some(i), Some(j))` pairs original `i` with revised `j` (equal when the
/// text matches, otherwise a modified paragraph), `(Some(i), None)` was deleted and
/// `(None, Some(j))` was inserted.
#[must_use]
pub fn align_paragraphs(
    original: &[String],
    revised: &[String],
) -> Vec<(Option<usize>, Option<usize>)> {
    let mut out = Vec::new();
    let (mut ai, mut bi) = (0, 0);
    let gap = |out: &mut Vec<_>, a0: usize, a1: usize, b0: usize, b1: usize| {
        let common = (a1 - a0).min(b1 - b0);
        for k in 0..common {
            out.push((Some(a0 + k), Some(b0 + k)));
        }
        for i in a0 + common..a1 {
            out.push((Some(i), None));
        }
        for j in b0 + common..b1 {
            out.push((None, Some(j)));
        }
    };
    for (i, j) in lcs(original, revised) {
        gap(&mut out, ai, i, bi, j);
        out.push((Some(i), Some(j)));
        ai = i + 1;
        bi = j + 1;
    }
    gap(&mut out, ai, original.len(), bi, revised.len());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_edits() {
        let e = diff_text("The quick fox", "The slow fox");
        assert_eq!(
            e,
            vec![TextEdit {
                at: 4,
                delete: 5,
                insert: "slow".into()
            }]
        );
        assert!(diff_text("same", "same").is_empty());
    }

    #[test]
    fn paragraphs_align() {
        let a = ["a".to_owned(), "b".to_owned(), "c".to_owned()];
        let b = ["a".to_owned(), "c".to_owned(), "d".to_owned()];
        let al = align_paragraphs(&a, &b);
        assert_eq!(
            al,
            vec![
                (Some(0), Some(0)),
                (Some(1), None),
                (Some(2), Some(1)),
                (None, Some(2))
            ]
        );
    }
}
