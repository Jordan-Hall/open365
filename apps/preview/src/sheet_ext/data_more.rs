//! Data tab, second part: Quick Fill (pattern inference), From Text/CSV, Scenario Manager,
//! Data Table, Forecast Sheet, Circle Invalid Data and the list-only Queries & Connections.
//!
//! The inference, the text parser and the forecast maths are pure functions on strings and
//! numbers; the `impl SheetModel` block applies the results as one command each.

use std::fmt::Write as _;

use open365_commands::{Command, DataOp, ObjectsOp, RangeRef};
use open365_xlsx::CellValue;

use super::super::{Rect4, SheetModel, a1};

// ---- Quick Fill (Flash Fill) -------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Case {
    Keep,
    Upper,
    Lower,
    Proper,
}

const CASES: [Case; 4] = [Case::Keep, Case::Upper, Case::Lower, Case::Proper];

fn apply_case(s: &str, case: Case) -> String {
    match case {
        Case::Keep => s.to_owned(),
        Case::Upper => s.to_uppercase(),
        Case::Lower => s.to_lowercase(),
        Case::Proper => {
            let mut out = String::new();
            let mut start = true;
            for c in s.chars() {
                if c.is_alphanumeric() {
                    if start {
                        out.extend(c.to_uppercase());
                    } else {
                        out.extend(c.to_lowercase());
                    }
                    start = false;
                } else {
                    out.push(c);
                    start = true;
                }
            }
            out
        }
    }
}

/// One building block of an inferred formula.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece {
    Lit(String),
    /// Word `idx` of source column `src` (negative counts from the end).
    Word {
        src: usize,
        idx: i32,
        case: Case,
    },
    /// First letter of that word.
    Initial {
        src: usize,
        idx: i32,
        case: Case,
    },
    /// The first `n` characters of the whole source text.
    Head {
        src: usize,
        n: usize,
    },
    /// The last `n` characters.
    Tail {
        src: usize,
        n: usize,
    },
    Whole {
        src: usize,
        case: Case,
    },
    /// Every digit of the source, in order.
    Digits {
        src: usize,
    },
}

fn words(s: &str) -> Vec<&str> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect()
}

fn pick(w: &[&str], idx: i32) -> Option<String> {
    let i = if idx >= 0 {
        usize::try_from(idx).ok()?
    } else {
        w.len().checked_sub(usize::try_from(-idx).ok()?)?
    };
    w.get(i).map(|s| (*s).to_owned())
}

impl Piece {
    fn eval(&self, srcs: &[String]) -> Option<String> {
        match self {
            Self::Lit(s) => Some(s.clone()),
            Self::Word { src, idx, case } => {
                let s = srcs.get(*src)?;
                pick(&words(s), *idx).map(|w| apply_case(&w, *case))
            }
            Self::Initial { src, idx, case } => {
                let s = srcs.get(*src)?;
                let w = pick(&words(s), *idx)?;
                w.chars().next().map(|c| apply_case(&c.to_string(), *case))
            }
            Self::Head { src, n } => {
                let s = srcs.get(*src)?;
                (s.chars().count() >= *n).then(|| s.chars().take(*n).collect())
            }
            Self::Tail { src, n } => {
                let s = srcs.get(*src)?;
                let len = s.chars().count();
                (len >= *n).then(|| s.chars().skip(len - n).collect())
            }
            Self::Whole { src, case } => srcs.get(*src).map(|s| apply_case(s, *case)),
            Self::Digits { src } => {
                let d: String = srcs
                    .get(*src)?
                    .chars()
                    .filter(char::is_ascii_digit)
                    .collect();
                (!d.is_empty()).then_some(d)
            }
        }
    }

    /// Lower is preferred when several programs explain the examples.
    fn cost(&self) -> usize {
        match self {
            Self::Lit(s) => 3 * s.chars().count(),
            Self::Whole { .. } | Self::Word { .. } => 1,
            Self::Initial { .. } | Self::Digits { .. } => 2,
            Self::Head { .. } | Self::Tail { .. } => 6,
        }
    }
}

