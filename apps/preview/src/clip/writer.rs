//! Writer side of the rich clipboard.

use open365_clipboard::{
    Align, CharFmt, ListKind as ClipList, Para, Rgb, RichText, Run as ClipRun,
};
use open365_commands::{
    Alignment, Command, DocPos, ListKind, RunPropsPatch, VertAlign, WriterHomeOp,
};
use open365_docx::{Document, Inline, Paragraph, Run, RunContent};

const HIGHLIGHTS: [(&str, [u8; 3]); 16] = [
    ("yellow", [255, 255, 0]),
    ("green", [0, 255, 0]),
    ("cyan", [0, 255, 255]),
    ("magenta", [255, 0, 255]),
    ("blue", [0, 0, 255]),
    ("red", [255, 0, 0]),
    ("darkBlue", [0, 0, 128]),
    ("darkCyan", [0, 128, 128]),
    ("darkGreen", [0, 128, 0]),
    ("darkMagenta", [128, 0, 128]),
    ("darkRed", [128, 0, 0]),
    ("darkYellow", [128, 128, 0]),
    ("darkGray", [128, 128, 128]),
    ("lightGray", [192, 192, 192]),
    ("black", [0, 0, 0]),
    ("white", [255, 255, 255]),
];

fn collect_runs<'a>(content: &'a [Inline], out: &mut Vec<&'a Run>) {
    for inline in content {
        match inline {
            Inline::Run(r) => out.push(r),
            Inline::Group(g) => collect_runs(&g.content, out),
            Inline::Raw(_) => {}
        }
    }
}

fn run_text(run: &Run) -> String {
    let mut s = String::new();
    for item in &run.content {
        match item {
            RunContent::Text { text, .. } => s.push_str(text),
            RunContent::Tab => s.push('\t'),
            RunContent::Break(_) => s.push('\n'),
            RunContent::Raw(_) => {}
        }
    }
    s
}

fn run_format(doc: &Document, para: &Paragraph, run: &Run) -> CharFmt {
    let mut f = CharFmt::default();
    if let Some(styles) = doc.styles() {
        let e = styles.effective_run_props(para, run);
        f.font = Some(e.font);
        f.size_pt = Some(e.size_half_points as f32 / 2.0);
        f.bold = e.bold;
        f.italic = e.italic;
        f.underline = e.underline;
        f.strike = e.strike;
        f.color = e.color.map(|[r, g, b]| Rgb::new(r, g, b));
    }
    match run.props.vert_align() {
        Some("superscript") => f.superscript = true,
        Some("subscript") => f.subscript = true,
        _ => {}
    }
    if let Some(h) = run.props.highlight() {
        f.highlight = HIGHLIGHTS
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(h))
            .map(|(_, [r, g, b])| Rgb::new(*r, *g, *b));
    }
    f
}

/// The text between two positions (either order) with its character formatting, paragraph
/// alignment and heading levels, for the HTML, RTF and private clipboard formats.
pub fn copy_selection(doc: &Document, a: DocPos, b: DocPos) -> RichText {
    let (start, end) = if a <= b { (a, b) } else { (b, a) };
    let mut out = RichText::default();
    let last = end.para.min(doc.paragraph_count().saturating_sub(1));
    for index in start.para..=last {
        let Some(para) = doc.paragraph(index) else {
            continue;
        };
        let lo = if index == start.para { start.offset } else { 0 };
        let hi = if index == end.para {
            end.offset
        } else {
            usize::MAX
        };
        let align = doc
            .styles()
            .map(|s| s.effective_para_props(para).alignment)
            .unwrap_or(Alignment::Left);
        let heading = match para.style_id() {
            Some(id) => id
                .strip_prefix("Heading")
                .and_then(|n| n.parse::<u8>().ok())
                .filter(|n| (1..=6).contains(n)),
            None => None,
        };
        let mut p = Para {
            runs: Vec::new(),
            align: match align {
                Alignment::Center => Align::Center,
                Alignment::Right => Align::Right,
                Alignment::Justify => Align::Justify,
                _ => Align::Left,
            },
            heading,
            list: None,
        };
        let mut runs = Vec::new();
        collect_runs(&para.content, &mut runs);
        let mut at = 0;
        for run in runs {
            let text = run_text(run);
            let len = text.chars().count();
            let (from, to) = (at.max(lo), (at + len).min(hi));
            let base = at;
            at += len;
            if from >= to {
                continue;
            }
            let piece: String = text.chars().skip(from - base).take(to - from).collect();
            p.runs.push(ClipRun {
                text: piece,
                fmt: run_format(doc, para, run),
                link: None,
            });
        }
        out.paras.push(p);
    }
    out.normalize();
    out
}

