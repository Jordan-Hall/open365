#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::print_stdout)]
//! Dev tool: dumps ranking features for every candidate of each word Word flags, labelled with Word's rank.
//! `cargo run --release --example pool_dump -- <fixtures dir> > pool.tsv`
use open365_spell::Dictionary;
use std::path::Path;
fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| format!("{}/../../fixtures/spell", env!("CARGO_MANIFEST_DIR")));
    let tags = std::env::args()
        .nth(2)
        .unwrap_or("en-US,en-GB,en-CA,en-AU".into());
    for tag in tags.split(',') {
        let base = format!(
            "{}/dictionaries/{}",
            env!("CARGO_MANIFEST_DIR"),
            tag.replace('-', "_")
        );
        let dict = Dictionary::from_files(
            Path::new(&format!("{base}.aff")),
            Path::new(&format!("{base}.dic")),
        )
        .unwrap();
        let text = std::fs::read_to_string(format!("{dir}/word-refs-{tag}.tsv")).unwrap();
        for l in text
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
        {
            let c: Vec<&str> = l.split('\t').collect();
            if c[1] == "1" || c.len() < 3 || c[2].is_empty() {
                continue;
            }
            let theirs: Vec<&str> = c[2].split('|').collect();
            let pool = dict.suggest_pool(c[0]);
            if pool.is_empty() {
                continue;
            }
            for (cand, f) in pool {
                let rank = theirs.iter().position(|t| *t == cand).map_or(99, |r| r);
                let fs: Vec<String> = f.iter().map(|x| format!("{x}")).collect();
                println!("{tag}\t{}\t{rank}\t{}\t{}", c[0], cand, fs.join("\t"));
            }
        }
    }
}