fn candidates(srcs: &[String]) -> Vec<(Piece, String)> {
    let mut out = Vec::new();
    for (src, s) in srcs.iter().enumerate() {
        let w = words(s);
        let n = i32::try_from(w.len()).unwrap_or(0);
        for case in CASES {
            out.push(Piece::Whole { src, case });
        }
        for i in 0..n {
            for idx in [i, i - n] {
                for case in CASES {
                    out.push(Piece::Word { src, idx, case });
                    out.push(Piece::Initial { src, idx, case });
                }
            }
        }
        out.push(Piece::Digits { src });
        for k in 1..=s.chars().count().min(12) {
            out.push(Piece::Head { src, n: k });
            out.push(Piece::Tail { src, n: k });
        }
    }
    let mut seen: Vec<(Piece, String)> = Vec::new();
    for p in out {
        if let Some(v) = p.eval(srcs)
            && !v.is_empty()
            && !seen.iter().any(|(q, _)| *q == p)
        {
            seen.push((p, v));
        }
    }
    // Longer matches first, so the search meets the sensible programs early.
    seen.sort_by_key(|(p, v)| (std::cmp::Reverse(v.chars().count()), p.cost()));
    seen
}

fn explain(
    out: &[char],
    pos: usize,
    cands: &[(Piece, Vec<char>)],
    path: &mut Vec<Piece>,
    found: &mut Vec<Vec<Piece>>,
    budget: &mut usize,
) {
    if *budget == 0 || found.len() >= 400 {
        return;
    }
    *budget -= 1;
    if pos == out.len() {
        found.push(path.clone());
        return;
    }
    for (p, v) in cands {
        if out[pos..].starts_with(v) {
            path.push(p.clone());
            explain(out, pos + v.len(), cands, path, found, budget);
            path.pop();
        }
    }
    // A literal character (merged with a literal before it).
    let c = out[pos];
    let merged = matches!(path.last(), Some(Piece::Lit(_)));
    if let Some(Piece::Lit(s)) = path.last_mut() {
        s.push(c);
    } else {
        path.push(Piece::Lit(c.to_string()));
    }
    explain(out, pos + 1, cands, path, found, budget);
    if merged {
        if let Some(Piece::Lit(s)) = path.last_mut() {
            s.pop();
        }
    } else {
        path.pop();
    }
}

fn run_program(program: &[Piece], srcs: &[String]) -> Option<String> {
    let mut out = String::new();
    for p in program {
        out.push_str(&p.eval(srcs)?);
    }
    Some(out)
}

/// Infers how `examples` (source texts of the neighbouring columns, and the typed result) map
/// from the source to the result, then applies that to `rows`. `None` when no single pattern
/// explains every example.
#[must_use]
pub fn infer_and_apply(
    examples: &[(Vec<String>, String)],
    rows: &[Vec<String>],
) -> Option<Vec<Option<String>>> {
    let (first_src, first_out) = examples.first()?;
    let cands: Vec<(Piece, Vec<char>)> = candidates(first_src)
        .into_iter()
        .map(|(p, v)| (p, v.chars().collect()))
        .collect();
    let out: Vec<char> = first_out.chars().collect();
    let mut found = Vec::new();
    let mut budget = 200_000;
    explain(&out, 0, &cands, &mut Vec::new(), &mut found, &mut budget);
    found.sort_by_key(|p| (p.iter().map(Piece::cost).sum::<usize>(), p.len()));
    let program = found.into_iter().find(|p| {
        examples
            .iter()
            .all(|(s, o)| run_program(p, s).as_deref() == Some(o.as_str()))
    })?;
    // A program with no reference to the source is just a constant; Excel needs a pattern.
    if program.iter().all(|p| matches!(p, Piece::Lit(_))) && examples.len() < 2 {
        return None;
    }
    Some(rows.iter().map(|s| run_program(&program, s)).collect())
}

// ---- text import ------------------------------------------------------------------------

