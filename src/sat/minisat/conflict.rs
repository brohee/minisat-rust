use sat::formula::{Var, Lit, VarMap, LitMap};
use sat::formula::clause::*;
use sat::formula::assignment::*;
use sat::minisat::clause_db::*;
use sat::minisat::decision_heuristic::*;


#[derive(PartialEq, Eq)]
pub enum CCMinMode {
    None,
    Basic,
    Deep
}


#[derive(PartialEq, Eq, Clone, Copy, Debug)]
#[repr(u8)]
pub enum Seen {
    Undef     = 0,
    Source    = 1,
    Removable = 2,
    Failed    = 3
}


pub enum Conflict {
    Ground,
    Unit(DecisionLevel, Lit),
    Learned(DecisionLevel, Lit, Box<[Lit]>)
}


pub struct AnalyzeContext {
    ccmin_mode       : CCMinMode,    // Controls conflict clause minimization
    pub seen         : VarMap<Seen>,
    analyze_toclear  : Vec<Lit>,
    pub max_literals : u64,
    pub tot_literals : u64
}

impl AnalyzeContext {
    pub fn new(ccmin_mode : CCMinMode) -> AnalyzeContext {
        AnalyzeContext { ccmin_mode      : ccmin_mode
                       , seen            : VarMap::new()
                       , analyze_toclear : Vec::new()
                       , max_literals    : 0
                       , tot_literals    : 0
                       }
    }

    pub fn initVar(&mut self, v : Var) {
        self.seen.insert(&v, Seen::Undef);
    }

    // Description:
    //   Analyze conflict and produce a reason clause.
    //
    //   Pre-conditions:
    //     * 'out_learnt' is assumed to be cleared.
    //     * Current decision level must be greater than root level.
    //
    //   Post-conditions:
    //     * 'out_learnt[0]' is the asserting literal at level 'out_btlevel'.
    //     * If out_learnt.size() > 1 then 'out_learnt[1]' has the greatest decision level of the
    //       rest of literals. There may be others from the same level though.
    //
    pub fn analyze(&mut self, db : &mut ClauseDB, heur : &mut DecisionHeuristic, assigns : &Assignment, confl0 : ClauseRef) -> Conflict {
        if assigns.isGroundLevel() {
            return Conflict::Ground;
        }

        // Generate conflict clause:
        let mut out_learnt = Vec::new();

        {
            let mut confl = confl0;
            let mut pathC = 0;
            let mut index = assigns.numberOfAssigns();
            loop {
                db.bumpActivity(confl);

                for q in db.ca.view(confl).iterFrom(if confl == confl0 { 0 } else { 1 }) {
                    let v = q.var();
                    if self.seen[&v] == Seen::Undef && assigns.vardata(v).level > GroundLevel {
                        self.seen[&v] = Seen::Source;
                        heur.bumpActivity(&v);
                        if assigns.vardata(v).level >= assigns.decisionLevel() {
                            pathC += 1;
                        } else {
                            out_learnt.push(q);
                        }
                    }
                }

                // Select next clause to look at:
                let pl = {
                    loop {
                        index -= 1;
                        if self.seen[&assigns.assignAt(index).var()] != Seen::Undef { break; }
                    }
                    assigns.assignAt(index)
                };

                self.seen[&pl.var()] = Seen::Undef;

                pathC -= 1;
                if pathC <= 0 {
                    out_learnt.insert(0, !pl);
                    break;
                }

                confl = {
                    let reason = assigns.vardata(pl.var()).reason;
                    assert!(reason.is_some()); // (otherwise should be UIP)
                    reason.unwrap()
                };
            }
        }


        // Simplify conflict clause:
        self.analyze_toclear = out_learnt.clone();
        self.max_literals += out_learnt.len() as u64;
        match self.ccmin_mode {
            CCMinMode::Deep  => { out_learnt.retain(|&l| { !self.litRedundant(&db.ca, assigns, l) }); }
            CCMinMode::Basic => { out_learnt.retain(|&l| { !self.litRedundantBasic(&db.ca, assigns, l) }); }
            CCMinMode::None  => {}
        }
        self.tot_literals += out_learnt.len() as u64;

        for l in self.analyze_toclear.iter() {
            self.seen[&l.var()] = Seen::Undef;    // ('seen[]' is now cleared)
        }

        // Find correct backtrack level:
        if out_learnt.len() == 1 {
            Conflict::Unit(GroundLevel, out_learnt[0])
        } else {
            // Find the first literal assigned at the next-highest level:
            let mut max_i = 1;
            let mut max_level = assigns.vardata(out_learnt[1].var()).level;
            for i in 2 .. out_learnt.len() {
                let level = assigns.vardata(out_learnt[i].var()).level;
                if level > max_level {
                    max_i = i;
                    max_level = level;
                }
            }

            // Swap-in this literal at index 1:
            out_learnt.swap(1, max_i);
            Conflict::Learned(max_level, out_learnt[0], out_learnt.into_boxed_slice())
        }
    }

