//! Argument-count checks of the parser against what Excel accepts when a formula is entered.
//!
//! `fixtures/calc/arity/excel-arity.tsv` is the result of `tools/office/make-numfmt-calc-refs.ps1 -Mode Arity`: for every
//! function of the registry, the counts Excel's `Range.Formula` assignment accepted (`NAME min max step`, or `irregular`
//! with the list of counts). The parser's table (`parser_arity.rs`) is generated from the same run.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use open365_calc::functions::function_names;
use open365_calc::{ParseErrorKind, parse_formula};

const NAMES: &str = include_str!("../../../fixtures/calc/arity/names.txt");
const MEASURED: &str = include_str!("../../../fixtures/calc/arity/excel-arity.tsv");

fn formula(name: &str, n: usize) -> String {
    format!("={name}({})", vec!["A1"; n].join(","))
}

fn accepted(line: &str) -> (String, Vec<usize>) {
    let f: Vec<&str> = line.split('\t').collect();
    let list = match f[1] {
        "none" => vec![],
        "irregular" => f[2].split(',').map(|n| n.parse().unwrap()).collect(),
        _ => {
            let (min, max, step): (usize, usize, usize) = (
                f[1].parse().unwrap(),
                f[2].parse().unwrap(),
                f[3].parse().unwrap(),
            );
            let mut list: Vec<usize> = (min..=max).step_by(step).collect();
            list.extend(f.get(4).map(|e| e.parse::<usize>().unwrap()));
            list
        }
    };
    (f[0].to_owned(), list)
}

#[test]
fn the_measured_list_is_consistent_with_the_registry() {
    let names: Vec<&str> = NAMES.lines().collect();
    let measured: Vec<&str> = MEASURED
        .lines()
        .map(|l| l.split(char::from(9)).next().unwrap())
        .collect();
    assert_eq!(
        measured, names,
        "re-measure with make-numfmt-calc-refs.ps1 -Mode Arity"
    );
    // Functions added to the registry after the last measurement are not checked until the script is run again; a
    // measured name that left the registry is a stale list.
    let registry = function_names();
    let stale: Vec<_> = names.iter().filter(|n| !registry.contains(n)).collect();
    assert!(stale.is_empty(), "not in the registry any more: {stale:?}");
    assert!(names.len() >= 486);
}

#[test]
fn the_parser_accepts_exactly_the_counts_excel_accepts() {
    let mut checked = 0usize;
    let mut bad = Vec::new();
    for line in MEASURED.lines() {
        let (name, ok) = accepted(line);
        // Every small count, and the neighbourhood of both ends of the accepted run (and of its first and last members).
        let max = ok.iter().copied().max().unwrap_or(0);
        let mut counts: Vec<usize> = (0..=10).collect();
        counts.extend(
            ok.iter()
                .flat_map(|&n| [n.saturating_sub(1), n, n + 1])
                .filter(|&n| n <= 10 || n + 4 >= max),
        );
        counts.extend(max.saturating_sub(2)..=max + 2);
        counts.sort_unstable();
        counts.dedup();
        for n in counts {
            let excel = ok.contains(&n);
            let ours = parse_formula(&formula(&name, n)).is_ok();
            checked += 1;
            if excel != ours {
                bad.push(format!(
                    "{name} with {n} arguments: Excel {excel}, parser {ours}"
                ));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "{} of {checked} differ:\n{}",
        bad.len(),
        bad[..bad.len().min(30)].join("\n")
    );
}

#[test]
fn rejection_is_a_typed_error() {
    // conformance case randarray.c0169
    let e = parse_formula("=COUNT(RANDARRAY(1,2,3,4,5,6))").unwrap_err();
    assert_eq!(
        e.kind,
        ParseErrorKind::ArgumentCount {
            name: "RANDARRAY".into(),
            given: 6
        }
    );
    assert!(parse_formula("=PI(1)").is_err());
    assert!(parse_formula("=ABS()").is_err());
    assert!(parse_formula("=_xlfn.XLOOKUP(1,A:A)").is_err());
    assert!(parse_formula("=IF(A1,,2)").is_ok());
    assert!(parse_formula("=NoSuchFunction(1,2,3)").is_ok());
    assert!(parse_formula("=PI()").is_ok());
}