/// What the Text Import Wizard collects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextImport {
    /// Delimiter characters; empty with `fixed` set means fixed width.
    pub delimiters: Vec<char>,
    /// Column break positions (characters from the line start) for fixed width.
    pub fixed: Option<Vec<usize>>,
    pub consecutive: bool,
    pub qualifier: Option<char>,
    /// 1-based row to start importing at.
    pub start_row: usize,
}

impl TextImport {
    /// The wizard's first guess: the delimiter that splits the first lines most evenly.
    #[must_use]
    pub fn detect(text: &str) -> Self {
        let sample: Vec<&str> = text.lines().take(20).collect();
        let mut best = (0usize, ',');
        for d in [',', ';', '\t', '|'] {
            let counts: Vec<usize> = sample.iter().map(|l| l.matches(d).count()).collect();
            let first = counts.first().copied().unwrap_or(0);
            if first == 0 {
                continue;
            }
            let even = counts.iter().filter(|c| **c == first).count();
            let score = even * 1000 + first;
            if score > best.0 {
                best = (score, d);
            }
        }
        Self {
            delimiters: vec![best.1],
            fixed: None,
            consecutive: false,
            qualifier: Some('"'),
            start_row: 1,
        }
    }

    /// The lines as rows of fields.
    #[must_use]
    pub fn parse(&self, text: &str) -> Vec<Vec<String>> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let mut rows: Vec<Vec<String>> = Vec::new();
        if let Some(breaks) = &self.fixed {
            for line in text.lines() {
                let chars: Vec<char> = line.chars().collect();
                let mut row = Vec::new();
                let mut from = 0;
                for b in breaks.iter().copied().chain([chars.len()]) {
                    let b = b.min(chars.len());
                    if b >= from {
                        row.push(
                            chars[from..b]
                                .iter()
                                .collect::<String>()
                                .trim_end()
                                .to_owned(),
                        );
                        from = b;
                    }
                }
                rows.push(row);
            }
        } else {
            let mut row: Vec<String> = Vec::new();
            let mut field = String::new();
            let mut quoted = false;
            let mut was_quoted = false;
            let mut chars = text.chars().peekable();
            let mut last_delim = false;
            while let Some(c) = chars.next() {
                if quoted {
                    if Some(c) == self.qualifier {
                        if chars.peek() == Some(&c) {
                            field.push(c);
                            chars.next();
                        } else {
                            quoted = false;
                        }
                    } else {
                        field.push(c);
                    }
                } else if Some(c) == self.qualifier && field.is_empty() && !was_quoted {
                    quoted = true;
                    was_quoted = true;
                    last_delim = false;
                } else if self.delimiters.contains(&c) {
                    if !(self.consecutive && last_delim) {
                        row.push(std::mem::take(&mut field));
                    }
                    was_quoted = false;
                    last_delim = true;
                } else if c == '\r' || c == '\n' {
                    if c == '\r' && chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    row.push(std::mem::take(&mut field));
                    rows.push(std::mem::take(&mut row));
                    was_quoted = false;
                    last_delim = false;
                } else {
                    field.push(c);
                    last_delim = false;
                }
            }
            if !field.is_empty() || !row.is_empty() || was_quoted {
                row.push(field);
                rows.push(row);
            }
        }
        rows.into_iter()
            .skip(self.start_row.saturating_sub(1))
            .collect()
    }
}

// ---- scenarios --------------------------------------------------------------------------

/// A saved set of values for some changing cells (Scenario Manager).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scenario {
    pub sheet: usize,
    pub name: String,
    pub cells: Vec<(u32, u32)>,
    pub values: Vec<String>,
    pub comment: String,
}

/// State of the part of the Data tab that is not in the file.
#[derive(Debug, Default)]
pub struct MoreState {
    /// Scenarios (kept for the session; the file format has no home for them here).
    pub scenarios: Vec<Scenario>,
    /// Cells circled by "Circle Invalid Data".
    pub circles: Vec<(u32, u32)>,
}

// ---- forecast ---------------------------------------------------------------------------

