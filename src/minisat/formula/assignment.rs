use super::{Var, Lit};
use super::clause;
use super::index_map::VarMap;
use minisat::propagation_trail::*;


#[derive(Clone, Copy)]
#[repr(u8)]
pub enum Value { Undef, False, True }

impl Value {
    #[inline]
    fn isUndef(&self) -> bool {
        match *self {
            Value::Undef => { true }
            _            => { false }
        }
    }
}


struct VarData {
    pub reason : Option<clause::ClauseRef>,
    pub level  : DecisionLevel
}


struct VarLine {
    assign : [Value; 2],
    vd     : VarData
}


pub struct Assignment {
    assignment : Vec<VarLine>,
    free_vars  : Vec<usize>
}

impl Assignment {
    pub fn new() -> Assignment {
        Assignment { assignment : Vec::new()
                   , free_vars  : Vec::new()
                   }
    }

    #[inline]
    pub fn nVars(&self) -> usize {
        self.assignment.len()
    }

    pub fn newVar(&mut self) -> Var {
        let line = VarLine { assign : [Value::Undef, Value::Undef], vd : VarData { reason : None, level : 0 } };
        let vid =
            match self.free_vars.pop() {
                Some(v) => {
                    self.assignment[v] = line;
                    v
                }

                None    => {
                    self.assignment.push(line);
                    self.assignment.len() - 1
                }
            };

        Var(vid)
    }

    pub fn freeVar(&mut self, Var(v) : Var) {
        self.free_vars.push(v);
    }

    #[inline]
    pub fn assignLit(&mut self, Lit(p) : Lit, level : DecisionLevel, reason : Option<clause::ClauseRef>) {
        let ref mut line = self.assignment[p >> 1];

        assert!(line.assign[0].isUndef());
        line.assign[p & 1] = Value::True;
        line.assign[(p & 1) ^ 1] = Value::False;
        line.vd.level = level;
        line.vd.reason = reason;
    }

    #[inline]
    pub fn cancel(&mut self, Var(v) : Var) {
        let ref mut line = self.assignment[v];
        line.assign = [Value::Undef, Value::Undef];
    }

    #[inline]
    pub fn undef(&self, Var(v) : Var) -> bool {
        let ref line = self.assignment[v];
        line.assign[0].isUndef()
    }

    #[inline]
    pub fn sat(&self, p : Lit) -> bool {
        match self.ofLit(p) {
            Value::True => { true }
            _           => { false }
        }
    }

    #[inline]
    pub fn unsat(&self, p : Lit) -> bool {
        match self.ofLit(p) {
            Value::False => { true }
            _            => { false }
        }
    }

    #[inline]
    pub fn ofLit(&self, Lit(p) : Lit) -> Value {
        let ref line = self.assignment[p >> 1];
        line.assign[p & 1]
    }

    #[inline]
    pub fn vardata(&self, Var(v) : Var) -> &VarData {
        let ref line = self.assignment[v];
        assert!(!line.assign[0].isUndef());
        &line.vd
    }

    pub fn extractModel(&self) -> VarMap<bool> {
        let mut model = VarMap::new();
        for i in 0 .. self.assignment.len() {
            match self.assignment[i].assign[0] {
                Value::Undef  => {}
                Value::False  => { model.insert(&Var(i), false); }
                Value::True   => { model.insert(&Var(i), true); }
            }
        }
        model
    }

    pub fn relocGC(&mut self, trail : &PropagationTrail<Lit>, from : &mut clause::ClauseAllocator, to : &mut clause::ClauseAllocator) {
        for l in trail.trail.iter() {
            let Var(v) = l.var();

            // Note: it is not safe to call 'locked()' on a relocated clause. This is why we keep
            // 'dangling' reasons here. It is safe and does not hurt.
            match self.assignment[v].vd.reason {
                Some(cr) if from[cr].reloced() || self.isLocked(from, cr) => {
                    assert!(!from[cr].is_deleted());
                    self.assignment[v].vd.reason = Some(from.relocTo(to, cr));
                }

                _ => {}
            }
        }
    }

    pub fn isLocked(&self, ca : &clause::ClauseAllocator, cr : clause::ClauseRef) -> bool {
        let lit = ca[cr][0];
        if !self.sat(lit) { return false; }
        match self.vardata(lit.var()).reason {
            Some(r) if cr == r => { true }
            _                  => { false }
        }
    }

    pub fn forgetReason(&mut self, ca : &clause::ClauseAllocator, cr : clause::ClauseRef) {
        // Don't leave pointers to free'd memory!
        if self.isLocked(ca, cr) {
            let Var(v) = ca[cr][0].var();
            self.assignment[v].vd.reason = None;
        }
    }
}


// Returns TRUE if a clause is satisfied in the current state.
pub fn satisfiedWith(c : &clause::Clause, s : &Assignment) -> bool {
    for i in 0 .. c.len() {
        if s.sat(c[i]) {
            return true;
        }
    }
    false
}
