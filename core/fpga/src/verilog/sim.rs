//! Elaboration (names -> slots, parameters -> constants, comb processes
//! ordered) and two-phase cycle simulation.
//!
//! ```text
//!  drive inputs -> settle comb (topological order; iterate if cyclic)
//!  posedge:  clocked blocks run; `<=` queued, `=` immediate
//!            queue applied in order (a later `<=` wins), settle again
//! ```
//!
//! Values are at most 64 bits. An expression is as wide as its widest
//! operand (no context-determined widening), and signed only through
//! `$signed`, `signed` declarations or unsized decimals.

use std::collections::HashMap;

use super::{
    VerilogError,
    parse::{BinOp, Decl, Dir, Expr, Item, LValue, Module, Stmt, UnOp},
};

type R<T> = Result<T, VerilogError>;

fn err<T>(msg: impl Into<String>) -> R<T> {
    Err(VerilogError(msg.into()))
}

const fn mask(w: u32) -> u64 {
    if w >= 64 { u64::MAX } else { (1 << w) - 1 }
}

const fn sext(v: u64, w: u32) -> i64 {
    if w >= 64 || w == 0 {
        v as i64
    } else {
        let s = 64 - w;
        ((v << s) as i64) >> s
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Val {
    pub v: u64,
    pub w: u32,
    pub s: bool,
}

impl Val {
    const fn new(v: u64, w: u32, s: bool) -> Self {
        Self { v: v & mask(w), w, s }
    }

    /// The value extended to 64 bits (sign-extended when signed).
    const fn ext(self) -> u64 {
        if self.s { sext(self.v, self.w) as u64 } else { self.v }
    }
}

/// A Verilog literal: `32'hFF`, `'b1`, `12`.
fn number(text: &str) -> R<Val> {
    let t = text.replace('_', "");
    let Some(q) = t.find('\'') else {
        return t
            .parse::<u64>()
            .map(|v| Val::new(v, 32, true))
            .map_err(|_| VerilogError(format!("bad number `{text}`")));
    };
    let w = if q == 0 {
        32
    } else {
        t[..q].parse::<u32>().map_err(|_| VerilogError(format!("bad width in `{text}`")))?
    };
    if w == 0 || w > 64 {
        return err(format!("`{text}`: widths above 64 bits are not supported"));
    }
    let mut rest = &t[q + 1..];
    let signed = rest.starts_with(['s', 'S']);
    if signed {
        rest = &rest[1..];
    }
    let radix = match rest.chars().next() {
        Some('h' | 'H') => 16,
        Some('d' | 'D') => 10,
        Some('o' | 'O') => 8,
        Some('b' | 'B') => 2,
        _ => return err(format!("bad base in `{text}`")),
    };
    // x / z / ? digits read as 0.
    let digits: String = rest[1..]
        .chars()
        .map(|c| if matches!(c, 'x' | 'X' | 'z' | 'Z' | '?') { '0' } else { c })
        .collect();
    let v = u64::from_str_radix(&digits, radix)
        .map_err(|_| VerilogError(format!("bad digits in `{text}`")))?;
    Ok(Val::new(v, w, signed))
}

#[derive(Debug, Clone)]
pub struct Sig {
    pub name: String,
    pub width: u32,
    lsb: i64,
    signed: bool,
}

#[derive(Debug, Clone)]
pub struct Mem {
    pub name: String,
    pub width: u32,
    lo: i64,
    pub words: usize,
}

#[derive(Debug, Clone)]
enum E {
    Const(Val),
    Sig(usize),
    Bit(usize, Box<Self>),
    /// (signal, shift, width)
    Part(usize, u32, u32),
    IPart(usize, Box<Self>, u32, bool),
    MemRd(usize, Box<Self>),
    Un(UnOp, Box<Self>),
    Bin(BinOp, Box<Self>, Box<Self>),
    Tern(Box<Self>, Box<Self>, Box<Self>),
    Concat(Vec<Self>),
    Repl(u32, Vec<Self>),
    Sign(Box<Self>, bool),
}

#[derive(Debug, Clone)]
enum L {
    Sig(usize),
    Bit(usize, E),
    Part(usize, u32, u32),
    Mem(usize, E),
}

#[derive(Debug, Clone)]
enum S {
    Block(Vec<Self>),
    If(E, Box<Self>, Option<Box<Self>>),
    Case(E, Vec<(Vec<E>, Self)>, Option<Box<Self>>),
    Assign(L, E, bool),
}

/// A resolved write, applied immediately or at the end of the edge.
enum Write {
    Sig { id: usize, shift: u32, width: u32, v: u64 },
    Mem { id: usize, idx: usize, v: u64 },
}

/// Slots of the bus pins.
#[derive(Debug, Clone, Copy)]
pub struct Pins {
    pub rst: usize,
    pub sel: usize,
    pub we: usize,
    pub addr: usize,
    pub be: usize,
    pub wdata: usize,
    pub rdata: usize,
    pub irq: usize,
}

#[derive(Debug, Clone)]
pub struct State {
    pub vals: Vec<u64>,
    pub mems: Vec<Vec<u64>>,
}

#[derive(Debug, Clone)]
pub struct Design {
    pub name: String,
    pub sigs: Vec<Sig>,
    pub mems: Vec<Mem>,
    comb: Vec<S>,
    /// Whether `comb` is in dependency order (else settle iterates).
    ordered: bool,
    ff: Vec<S>,
    init: Vec<S>,
    pub pins: Pins,
    /// Parameters, for register-map inference.
    pub params: Vec<(String, u64)>,
}

struct Scope {
    sigs: HashMap<String, usize>,
    mems: HashMap<String, usize>,
    params: HashMap<String, Val>,
}

impl Design {
    pub fn elaborate(m: &Module) -> R<Self> {
        let mut scope =
            Scope { sigs: HashMap::new(), mems: HashMap::new(), params: HashMap::new() };
        let mut d = Self {
            name: m.name.clone(),
            sigs: Vec::new(),
            mems: Vec::new(),
            comb: Vec::new(),
            ordered: true,
            ff: Vec::new(),
            init: Vec::new(),
            pins: Pins { rst: 0, sel: 0, we: 0, addr: 0, be: 0, wdata: 0, rdata: 0, irq: 0 },
            params: Vec::new(),
        };
        let mut dirs = HashMap::new();
        for item in &m.items {
            match item {
                Item::Param(name, e) => {
                    let v = d.constant(&scope, e)?;
                    scope.params.insert(name.clone(), v);
                    d.params.push((name.clone(), v.v));
                }
                Item::Decl(decl) => d.declare(&mut scope, decl, &mut dirs)?,
                _ => {}
            }
        }
        for item in &m.items {
            match item {
                Item::Assign(lv, e) => {
                    let s = S::Assign(d.lvalue(&scope, lv)?, d.expr(&scope, e)?, false);
                    d.comb.push(s);
                }
                Item::Always(true, st) => {
                    let s = d.stmt(&scope, st)?;
                    d.ff.push(s);
                }
                Item::Always(false, st) => {
                    let s = d.stmt(&scope, st)?;
                    d.comb.push(s);
                }
                Item::Initial(st) => {
                    let s = d.stmt(&scope, st)?;
                    d.init.push(s);
                }
                Item::Decl(decl) => {
                    if let Some(init) = &decl.init {
                        let target =
                            S::Assign(L::Sig(scope.sigs[&decl.name]), d.expr(&scope, init)?, false);
                        if decl.is_reg { d.init.push(target) } else { d.comb.push(target) }
                    }
                }
                Item::Param(..) => {}
            }
        }
        d.pins = d.bind(&scope, &dirs)?;
        d.order_comb();
        Ok(d)
    }

    fn declare(
        &mut self,
        scope: &mut Scope,
        decl: &Decl,
        dirs: &mut HashMap<String, Dir>,
    ) -> R<()> {
        let (width, lsb) = match &decl.range {
            Some((a, b)) => {
                let (a, b) =
                    (self.constant(scope, a)?.ext() as i64, self.constant(scope, b)?.ext() as i64);
                ((a - b).unsigned_abs() as u32 + 1, a.min(b))
            }
            None => (1, 0),
        };
        if width > 64 {
            return err(format!("`{}`: signals wider than 64 bits are not supported", decl.name));
        }
        if let Some(dir) = decl.dir {
            dirs.insert(decl.name.clone(), dir);
        }
        if let Some((a, b)) = &decl.array {
            let (a, b) =
                (self.constant(scope, a)?.ext() as i64, self.constant(scope, b)?.ext() as i64);
            let words = (a - b).unsigned_abs() as usize + 1;
            if words > 1 << 20 {
                return err(format!("`{}`: memory too large", decl.name));
            }
            scope.mems.insert(decl.name.clone(), self.mems.len());
            self.mems.push(Mem { name: decl.name.clone(), width, lo: a.min(b), words });
        } else {
            scope.sigs.insert(decl.name.clone(), self.sigs.len());
            self.sigs.push(Sig { name: decl.name.clone(), width, lsb, signed: decl.signed });
        }
        Ok(())
    }

    fn bind(&self, scope: &Scope, dirs: &HashMap<String, Dir>) -> R<Pins> {
        let pin = |name: &str, dir: Dir| -> R<usize> {
            match (scope.sigs.get(name), dirs.get(name)) {
                (Some(&id), Some(&d)) if d == dir => Ok(id),
                _ => err(format!(
                    "module `{}` needs the {} port `{name}` (see core/fpga/src/signals.rs)",
                    self.name,
                    if dir == Dir::Input { "input" } else { "output" }
                )),
            }
        };
        Ok(Pins {
            rst: pin("rst", Dir::Input)?,
            sel: pin("bus_sel", Dir::Input)?,
            we: pin("bus_we", Dir::Input)?,
            addr: pin("bus_addr", Dir::Input)?,
            be: pin("bus_be", Dir::Input)?,
            wdata: pin("bus_wdata", Dir::Input)?,
            rdata: pin("bus_rdata", Dir::Output)?,
            irq: pin("irq", Dir::Output)?,
        })
    }

    fn constant(&self, scope: &Scope, e: &Expr) -> R<Val> {
        let resolved = self.expr(scope, e)?;
        if !Self::is_const(&resolved) {
            return err("expected a constant expression");
        }
        let empty = State { vals: Vec::new(), mems: Vec::new() };
        Ok(self.ev(&resolved, &empty))
    }

    fn is_const(e: &E) -> bool {
        match e {
            E::Const(_) => true,
            E::Un(_, a) | E::Sign(a, _) => Self::is_const(a),
            E::Bin(_, a, b) => Self::is_const(a) && Self::is_const(b),
            E::Tern(a, b, c) => Self::is_const(a) && Self::is_const(b) && Self::is_const(c),
            E::Concat(v) | E::Repl(_, v) => v.iter().all(Self::is_const),
            _ => false,
        }
    }

    fn sig(scope: &Scope, name: &str) -> R<usize> {
        scope
            .sigs
            .get(name)
            .copied()
            .ok_or_else(|| VerilogError(format!("unknown signal `{name}`")))
    }

    fn part(&self, scope: &Scope, id: usize, a: &Expr, b: &Expr) -> R<(u32, u32)> {
        let (a, b) = (self.constant(scope, a)?.ext() as i64, self.constant(scope, b)?.ext() as i64);
        let lo = a.min(b) - self.sigs[id].lsb;
        if lo < 0 {
            return err(format!("part select below `{}`'s range", self.sigs[id].name));
        }
        Ok((lo as u32, (a - b).unsigned_abs() as u32 + 1))
    }

    fn expr(&self, scope: &Scope, e: &Expr) -> R<E> {
        Ok(match e {
            Expr::Num(n) => E::Const(number(n)?),
            Expr::Ident(name) => match scope.params.get(name) {
                Some(v) => E::Const(*v),
                None => E::Sig(Self::sig(scope, name)?),
            },
            Expr::Index(name, i) => match scope.mems.get(name) {
                Some(&m) => E::MemRd(m, Box::new(self.expr(scope, i)?)),
                None => E::Bit(Self::sig(scope, name)?, Box::new(self.expr(scope, i)?)),
            },
            Expr::Part(name, a, b) => {
                let id = Self::sig(scope, name)?;
                let (shift, width) = self.part(scope, id, a, b)?;
                E::Part(id, shift, width)
            }
            Expr::IndexedPart(name, base, w, up) => E::IPart(
                Self::sig(scope, name)?,
                Box::new(self.expr(scope, base)?),
                self.constant(scope, w)?.v as u32,
                *up,
            ),
            Expr::Unary(op, a) => E::Un(*op, Box::new(self.expr(scope, a)?)),
            Expr::Binary(op, a, b) => {
                E::Bin(*op, Box::new(self.expr(scope, a)?), Box::new(self.expr(scope, b)?))
            }
            Expr::Ternary(c, a, b) => E::Tern(
                Box::new(self.expr(scope, c)?),
                Box::new(self.expr(scope, a)?),
                Box::new(self.expr(scope, b)?),
            ),
            Expr::Concat(parts) => {
                E::Concat(parts.iter().map(|p| self.expr(scope, p)).collect::<R<_>>()?)
            }
            Expr::Repl(n, parts) => {
                let n = self.constant(scope, n)?.v as u32;
                E::Repl(n, parts.iter().map(|p| self.expr(scope, p)).collect::<R<_>>()?)
            }
            Expr::Call(f, args) => match (f.as_str(), args.as_slice()) {
                ("$signed", [a]) => E::Sign(Box::new(self.expr(scope, a)?), true),
                ("$unsigned", [a]) => E::Sign(Box::new(self.expr(scope, a)?), false),
                _ => return err(format!("`{f}` is not supported in expressions")),
            },
        })
    }

    fn lvalue(&self, scope: &Scope, lv: &LValue) -> R<L> {
        Ok(match lv {
            LValue::Name(n) => L::Sig(Self::sig(scope, n)?),
            LValue::Index(n, i) => match scope.mems.get(n) {
                Some(&m) => L::Mem(m, self.expr(scope, i)?),
                None => L::Bit(Self::sig(scope, n)?, self.expr(scope, i)?),
            },
            LValue::Part(n, a, b) => {
                let id = Self::sig(scope, n)?;
                let (shift, width) = self.part(scope, id, a, b)?;
                L::Part(id, shift, width)
            }
        })
    }

    fn stmt(&self, scope: &Scope, s: &Stmt) -> R<S> {
        Ok(match s {
            Stmt::Block(v) => S::Block(v.iter().map(|s| self.stmt(scope, s)).collect::<R<_>>()?),
            Stmt::Empty => S::Block(Vec::new()),
            Stmt::If(c, a, b) => S::If(
                self.expr(scope, c)?,
                Box::new(self.stmt(scope, a)?),
                b.as_ref().map(|b| self.stmt(scope, b).map(Box::new)).transpose()?,
            ),
            Stmt::Case(sel, arms, default) => S::Case(
                self.expr(scope, sel)?,
                arms.iter()
                    .map(|(labels, s)| {
                        Ok((
                            labels.iter().map(|l| self.expr(scope, l)).collect::<R<_>>()?,
                            self.stmt(scope, s)?,
                        ))
                    })
                    .collect::<R<_>>()?,
                default.as_ref().map(|d| self.stmt(scope, d).map(Box::new)).transpose()?,
            ),
            Stmt::Assign(lv, e, nb) => {
                S::Assign(self.lvalue(scope, lv)?, self.expr(scope, e)?, *nb)
            }
        })
    }

    // ---- comb ordering ----

    fn reads_e(&self, e: &E, out: &mut Vec<usize>) {
        let n = self.sigs.len();
        match e {
            E::Const(_) => {}
            E::Sig(i) | E::Part(i, ..) => out.push(*i),
            E::Bit(i, a) | E::IPart(i, a, ..) => {
                out.push(*i);
                self.reads_e(a, out);
            }
            E::MemRd(m, a) => {
                out.push(n + m);
                self.reads_e(a, out);
            }
            E::Un(_, a) | E::Sign(a, _) => self.reads_e(a, out),
            E::Bin(_, a, b) => {
                self.reads_e(a, out);
                self.reads_e(b, out);
            }
            E::Tern(a, b, c) => {
                self.reads_e(a, out);
                self.reads_e(b, out);
                self.reads_e(c, out);
            }
            E::Concat(v) | E::Repl(_, v) => v.iter().for_each(|x| self.reads_e(x, out)),
        }
    }

    fn rw_s(&self, s: &S, reads: &mut Vec<usize>, writes: &mut Vec<usize>) {
        match s {
            S::Block(v) => v.iter().for_each(|s| self.rw_s(s, reads, writes)),
            S::If(c, a, b) => {
                self.reads_e(c, reads);
                self.rw_s(a, reads, writes);
                if let Some(b) = b {
                    self.rw_s(b, reads, writes);
                }
            }
            S::Case(sel, arms, d) => {
                self.reads_e(sel, reads);
                for (labels, s) in arms {
                    for l in labels {
                        self.reads_e(l, reads);
                    }
                    self.rw_s(s, reads, writes);
                }
                if let Some(d) = d {
                    self.rw_s(d, reads, writes);
                }
            }
            S::Assign(l, e, _) => {
                self.reads_e(e, reads);
                match l {
                    L::Sig(i) | L::Part(i, ..) => writes.push(*i),
                    L::Bit(i, x) => {
                        writes.push(*i);
                        self.reads_e(x, reads);
                    }
                    L::Mem(m, x) => {
                        writes.push(self.sigs.len() + m);
                        self.reads_e(x, reads);
                    }
                }
            }
        }
    }

    /// Kahn's algorithm over "writes a slot another process reads".
    fn order_comb(&mut self) {
        let n = self.comb.len();
        let rw: Vec<(Vec<usize>, Vec<usize>)> = self
            .comb
            .iter()
            .map(|s| {
                let (mut r, mut w) = (Vec::new(), Vec::new());
                self.rw_s(s, &mut r, &mut w);
                (r, w)
            })
            .collect();
        let mut indeg = vec![0; n];
        let mut edges = vec![Vec::new(); n];
        for (p, (_, wp)) in rw.iter().enumerate() {
            for (q, (rq, _)) in rw.iter().enumerate() {
                if p != q && wp.iter().any(|w| rq.contains(w)) {
                    edges[p].push(q);
                    indeg[q] += 1;
                }
            }
        }
        let mut ready: Vec<usize> = (0..n).filter(|&i| indeg[i] == 0).collect();
        let mut order = Vec::with_capacity(n);
        while let Some(p) = ready.pop() {
            order.push(p);
            for &q in &edges[p] {
                indeg[q] -= 1;
                if indeg[q] == 0 {
                    ready.push(q);
                }
            }
        }
        if order.len() == n {
            let mut comb: Vec<Option<S>> =
                std::mem::take(&mut self.comb).into_iter().map(Some).collect();
            self.comb = order.into_iter().filter_map(|i| comb[i].take()).collect();
        } else {
            self.ordered = false;
        }
    }

    // ---- simulation ----

    pub fn new_state(&self) -> State {
        let mut st = State {
            vals: vec![0; self.sigs.len()],
            mems: self.mems.iter().map(|m| vec![0; m.words]).collect(),
        };
        let mut queue = Vec::new();
        for s in &self.init {
            self.exec(s, &mut st, &mut queue);
        }
        self.apply(&mut st, queue);
        self.settle(&mut st);
        st
    }

    pub fn set(&self, st: &mut State, id: usize, v: u64) {
        st.vals[id] = v & mask(self.sigs[id].width);
    }

    pub fn settle(&self, st: &mut State) {
        let mut queue = Vec::new();
        let passes = if self.ordered { 1 } else { 64 };
        for _ in 0..passes {
            let before = if self.ordered { Vec::new() } else { st.vals.clone() };
            for s in &self.comb {
                self.exec(s, st, &mut queue);
            }
            if self.ordered || st.vals == before {
                break;
            }
        }
    }

    /// One rising edge; returns whether any register changed.
    pub fn clock(&self, st: &mut State) -> bool {
        let mut queue = Vec::new();
        let before_vals = st.vals.clone();
        for s in &self.ff {
            self.exec(s, st, &mut queue);
        }
        let mut changed = self.apply(st, queue);
        changed |= st.vals != before_vals;
        self.settle(st);
        changed
    }

    fn apply(&self, st: &mut State, queue: Vec<Write>) -> bool {
        let mut changed = false;
        for w in queue {
            changed |= self.write(st, &w);
        }
        changed
    }

    fn write(&self, st: &mut State, w: &Write) -> bool {
        match *w {
            Write::Sig { id, shift, width, v } => {
                let m = mask(width) << shift;
                let new = ((st.vals[id] & !m) | ((v << shift) & m)) & mask(self.sigs[id].width);
                let changed = new != st.vals[id];
                st.vals[id] = new;
                changed
            }
            Write::Mem { id, idx, v } => match st.mems[id].get_mut(idx) {
                Some(slot) => {
                    let new = v & mask(self.mems[id].width);
                    let changed = *slot != new;
                    *slot = new;
                    changed
                }
                None => false,
            },
        }
    }

    fn exec(&self, s: &S, st: &mut State, queue: &mut Vec<Write>) {
        match s {
            S::Block(v) => v.iter().for_each(|s| self.exec(s, st, queue)),
            S::If(c, a, b) => {
                if self.ev(c, st).v != 0 {
                    self.exec(a, st, queue);
                } else if let Some(b) = b {
                    self.exec(b, st, queue);
                }
            }
            S::Case(sel, arms, default) => {
                let sel = self.ev(sel, st);
                for (labels, body) in arms {
                    if labels.iter().any(|l| bin(BinOp::Eq, sel, self.ev(l, st)).v != 0) {
                        self.exec(body, st, queue);
                        return;
                    }
                }
                if let Some(d) = default {
                    self.exec(d, st, queue);
                }
            }
            S::Assign(l, e, nonblocking) => {
                let v = self.ev(e, st).ext();
                let w = match l {
                    L::Sig(id) => Write::Sig { id: *id, shift: 0, width: self.sigs[*id].width, v },
                    L::Part(id, shift, width) => {
                        Write::Sig { id: *id, shift: *shift, width: *width, v }
                    }
                    L::Bit(id, i) => {
                        let bit = self.ev(i, st).ext() as i64 - self.sigs[*id].lsb;
                        if !(0..i64::from(self.sigs[*id].width)).contains(&bit) {
                            return;
                        }
                        Write::Sig { id: *id, shift: bit as u32, width: 1, v }
                    }
                    L::Mem(m, i) => {
                        let idx = self.ev(i, st).ext() as i64 - self.mems[*m].lo;
                        if idx < 0 {
                            return;
                        }
                        Write::Mem { id: *m, idx: idx as usize, v }
                    }
                };
                if *nonblocking {
                    queue.push(w);
                } else {
                    self.write(st, &w);
                }
            }
        }
    }

    fn ev(&self, e: &E, st: &State) -> Val {
        match e {
            E::Const(v) => *v,
            E::Sig(i) => Val { v: st.vals[*i], w: self.sigs[*i].width, s: self.sigs[*i].signed },
            E::Bit(i, idx) => {
                let bit = self.ev(idx, st).ext() as i64 - self.sigs[*i].lsb;
                let v = if (0..64).contains(&bit) { (st.vals[*i] >> bit) & 1 } else { 0 };
                Val::new(v, 1, false)
            }
            E::Part(i, shift, width) => Val::new(st.vals[*i] >> shift, *width, false),
            E::IPart(i, base, width, up) => {
                let b = self.ev(base, st).ext() as i64 - self.sigs[*i].lsb;
                let lo = if *up { b } else { b - i64::from(*width) + 1 };
                let v = if (0..64).contains(&lo) { st.vals[*i] >> lo } else { 0 };
                Val::new(v, *width, false)
            }
            E::MemRd(m, idx) => {
                let i = self.ev(idx, st).ext() as i64 - self.mems[*m].lo;
                let v =
                    usize::try_from(i).ok().and_then(|i| st.mems[*m].get(i)).copied().unwrap_or(0);
                Val::new(v, self.mems[*m].width, false)
            }
            E::Un(op, a) => unary(*op, self.ev(a, st)),
            E::Bin(op, a, b) => bin(*op, self.ev(a, st), self.ev(b, st)),
            E::Tern(c, a, b) => {
                let (a, b) = (self.ev(a, st), self.ev(b, st));
                let (w, s) = (a.w.max(b.w), a.s && b.s);
                let pick = if self.ev(c, st).v != 0 { a } else { b };
                Val::new(if s { pick.ext() } else { pick.v }, w, s)
            }
            E::Concat(parts) => self.concat(parts, st),
            E::Repl(n, parts) => {
                let one = self.concat(parts, st);
                let mut acc = Val::new(0, 0, false);
                for _ in 0..*n {
                    acc = Val::new((acc.v << one.w) | one.v, (acc.w + one.w).min(64), false);
                }
                acc
            }
            E::Sign(a, s) => {
                let a = self.ev(a, st);
                Val { s: *s, ..a }
            }
        }
    }

    fn concat(&self, parts: &[E], st: &State) -> Val {
        parts.iter().fold(Val::new(0, 0, false), |acc, p| {
            let v = self.ev(p, st);
            let shifted = if v.w >= 64 { 0 } else { acc.v << v.w };
            Val::new(shifted | v.v, (acc.w + v.w).min(64), false)
        })
    }
}

fn unary(op: UnOp, a: Val) -> Val {
    let bit = |b: bool| Val::new(u64::from(b), 1, false);
    match op {
        UnOp::Plus => a,
        UnOp::Neg => Val::new(a.ext().wrapping_neg(), a.w, a.s),
        UnOp::Not => Val::new(!a.v, a.w, a.s),
        UnOp::LNot | UnOp::RedNor => bit(a.v == 0),
        UnOp::RedAnd => bit(a.v == mask(a.w)),
        UnOp::RedOr => bit(a.v != 0),
        UnOp::RedXor => bit(a.v.count_ones() % 2 == 1),
        UnOp::RedNand => bit(a.v != mask(a.w)),
        UnOp::RedXnor => bit(a.v.count_ones().is_multiple_of(2)),
    }
}

fn bin(op: BinOp, a: Val, b: Val) -> Val {
    let (w, s) = (a.w.max(b.w), a.s && b.s);
    let (x, y) = if s { (a.ext(), b.ext()) } else { (a.v, b.v) };
    let bit = |c: bool| Val::new(u64::from(c), 1, false);
    let cmp = |f: fn(i64, i64) -> bool, g: fn(u64, u64) -> bool| {
        bit(if s { f(x as i64, y as i64) } else { g(x, y) })
    };
    let amount = b.v;
    match op {
        BinOp::Add => Val::new(x.wrapping_add(y), w, s),
        BinOp::Sub => Val::new(x.wrapping_sub(y), w, s),
        BinOp::Mul => Val::new(x.wrapping_mul(y), w, s),
        BinOp::Div => Val::new(
            if y == 0 {
                mask(w)
            } else if s {
                (x as i64).wrapping_div(y as i64) as u64
            } else {
                x / y
            },
            w,
            s,
        ),
        BinOp::Rem => Val::new(
            if y == 0 {
                mask(w)
            } else if s {
                (x as i64).wrapping_rem(y as i64) as u64
            } else {
                x % y
            },
            w,
            s,
        ),
        BinOp::Shl | BinOp::AShl => {
            Val::new(if amount >= 64 { 0 } else { a.v << amount }, a.w, a.s)
        }
        BinOp::Shr => Val::new(if amount >= 64 { 0 } else { a.v >> amount }, a.w, a.s),
        BinOp::AShr => {
            let v = if a.s {
                (sext(a.v, a.w) >> amount.min(63)) as u64
            } else if amount >= 64 {
                0
            } else {
                a.v >> amount
            };
            Val::new(v, a.w, a.s)
        }
        BinOp::Lt => cmp(|p, q| p < q, |p, q| p < q),
        BinOp::Le => cmp(|p, q| p <= q, |p, q| p <= q),
        BinOp::Gt => cmp(|p, q| p > q, |p, q| p > q),
        BinOp::Ge => cmp(|p, q| p >= q, |p, q| p >= q),
        BinOp::Eq => bit(x & mask(w) == y & mask(w)),
        BinOp::Ne => bit(x & mask(w) != y & mask(w)),
        BinOp::And => Val::new(x & y, w, s),
        BinOp::Or => Val::new(x | y, w, s),
        BinOp::Xor => Val::new(x ^ y, w, s),
        BinOp::Xnor => Val::new(!(x ^ y), w, s),
        BinOp::LAnd => bit(a.v != 0 && b.v != 0),
        BinOp::LOr => bit(a.v != 0 || b.v != 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals() {
        assert_eq!(number("32'hFFFF_0000").map(|v| (v.v, v.w)), Ok((0xFFFF_0000, 32)));
        assert_eq!(number("4'b1010").map(|v| v.v), Ok(10));
        assert_eq!(number("12").map(|v| (v.v, v.w, v.s)), Ok((12, 32, true)));
        assert_eq!(number("'d7").map(|v| v.v), Ok(7));
    }

    #[test]
    fn signed_ops() {
        let a = Val::new(0xFFFF_FFF0, 32, true); // -16
        let b = Val::new(2, 32, true);
        assert_eq!(bin(BinOp::Lt, a, b).v, 1);
        assert_eq!(bin(BinOp::Lt, Val { s: false, ..a }, Val { s: false, ..b }).v, 0);
        assert_eq!(bin(BinOp::AShr, a, Val::new(2, 5, false)).v, 0xFFFF_FFFC);
        assert_eq!(bin(BinOp::Shr, Val { s: false, ..a }, Val::new(2, 5, false)).v, 0x3FFF_FFFC);
    }
}
