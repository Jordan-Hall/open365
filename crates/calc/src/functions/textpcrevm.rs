//! The backtracking matcher of the PCRE2-compatible engine (see `textpcre.rs` for the syntax).
//!
//! The program runs on an explicit backtrack stack, so long subjects and deep repetitions cannot overflow the
//! native stack. Like PCRE2 it gives up (`Abort`, which Excel shows as `#VALUE!`) when a match needs too many
//! backtracking steps or too much backtrack memory; the limits are calibrated against Excel:
//! `^(a+)+$` on 15 `a` plus `!` finishes, on 20 it aborts, and `^(a|b)*$` on 10,000 characters matches while
//! 100,000 aborts.
//!
//! Differences from PCRE2 are recorded in `docs/testing/calc-regex.md`: recursion and atomic groups are
//! atomic, and the backtracking-control verbs `(*COMMIT)`, `(*PRUNE)`, `(*SKIP)` and `(*THEN)` do nothing.

use super::textpcre::{AssertKind, Class, ClassItem, INF, Inst, Regex, RepKind, Single};
use super::textpcreprops::{case_variants, extends_grapheme, fold, is_word};

/// The match or the backtracking limit was exceeded.
#[derive(Debug, Clone, Copy)]
pub(super) struct Abort;

const STEP_LIMIT: u64 = 600_000;
const STACK_LIMIT: usize = 400_000;
const RECURSION_LIMIT: u32 = 250;
const UNSET: usize = usize::MAX;

/// The capture groups of a match as character offsets; group 0 is the whole match.
pub(super) type Caps = Vec<Option<(usize, usize)>>;

#[derive(Debug, Clone, Copy)]
enum Bt {
    /// Resume at `pc` with `pos`.
    Alt { pc: usize, pos: usize },
    /// Restore a register.
    Reg { idx: usize, old: usize },
    /// A greedy single-character loop gives back one character at a time down to `lo`.
    Giveback { next: usize, lo: usize, hi: usize },
    /// A lazy single-character loop takes one more character.
    Lazy { pc: usize, pos: usize, count: u32 },
}

fn class_hit(c: &Class, ch: char) -> bool {
    let range = |x: char| {
        c.items
            .iter()
            .any(|it| matches!(it, ClassItem::Range(lo, hi) if *lo <= x && x <= *hi))
    };
    let prop = |x: char| {
        c.items
            .iter()
            .any(|it| matches!(it, ClassItem::Prop(p, neg) if p.matches(x) != *neg))
    };
    let hit = if c.caseless {
        case_variants(ch).iter().any(|&x| range(x)) || prop(ch)
    } else {
        range(ch) || prop(ch)
    };
    hit != c.negated
}

fn is_newline(c: char) -> bool {
    c == '\n' || c == '\r'
}

struct Vm<'a> {
    re: &'a Regex,
    hay: &'a [char],
    regs: Vec<usize>,
    stack: Vec<Bt>,
    steps: u64,
    search_start: usize,
    notempty: bool,
    depth: u32,
}

#[cfg(test)]
thread_local! {
    /// Backtracking steps of the last `exec` on this thread, to calibrate `STEP_LIMIT`.
    pub(super) static LAST_STEPS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
impl Drop for Vm<'_> {
    fn drop(&mut self) {
        LAST_STEPS.with(|c| c.set(self.steps));
    }
}

