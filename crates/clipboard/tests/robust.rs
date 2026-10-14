//! Untrusted input: mutated and random clipboard data must never panic and must stay fast.

use open365_clipboard::cfhtml::{self, Flavor};
use open365_clipboard::{PrivateClip, SheetClip, rtf, writer_html};

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }
}

const HTML: &str = "<html><head><style>.a{color:red;background:url(http://x/y)} td{mso-number-format:\"0\\.00\"}</style></head>\
<body><!--StartFragment--><p class=a style='font-family:\"A\";font-size:12pt'>x<b>y</b><a href='javascript:1'>z</a></p>\
<table><tr><td class=a x:num=\"1.5\" colspan=2 style='border-top:.5pt solid red'>1.50</td><td x:fmla=\"=A1\">q</td></tr></table>\
<!--EndFragment--></body></html>";
const RTF: &[u8] = br"{\rtf1\ansi{\fonttbl{\f0 Arial;}}{\colortbl ;\red1\green2\blue3;}\pard\qc\f0\fs24\cf1 a\'e9\u8364?{\b b}{\*\x y}{\pict 00ff}\par}";

fn mutate(src: &[u8], rng: &mut Lcg) -> Vec<u8> {
    let mut v = src.to_vec();
    for _ in 0..=(rng.next() % 6) {
        if v.is_empty() {
            break;
        }
        let i = (rng.next() as usize) % v.len();
        match rng.next() % 4 {
            0 => v[i] = (rng.next() & 0xFF) as u8,
            1 => {
                v.remove(i);
            }
            2 => v.insert(i, b"<>{}\\&;\"'"[(rng.next() % 9) as usize]),
            _ => v.truncate(i),
        }
    }
    v
}

#[test]
fn mutated_html_rtf_and_containers_never_panic() {
    let mut rng = Lcg(7);
    let wrapped = cfhtml::wrap(HTML, Flavor::Sheet);
    let private = PrivateClip::Sheet(SheetClip::from_html(HTML).unwrap())
        .encode()
        .unwrap();
    for _ in 0..3000 {
        let h = mutate(HTML.as_bytes(), &mut rng);
        let s = String::from_utf8_lossy(&h);
        let _ = writer_html::parse(&s);
        let _ = SheetClip::from_html(&s);
        let _ = rtf::parse(&mutate(RTF, &mut rng));
        if let Ok(u) = cfhtml::unwrap(&mutate(&wrapped, &mut rng)) {
            let _ = writer_html::parse(&u);
        }
        let _ = PrivateClip::decode(&mutate(&private, &mut rng));
    }
}

#[test]
fn random_bytes_never_panic() {
    let mut rng = Lcg(99);
    for _ in 0..500 {
        let n = (rng.next() % 400) as usize;
        let data: Vec<u8> = (0..n).map(|_| (rng.next() & 0xFF) as u8).collect();
        let s = String::from_utf8_lossy(&data);
        let _ = writer_html::parse(&s);
        let _ = SheetClip::from_html(&s);
        let _ = rtf::parse(&data);
        let _ = cfhtml::unwrap(&data);
        let _ = PrivateClip::decode(&data);
    }
}

#[test]
fn pathological_inputs_are_bounded() {
    let start = std::time::Instant::now();
    // Many unclosed elements, many attributes, huge comment, long run of ampersands.
    let a = "<b ".repeat(200_000);
    let _ = writer_html::parse(&a);
    let b = format!("<p {}>x</p>", "a=1 ".repeat(100_000));
    let _ = writer_html::parse(&b);
    let c = "&".repeat(2_000_000);
    let _ = writer_html::parse(&c);
    let d = format!("<table>{}</table>", "<tr><td>x</td></tr>".repeat(50_000));
    let clip = SheetClip::from_html(&d).unwrap();
    assert_eq!(clip.rows.len(), 50_000);
    let e = format!("{{\\rtf1{}}}", "\\b0 x".repeat(500_000));
    let _ = rtf::parse(e.as_bytes());
    assert!(
        start.elapsed() < std::time::Duration::from_secs(60),
        "took {:?}",
        start.elapsed()
    );
}
