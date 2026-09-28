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

use std::collections::BinaryHeap;

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
    /// 变量 → VSIDS 活动度（定点 u64，便于放入二叉堆；f64 无 Ord）
    activity: Vec<u64>,
    activity_inc: u64,
    /// 决策候选最大堆：(活动度, 变量)。允许陈旧重复条目，出堆时校验。
    heap: BinaryHeap<(u64, u32)>,
    /// 变量是否已在堆中（避免重复插入导致堆无限膨胀）
    in_heap: Vec<bool>,
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
    /// 触发学习子句库缩减的最少可删子句数（测试中调 0 可强制走缩减路径）。
    pub reduce_threshold: usize,
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
            activity: vec![0; (n + 1) as usize],
            activity_inc: 1,
            heap: BinaryHeap::new(),
            in_heap: vec![false; (n + 1) as usize],
            watches: vec![Vec::new(); ((n as usize) + 1) * 2],
            trail: Vec::new(),
            trail_lim: Vec::new(),
            qhead: 0,
            seen: vec![false; (n + 1) as usize],
            init_conflict: false,
            max_conflicts: 2_000_000,
            max_learnts: 60_000,
            reduce_threshold: 5_000,
            stats: SatStats::default(),
        };
        // 预填充决策堆：所有变量初始活动度 0，必须显式入堆，
        // 否则 pick_branch 会退化为 O(n) 线性扫描（大实例上极慢）。
        for v in 1..=n as usize {
            s.insert_var(v);
        }
        // 载入子句
        for c in &cnf.clauses {            if c.is_empty() {
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
                let ciu = ci as usize;
                let len = match self.clauses.get(ciu) {
                    Some(c) => c.lits.len(),
                    None => continue,
                };
                // 只取两个监视文字（**不克隆整条子句** —— 这是传播热路径）
                let mut first = self.clauses[ciu].lits[0];
                let mut second = self.clauses[ciu].lits[1];
                if first == fl {
                    std::mem::swap(&mut first, &mut second);
                }
                if second != fl {
                    // 监视表与实际不符 → 保守保留，不静默丢弃
                    ws[out] = ci;
                    out += 1;
                    continue;
                }
                self.stats.propagations += 1;
                if self.value_lit(first) == Some(true) {
                    // 规范化：确保 lits[1] == fl
                    self.clauses[ciu].lits[0] = first;
                    self.clauses[ciu].lits[1] = second;
                    ws[out] = ci;
                    out += 1;
                    continue;
                }
                // 寻找替代监视（只读扫描）
                let mut repl: Option<(usize, Lit)> = None;
                for k in 2..len {
                    let lk = self.clauses[ciu].lits[k];
                    if self.value_lit(lk) != Some(false) {
                        repl = Some((k, lk));
                        break;
                    }
                }
                if let Some((k, lk)) = repl {
                    self.clauses[ciu].lits[0] = first;
                    self.clauses[ciu].lits[1] = lk;
                    self.clauses[ciu].lits[k] = second; // second == fl
                    self.watches[lit_idx(lk)].push(ci);
                    continue; // 不再保留在 wi
                }
                // 无替代：成为单位或冲突
                self.clauses[ciu].lits[0] = first;
                self.clauses[ciu].lits[1] = second;
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
        if self.activity[v] > (1u64 << 60) {
            // 定点溢出保护：整体右移，保持相对次序
            for a in self.activity.iter_mut() {
                *a >>= 1;
            }
            self.activity_inc = (self.activity_inc >> 1).max(1);
        }
    }

    /// 把变量插入决策堆（若不在堆中）。
    fn insert_var(&mut self, v: usize) {
        if self.assign[v] == 0 && !self.in_heap[v] {
            self.heap.push((self.activity[v], v as u32));
            self.in_heap[v] = true;
        }
    }

    /// VSIDS 衰减：增量增长等价于活动度相对衰减。
    fn decay(&mut self) {
        self.activity_inc += self.activity_inc / 20 + 1;
    }

    /// 1-UIP 冲突分析。返回（学习子句, 回跳层）。
    ///
    /// 返回 `None` 表示推理图无法收敛（内部不变量被破坏）——
    /// 调用方必须 **fail-closed**（返回 `Unknown`），绝不能猜一个结论。
    fn analyze(&mut self, mut confl: usize) -> Option<(Vec<Lit>, u32)> {
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
                // 推理图不完整：不能猜结论，交给调用方 fail-closed。
                return None;
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
                // 决策变量出现而 path_c 仍 > 0：推理图异常，fail-closed
                return None;
            }
            confl = r as usize;
        }
        if p == 0 {
            // 未能定位断言文字：绝不允许写出 `-0` 这样的非法文字
            return None;
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
        Some((learnt, bt))
    }

    fn cancel_until(&mut self, level: u32) {
        if self.decision_level() <= level {
            return;
        }
        let lim = self.trail_lim[level as usize];
        let mut freed: Vec<usize> = Vec::new();
        for i in (lim..self.trail.len()).rev() {
            let l = self.trail[i];
            let v = l.abs() as usize;
            self.phase[v] = if l > 0 { 1 } else { -1 };
            self.assign[v] = 0;
            self.reason[v] = -1;
            freed.push(v);
        }
        self.trail.truncate(lim);
        self.trail_lim.truncate(level as usize);
        self.qhead = self.trail.len();
        // 重新变为未赋值的变量必须回到决策堆，否则会被漏掉
        for v in freed {
            self.insert_var(v);
        }
    }

    fn pick_branch(&mut self) -> Option<Lit> {
        while let Some((act, v)) = self.heap.pop() {
            let v = v as usize;
            self.in_heap[v] = false;
            if self.assign[v] != 0 {
                continue; // 已赋值，丢弃（取消赋值时会重新插入）
            }
            if act != self.activity[v] {
                // 陈旧条目：用当前活动度重新入堆再比较
                self.insert_var(v);
                continue;
            }
            return Some(if self.phase[v] > 0 { v as i32 } else { -(v as i32) });
        }
        // 兜底：堆空但仍存在未赋值变量（正常流程不应到达）
        for v in 1..=self.nvars as usize {
            if self.assign[v] == 0 {
                return Some(if self.phase[v] > 0 { v as i32 } else { -(v as i32) });
            }
        }
        None
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

    /// 学习子句库缩减：保留二元子句、**当前作为蕴含原因的子句**，以及后加入的一半。
    ///
    /// 删除仍是 `reason` 的子句会破坏后续冲突分析 —— 可能推出错误的 UNSAT（假证明）。
    /// 因此这些子句**必须**保留。
    fn reduce_db(&mut self) {
        let mut keep_reason: std::collections::HashSet<usize> = std::collections::HashSet::new();
        for v in 1..=self.nvars as usize {
            let r = self.reason[v];
            if r >= 0 {
                keep_reason.insert(r as usize);
            }
        }
        let mut learnt_idx: Vec<usize> = (0..self.clauses.len())
            .filter(|&i| {
                self.clauses[i].learnt
                    && self.clauses[i].lits.len() > 2
                    && !keep_reason.contains(&i)
            })
            .collect();
        if learnt_idx.len() < self.reduce_threshold {
            return;
        }
        // 简单策略：删一半（按下标，保留后加入的）
        learnt_idx.sort_unstable();
        let to_remove: std::collections::HashSet<usize> =
            learnt_idx.iter().take(learnt_idx.len() / 2).copied().collect();
        let mut new_clauses: Vec<Clause> = Vec::with_capacity(self.clauses.len());
        let mut remap: Vec<i32> = vec![-1; self.clauses.len()];
        // 移动而非克隆（`mem::take` + into_iter），避免缩减时的大规模分配
        for (i, c) in std::mem::take(&mut self.clauses).into_iter().enumerate() {
            if to_remove.contains(&i) {
                continue;
            }
            remap[i] = new_clauses.len() as i32;
            new_clauses.push(c);
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
                let (learnt, bt) = match self.analyze(confl) {
                    Some(x) => x,
                    // 冲突分析无法收敛 → fail-closed，绝不冒充 UNSAT/SAT
                    None => return SolveResult::Unknown,
                };
                self.cancel_until(bt);
                if learnt.len() == 1 {
                    self.enqueue(learnt[0], None);
                } else {
                    let ci = self.add_learnt(learnt);
                    let l0 = self.clauses[ci as usize].lits[0];
                    self.enqueue(l0, Some(ci));
                }
                self.decay();
                if self.stats.conflicts >= restart_budget {
                    self.stats.restarts += 1;
                    self.cancel_until(0);
                    // 几何增长的重启预算（实测：加上限反而变慢，故不设上限）
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

    /// 强制走学习子句库缩减路径（`reduce_threshold = 0`），
    /// 结论必须与穷举一致 —— 缩减后 `reason` 重映射若出错会直接产生假 UNSAT。
    #[test]
    fn differential_with_db_reduction_enabled() {
        let mut state: u64 = 0xDEADBEEFCAFEBABE;
        let mut next = |m: u64| -> u64 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state % m
        };
        for trial in 0..120 {
            let nv = 4 + (next(7) as i32);
            let nc = (nv as u64 * 6) + next(10);
            let mut c = Cnf::new();
            for _ in 0..nv {
                c.new_var();
            }
            for _ in 0..nc {
                let k = 2 + next(3);
                let mut cl = Vec::new();
                for _ in 0..k {
                    let v = 1 + next(nv as u64) as i32;
                    let l = if next(2) == 1 { -v } else { v };
                    if !cl.contains(&l) {
                        cl.push(l);
                    }
                }
                c.add_clause(cl);
            }
            let mut s = Solver::new(&c);
            s.reduce_threshold = 0; // 每次重启都触发缩减
            let got = s.solve();
            let expect = brute_force_sat(&c);
            assert_ne!(got, SolveResult::Unknown, "trial {trial} 不应 Unknown");
            assert_eq!(got == SolveResult::Sat, expect, "trial {trial}: 缩减路径下结论不一致");
        }
    }

    /// 鸽巢原理在强制缩减下仍必须报 UNSAT。
    #[test]
    fn pigeonhole_unsat_with_db_reduction() {
        for n in 3..6usize {
            let mut c = Cnf::new();
            let var = |i: usize, j: usize| -> i32 { (i * n + j + 1) as i32 };
            for _ in 0..((n + 1) * n) {
                c.new_var();
            }
            for i in 0..=n {
                c.add_clause((0..n).map(|j| var(i, j)).collect());
            }
            for j in 0..n {
                for a in 0..=n {
                    for b in (a + 1)..=n {
                        c.add_clause(vec![-var(a, j), -var(b, j)]);
                    }
                }
            }
            let mut s = Solver::new(&c);
            s.reduce_threshold = 0;
            assert_eq!(s.solve(), SolveResult::Unsat, "PHP 缩减路径下应 UNSAT");
        }
    }

    /// 大规模随机差分（3000 组）：覆盖单位子句、二元、长子句、重复文字、
    /// 恒真子句、互补单位子句等形状，并与穷举参考求解器比对；
    /// 同时强制打开学习子句库缩减路径。
    #[test]
    fn differential_large_scale_mixed_shapes() {
        let mut state: u64 = 0x123456789ABCDEF;
        let mut next = |m: u64| -> u64 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state % m
        };
        for trial in 0..3000 {
            let nv = 1 + (next(8) as i32); // 1..8
            let nc = next(14);
            let mut c = Cnf::new();
            for _ in 0..nv {
                c.new_var();
            }
            for _ in 0..nc {
                let shape = next(5);
                let mut cl: Vec<i32> = Vec::new();
                let k = match shape {
                    0 => 1,           // 单位
                    1 => 2,           // 二元
                    2 => 3,           // 三元
                    3 => 1 + next(6), // 长子句
                    _ => 0,           // 空子句（偶发 UNSAT）
                };
                for _ in 0..k {
                    let v = 1 + next(nv as u64) as i32;
                    let l = if next(2) == 1 { -v } else { v };
                    if !cl.contains(&l) {
                        cl.push(l);
                    }
                }
                c.add_clause(cl);
            }
            let expect = brute_force_sat(&c);
            let mut s = Solver::new(&c);
            // 一半用例强制走缩减路径
            if trial % 2 == 0 {
                s.reduce_threshold = 0;
            }
            let got = s.solve();
            assert_ne!(got, SolveResult::Unknown, "trial {trial} 不应 Unknown");
            assert_eq!(got == SolveResult::Sat, expect, "trial {trial}: nv={nv} nc={nc} 结论不一致");
            if got == SolveResult::Sat {
                assert!(
                    Solver::check_model(&s.model(), &c),
                    "trial {trial}: 返回的模型不满足公式"
                );
            }
        }
    }

    /// 互补单位子句 + 空子句混合的确定性边界用例。
    #[test]
    fn unit_conflicts_and_empty_clauses() {
        // [1] [-1] → UNSAT
        let mut c = Cnf::new();
        c.new_var();
        c.add_unit(1);
        c.add_unit(-1);
        assert_eq!(solve_cnf(&c), SolveResult::Unsat);
        // [1] [1] [-1] → UNSAT（重复单位）
        let mut c = Cnf::new();
        c.new_var();
        c.add_unit(1);
        c.add_unit(1);
        c.add_unit(-1);
        assert_eq!(solve_cnf(&c), SolveResult::Unsat);
        // 空子句与单位并存 → UNSAT
        let mut c = Cnf::new();
        c.new_var();
        c.add_unit(1);
        c.add_empty();
        assert_eq!(solve_cnf(&c), SolveResult::Unsat);
    }
}