/// What a paste inserts: `text` goes through the Writer's normal text paste (line breaks start
/// paragraphs) and `commands` restore the formatting on the inserted range.
#[derive(Debug, Clone, PartialEq)]
pub struct Insertion {
    pub text: String,
    pub commands: Vec<Command>,
}

/// `RRGGBB`.
fn hex(c: Rgb) -> String {
    c.css()[1..].to_owned()
}

fn alignment(a: Align) -> Alignment {
    match a {
        Align::Left => Alignment::Left,
        Align::Center => Alignment::Center,
        Align::Right => Alignment::Right,
        Align::Justify => Alignment::Justify,
    }
}

fn patch(f: &CharFmt) -> RunPropsPatch {
    RunPropsPatch {
        bold: Some(f.bold),
        italic: Some(f.italic),
        underline: Some(f.underline),
        strike: Some(f.strike),
        size_half_points: f.size_pt.map(|s| (s * 2.0).round() as u32),
        color: f.color.map(hex),
        font: f.font.clone(),
        vert_align: Some(if f.superscript {
            VertAlign::Superscript
        } else if f.subscript {
            VertAlign::Subscript
        } else {
            VertAlign::Baseline
        }),
    }
}

/// Plans the insertion of `rt` at `start` (the caret, after any selection was deleted).
/// `formatted` is false for plain-text pastes: then only the text is inserted.
pub fn insertion(rt: &RichText, start: DocPos, formatted: bool) -> Insertion {
    let text = rt.plain();
    if !formatted {
        return Insertion {
            text,
            commands: Vec::new(),
        };
    }
    let mut para_cmds = Vec::new();
    let mut run_cmds = Vec::new();
    let mut high_cmds = Vec::new();
    let mut lists: Vec<(usize, usize, ListKind)> = Vec::new();
    let mut pos = start;
    for (i, p) in rt.paras.iter().enumerate() {
        let first = pos.para;
        for r in &p.runs {
            let from = pos;
            for c in r.text.chars() {
                if c == '\n' {
                    pos = DocPos::new(pos.para + 1, 0);
                } else {
                    pos = DocPos::new(pos.para, pos.offset + 1);
                }
            }
            if from == pos {
                continue;
            }
            run_cmds.push(Command::DocxSetRunProps {
                start: from,
                end: pos,
                patch: patch(&r.fmt),
            });
            if let Some(h) = r.fmt.highlight {
                high_cmds.push(Command::DocxHome {
                    op: WriterHomeOp::Highlight {
                        start: from,
                        end: pos,
                        color: Some(hex(h)),
                    },
                });
            }
        }
        let last = pos.para;
        // The first inserted paragraph joins the paragraph at the caret unless the caret
        // is at its start, so it keeps that paragraph's own properties.
        let owns_first = i > 0 || start.offset == 0;
        for para in first..=last {
            if para == first && !owns_first {
                continue;
            }
            if p.align != Align::Left {
                para_cmds.push(Command::DocxSetAlignment {
                    para,
                    alignment: alignment(p.align),
                });
            }
            if let Some(h) = p.heading {
                para_cmds.push(Command::DocxSetParagraphStyle {
                    para,
                    style_id: format!("Heading{h}"),
                });
            }
        }
        if let Some(k) = p.list {
            let kind = match k {
                ClipList::Bullet => ListKind::Bullet,
                ClipList::Numbered => ListKind::Number,
            };
            match lists.last_mut() {
                Some((_, l, lk)) if *lk == kind && *l + 1 == first => *l = last,
                _ => lists.push((first, last, kind)),
            }
        }
        pos = DocPos::new(pos.para + 1, 0);
    }
    let mut commands = para_cmds;
    commands.extend(run_cmds);
    commands.extend(high_cmds);
    for (first, last, kind) in lists {
        commands.push(Command::DocxHome {
            op: WriterHomeOp::List { first, last, kind },
        });
    }
    Insertion { text, commands }
}

