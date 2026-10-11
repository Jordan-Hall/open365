//! Unit tests of the PCRE2-compatible engine (`textpcre.rs`, `textpcrevm.rs`). The conformance corpus
//! (`regextest.*`, `regexextract.*`, `regexreplace.*`) compares the engine with Excel; these cover the
//! semantics that the corpus reaches only indirectly.

use super::textpcre::compile;
use super::textpcrevm::{Abort, LAST_STEPS};

fn run(pattern: &str, subject: &str, caseless: bool) -> Result<Option<(usize, usize)>, Abort> {
    let re = compile(pattern, caseless).expect("pattern compiles");
    let hay: Vec<char> = subject.chars().collect();
    Ok(re.exec(&hay, 0, false, false)?.and_then(|c| c[0]))
}

fn found(pattern: &str, subject: &str) -> bool {
    run(pattern, subject, false).expect("no abort").is_some()
}

fn span(pattern: &str, subject: &str) -> Option<(usize, usize)> {
    run(pattern, subject, false).expect("no abort")
}

#[test]
fn literals_classes_and_quantifiers() {
    assert_eq!(span("b+", "abbbc"), Some((1, 4)));
    assert_eq!(span("b+?", "abbbc"), Some((1, 2)));
    assert_eq!(span("a{2,3}", "aaaa"), Some((0, 3)));
    assert_eq!(span("a{,2}", "aaaa"), Some((0, 2)));
    assert_eq!(span("[a-c]+", "xxabcabx"), Some((2, 7)));
    assert_eq!(span("[^a-c]+", "abxyc"), Some((2, 4)));
    assert_eq!(span("\\d+", "ab123c"), Some((2, 5)));
    assert_eq!(span("a.c", "abc"), Some((0, 3)));
    assert!(!found("a.c", "a\nc"));
    assert!(found("(?s)a.c", "a\nc"));
}

#[test]
fn anchors_use_the_anycrlf_convention() {
    assert!(found("ab$", "ab\n"));
    assert!(found("ab$", "ab\r\n"));
    assert!(!found("ab\\z", "ab\n"));
    assert!(found("ab\\Z", "ab\n"));
    assert!(found("(?m)^b", "a\r\nb"));
    assert!(found("(?m)^b", "a\rb"));
    assert!(!found("(?m)^$", "a"));
    assert!(!found("a.b", "a\rb"));
    assert!(found("a.b", "a\u{85}b"));
}

#[test]
fn groups_alternation_and_backreferences() {
    assert_eq!(span("(a|ab)(c|bcd)(d*)", "abcd"), Some((0, 4)));
    assert!(found("(ab)\\1", "abab"));
    assert!(!found("(ab)\\1", "abba"));
    assert!(found("(?<x>ab)\\k<x>", "abab"));
    assert!(found("(?P<x>a)(?P=x)", "aa"));
    assert!(found("(?|(a)|(b))\\1", "bb"));
    assert!(found("(?i)straSSe", "STRASSE"));
}

#[test]
fn lookaround_atomic_and_possessive() {
    assert_eq!(span("a(?=b)", "acab"), Some((2, 3)));
    assert_eq!(span("(?<=a)b", "cbab"), Some((3, 4)));
    assert!(found("(?<=ab?)c", "abc"));
    assert!(!found("(?<!a)b", "ab"));
    assert!(!found("(?>a+)a", "aaa"));
    assert!(!found("a++a", "aaa"));
    assert!(found("a*+b", "aab"));
    assert_eq!(span("a\\Kb", "ab"), Some((1, 2)));
}

#[test]
fn recursion_conditionals_and_verbs() {
    assert!(found("^(\\((?1)*\\))$", "(()())"));
    assert!(!found("^(\\((?1)*\\))$", "(()"));
    assert!(found("^(a)?(?(1)b|c)$", "ab"));
    assert!(found("^(a)?(?(1)b|c)$", "c"));
    assert!(!found("^(a)?(?(1)b|c)$", "ac"));
    assert!(found("(?(?=a)ab|cd)", "ab"));
    assert!(!found("a(*FAIL)", "a"));
}

#[test]
fn unicode_properties_and_case() {
    assert!(found("\\w", "\u{e9}"));
    assert!(found("\\d", "\u{663}"));
    assert!(!found("\\d", "\u{b2}"));
    assert!(found("\\p{Lu}", "\u{c9}"));
    assert!(found("\\p{Greek}", "\u{3b3}"));
    assert!(found("[[:alpha:]]", "\u{e9}"));
    assert_eq!(span("^.$", "\u{1f600}"), Some((0, 1)));
    assert!(run("\u{e9}", "\u{c9}", true).expect("no abort").is_some());
    assert!(run("k", "\u{212a}", true).expect("no abort").is_some());
    assert!(run("i", "\u{130}", true).expect("no abort").is_none());
}

#[test]
fn invalid_patterns_are_rejected() {
    for bad in [
        "(",
        "[",
        "*",
        "a**",
        "a{2,1}",
        "(?<=a*)b",
        "\\u0061",
        "(?<n>a)(?<n>b)",
        "[z-a]",
        "\\p{Foo}",
        "\\p{Letter}",
        "(?z)a",
        "a)",
        "^*",
        "\\b+",
        "[[:foo:]]",
        "\\8",
        "a{1,65536}",
    ] {
        assert!(compile(bad, false).is_err(), "{bad} should not compile");
    }
    for good in [
        "a{1",
        "a{,3}",
        "[]a]",
        "[a-]",
        "(?x) a b # comment",
        "\\Qa.b\\E",
        "(?xx)a [ b ] c",
    ] {
        assert!(compile(good, false).is_ok(), "{good} should compile");
    }
}

#[test]
fn the_backtracking_limits_match_excel() {
    // Measured in Excel: `^(a+)+$` on n `a` plus `!` answers up to 18 and gives #VALUE! from 19.
    let subject = |n: usize| format!("{}!", "a".repeat(n));
    let mut steps = Vec::new();
    for n in [15, 18, 19, 20] {
        let r = run("^(a+)+$", &subject(n), false);
        steps.push((n, LAST_STEPS.with(std::cell::Cell::get), r.is_err()));
    }
    assert_eq!(
        steps.iter().map(|s| s.2).collect::<Vec<_>>(),
        [false, false, true, true],
        "{steps:?}"
    );
    // `^(a|aa)+$`: answers on 23, aborts on 26.
    for (n, aborts) in [(20, false), (23, false), (26, true), (29, true)] {
        let r = run("^(a|aa)+$", &subject(n), false);
        steps.push((n, LAST_STEPS.with(std::cell::Cell::get), r.is_err()));
        assert_eq!(r.is_err(), aborts, "(a|aa)+ with {n}: {steps:?}");
    }
    // A pattern whose required literal is absent fails at once, as in PCRE2.
    assert_eq!(
        run("(x+x+)+y", &"x".repeat(40), false).expect("no abort"),
        None
    );
    // A long subject with a simple loop neither aborts nor overflows the stack.
    let long = "a".repeat(20_000);
    assert!(run("^(a)*$", &long, false).expect("no abort").is_some());
    assert!(run("^(a|b)*$", &"a".repeat(100_000), false).is_err());
    assert!(
        run("^(a|b)*$", &"a".repeat(10_000), false)
            .expect("no abort")
            .is_some()
    );
}
