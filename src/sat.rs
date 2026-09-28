//! CDCL SAT 求解器（零依赖，自研）。
//!
//! 结构：双监视文字 + 单位传播 + 1-UIP 冲突分析 + 非时序回跳 + VSIDS 活动度
//! + 相位保存 + 几何重启 + 学习子句库上限。
//!
//! ## 为什么不用现成求解器
//!
//! README 的定位是 `dependencies-0`；且形式化验证的信任链要求**裁定者本身可信**。
//! 自研意味着正确性责任在自己身上，因此：
//! - 本模块带**穷举参考求解器差分测试**（`tests` 内 `brute_force_sat`），
//!   对随机 CNF 逐一比对 SAT/UNSAT 结论与模型合法性；
//! - 对网表编码的端到端结论还会与既有穷举验证器 `verify.rs`/`equiv.rs` 交叉核对。
//!
//! 求解器返回 `Unknown`（而非错误的 SAT/UNSAT）当触及资源上限 —— **fail-closed**。

use crate::cnf::{Cnf, Lit};

/// 求解结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SolveResult {
    Sat,
    Unsat,
    /// 触及资源上限，未得出结论（绝不冒充 SAT/UNSAT）。
    Unknown,
}

#[derive(Debug, Clone)]
struct Clause {
    lits: Vec<Lit>,
    learnt: bool,
}

/// 求解统计。
#[derive(Debug, Clone, Default)]
pub struct SatStats {
    pub decisions: u64,
    pub propagations: u64,
    pub conflicts: u64,
    pub learnts: u64,
    pub restarts: u64,
}

pub struct Solver {
    nvars: u32,
    clauses: Vec<Clause>,
    /// 变量 → 赋值（0 未定 / 1 真 / -1 假）
    assign: Vec<i8>,
    /// 变量 → 决策层
    level: Vec<u32>,
    /// 变量 → 蕴含原因子句下标（-1 表示决策或无原因）
    reason: Vec<i32>,
    /// 变量 → 相位保存（上次赋值）
    phase: Vec<i8>,
    /// 变量 → VSIDS 活动度
    activity: Vec<f64>,
    activity_inc: f64,
    /// 双监视：literals 索引 → 子句下标
    watches: Vec<Vec<i32>>,
    trail: Vec<Lit>,
    trail_lim: Vec<usize>,
    qhead: usize,
    /// 冲突分析用标记
    seen: Vec<bool>,
    /// 载入阶段即发现矛盾（互补单位子句），`solve()` 直接判 UNSAT。
    /// 不能静默丢弃冲突单位子句 —— 那会把 UNSAT 误判成 SAT（假证明源头）。
    init_conflict: bool,
    /// 资源上限
    pub max_conflicts: u64,
    pub max_learnts: usize,
    pub stats: SatStats,
}

#[inline]
fn lit_idx(l: Lit) -> usize {
    (((l.abs() - 1) as usize) << 1) | if l < 0 { 1 } else { 0 }
}

