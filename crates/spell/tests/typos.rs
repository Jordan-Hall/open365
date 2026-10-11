#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::print_stdout)]
//! Natural-typo ranking: how often the expected correction is among the top suggestions for
//! real-world misspellings. Source and licence of the corpus: `tests/data/typos/NOTICE.md`.

use open365_spell::Dictionary;
use std::path::Path;

fn load(tag: &str) -> Dictionary {
    let base = format!("{}/dictionaries/{tag}", env!("CARGO_MANIFEST_DIR"));
    Dictionary::from_files(
        Path::new(&format!("{base}.aff")),
        Path::new(&format!("{base}.dic")),
    )
    .unwrap()
}

#[test]
fn natural_typos_rank_the_expected_word_near_the_top() {
    let dict = load("en_US");
    let text = std::fs::read_to_string(format!(
        "{}/tests/data/typos/wikipedia-en.tsv",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let (mut n, mut top1, mut top3, mut top10) = (0usize, 0usize, 0usize, 0usize);
    let mut misses = Vec::new();
    for line in text.lines().filter(|l| !l.is_empty()).step_by(2) {
        let (typo, good) = line.split_once('\t').unwrap();
        // Only rows where the dictionary agrees: typo unknown, correction known.
        if dict.spell(typo) || !dict.spell(good) {
            continue;
        }
        n += 1;
        let got = dict.suggest(typo, &open365_spell::SuggestOptions::default());
        match got.iter().position(|s| s == good) {
            Some(0) => {
                top1 += 1;
                top3 += 1;
                top10 += 1;
            }
            Some(1..=2) => {
                top3 += 1;
                top10 += 1;
            }
            Some(_) => top10 += 1,
            None => misses.push(format!("{typo} -> {good}: {got:?}")),
        }
    }
    let pct = |k: usize| k * 1000 / n;
    println!(
        "natural typos n={n} top1={}‰ top3={}‰ top10={}‰",
        pct(top1),
        pct(top3),
        pct(top10)
    );
    for m in misses.iter().take(15) {
        println!("miss {m}");
    }
    assert!(n >= 300, "corpus too small after filtering: {n}");
    assert!(
        pct(top1) >= 840,
        "top1 {}‰ fell below the 840‰ floor",
        pct(top1)
    );
    assert!(
        pct(top3) >= 910,
        "top3 {}‰ fell below the 910‰ floor",
        pct(top3)
    );
    assert!(
        pct(top10) >= 925,
        "top10 {}‰ fell below the 925‰ floor",
        pct(top10)
    );
}
