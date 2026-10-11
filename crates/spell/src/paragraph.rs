//! Paragraph proofing: misspelled words and repeated words with byte, `char` and UTF-16 ranges.

use crate::casing::Caser;
use crate::tokenize::{IssueKind, Misspelling, Token, TokenizerOptions, scan};

/// What a paragraph check looks for, and how many suggestions the UI wants.
#[derive(Clone, Debug)]
pub struct ProofOptions {
    /// Which words are skipped (all-caps, digits, addresses) and how they are joined.
    pub tokenizer: TokenizerOptions,
    /// Flag a word that repeats the previous one (`the the`), as Word does.
    pub flag_repeated_words: bool,
    /// Suggestions offered per word (Word's context menu shows about five).
    pub max_suggestions: usize,
}

impl Default for ProofOptions {
    fn default() -> Self {
        Self {
            tokenizer: TokenizerOptions::default(),
            flag_repeated_words: true,
            max_suggestions: 5,
        }
    }
}

/// Finds misspelled and repeated words. `is_correct` is asked about each checkable word once.
pub(crate) fn proof_text(
    text: &str,
    options: &ProofOptions,
    tokenizer: &TokenizerOptions,
    caser: &Caser,
    is_correct: &mut dyn FnMut(&str) -> bool,
) -> Vec<Misspelling> {
    let tokens = scan(text, tokenizer);
    let mut out = Vec::new();
    for (i, token) in tokens.iter().enumerate() {
        if token.checkable && !is_correct(token.word.text) {
            out.push(Misspelling::from(&token.word));
        } else if options.flag_repeated_words
            && i > 0
            && is_repeat(text, &tokens[i - 1], token, caser)
        {
            let mut issue = Misspelling::from(&token.word);
            issue.kind = IssueKind::RepeatedWord;
            out.push(issue);
        }
    }
    out
}

/// Words Word does not flag when doubled, observed with Word 16 (`fixtures/spell/repeat-refs-*.tsv`):
/// ordinary constructions (`that that`, `had had`) and a few reduplications.
const ALLOWED_DOUBLES: &[&str] = &["that", "had", "in", "then", "blah", "hush"];

fn is_repeat(text: &str, previous: &Token<'_>, current: &Token<'_>, caser: &Caser) -> bool {
    if !(previous.repeatable && current.repeatable) {
        return false;
    }
    let gap = &text[previous.word.byte_range.end..current.word.byte_range.start];
    // Word only pairs words separated by plain spaces: not a comma, a no-break space or a quote.
    if gap.is_empty() || !gap.chars().all(|c| c == ' ' || c == '\t') {
        return false;
    }
    let (a, b) = (
        caser.lower(previous.word.text),
        caser.lower(current.word.text),
    );
    a == b && !ALLOWED_DOUBLES.contains(&a.as_str())
}