/// Natural log of the gamma function (Lanczos).
fn ln_gamma(x: f64) -> f64 {
    const G: [f64; 6] = [
        76.180_091_729_471_46,
        -86.505_320_329_416_77,
        24.014_098_240_830_91,
        -1.231_739_572_450_155,
        0.001_208_650_973_866_179,
        -0.000_005_395_239_384_953,
    ];
    let mut y = x;
    let tmp = x + 5.5 - (x + 0.5) * (x + 5.5).ln();
    let mut ser = 1.000_000_000_190_015;
    for g in G {
        y += 1.0;
        ser += g / y;
    }
    -tmp + (2.506_628_274_631_000_5 * ser / x).ln()
}

fn beta_cf(a: f64, b: f64, x: f64) -> f64 {
    let (qab, qap, qam) = (a + b, a + 1.0, a - 1.0);
    let mut c = 1.0;
    let mut d = 1.0 - qab * x / qap;
    if d.abs() < 1e-300 {
        d = 1e-300;
    }
    d = 1.0 / d;
    let mut h = d;
    for m in 1..200 {
        let m = f64::from(m);
        let m2 = 2.0 * m;
        let aa = m * (b - m) * x / ((qam + m2) * (a + m2));
        d = 1.0 + aa * d;
        if d.abs() < 1e-300 {
            d = 1e-300;
        }
        c = 1.0 + aa / c;
        if c.abs() < 1e-300 {
            c = 1e-300;
        }
        d = 1.0 / d;
        h *= d * c;
        let aa = -(a + m) * (qab + m) * x / ((a + m2) * (qap + m2));
        d = 1.0 + aa * d;
        if d.abs() < 1e-300 {
            d = 1e-300;
        }
        c = 1.0 + aa / c;
        if c.abs() < 1e-300 {
            c = 1e-300;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < 1e-14 {
            break;
        }
    }
    h
}

/// Regularised incomplete beta function.
fn inc_beta(a: f64, b: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let bt = (ln_gamma(a + b) - ln_gamma(a) - ln_gamma(b) + a * x.ln() + b * (1.0 - x).ln()).exp();
    if x < (a + 1.0) / (a + b + 2.0) {
        bt * beta_cf(a, b, x) / a
    } else {
        1.0 - bt * beta_cf(b, a, 1.0 - x) / b
    }
}

/// The two-sided critical value of Student's t for `confidence` (0.95) and `df` degrees.
#[must_use]
pub fn t_critical(confidence: f64, df: f64) -> f64 {
    let target = 1.0 - (1.0 - confidence) / 2.0;
    let cdf = |t: f64| {
        let x = df / (df + t * t);
        let tail = 0.5 * inc_beta(df / 2.0, 0.5, x);
        if t >= 0.0 { 1.0 - tail } else { tail }
    };
    let (mut lo, mut hi) = (0.0_f64, 1000.0_f64);
    for _ in 0..200 {
        let mid = (lo + hi) / 2.0;
        if cdf(mid) < target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo + hi) / 2.0
}

/// One forecast row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ForecastPoint {
    pub x: f64,
    pub y: f64,
    pub lower: f64,
    pub upper: f64,
}

/// Linear forecast (the `FORECAST.LINEAR` line) of `periods` steps past the last of `xs`, with
/// the prediction interval at `confidence`. The step is the smallest gap between timeline values.
pub fn forecast_linear(
    xs: &[f64],
    ys: &[f64],
    periods: usize,
    confidence: f64,
) -> Result<Vec<ForecastPoint>, String> {
    let n = xs.len();
    if n < 3 || n != ys.len() {
        return Err("Enter at least three data points for a forecast.".to_owned());
    }
    if xs.windows(2).any(|w| w[1] <= w[0]) {
        return Err("The timeline must be increasing with no repeated values.".to_owned());
    }
    let nf = n as f64;
    let (mx, my) = (xs.iter().sum::<f64>() / nf, ys.iter().sum::<f64>() / nf);
    let sxx: f64 = xs.iter().map(|x| (x - mx).powi(2)).sum();
    let sxy: f64 = xs.iter().zip(ys).map(|(x, y)| (x - mx) * (y - my)).sum();
    let slope = sxy / sxx;
    let intercept = my - slope * mx;
    let sse: f64 = xs
        .iter()
        .zip(ys)
        .map(|(x, y)| (y - (intercept + slope * x)).powi(2))
        .sum();
    let s = (sse / (nf - 2.0)).sqrt();
    let t = t_critical(confidence, nf - 2.0);
    let step = xs
        .windows(2)
        .map(|w| w[1] - w[0])
        .fold(f64::INFINITY, f64::min);
    let last = xs[n - 1];
    Ok((1..=periods)
        .map(|k| {
            let x = last + step * k as f64;
            let y = intercept + slope * x;
            let half = t * s * (1.0 + 1.0 / nf + (x - mx).powi(2) / sxx).sqrt();
            ForecastPoint {
                x,
                y,
                lower: y - half,
                upper: y + half,
            }
        })
        .collect())
}