    fn litRedundantBasic(&self, ca : &ClauseAllocator, assigns : &Assignment, literal : Lit) -> bool {
        match assigns.vardata(literal.var()).reason {
            None     => { false }
            Some(cr) => {
                for lit in ca.view(cr).iterFrom(1) {
                    let y = lit.var();
                    if self.seen[&y] == Seen::Undef && assigns.vardata(y).level > GroundLevel {
                        return false;
                    }
                }
                true
            }
        }
    }

    // Check if 'p' can be removed from a conflict clause.
    fn litRedundant(&mut self, ca : &ClauseAllocator, assigns : &Assignment, literal : Lit) -> bool {
        assert!({ let s = self.seen[&literal.var()]; s == Seen::Undef || s == Seen::Source });

        let mut analyze_stack =
            match assigns.vardata(literal.var()).reason {
                None     => { return false; }
                Some(cr) => { vec![(literal, ca.view(cr).iterFrom(1))] }
            };

        while let Some((p, mut it)) = analyze_stack.pop() {
            match it.next() {
                Some(l) => {
                    analyze_stack.push((p, it));
                    let ref vd = assigns.vardata(l.var());
                    let seen = self.seen[&l.var()];

                    // Variable at level 0 or previously removable:
                    if vd.level == GroundLevel || seen == Seen::Source || seen == Seen::Removable {
                        continue;
                    }

                    match vd.reason {
                        // Recursively check 'l':
                        Some(cr) if seen == Seen::Undef => {
                            analyze_stack.push((l, ca.view(cr).iterFrom(1)));
                        }

                        // Check variable can not be removed for some local reason:
                        _                                => {
                            for &(l, _) in analyze_stack.iter() {
                                if self.seen[&l.var()] == Seen::Undef {
                                    self.seen[&l.var()] = Seen::Failed;
                                    self.analyze_toclear.push(l);
                                }
                            }
                            return false;
                        }
                    }
                }

                None    => {
                    // Finished with current element 'p' and reason 'c':
                    if self.seen[&p.var()] == Seen::Undef {
                        self.seen[&p.var()] = Seen::Removable;
                        self.analyze_toclear.push(p);
                    }
                }
            }
        }

        true
    }

    // Description:
    //   Specialized analysis procedure to express the final conflict in terms of assumptions.
    //   Calculates the (possibly empty) set of assumptions that led to the assignment of 'p', and
    //   stores the result in 'out_conflict'.
    pub fn analyzeFinal(&mut self, ca : &ClauseAllocator, assigns : &Assignment, p : Lit) -> LitMap<()> {
        let mut out_conflict = LitMap::new();
        out_conflict.insert(&p, ());

        assigns.inspectUntilLevel(GroundLevel, |lit| {
            let x = lit.var();
            if self.seen[&x] != Seen::Undef {
                match assigns.vardata(x).reason {
                    None     => {
                        assert!(assigns.vardata(x).level > GroundLevel);
                        out_conflict.insert(&!lit, ());
                    }

                    Some(cr) => {
                        for lit in ca.view(cr).iterFrom(1) {
                            let v = lit.var();
                            if assigns.vardata(v).level > GroundLevel {
                                self.seen[&v] = Seen::Source;
                            }
                        }
                    }
                }
            }
        });

        out_conflict
    }
}
