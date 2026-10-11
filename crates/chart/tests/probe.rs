mod common;
use open365_chart::{FontSpec, TextMeasure};
#[test]
fn probe() {
    let m = common::CarlitoMeasure::new();
    let f = |s: f64, b: bool| FontSpec {
        family: "Calibri".into(),
        size_pt: s,
        bold: b,
        italic: false,
    };
    let titles = [
        ("Sales by region", 117.42),
        ("Trend", 47.42),
        ("Stacked", 66.36),
        ("Fixed", 45.46),
        ("Scatter", 59.97),
        ("Share", 49.08),
        ("Units", 0.0),
    ];
    for (t, want) in titles {
        eprintln!(
            "{t:16} want {want:7.2} | 18b {:.2} 18 {:.2} 14b {:.2} | lh18 {:.3}",
            m.width(t, &f(18.0, true)),
            m.width(t, &f(18.0, false)),
            m.width(t, &f(14.0, true)),
            m.line_height(&f(18.0, true))
        );
    }
    for t in [
        "Alpha", "Beta", "Gamma", "North", "South", "East", "West", "Centre", "25", "100%", "40",
        "Units", "Region",
    ] {
        eprintln!(
            "{t:8} 10: {:.2}  10b: {:.2}",
            m.width(t, &f(10.0, false)),
            m.width(t, &f(10.0, true))
        );
    }
    eprintln!("lh10 {}", m.line_height(&f(10.0, false)));
}