// ---- the model methods ------------------------------------------------------------------

fn abs_ref(r: u32, c: u32) -> String {
    let a = a1(r, c);
    let split = a.find(|ch: char| ch.is_ascii_digit()).unwrap_or(0);
    format!("${}${}", &a[..split], &a[split..])
}

impl SheetModel {
    fn more_number(&self, r: u32, c: u32) -> Option<f64> {
        match self
            .target
            .xlsx()?
            .cell(self.sheet, r, c)
            .map(|cell| &cell.value)
        {
            Some(CellValue::Number(n)) => Some(*n),
            _ => None,
        }
    }

    // ---- Quick Fill --------------------------------------------------------------------

    fn qf_sources(&self, row: u32, cols: &[u32]) -> Vec<String> {
        cols.iter().map(|c| self.display_text(row, *c)).collect()
    }

    fn display_text(&self, row: u32, col: u32) -> String {
        self.target
            .xlsx()
            .map(|wb| open365_commands::cell_display_text(wb, self.sheet, row, col))
            .unwrap_or_default()
    }

    /// Data > Quick Fill (Ctrl+E): fills the active cell's column from the pattern in the
    /// examples already typed beside the neighbouring data.
    pub fn data_quick_fill(&mut self) -> Result<(), String> {
        let (arow, target) = self.active;
        // Source columns: the columns to the left that hold data next to the active row.
        let has = |this: &Self, r: u32, c: u32| !this.display_text(r, c).is_empty();
        let mut cols: Vec<u32> = Vec::new();
        let mut c = target;
        while c > 1 && cols.len() < 4 {
            c -= 1;
            let near = [arow.saturating_sub(1).max(1), arow, arow + 1];
            if near.iter().any(|r| has(self, *r, c)) {
                cols.insert(0, c);
            } else {
                break;
            }
        }
        if cols.is_empty() {
            let mut c = target + 1;
            while cols.len() < 4
                && [arow.saturating_sub(1).max(1), arow, arow + 1]
                    .iter()
                    .any(|r| has(self, *r, c))
            {
                cols.push(c);
                c += 1;
            }
        }
        if cols.is_empty() {
            return Err("Quick Fill needs data next to the column to fill.".to_owned());
        }
        let any = |this: &Self, r: u32| cols.iter().any(|c| has(this, r, *c));
        let mut top = arow;
        while top > 1 && any(self, top - 1) {
            top -= 1;
        }
        let mut bottom = arow;
        while bottom < open365_xlsx::MAX_ROW && any(self, bottom + 1) {
            bottom += 1;
        }
        let mut examples = Vec::new();
        let mut rows = Vec::new();
        let mut blanks = Vec::new();
        for r in top..=bottom {
            let typed = self.source(r, target);
            if typed.is_empty() {
                blanks.push(r);
                rows.push(self.qf_sources(r, &cols));
            } else {
                examples.push((self.qf_sources(r, &cols), self.display_text(r, target)));
            }
        }
        if examples.is_empty() {
            return Err("Type an example in the column first.".to_owned());
        }
        let Some(filled) = infer_and_apply(&examples, &rows) else {
            self.status = "Quick Fill could not recognise a pattern.".to_owned();
            return Err(self.status.clone());
        };
        let mut lines: Vec<Vec<String>> = Vec::new();
        let mut k = 0;
        for r in top..=bottom {
            let typed = self.source(r, target);
            if typed.is_empty() {
                lines.push(vec![filled[k].clone().unwrap_or_default()]);
                k += 1;
            } else {
                lines.push(vec![typed]);
            }
        }
        self.apply(Command::XlsxPaste {
            sheet: self.sheet,
            row: top,
            col: target,
            rows: lines,
        })?;
        self.status = format!("Quick Fill filled {} cells.", blanks.len());
        Ok(())
    }

