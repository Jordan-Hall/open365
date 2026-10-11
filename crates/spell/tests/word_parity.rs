//! Agreement with Microsoft Word's proofing, measured against reference outputs recorded by
//! `tools/office/make-spell-refs.ps1` (fixtures/spell/*.tsv). Only Word's outputs were observed.
//!
//! Run `cargo test -p open365-spell --test word_parity -- --nocapture` to print the agreement
//! numbers (add SPELL_PARITY_FULL=1 and --release for every suggestion). The asserted floors sit just below the measured values so a regression fails the test.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use open365_spell::{IssueKind, Proofing};

fn fixture(name: &str) -> Option<Vec<Vec<String>>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/spell")
        .join(name);
    let text = fs::read_to_string(path).ok()?;
    Some(
        text.lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
            .map(|l| l.split('\t').map(str::to_owned).collect())
            .collect(),
    )
}

struct WordStats {
    total: usize,
    agree: usize,
    word_flagged: usize,
    both_flagged: usize,
    top1: usize,
    word_top1_in_ours_top5: usize,
    ours_top1_in_words_top5: usize,
    set_overlap_top5: f64,
    suggestion_rows: usize,
}

fn measure_words(tag: &str) -> Option<WordStats> {
    let rows = fixture(&format!("word-refs-{tag}.tsv"))?;
    let proofing = Proofing::new();
    let mut stats = WordStats {
        total: 0,
        agree: 0,
        word_flagged: 0,
        both_flagged: 0,
        top1: 0,
        word_top1_in_ours_top5: 0,
        ours_top1_in_words_top5: 0,
        set_overlap_top5: 0.0,
        suggestion_rows: 0,
    };
    let mut disagreements = Vec::new();
    // Suggestions are slow without optimisation, so the default run samples the first 40 words
    // Word flags; SPELL_PARITY_FULL=1 (best with --release) measures all of them.
    let suggestion_sample = if std::env::var_os("SPELL_PARITY_FULL").is_some() {
        usize::MAX
    } else {
        40
    };
    for row in &rows {
        let word = &row[0];
        let word_ok = row[1] == "1";
        let theirs: Vec<&str> = row
            .get(2)
            .map(|s| s.split('|').filter(|x| !x.is_empty()).collect())
            .unwrap_or_default();
        let ours_flagged = !proofing.check_paragraph(word, tag).issues.is_empty();
        stats.total += 1;
        stats.word_flagged += usize::from(!word_ok);
        if ours_flagged != word_ok {
            stats.agree += 1;
        } else {
            disagreements.push(format!(
                "{word}: Word {}, ours {}",
                if word_ok { "accepts" } else { "flags" },
                if ours_flagged { "flags" } else { "accepts" }
            ));
        }
        if ours_flagged && !word_ok {
            stats.both_flagged += 1;
            if stats.suggestion_rows >= suggestion_sample {
                continue;
            }
            let ours = proofing.suggest(word, tag);
            if let (Some(a), Some(b)) = (ours.first(), theirs.first()) {
                stats.suggestion_rows += 1;
                stats.top1 += usize::from(a == b);
                stats.word_top1_in_ours_top5 += usize::from(ours.iter().take(5).any(|x| x == b));
                stats.ours_top1_in_words_top5 += usize::from(theirs.iter().take(5).any(|x| x == a));
                let a5: BTreeSet<&str> = ours.iter().take(5).map(String::as_str).collect();
                let b5: BTreeSet<&str> = theirs.iter().take(5).copied().collect();
                stats.set_overlap_top5 +=
                    a5.intersection(&b5).count() as f64 / b5.len().clamp(1, 5) as f64;
            }
        }
    }
    if std::env::var_os("SPELL_PARITY_VERBOSE").is_some() {
        for d in disagreements.iter().take(80) {
            eprintln!("  {tag} {d}");
        }
    }
    Some(stats)
}

fn pct(n: usize, d: usize) -> f64 {
    if d == 0 {
        0.0
    } else {
        100.0 * n as f64 / d as f64
    }
}

