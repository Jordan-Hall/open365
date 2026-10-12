//! Every ribbon control id of the two specs, which ribbon module handles it, and the coverage per
//! tab. Two modules must never claim one id.

#![allow(clippy::unwrap_used)]

use std::collections::{BTreeMap, BTreeSet};

use open365_preview::doc::WriterPane;
use open365_preview::ribbon::{self, Handled};
use open365_preview::spec::AppSpec;
use open365_preview::{AppKind, Preview};

fn spec_of(kind: AppKind) -> AppSpec {
    match kind {
        AppKind::Writer => AppSpec::writer(),
        AppKind::Sheets => AppSpec::sheets(),
    }
}

/// Every claimed id with the module (prefix) that claims it; panics on a double claim.
fn claims(kind: AppKind) -> BTreeMap<&'static str, &'static str> {
    let mut out = BTreeMap::new();
    for m in ribbon::modules(kind) {
        for id in m.ids {
            assert!(
                id.strip_prefix(m.prefix)
                    .is_some_and(|r| r.starts_with('.')),
                "{kind:?}: `{id}` is claimed by the `{}` module",
                m.prefix
            );
            if let Some(other) = out.insert(*id, m.prefix) {
                panic!(
                    "{kind:?}: `{id}` is claimed twice ({other} and {})",
                    m.prefix
                );
            }
        }
    }
    out
}

#[test]
fn no_id_is_claimed_twice() {
    for kind in [AppKind::Writer, AppKind::Sheets] {
        let _ = claims(kind);
    }
}

#[test]
fn coverage_per_tab() {
    for kind in [AppKind::Writer, AppKind::Sheets] {
        let spec = spec_of(kind);
        let claimed = claims(kind);
        let (mut all, mut done) = (0usize, 0usize);
        let mut seen = BTreeSet::new();
        for tab in &spec.tabs {
            let ids: BTreeSet<&str> = tab
                .groups
                .iter()
                .flat_map(|g| g.controls())
                .map(|c| c.id.as_str())
                .collect();
            let handled = ids.iter().filter(|id| claimed.contains_key(**id)).count();
            println!(
                "{kind:?} {:<12} {handled:>3}/{:<3} {:>5.1}%",
                tab.id,
                ids.len(),
                100.0 * handled as f64 / ids.len().max(1) as f64
            );
            for id in &ids {
                if claimed.contains_key(id) {
                    println!("    handled {id}");
                }
            }
            all += ids.len();
            done += handled;
            seen.extend(ids);
        }
        println!(
            "{kind:?} total        {done:>3}/{all:<3} {:>5.1}%",
            100.0 * done as f64 / all.max(1) as f64
        );
        // A claimed id that is no control would be a typo.
        for id in claimed.keys() {
            assert!(
                seen.contains(id),
                "{kind:?}: `{id}` is claimed but is no control in the spec"
            );
        }
    }
}

/// Every claimed id is answered by its module on a live shell (also with the suffix a combo
/// box adds).
#[test]
fn claimed_ids_are_handled() {
    for kind in [AppKind::Writer, AppKind::Sheets] {
        let mut p = Preview::new(kind, false, 1600, 1000, 1.25);
        if kind == AppKind::Writer {
            let mut pane = WriterPane::blank();
            pane.insert_text("hello");
            p.shell.set_writer(&mut p.ui, pane);
        }
        for id in claims(kind).keys() {
            let yes = [(*id).to_owned(), format!("{id}=12")]
                .iter()
                .any(|i| p.shell.run_command(&mut p.ui, i) == Handled::Yes);
            assert!(
                yes,
                "{kind:?}: `{id}` is listed but its module does not handle it"
            );
        }
    }
}

#[test]
fn unknown_ids_are_not_handled() {
    let mut p = Preview::new(AppKind::Writer, false, 1600, 1000, 1.25);
    for id in [
        "",
        "nothing",
        "share",
        "qat.undo",
        "home.no-such",
        "view.nothing",
    ] {
        assert_eq!(p.shell.run_command(&mut p.ui, id), Handled::No, "{id}");
    }
}