    // ---- From Text/CSV -----------------------------------------------------------------

    /// Data > From Text/CSV: puts `text` parsed with `opts` at the active cell (one undo
    /// step). Returns the rows and columns imported.
    pub fn data_import_text(
        &mut self,
        text: &str,
        opts: &TextImport,
        at: Option<(u32, u32)>,
    ) -> Result<(u32, u32), String> {
        let rows = opts.parse(text);
        if rows.is_empty() {
            return Err("The file has no data to import.".to_owned());
        }
        let width = rows.iter().map(Vec::len).max().unwrap_or(0);
        let (row, col) = at.unwrap_or(self.active);
        if row as usize + rows.len() > open365_xlsx::MAX_ROW as usize
            || col as usize + width > open365_xlsx::MAX_COL as usize + 1
        {
            return Err("The data does not fit on the sheet.".to_owned());
        }
        let count = (rows.len() as u32, width as u32);
        self.apply(Command::XlsxPaste {
            sheet: self.sheet,
            row,
            col,
            rows,
        })?;
        self.select_range_cells((row, col), (row + count.0 - 1, col + count.1.max(1) - 1));
        Ok(count)
    }

    fn select_range_cells(&mut self, a: (u32, u32), b: (u32, u32)) {
        self.select_cell(a);
        self.extend_to(b);
    }

    // ---- Queries & Connections ---------------------------------------------------------

    /// Queries & Connections, Existing Connections, Workbook Links: the workbook has none
    /// (the lists are empty); the result is the message the pane shows.
    pub fn data_connections(&mut self) -> Vec<String> {
        self.status = "There are no queries or connections in this workbook.".to_owned();
        Vec::new()
    }

    /// Refresh All: with no external data, there is nothing to refresh.
    pub fn data_refresh_all(&mut self) {
        self.status = "There are no connections to refresh.".to_owned();
    }

    // ---- Data Table --------------------------------------------------------------------

    /// What-If Analysis > Data Table over the selection (see `data_table` in the commands).
    pub fn data_table(
        &mut self,
        row_input: Option<(u32, u32)>,
        col_input: Option<(u32, u32)>,
    ) -> Result<(), String> {
        let r: Rect4 = self.range();
        let range = RangeRef {
            first_row: r.0,
            first_col: r.1,
            last_row: r.2,
            last_col: r.3,
        };
        self.apply(Command::DataOp {
            sheet: self.sheet,
            op: DataOp::DataTable {
                range,
                row_input,
                col_input,
            },
        })
    }

    // ---- Scenario Manager --------------------------------------------------------------

    /// Scenarios of the active sheet.
    #[must_use]
    pub fn data_scenarios(&self) -> Vec<&Scenario> {
        self.ext_data
            .more
            .scenarios
            .iter()
            .filter(|s| s.sheet == self.sheet)
            .collect()
    }