impl Vm<'_> {
    fn single_ok(&self, s: &Single, ch: char) -> bool {
        match s {
            Single::Char(c) => ch == *c,
            Single::CharCi(c) => fold(ch) == *c,
            Single::Any => true,
            Single::AnyNl | Single::NotNl => !is_newline(ch),
            Single::Class(i) => class_hit(&self.re.classes[*i], ch),
        }
    }

    fn assert_ok(&self, kind: AssertKind, pos: usize) -> bool {
        let h = self.hay;
        let len = h.len();
        let before = if pos > 0 { Some(h[pos - 1]) } else { None };
        let after = h.get(pos).copied();
        // A position between CR and LF is inside a line break.
        let split_crlf = before == Some('\r') && after == Some('\n');
        match kind {
            AssertKind::Bol | AssertKind::StrStart => pos == 0,
            AssertKind::BolMulti => {
                pos == 0 || (pos < len && before.is_some_and(is_newline) && !split_crlf)
            }
            AssertKind::StrEnd => pos == len,
            AssertKind::Eol => {
                pos == len
                    || (after.is_some_and(is_newline) && !split_crlf && {
                        // Only a final line break: LF, CR or CRLF.
                        pos + 1 == len
                            || (after == Some('\r')
                                && h.get(pos + 1) == Some(&'\n')
                                && pos + 2 == len)
                    })
            }
            AssertKind::EolMulti => pos == len || (after.is_some_and(is_newline) && !split_crlf),
            AssertKind::WordB | AssertKind::NotWordB => {
                let a = before.is_some_and(is_word);
                let b = after.is_some_and(is_word);
                (a != b) == (kind == AssertKind::WordB)
            }
            AssertKind::SearchStart => pos == self.search_start,
        }
    }

    fn push(&mut self, bt: Bt) -> Result<(), Abort> {
        if self.stack.len() >= STACK_LIMIT {
            return Err(Abort);
        }
        self.stack.push(bt);
        Ok(())
    }

    fn set_reg(&mut self, idx: usize, value: usize) -> Result<(), Abort> {
        let old = self.regs[idx];
        self.push(Bt::Reg { idx, old })?;
        self.regs[idx] = value;
        Ok(())
    }

    /// Drops the backtrack points above `base` but keeps the register undo records, so that a later failure
    /// outside still restores the captures (atomic groups and positive lookaround).
    fn commit(&mut self, base: usize) {
        let mut w = base;
        for r in base..self.stack.len() {
            if matches!(self.stack[r], Bt::Reg { .. }) {
                self.stack.swap(w, r);
                w += 1;
            }
        }
        self.stack.truncate(w);
    }

    /// Undoes everything above `base`.
    fn unwind(&mut self, base: usize) {
        while self.stack.len() > base {
            if let Some(Bt::Reg { idx, old }) = self.stack.pop() {
                self.regs[idx] = old;
            }
        }
    }

    fn step(&mut self) -> Result<(), Abort> {
        self.steps += 1;
        if self.steps > STEP_LIMIT {
            Err(Abort)
        } else {
            Ok(())
        }
    }

    /// Runs from `pc` at `pos` until the sub-program ends. `base` is the bottom of this run's backtrack stack.
    /// `must_end` is where a lookbehind has to end; `rec` is the group whose end finishes a recursive call.
    /// `top` marks the run of the whole pattern.
    fn run(
        &mut self,
        mut pc: usize,
        mut pos: usize,
        base: usize,
        must_end: Option<usize>,
        rec: Option<usize>,
        top: bool,
    ) -> Result<Option<usize>, Abort> {
        let re = self.re;
        let hay = self.hay;
        let len = hay.len();
        loop {
            let mut ok = true;
            match &re.prog[pc] {
                Inst::One(s) => {
                    if pos < len && self.single_ok(s, hay[pos]) {
                        pos += 1;
                        pc += 1;
                    } else {
                        ok = false;
                    }
                }
                Inst::Newline => match hay.get(pos) {
                    Some('\r') if hay.get(pos + 1) == Some(&'\n') => {
                        pos += 2;
                        pc += 1;
                    }
                    Some('\n' | '\u{b}' | '\u{c}' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}') => {
                        pos += 1;
                        pc += 1;
                    }
                    _ => ok = false,
                },
                Inst::Grapheme => match hay.get(pos) {
                    None => ok = false,
                    Some('\r') if hay.get(pos + 1) == Some(&'\n') => {
                        pos += 2;
                        pc += 1;
                    }
                    Some(&first) => {
                        pos += 1;
                        let mut prev = first;
                        while let Some(&c) = hay.get(pos) {
                            if extends_grapheme(c) || prev == '\u{200d}' {
                                prev = c;
                                pos += 1;
                            } else {
                                break;
                            }
                        }
                        pc += 1;
                    }
                },
                Inst::Assert(k) => {
                    if self.assert_ok(*k, pos) {
                        pc += 1;
                    } else {
                        ok = false;
                    }
                }
                Inst::Split(a, b) => {
                    self.push(Bt::Alt { pc: *b, pos })?;
                    pc = *a;
                }
                Inst::Jmp(t) => pc = *t,
                Inst::Save(i) => {
                    self.set_reg(*i, pos)?;
                    pc += 1;
                }
                Inst::SetReg(i, v) => {
                    self.set_reg(*i, *v)?;
                    pc += 1;
                }
                Inst::Rep {
                    single,
                    min,
                    max,
                    kind,
                } => {
                    let min = *min as usize;
                    let limit = if *max == INF {
                        usize::MAX
                    } else {
                        *max as usize
                    };
                    match kind {
                        RepKind::Greedy | RepKind::Possessive => {
                            let mut n = 0;
                            while n < limit && pos + n < len && self.single_ok(single, hay[pos + n])
                            {
                                n += 1;
                            }
                            if n < min {
                                ok = false;
                            } else {
                                if *kind == RepKind::Greedy && n > min {
                                    self.push(Bt::Giveback {
                                        next: pc + 1,
                                        lo: pos + min,
                                        hi: pos + n,
                                    })?;
                                }
                                pos += n;
                                pc += 1;
                            }
                        }
                        RepKind::Lazy => {
                            let mut n = 0;
                            while n < min && pos + n < len && self.single_ok(single, hay[pos + n]) {
                                n += 1;
                            }
                            if n < min {
                                ok = false;
                            } else {
                                if limit > min {
                                    self.push(Bt::Lazy {
                                        pc,
                                        pos: pos + min,
                                        count: min as u32,
                                    })?;
                                }
                                pos += min;
                                pc += 1;
                            }
                        }
                    }
                }
                Inst::LoopHead {
                    counter,
                    min,
                    max,
                    greedy,
                    exit,
                } => {
                    let count = self.regs[*counter] as u64;
                    if count < u64::from(*min) {
                        pc += 1;
                    } else if *max != INF && count >= u64::from(*max) {
                        pc = *exit;
                    } else if *greedy {
                        self.push(Bt::Alt { pc: *exit, pos })?;
                        pc += 1;
                    } else {
                        self.push(Bt::Alt { pc: pc + 1, pos })?;
                        pc = *exit;
                    }
                }
                Inst::LoopBody { counter, mark } => {
                    self.set_reg(*mark, pos)?;
                    let n = self.regs[*counter] + 1;
                    self.set_reg(*counter, n)?;
                    pc += 1;
                }
                Inst::LoopEnd {
                    counter,
                    mark,
                    min,
                    head,
                } => {
                    if pos == self.regs[*mark] && self.regs[*counter] >= *min as usize {
                        pc += 1;
                    } else {
                        pc = *head;
                    }
                }
                Inst::Atomic { end } => {
                    let inner = self.stack.len();
                    match self.run(pc + 1, pos, inner, None, None, false)? {
                        Some(q) => {
                            self.commit(inner);
                            pos = q;
                            pc = *end;
                        }
                        None => ok = false,
                    }
                }
                Inst::Look {
                    ahead,
                    neg,
                    min,
                    max,
                    end,
                    else_pc,
                } => {
                    let inner = self.stack.len();
                    let found = if *ahead {
                        self.run(pc + 1, pos, inner, None, None, false)?.is_some()
                    } else {
                        let mut hit = false;
                        for width in *min as usize..=*max as usize {
                            if width > pos {
                                break;
                            }
                            if self
                                .run(pc + 1, pos - width, inner, Some(pos), None, false)?
                                .is_some()
                            {
                                hit = true;
                                break;
                            }
                        }
                        hit
                    };
                    if found != *neg {
                        if found {
                            self.commit(inner);
                        }
                        pc = *end;
                    } else {
                        if found {
                            self.unwind(inner);
                        }
                        match else_pc {
                            Some(e) => pc = *e,
                            None => ok = false,
                        }
                    }
                }
                Inst::SubEnd => {
                    if must_end.is_none_or(|e| e == pos) {
                        return Ok(Some(pos));
                    }
                    ok = false;
                }
                Inst::BackRef(groups, ci) => {
                    let set = groups
                        .iter()
                        .find(|&&g| self.regs[2 * g] != UNSET && self.regs[2 * g + 1] != UNSET);
                    match set {
                        None => ok = false,
                        Some(&g) => {
                            let (s, e) = (self.regs[2 * g], self.regs[2 * g + 1]);
                            let n = e.saturating_sub(s);
                            if pos + n <= len
                                && (0..n).all(|k| {
                                    let (a, b) = (hay[s + k], hay[pos + k]);
                                    a == b || (*ci && fold(a) == fold(b))
                                })
                            {
                                pos += n;
                                pc += 1;
                            } else {
                                ok = false;
                            }
                        }
                    }
                }
                Inst::Call(g) => {
                    self.depth += 1;
                    if self.depth > RECURSION_LIMIT {
                        return Err(Abort);
                    }
                    let saved = self.regs.clone();
                    let inner = self.stack.len();
                    let r = self.run(re.group_pc[*g], pos, inner, None, Some(*g), false)?;
                    self.depth -= 1;
                    match r {
                        Some(q) => {
                            self.stack.truncate(inner);
                            self.regs = saved;
                            pos = q;
                            pc += 1;
                        }
                        None => ok = false,
                    }
                }
                Inst::GroupEnd(i) => {
                    if rec == Some(*i) {
                        return Ok(Some(pos));
                    }
                    pc += 1;
                }
                Inst::CondGroup { groups, else_pc } => {
                    if groups.iter().any(|&g| self.regs[2 * g + 1] != UNSET) {
                        pc += 1;
                    } else {
                        pc = *else_pc;
                    }
                }
                Inst::Fail => ok = false,
                Inst::Accept => return Ok(Some(pos)),
                Inst::Match => {
                    if rec == Some(0) {
                        return Ok(Some(pos));
                    }
                    if top && self.notempty && self.regs[0] == pos && pos == self.search_start {
                        ok = false;
                    } else {
                        return Ok(Some(pos));
                    }
                }
            }
            if ok {
                continue;
            }
            // Backtrack.
            loop {
                if self.stack.len() <= base {
                    return Ok(None);
                }
                match self.stack.pop() {
                    None => return Ok(None),
                    Some(Bt::Reg { idx, old }) => self.regs[idx] = old,
                    Some(Bt::Alt { pc: p, pos: q }) => {
                        self.step()?;
                        pc = p;
                        pos = q;
                        break;
                    }
                    Some(Bt::Giveback { next, lo, hi }) => {
                        self.step()?;
                        let hi = hi - 1;
                        if hi > lo {
                            self.push(Bt::Giveback { next, lo, hi })?;
                        }
                        pc = next;
                        pos = hi;
                        break;
                    }
                    Some(Bt::Lazy {
                        pc: ip,
                        pos: q,
                        count,
                    }) => {
                        self.step()?;
                        if let Inst::Rep { single, max, .. } = &re.prog[ip]
                            && count < *max
                            && q < len
                            && self.single_ok(single, hay[q])
                        {
                            if count + 1 < *max {
                                self.push(Bt::Lazy {
                                    pc: ip,
                                    pos: q + 1,
                                    count: count + 1,
                                })?;
                            }
                            pc = ip + 1;
                            pos = q + 1;
                            break;
                        }
                    }
                }
            }
        }
    }
}