#[test]
fn word_verdicts_and_suggestions_agree_with_word() {
    // Floors, in percent, per tag: (verdict agreement, Word's top suggestion within our top 5).
    let floors = [
        ("en-US", 97.0, 88.0),
        ("en-GB", 97.0, 88.0),
        ("en-CA", 97.0, 88.0),
        ("en-AU", 97.0, 88.0),
    ];
    for (tag, verdict_floor, top5_floor) in floors {
        let Some(s) = measure_words(tag) else {
            eprintln!("{tag}: no reference fixture, skipped");
            continue;
        };
        let verdict = pct(s.agree, s.total);
        let in_top5 = pct(s.word_top1_in_ours_top5, s.suggestion_rows);
        eprintln!(
            "{tag}: {} words, verdict agreement {:.2}% ({} of {}); Word flags {}, both flag {}; \
             suggestions over {} words: first equal {:.1}%, Word's first in our top 5 {:.1}%, \
             our first in Word's top 5 {:.1}%, mean top-5 overlap {:.1}%",
            s.total,
            verdict,
            s.agree,
            s.total,
            s.word_flagged,
            s.both_flagged,
            s.suggestion_rows,
            pct(s.top1, s.suggestion_rows),
            in_top5,
            pct(s.ours_top1_in_words_top5, s.suggestion_rows),
            100.0 * s.set_overlap_top5 / s.suggestion_rows.max(1) as f64,
        );
        assert!(
            verdict >= verdict_floor,
            "{tag} verdict agreement {verdict:.2}%"
        );
        assert!(
            in_top5 >= top5_floor,
            "{tag} suggestion recall {in_top5:.1}%"
        );
    }
}

fn ranges(cell: &str) -> Vec<(usize, usize)> {
    cell.split(',')
        .filter(|x| !x.is_empty())
        .filter_map(|r| r.split_once('-'))
        .filter_map(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)))
        .collect()
}

fn measure_paragraphs(file: &str, tag: &str) -> Option<(usize, usize, usize, usize, usize)> {
    let rows = fixture(file)?;
    let proofing = Proofing::new();
    let (mut paragraphs, mut identical, mut theirs_total, mut ours_total, mut common) =
        (0, 0, 0, 0, 0);
    for row in &rows {
        let text = &row[0];
        let theirs = ranges(row.get(1).map_or("", String::as_str));
        let ours: Vec<(usize, usize)> = proofing
            .check_paragraph(text, tag)
            .issues
            .iter()
            .map(|i| (i.utf16_range.start, i.utf16_range.end))
            .collect();
        paragraphs += 1;
        theirs_total += theirs.len();
        ours_total += ours.len();
        common += ours.iter().filter(|r| theirs.contains(r)).count();
        if ours == theirs {
            identical += 1;
        } else if std::env::var_os("SPELL_PARITY_VERBOSE").is_some() {
            eprintln!("  {tag} {text:?}\n     Word {theirs:?}\n     ours {ours:?}");
        }
    }
    Some((paragraphs, identical, theirs_total, ours_total, common))
}

#[test]
fn paragraph_ranges_agree_with_word() {
    for (tag, floor) in [("en-US", 66.0), ("en-GB", 69.0)] {
        let Some((n, same, theirs, ours, common)) =
            measure_paragraphs(&format!("paragraph-refs-{tag}.tsv"), tag)
        else {
            eprintln!("{tag}: no paragraph fixture, skipped");
            continue;
        };
        let identical = pct(same, n);
        eprintln!(
            "{tag}: {n} paragraphs, identical flag sets {identical:.1}% ({same}); flags: Word {theirs}, \
             ours {ours}, common {common} (precision {:.1}%, recall {:.1}%)",
            pct(common, ours),
            pct(common, theirs),
        );
        assert!(
            identical >= floor,
            "{tag} paragraph agreement {identical:.1}%"
        );
    }
}

#[test]
fn repeated_words_agree_with_word() {
    let Some((n, same, theirs, ours, common)) =
        measure_paragraphs("repeat-refs-en-US.tsv", "en-US")
    else {
        eprintln!("no repeat fixture, skipped");
        return;
    };
    eprintln!(
        "en-US repeated words: {n} doubled-word paragraphs, identical {:.1}% ({same}); flags: Word {theirs}, \
         ours {ours}, common {common}",
        pct(same, n)
    );
    assert!(
        pct(same, n) >= 95.0,
        "repeat agreement {:.1}%",
        pct(same, n)
    );
    let proofing = Proofing::new();
    let report = proofing.check_paragraph("This is is a test.", "en-US");
    assert_eq!(report.issues.len(), 1);
    assert_eq!(report.issues[0].kind, IssueKind::RepeatedWord);
}