    /// Add (or, for an existing name, replace) a scenario. `values` default to the cells'
    /// current contents.
    pub fn data_scenario_add(
        &mut self,
        name: &str,
        cells: Vec<(u32, u32)>,
        values: Option<Vec<String>>,
        comment: &str,
    ) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("Enter a name for the scenario.".to_owned());
        }
        if cells.is_empty() {
            return Err("Enter at least one changing cell.".to_owned());
        }
        if cells.len() > 32 {
            return Err("A scenario can change at most 32 cells.".to_owned());
        }
        let values =
            values.unwrap_or_else(|| cells.iter().map(|c| self.source(c.0, c.1)).collect());
        if values.len() != cells.len() {
            return Err("Give one value for each changing cell.".to_owned());
        }
        let sheet = self.sheet;
        let list = &mut self.ext_data.more.scenarios;
        list.retain(|s| !(s.sheet == sheet && s.name.eq_ignore_ascii_case(name)));
        list.push(Scenario {
            sheet,
            name: name.to_owned(),
            cells,
            values,
            comment: comment.to_owned(),
        });
        Ok(())
    }

    /// Scenario Manager > Show: puts the scenario's values in its cells (one undo step).
    pub fn data_scenario_show(&mut self, name: &str) -> Result<(), String> {
        let sc = self
            .data_scenarios()
            .into_iter()
            .find(|s| s.name.eq_ignore_ascii_case(name.trim()))
            .cloned()
            .ok_or_else(|| format!("There is no scenario called '{}'.", name.trim()))?;
        let top = sc.cells.iter().map(|c| c.0).min().unwrap_or(1);
        let bottom = sc.cells.iter().map(|c| c.0).max().unwrap_or(1);
        let left = sc.cells.iter().map(|c| c.1).min().unwrap_or(1);
        let right = sc.cells.iter().map(|c| c.1).max().unwrap_or(1);
        let mut rows = Vec::new();
        for r in top..=bottom {
            let mut line = Vec::new();
            for c in left..=right {
                let text = match sc.cells.iter().position(|x| *x == (r, c)) {
                    Some(i) => sc.values[i].clone(),
                    None => self.source(r, c),
                };
                line.push(text);
            }
            rows.push(line);
        }
        self.apply(Command::XlsxPaste {
            sheet: self.sheet,
            row: top,
            col: left,
            rows,
        })
    }

    pub fn data_scenario_delete(&mut self, name: &str) -> Result<(), String> {
        let sheet = self.sheet;
        let list = &mut self.ext_data.more.scenarios;
        let before = list.len();
        list.retain(|s| !(s.sheet == sheet && s.name.eq_ignore_ascii_case(name.trim())));
        if list.len() == before {
            return Err(format!("There is no scenario called '{}'.", name.trim()));
        }
        Ok(())
    }

    /// Scenario Manager > Summary: a "Scenario Summary" sheet with each scenario's changing
    /// cells and the result cells' values under it.
    pub fn data_scenario_summary(&mut self, results: &[(u32, u32)]) -> Result<(), String> {
        let list: Vec<Scenario> = self.data_scenarios().into_iter().cloned().collect();
        if list.is_empty() {
            return Err("There are no scenarios to summarise.".to_owned());
        }
        let mut changing: Vec<(u32, u32)> = Vec::new();
        for s in &list {
            for c in &s.cells {
                if !changing.contains(c) {
                    changing.push(*c);
                }
            }
        }
        let mut grid: Vec<Vec<String>> = vec![
            vec![String::new(), "Scenario Summary".to_owned()],
            {
                let mut h = vec![String::new(), String::new(), "Current Values:".to_owned()];
                h.extend(list.iter().map(|s| s.name.clone()));
                h
            },
            vec![String::new(), "Changing Cells:".to_owned()],
        ];
        for c in &changing {
            let mut line = vec![String::new(), abs_ref(c.0, c.1), self.source(c.0, c.1)];
            for s in &list {
                line.push(
                    s.cells
                        .iter()
                        .position(|x| x == c)
                        .map_or_else(|| self.source(c.0, c.1), |i| s.values[i].clone()),
                );
            }
            grid.push(line);
        }
        if !results.is_empty() {
            grid.push(vec![String::new(), "Result Cells:".to_owned()]);
            for c in results {
                let mut line = vec![String::new(), abs_ref(c.0, c.1), self.source(c.0, c.1)];
                line.extend(list.iter().map(|_| String::new()));
                grid.push(line);
            }
        }
        self.new_sheet_with(grid)
    }

    /// Appends a sheet, writes `grid` at A1 and makes it the active sheet (two undo steps are
    /// folded back into nothing on failure).
    fn new_sheet_with(&mut self, grid: Vec<Vec<String>>) -> Result<(), String> {
        let before = self.sheet;
        self.apply(Command::SheetObjects {
            sheet: before,
            op: ObjectsOp::AddSheet {
                at: None,
                name: None,
            },
        })?;
        let new = self
            .target
            .xlsx()
            .map_or(before, |wb| wb.sheets().len().saturating_sub(1));
        if let Err(e) = self.apply(Command::XlsxPaste {
            sheet: new,
            row: 1,
            col: 1,
            rows: grid,
        }) {
            self.undo();
            self.sheet = before;
            return Err(e);
        }
        self.sheet = new;
        self.select_cell((1, 1));
        Ok(())
    }

    // ---- Forecast Sheet ----------------------------------------------------------------

    /// Data > Forecast Sheet over the selection (timeline column, then values column): a new
    /// sheet with the history, the linear forecast and its confidence bounds.
    pub fn data_forecast_sheet(&mut self, periods: usize, confidence: f64) -> Result<(), String> {
        let (r1, c1, r2, c2) = self.data_target();
        if c2 <= c1 {
            return Err("Select a timeline column and a values column.".to_owned());
        }
        let (tc, vc) = (c1, c1 + 1);
        let mut start = r1;
        if self.more_number(r1, tc).is_none() {
            start += 1;
        }
        let (mut xs, mut ys) = (Vec::new(), Vec::new());
        for r in start..=r2 {
            match (self.more_number(r, tc), self.more_number(r, vc)) {
                (Some(x), Some(y)) => {
                    xs.push(x);
                    ys.push(y);
                }
                _ => return Err("The timeline and the values must be numbers or dates.".to_owned()),
            }
        }
        if !(0.0..1.0).contains(&confidence) || confidence == 0.0 {
            return Err("The confidence interval must be between 0 and 100%.".to_owned());
        }
        let points = forecast_linear(&xs, &ys, periods.max(1), confidence)?;
        let mut grid = vec![vec![
            "Timeline".to_owned(),
            "Values".to_owned(),
            "Forecast".to_owned(),
            "Lower Confidence Bound".to_owned(),
            "Upper Confidence Bound".to_owned(),
        ]];
        let num = |v: f64| {
            let mut s = String::new();
            let _ = write!(s, "{v}");
            s
        };
        let n = xs.len();
        for (i, (x, y)) in xs.iter().zip(&ys).enumerate() {
            let mut line = vec![
                num(*x),
                num(*y),
                String::new(),
                String::new(),
                String::new(),
            ];
            if i == n - 1 {
                line[2] = num(*y);
                line[3] = num(*y);
                line[4] = num(*y);
            }
            grid.push(line);
        }
        for p in &points {
            grid.push(vec![
                num(p.x),
                String::new(),
                num(p.y),
                num(p.lower),
                num(p.upper),
            ]);
        }
        self.new_sheet_with(grid)
    }

    // ---- Circle Invalid Data -----------------------------------------------------------

    /// Circle Invalid Data: the cells whose value breaks their validation rule. Not saved.
    pub fn data_circle_invalid(&mut self) -> usize {
        let Some((last_row, last_col)) = self.used_end() else {
            self.ext_data.more.circles.clear();
            return 0;
        };
        let mut bad = Vec::new();
        for r in 1..=last_row {
            for c in 1..=last_col {
                if self.data_validation_at(r, c).is_none() {
                    continue;
                }
                let text = self.display_text(r, c);
                if !text.is_empty() && self.data_check_entry(r, c, &text).is_err() {
                    bad.push((r, c));
                }
            }
        }
        let n = bad.len();
        self.ext_data.more.circles = bad;
        self.status = if n == 0 {
            "No invalid data found.".to_owned()
        } else {
            format!("{n} invalid cells circled.")
        };
        n
    }

    pub fn data_clear_circles(&mut self) {
        self.ext_data.more.circles.clear();
    }

    /// The cells to draw a red circle around.
    #[must_use]
    pub fn data_circles(&self) -> &[(u32, u32)] {
        &self.ext_data.more.circles
    }
}