impl Regex {
    /// The first match at or after `from` (only at `from` when `anchored`). With `notempty` an empty match at
    /// `from` does not count (PCRE2's `NOTEMPTY_ATSTART`).
    pub(super) fn exec(
        &self,
        hay: &[char],
        from: usize,
        anchored: bool,
        notempty: bool,
    ) -> Result<Option<Caps>, Abort> {
        let mut vm = Vm {
            re: self,
            hay,
            regs: vec![UNSET; self.nregs],
            stack: Vec::new(),
            steps: 0,
            search_start: from,
            notempty,
            depth: 0,
        };
        if self
            .required
            .iter()
            .any(|c| !hay[from.min(hay.len())..].contains(c))
        {
            return Ok(None);
        }
        let mut start = from;
        loop {
            if start > hay.len() {
                return Ok(None);
            }
            if let (Some(fc), false) = (self.first_char, anchored) {
                while start < hay.len() && hay[start] != fc {
                    start += 1;
                }
                if start >= hay.len() {
                    return Ok(None);
                }
            }
            vm.regs.fill(UNSET);
            vm.stack.clear();
            vm.depth = 0;
            if let Some(end) = vm.run(0, start, 0, None, None, true)? {
                let mut caps: Caps = Vec::with_capacity(self.ngroups + 1);
                let s0 = vm.regs[0].min(end);
                caps.push(Some((s0, end)));
                for g in 1..=self.ngroups {
                    let (s, e) = (vm.regs[2 * g], vm.regs[2 * g + 1]);
                    caps.push((s != UNSET && e != UNSET).then_some((s, e)));
                }
                return Ok(Some(caps));
            }
            if anchored {
                return Ok(None);
            }
            start += 1;
        }
    }

    /// All matches the way `pcre2_substitute` finds them: after an empty match the next try at the same place
    /// must be non-empty, and when that fails the search moves on one character (a CR LF pair as one).
    pub(super) fn all_matches(&self, hay: &[char], limit: usize) -> Result<Vec<Caps>, Abort> {
        let mut out = Vec::new();
        let mut start = 0;
        let mut retry = false;
        while start <= hay.len() && out.len() < limit {
            match self.exec(hay, start, retry, retry)? {
                Some(caps) => {
                    let (s, e) = caps[0].unwrap_or((start, start));
                    out.push(caps);
                    start = e.max(start);
                    retry = s == e;
                }
                None => {
                    if !retry {
                        break;
                    }
                    retry = false;
                    if start >= hay.len() {
                        break;
                    }
                    start += if hay[start] == '\r' && hay.get(start + 1) == Some(&'\n') {
                        2
                    } else {
                        1
                    };
                }
            }
        }
        Ok(out)
    }
}