impl Solver {
    /// 从 CNF 构造求解器（去重、去恒真子句；单位子句立即在 0 层入队）。
    ///
    /// 若初始即冲突，`solve()` 会返回 `Unsat`。
    pub fn new(cnf: &Cnf) -> Self {
        let n = cnf.num_vars.max(0) as u32;
        let mut s = Solver {
            nvars: n,
            clauses: Vec::new(),
            assign: vec![0; (n + 1) as usize],
            level: vec![0; (n + 1) as usize],
            reason: vec![-1; (n + 1) as usize],
            phase: vec![-1; (n + 1) as usize],
            activity: vec![0.0; (n + 1) as usize],
            activity_inc: 1.0,
            watches: vec![Vec::new(); ((n as usize) + 1) * 2],
            trail: Vec::new(),
            trail_lim: Vec::new(),
            qhead: 0,
            seen: vec![false; (n + 1) as usize],
            init_conflict: false,
            max_conflicts: 2_000_000,
            max_learnts: 200_000,
            stats: SatStats::default(),
        };
        // 载入子句
        for c in &cnf.clauses {
            if c.is_empty() {
                s.clauses.push(Clause { lits: Vec::new(), learnt: false });
                continue;
            }
            // 去重 + 恒真检测
            let mut lits: Vec<Lit> = Vec::with_capacity(c.len());
            let mut taut = false;
            'lit: for &l in c {
                if l == 0 || l.abs() > n as i32 {
                    continue 'lit; // 越界文字直接忽略（应由 validate 提前拦下）
                }
                if lits.contains(&-l) {
                    taut = true;
                    break;
                }
                if !lits.contains(&l) {
                    lits.push(l);
                }
            }
            if taut {
                continue;
            }
            if lits.is_empty() {
                s.clauses.push(Clause { lits: Vec::new(), learnt: false });
                continue;
            }
            let ci = s.clauses.len() as i32;
            if lits.len() == 1 {
                // 单位子句：0 层入队；若与既有赋值矛盾则标记初始冲突
                s.clauses.push(Clause { lits, learnt: false });
                let l = s.clauses[ci as usize].lits[0];
                let v = l.abs() as usize;
                let want: i8 = if l > 0 { 1 } else { -1 };
                if s.assign[v] == 0 {
                    s.enqueue(l, Some(ci));
                } else if s.assign[v] != want {
                    s.init_conflict = true;
                }
            } else {
                s.watches[lit_idx(lits[0])].push(ci);
                s.watches[lit_idx(lits[1])].push(ci);
                s.clauses.push(Clause { lits, learnt: false });
            }
        }
        s
    }

    #[inline]
    fn value_lit(&self, l: Lit) -> Option<bool> {
        let a = self.assign[l.abs() as usize];
        if a == 0 {
            None
        } else {
            Some((a == 1) == (l > 0))
        }
    }

    #[inline]
    fn decision_level(&self) -> u32 {
        self.trail_lim.len() as u32
    }

    fn enqueue(&mut self, l: Lit, reason: Option<i32>) {
        let v = l.abs() as usize;
        if self.assign[v] != 0 {
            return;
        }
        self.assign[v] = if l > 0 { 1 } else { -1 };
        self.level[v] = self.decision_level();
        self.reason[v] = reason.unwrap_or(-1);
        self.trail.push(l);
    }

    /// 单位传播。返回冲突子句下标（若有）。
    fn propagate(&mut self) -> Option<usize> {
        let mut confl: Option<usize> = None;
        'outer: while self.qhead < self.trail.len() {
            let p = self.trail[self.qhead];
            self.qhead += 1;
            let fl = -p;
            let wi = lit_idx(fl);
            let mut ws = std::mem::take(&mut self.watches[wi]);
            let mut i = 0usize;
            let mut out = 0usize;
            while i < ws.len() {
                let ci = ws[i];
                i += 1;
                let mut lits = match self.clauses.get(ci as usize) {
                    Some(c) => c.lits.clone(),
                    None => continue,
                };
                // 规范：lits[1] == fl
                if lits[0] == fl {
                    lits.swap(0, 1);
                }
                if lits[1] != fl {
                    // 监视表与实际不符 → 保守保留，不静默丢弃
                    ws[out] = ci;
                    out += 1;
                    self.clauses[ci as usize].lits = lits;
                    continue;
                }
                let first = lits[0];
                self.stats.propagations += 1;
                if self.value_lit(first) == Some(true) {
                    ws[out] = ci;
                    out += 1;
                    self.clauses[ci as usize].lits = lits;
                    continue;
                }
                // 寻找替代监视
                let mut moved = false;
                for k in 2..lits.len() {
                    let lk = lits[k];
                    if self.value_lit(lk) != Some(false) {
                        lits[1] = lk;
                        lits[k] = fl;
                        self.watches[lit_idx(lk)].push(ci);
                        moved = true;
                        break;
                    }
                }
                self.clauses[ci as usize].lits = lits;
                if moved {
                    continue; // 不再保留在 wi
                }
                // 无替代：成为单位或冲突
                ws[out] = ci;
                out += 1;
                match self.value_lit(first) {
                    Some(false) => {
                        // 冲突：保留剩余监视，停止
                        while i < ws.len() {
                            ws[out] = ws[i];
                            out += 1;
                            i += 1;
                        }
                        self.qhead = self.trail.len();
                        confl = Some(ci as usize);
                        break;
                    }
                    None => {
                        self.enqueue(first, Some(ci));
                    }
                    Some(true) => {}
                }
            }
            ws.truncate(out);
            self.watches[wi] = ws;
            if confl.is_some() {
                break 'outer;
            }
        }
        confl
    }

    fn var_bump(&mut self, v: usize) {
        self.activity[v] += self.activity_inc;
        if self.activity[v] > 1e100 {
            for a in self.activity.iter_mut() {
                *a *= 1e-100;
            }
            self.activity_inc *= 1e-100;
        }
    }

    /// 1-UIP 冲突分析。返回（学习子句, 回跳层）。
    fn analyze(&mut self, mut confl: usize) -> (Vec<Lit>, u32) {
        let mut learnt: Vec<Lit> = vec![0]; // 占位：asserting literal
        let mut path_c: i32 = 0;
        let mut p: Lit = 0;
        let mut index = self.trail.len();
        let mut first_round = true;
        for s in self.seen.iter_mut() {
            *s = false;
        }
        let mut bt: u32 = 0;
        loop {
            let lits = self.clauses[confl].lits.clone();
            self.clauses[confl].learnt = self.clauses[confl].learnt; // no-op，保持语义清晰
            let start = if first_round { 0 } else { 1 };
            for j in start..lits.len() {
                let q = lits[j];
                let v = q.abs() as usize;
                if !self.seen[v] && self.level[v] > 0 {
                    self.var_bump(v);
                    self.seen[v] = true;
                    if self.level[v] >= self.decision_level() {
                        path_c += 1;
                    } else {
                        learnt.push(q);
                    }
                }
            }
            // 从 trail 反向找下一个已标记文字
            let mut found = false;
            while index > 0 {
                index -= 1;
                let l = self.trail[index];
                if self.seen[l.abs() as usize] {
                    p = l;
                    found = true;
                    break;
                }
            }
            if !found {
                // 不应发生：说明推理图不完整。保守返回单文字断言（若 p 仍为 0 则为恒假）
                break;
            }
            let v = p.abs() as usize;
            self.seen[v] = false;
            let r = self.reason[v];
            path_c -= 1;
            first_round = false;
            if path_c <= 0 {
                break;
            }
            if r < 0 {
                // 决策变量出现而 path_c 仍 > 0：推理图异常，保守收束
                break;
            }
            confl = r as usize;
        }
        learnt[0] = -p;
        // 次级最高层放到 [1]
        if learnt.len() > 1 {
            let mut mi = 1;
            for i in 2..learnt.len() {
                if self.level[learnt[i].abs() as usize] > self.level[learnt[mi].abs() as usize] {
                    mi = i;
                }
            }
            learnt.swap(1, mi);
            bt = self.level[learnt[1].abs() as usize];
        }
        for l in learnt.iter() {
            self.seen[l.abs() as usize] = false;
        }
        (learnt, bt)
    }

    fn cancel_until(&mut self, level: u32) {
        if self.decision_level() <= level {
            return;
        }
        let lim = self.trail_lim[level as usize];
        for i in (lim..self.trail.len()).rev() {
            let l = self.trail[i];
            let v = l.abs() as usize;
            self.phase[v] = if l > 0 { 1 } else { -1 };
            self.assign[v] = 0;
            self.reason[v] = -1;
        }
        self.trail.truncate(lim);
        self.trail_lim.truncate(level as usize);
        self.qhead = self.trail.len();
    }

    fn pick_branch(&self) -> Option<Lit> {
        let mut best: i32 = -1;
        let mut best_act = f64::NEG_INFINITY;
        for v in 1..=self.nvars as usize {
            if self.assign[v] == 0 && self.activity[v] > best_act {
                best_act = self.activity[v];
                best = v as i32;
            }
        }
        if best < 0 {
            return None;
        }
        let v = best as usize;
        let positive = self.phase[v] > 0;
        Some(if positive { best } else { -best })
    }

    fn add_learnt(&mut self, lits: Vec<Lit>) -> i32 {
        let ci = self.clauses.len() as i32;
        if lits.len() >= 2 {
            self.watches[lit_idx(lits[0])].push(ci);
            self.watches[lit_idx(lits[1])].push(ci);
        }
        self.clauses.push(Clause { lits, learnt: true });
        self.stats.learnts += 1;
        ci
    }

    /// 学习子句库缩减：保留二元子句与活动度最高的部分。
    fn reduce_db(&mut self) {
        // 不删除二元子句（对传播价值高）；删除一半三元以上学习子句。
        let mut learnt_idx: Vec<usize> = (0..self.clauses.len())
            .filter(|&i| self.clauses[i].learnt && self.clauses[i].lits.len() > 2)
            .collect();
        if learnt_idx.len() < 5_000 {
            return;
        }
        // 简单策略：删一半（按下标，保留后加入的）
        learnt_idx.sort_unstable();
        let to_remove: std::collections::HashSet<usize> =
            learnt_idx.iter().take(learnt_idx.len() / 2).copied().collect();
        let mut new_clauses: Vec<Clause> = Vec::with_capacity(self.clauses.len());
        let mut remap: Vec<i32> = vec![-1; self.clauses.len()];
        for (i, c) in self.clauses.iter().enumerate() {
            if to_remove.contains(&i) {
                continue;
            }
            remap[i] = new_clauses.len() as i32;
            new_clauses.push(c.clone());
        }
        self.clauses = new_clauses;
        // 重建监视表与 reason（索引变了）
        for w in self.watches.iter_mut() {
            w.clear();
        }
        for (i, c) in self.clauses.iter().enumerate() {
            if c.lits.len() >= 2 {
                self.watches[lit_idx(c.lits[0])].push(i as i32);
                self.watches[lit_idx(c.lits[1])].push(i as i32);
            }
        }
        for v in 1..=self.nvars as usize {
            let r = self.reason[v];
            if r >= 0 {
                self.reason[v] = remap[r as usize];
            }
        }
    }

    /// 求解。
    pub fn solve(&mut self) -> SolveResult {
        if self.init_conflict {
            return SolveResult::Unsat;
        }
        // 初始传播（单位子句已入队）
        if self.propagate().is_some() {
            return SolveResult::Unsat;
        }
        if self.clauses.iter().any(|c| c.lits.is_empty()) {
            return SolveResult::Unsat;
        }
        let mut restart_budget: u64 = 100;
        loop {
            if let Some(confl) = self.propagate() {
                self.stats.conflicts += 1;
                if self.stats.conflicts > self.max_conflicts {
                    return SolveResult::Unknown;
                }
                if self.decision_level() == 0 {
                    return SolveResult::Unsat;
                }
                let (learnt, bt) = self.analyze(confl);
                self.cancel_until(bt);
                if learnt.len() == 1 {
                    self.enqueue(learnt[0], None);
                } else {
                    let ci = self.add_learnt(learnt);
                    let l0 = self.clauses[ci as usize].lits[0];
                    self.enqueue(l0, Some(ci));
                }
                if self.stats.conflicts >= restart_budget {
                    self.stats.restarts += 1;
                    self.cancel_until(0);
                    restart_budget = restart_budget + restart_budget / 2 + 1;
                    if self.clauses.len() > self.max_learnts {
                        self.reduce_db();
                    }
                }
            } else {
                match self.pick_branch() {
                    Some(l) => {
                        self.stats.decisions += 1;
                        self.trail_lim.push(self.trail.len());
                        self.enqueue(l, None);
                    }
                    None => return SolveResult::Sat,
                }
            }
        }
    }

    /// 取模型：`model[v]` 为变量 v（1-based）的真值。
    pub fn model(&self) -> Vec<bool> {
        (0..=self.nvars as usize).map(|v| self.assign.get(v).copied() == Some(1)).collect()
    }

    /// 校验一个完整赋值是否满足全部子句（用于证明自检）。
    pub fn check_model(model: &[bool], cnf: &Cnf) -> bool {
        for c in &cnf.clauses {
            let ok = c.iter().any(|&l| {
                let v = l.abs() as usize;
                let val = model.get(v).copied().unwrap_or(false);
                (l > 0) == val
            });
            if !ok {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 穷举参考求解器（仅小变量数）。
    fn brute_force_sat(cnf: &Cnf) -> bool {
        let n = cnf.num_vars as usize;
        for m in 0u64..(1u64 << n) {
            let model: Vec<bool> = (0..=n).map(|v| v > 0 && (m >> (v - 1)) & 1 == 1).collect();
            if Solver::check_model(&model, cnf) {
                return true;
            }
        }
        false
    }

    fn solve_cnf(cnf: &Cnf) -> SolveResult {
        let mut s = Solver::new(cnf);
        s.solve()
    }

    #[test]
    fn trivial_sat() {
        let mut c = Cnf::new();
        let a = c.new_var();
        c.add_clause(vec![a]);
        assert_eq!(solve_cnf(&c), SolveResult::Sat);
    }

    #[test]
    fn trivial_unsat() {
        let mut c = Cnf::new();
        let a = c.new_var();
        c.add_clause(vec![a]);
        c.add_clause(vec![-a]);
        assert_eq!(solve_cnf(&c), SolveResult::Unsat);
    }

    #[test]
    fn empty_clause_is_unsat() {
        let mut c = Cnf::new();
        c.new_var();
        c.add_empty();
        assert_eq!(solve_cnf(&c), SolveResult::Unsat);
    }

    #[test]
    fn no_vars_no_clauses_is_sat() {
        let c = Cnf::new();
        assert_eq!(solve_cnf(&c), SolveResult::Sat);
    }

    #[test]
    fn xor_unsat_instance() {
        // a≠b, a=b 同时要求 → UNSAT
        let mut c = Cnf::new();
        let a = c.new_var();
        let b = c.new_var();
        c.add_clause(vec![a, b]);
        c.add_clause(vec![-a, -b]);
        c.add_clause(vec![a, -b]);
        c.add_clause(vec![-a, b]);
        assert_eq!(solve_cnf(&c), SolveResult::Unsat);
    }

    #[test]
    fn model_validates_when_sat() {
        let mut c = Cnf::new();
        let a = c.new_var();
        let b = c.new_var();
        let d = c.new_var();
        c.add_clause(vec![a, b]);
        c.add_clause(vec![-a, d]);
        c.add_clause(vec![-b, d]);
        let mut s = Solver::new(&c);
        assert_eq!(s.solve(), SolveResult::Sat);
        assert!(Solver::check_model(&s.model(), &c), "返回的模型必须满足全部子句");
    }

    /// 差分测试：随机 3-SAT 与穷举参考求解器逐一比对。
    /// 用确定性的 xorshift 序列，避免引入随机数依赖。
    #[test]
    fn differential_vs_brute_force_3sat() {
        let mut state: u64 = 0x2545F4914F6CDD1D;
        let mut next = |m: u64| -> u64 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state % m
        };
        for trial in 0..300 {
            let nv = 3 + (next(8) as i32); // 3..10
            let nc = (nv as u64 * 4) + next(6);
            let mut c = Cnf::new();
            for _ in 0..nv {
                c.new_var();
            }
            for _ in 0..nc {
                let mut cl = Vec::new();
                for _ in 0..3 {
                    let v = 1 + next(nv as u64) as i32;
                    let neg = next(2) == 1;
                    cl.push(if neg { -v } else { v });
                }
                c.add_clause(cl);
            }
            let expect = brute_force_sat(&c);
            let got = solve_cnf(&c);
            assert_ne!(got, SolveResult::Unknown, "trial {trial} 不应 Unknown");
            assert_eq!(
                got == SolveResult::Sat,
                expect,
                "trial {trial}: 变量 {nv} 子句 {nc} —— 求解器与穷举不一致"
            );
        }
    }

    /// 差分测试：随机 CNF（含不同子句长度）。
    #[test]
    fn differential_vs_brute_force_mixed_width() {
        let mut state: u64 = 0x9E3779B97F4A7C15;
        let mut next = |m: u64| -> u64 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state % m
        };
        for trial in 0..200 {
            let nv = 2 + (next(7) as i32);
            let nc = (nv as u64 * 3) + next(8);
            let mut c = Cnf::new();
            for _ in 0..nv {
                c.new_var();
            }
            for _ in 0..nc {
                let k = 1 + next(4);
                let mut cl = Vec::new();
                for _ in 0..k {
                    let v = 1 + next(nv as u64) as i32;
                    let neg = next(2) == 1;
                    let l = if neg { -v } else { v };
                    if !cl.contains(&l) {
                        cl.push(l);
                    }
                }
                c.add_clause(cl);
            }
            let expect = brute_force_sat(&c);
            let got = solve_cnf(&c);
            assert_ne!(got, SolveResult::Unknown, "trial {trial} 不应 Unknown");
            assert_eq!(got == SolveResult::Sat, expect, "trial {trial} 不一致");
        }
    }

    /// 鸽巢原理 PHP(n+1, n)：n+1 只鸽进 n 个巢 → UNSAT。
    /// 是 CDCL 的经典压力用例（需要大量学习子句）。
    #[test]
    fn pigeonhole_unsat() {
        for n in 2..6usize {
            let mut c = Cnf::new();
            // 变量 p[i][j]：i 鸽在 j 巢
            let var = |i: usize, j: usize| -> i32 { (i * n + j + 1) as i32 };
            for _ in 0..((n + 1) * n) {
                c.new_var();
            }
            // 每只鸽至少一个巢
            for i in 0..=n {
                let cl: Vec<i32> = (0..n).map(|j| var(i, j)).collect();
                c.add_clause(cl);
            }
            // 每个巢至多一只鸽
            for j in 0..n {
                for a in 0..=n {
                    for b in (a + 1)..=n {
                        c.add_clause(vec![-var(a, j), -var(b, j)]);
                    }
                }
            }
            assert_eq!(solve_cnf(&c), SolveResult::Unsat, "PHP({},{}) 应 UNSAT", n + 1, n);
        }
    }

    /// 大 UNIQUE-SAT 结构：链式蕴含 + 唯一解。
    #[test]
    fn chain_implication_sat() {
        let mut c = Cnf::new();
        let n = 50i32;
        for _ in 0..n {
            c.new_var();
        }
        c.add_clause(vec![1]);
        for v in 1..n {
            c.add_clause(vec![-v, v + 1]);
        }
        let mut s = Solver::new(&c);
        assert_eq!(s.solve(), SolveResult::Sat);
        let m = s.model();
        for v in 1..=n {
            assert!(m[v as usize], "链式蕴含应强制变量 {v} 为真");
        }
    }
}