#[cfg(test)]
mod tests {
    use super::*;
    use open365_commands::Target;

    fn rich() -> RichText {
        RichText {
            paras: vec![
                Para {
                    runs: vec![
                        ClipRun {
                            text: "Hello ".into(),
                            fmt: CharFmt::default(),
                            link: None,
                        },
                        ClipRun {
                            text: "bold".into(),
                            fmt: CharFmt {
                                bold: true,
                                color: Some(Rgb::new(255, 0, 0)),
                                size_pt: Some(14.0),
                                font: Some("Arial".into()),
                                highlight: Some(Rgb::new(255, 255, 0)),
                                ..CharFmt::default()
                            },
                            link: None,
                        },
                    ],
                    align: Align::Center,
                    heading: None,
                    list: None,
                },
                Para {
                    runs: vec![ClipRun {
                        text: "item".into(),
                        ..ClipRun::default()
                    }],
                    list: Some(ClipList::Bullet),
                    ..Para::default()
                },
            ],
        }
    }

    #[test]
    fn plan_positions_follow_paragraph_breaks() {
        let ins = insertion(&rich(), DocPos::new(3, 0), true);
        assert_eq!(ins.text, "Hello bold\nitem");
        assert!(ins.commands.contains(&Command::DocxSetAlignment {
            para: 3,
            alignment: Alignment::Center
        }));
        assert!(ins.commands.iter().any(|c| matches!(
            c,
            Command::DocxSetRunProps { start, end, patch }
                if *start == DocPos::new(3, 6) && *end == DocPos::new(3, 10)
                    && patch.bold == Some(true) && patch.size_half_points == Some(28)
                    && patch.color.as_deref() == Some("FF0000")
        )));
        assert!(ins.commands.iter().any(|c| matches!(
            c,
            Command::DocxHome {
                op: WriterHomeOp::List {
                    first: 4,
                    last: 4,
                    kind: ListKind::Bullet
                }
            }
        )));
        // Mid-paragraph paste keeps the receiving paragraph's alignment.
        let mid = insertion(&rich(), DocPos::new(3, 2), true);
        assert!(
            !mid.commands
                .iter()
                .any(|c| matches!(c, Command::DocxSetAlignment { .. }))
        );
        assert!(
            insertion(&rich(), DocPos::new(0, 0), false)
                .commands
                .is_empty()
        );
    }

    /// Copy a formatted selection, paste it into another document, and read it back.
    #[test]
    fn formatting_survives_copy_and_paste_between_documents() {
        let mut t = blank_document();
        let ins = insertion(&rich(), DocPos::new(0, 0), true);
        // Insert the text as the Writer does (one paragraph split per line break).
        let mut para = 0;
        let mut offset = 0;
        for (i, line) in ins.text.split('\n').enumerate() {
            if i > 0 {
                t.apply(Command::DocxSplitParagraph {
                    pos: DocPos::new(para, offset),
                })
                .unwrap();
                para += 1;
                offset = 0;
            }
            t.apply(Command::DocxInsertText {
                para,
                offset,
                text: line.to_owned(),
            })
            .unwrap();
            offset += line.chars().count();
        }
        for c in &ins.commands {
            t.apply(c.clone()).unwrap();
        }
        let doc = t.docx().unwrap();
        let back = copy_selection(doc, DocPos::new(0, 0), DocPos::new(1, 4));
        assert_eq!(back.plain(), "Hello bold\nitem");
        assert_eq!(back.paras[0].align, Align::Center);
        let bold = back.paras[0]
            .runs
            .iter()
            .find(|r| r.text == "bold")
            .unwrap();
        assert!(bold.fmt.bold);
        assert_eq!(bold.fmt.color, Some(Rgb::new(255, 0, 0)));
        assert_eq!(bold.fmt.size_pt, Some(14.0));
        assert_eq!(bold.fmt.font.as_deref(), Some("Arial"));
        assert_eq!(bold.fmt.highlight, Some(Rgb::new(255, 255, 0)));
        let plain = back.paras[0]
            .runs
            .iter()
            .find(|r| r.text.starts_with("Hello"))
            .unwrap();
        assert!(!plain.fmt.bold);
    }

    fn blank_document() -> Target {
        Target::open_docx(&crate::doc::blank::docx()).expect("the blank document opens")
    }
}
