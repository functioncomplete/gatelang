//! 形式化验证内核（SAT 后端）。
//!
//! 把「验证」从**穷举仿真**升级为**形式化证明**：
//! 规格表达式被综合为门级电路，与目标电路拼成同一网表，取反后交给 CDCL 求解器。
//!
//! - `UNSAT` ⇒ **对全部输入成立**（不是"测了 2^20 个没发现问题"）
//! - `SAT`   ⇒ 反例（具体输入向量，可复现）
//! - `Unknown` ⇒ 触及资源上限，**不冒充结论**（fail-closed）
//!
//! ## 语义保真（关键）
//!
//! 规格表达式按 **u128 回绕语义**综合，与 `spec::eval_spec` 逐位一致：
//! 端口值先零扩展进 128 位空间再运算，布尔运算（`&&`/`||`/`!`）先把操作数
//! 归约为 0/1。这保证了"形式化证明的结论"与"既有穷举验证器的结论"在
//! 可判定范围内完全一致 —— 二者互为交叉验证。
//!
//! ## 已验证的信任边界
//!
//! 本模块的结论依赖于：(1) `cnf.rs` 的 Tseitin 编码定义性正确（有穷举一致性测试）；
//! (2) `sat.rs` 的 CDCL 求解器正确（有穷举差分测试）；(3) 本模块的综合与
//! 语义保真（有与 `verify.rs` 的交叉验证测试）。

use std::collections::HashMap;

use crate::ast::{Decl, Spec};
use crate::cnf::{encode_netlist, Cnf, Lit};
use crate::lower::Compiled;
use crate::netlist::{Netlist, Sig};
use crate::sat::{SatStats, SolveResult, Solver};
use crate::spec::{parse_spec, SpecExpr};

/// 规格算术的机器宽度。与 `spec::eval_spec` 的 `u128` 表示一致。
pub const BV_W: usize = 128;

/// 综合门数预算（超出即 fail-closed，绝不以近似结果冒充证明）。
const SYNTH_GATE_BUDGET: usize = 400_000;

/// 单个证明义务的判定。
#[derive(Debug, Clone)]
pub enum Verdict {
    /// 对满足前置条件的全部输入成立（UNSAT）。
    Proven,
    /// 存在反例（SAT），附具体输入向量。
    Refuted { input: String },
    /// 触及资源上限，未得出结论。
    Unknown { reason: String },
}

impl Verdict {
    pub fn is_proven(&self) -> bool {
        matches!(self, Verdict::Proven)
    }
}

/// 单条义务的证明报告。
#[derive(Debug, Clone)]
pub struct Obligation {
    /// 义务名（如 `postcondition` / `invariant`）
    pub kind: String,
    /// 人类可读的命题
    pub statement: String,
    pub verdict: Verdict,
    pub stats: SatStats,
    /// 编码规模（变量数、子句数）
    pub cnf_vars: i32,
    pub cnf_clauses: usize,
}

/// 整个 spec 的证明报告。
#[derive(Debug, Clone, Default)]
pub struct ProveReport {
    pub circuit: String,
    pub obligations: Vec<Obligation>,
    /// 前置条件本身不可满足（无有效输入域）
    pub pre_unsatisfiable: bool,
}

impl ProveReport {
    pub fn all_proven(&self) -> bool {
        !self.pre_unsatisfiable
            && !self.obligations.is_empty()
            && self.obligations.iter().all(|o| o.verdict.is_proven())
    }
    pub fn failed(&self) -> Vec<&Obligation> {
        self.obligations.iter().filter(|o| !o.verdict.is_proven()).collect()
    }
}

/* ============================ 位向量综合 ============================ */

/// 综合出的位向量：恒为 `BV_W` 位（LSB 在前）。
#[derive(Debug, Clone)]
struct Bv {
    sigs: Vec<Sig>,
    /// 已知常量值（用于常量折叠与廉价 `*` / `%` 路径）
    konst: Option<u128>,
    /// 值**至多为 1**（布尔结果）：`*` 可退化为 mux，`%` 恒为 0
    bit: bool,
}

struct Synth<'a> {
    nl: &'a mut Netlist,
    env: HashMap<String, Vec<Sig>>,
    budget_hit: bool,
}

impl<'a> Synth<'a> {
    fn new(nl: &'a mut Netlist, env: HashMap<String, Vec<Sig>>) -> Self {
        Synth { nl, env, budget_hit: false }
    }

    fn over_budget(&self) -> bool {
        self.budget_hit || self.nl.gates.len() > SYNTH_GATE_BUDGET
    }

    fn konst(&mut self, v: u128) -> Bv {
        let sigs = (0..BV_W).map(|i| self.nl.add_const(((v >> i) & 1) as u8)).collect();
        Bv { sigs, konst: Some(v), bit: v <= 1 }
    }

    fn bool_bv(&mut self, sig: Sig) -> Bv {
        let mut sigs = Vec::with_capacity(BV_W);
        sigs.push(sig);
        for _ in 1..BV_W {
            sigs.push(self.nl.add_const(0));
        }
        Bv { sigs, konst: None, bit: true }
    }

    /// `a - b`（mod 2^W），即 `a + ~b + 1`（低位结果与拆分无关）。
    fn sub(&mut self, a: &Bv, b: &Bv) -> Bv {
        if let (Some(x), Some(y)) = (a.konst, b.konst) {
            return self.konst(x.wrapping_sub(y));
        }
        let nb: Vec<Sig> = b.sigs.iter().map(|&s| self.nl.not(s)).collect();
        let one = self.nl.add_const(1);
        let (r, _) = self.nl.adder_cin(&a.sigs, &nb, one);
        Bv { sigs: r, konst: None, bit: false }
    }

    fn add(&mut self, a: &Bv, b: &Bv) -> Bv {
        if let (Some(x), Some(y)) = (a.konst, b.konst) {
            return self.konst(x.wrapping_add(y));
        }
        let (r, _) = self.nl.adder(&a.sigs, &b.sigs);
        Bv { sigs: r, konst: None, bit: false }
    }

    /// 左移 k 位（k < BV_W）。
    fn shl(&mut self, a: &Bv, k: usize) -> Bv {
        if k == 0 {
            return a.clone();
        }
        let zero = self.nl.add_const(0);
        let mut sigs: Vec<Sig> = Vec::with_capacity(BV_W);
        for _ in 0..k {
            sigs.push(zero);
        }
        for i in 0..(BV_W - k) {
            sigs.push(a.sigs[i]);
        }
        Bv { sigs, konst: None, bit: false }
    }

    /// `a * b`（mod 2^W）。廉价路径：操作数为常量 / 至多 1 的值。
    fn mul(&mut self, a: &Bv, b: &Bv) -> Result<Bv, String> {
        if let (Some(x), Some(y)) = (a.konst, b.konst) {
            return Ok(self.konst(x.wrapping_mul(y)));
        }
        // x * bit  →  mux(0, x, bit)
        if b.bit && b.sigs.len() == BV_W {
            return Ok(self.mux_zero(a, b.sigs[0]));
        }
        if a.bit && a.sigs.len() == BV_W {
            let s = a.sigs[0];
            let x = b.clone();
            return Ok(self.mux_zero(&x, s));
        }
        // 常量乘数：只累加置位部分积
        if let Some(c) = b.konst {
            return self.mul_const(a, c);
        }
        if let Some(c) = a.konst {
            let x = b.clone();
            return self.mul_const(&x, c);
        }
        // 通用移位加法（受预算约束）
        if self.over_budget() {
            return Err("乘法综合超出预算".into());
        }
        let mut acc = self.konst(0);
        for i in 0..BV_W {
            // 部分积 = a AND b[i]
            let bi = b.sigs[i];
            let partial: Vec<Sig> = a.sigs.iter().map(|&aj| self.nl.and(aj, bi)).collect();
            let p = Bv { sigs: partial, konst: None, bit: false };
            let shifted = self.shl(&p, i);
            acc = self.add(&acc, &shifted);
            if self.over_budget() {
                return Err("乘法综合超出预算".into());
            }
        }
        Ok(acc)
    }

    fn mul_const(&mut self, a: &Bv, c: u128) -> Result<Bv, String> {
        if c == 0 {
            return Ok(self.konst(0));
        }
        let mut acc = self.konst(0);
        for i in 0..BV_W {
            if (c >> i) & 1 == 1 {
                let shifted = self.shl(a, i);
                acc = self.add(&acc, &shifted);
                if self.over_budget() {
                    return Err("常量乘法综合超出预算".into());
                }
            }
        }
        Ok(acc)
    }

    /// `sel ? x : 0`（逐位 AND，1 门/位）。
    fn mux_zero(&mut self, x: &Bv, sel: Sig) -> Bv {
        let sigs = x.sigs.iter().map(|&s| self.nl.and(s, sel)).collect();
        Bv { sigs, konst: if x.konst == Some(0) { Some(0) } else { None }, bit: false }
    }

    /// `a % b`，语义与 `eval_spec` 的 `Mod` 一致：`b == 0` 时结果为 `0`。
    fn modulo(&mut self, a: &Bv, b: &Bv) -> Result<Bv, String> {
        if let (Some(_), Some(0)) = (a.konst, b.konst) {
            return Ok(self.konst(0));
        }
        if let (Some(x), Some(y)) = (a.konst, b.konst) {
            return Ok(self.konst(if y == 0 { 0 } else { x % y }));
        }
        // 除数至多 1 → 结果恒 0（1%1=0, x%0→0）
        if b.bit {
            return Ok(self.konst(0));
        }
        // 常量除数
        if let Some(c) = b.konst {
            if c == 0 {
                return Ok(self.konst(0));
            }
            if c.is_power_of_two() {
                // 2^k → 取低 k 位
                let k = c.trailing_zeros() as usize;
                let k = k.min(BV_W);
                let mut sigs: Vec<Sig> = a.sigs[..k].to_vec();
                let n = sigs.len();
                for _ in n..BV_W {
                    sigs.push(self.nl.add_const(0));
                }
                return Ok(Bv { sigs, konst: None, bit: false });
            }
            return Err(format!("模常量 {c} 非二次幂：综合代价过高（fail-closed）"));
        }
        Err("非常量除数的取模需要除法器，超出原型能力（fail-closed）".into())
    }

    /// `a == b`（1 位信号）。
    fn eq(&mut self, a: &Bv, b: &Bv) -> Sig {
        if let (Some(x), Some(y)) = (a.konst, b.konst) {
            return self.nl.add_const((x == y) as u8);
        }
        let mut acc = self.nl.add_const(1);
        for i in 0..BV_W {
            let x = self.nl.xor(a.sigs[i], b.sigs[i]);
            let xn = self.nl.not(x);
            acc = self.nl.and(acc, xn);
        }
        acc
    }

    /// `a < b`（无符号，1 位信号）。
    ///
    /// 用**单条进位链**算 `a + ~b + 1`：进位输出 == 1 ⟺ `a >= b`。
    /// （拆成两次加法会丢进位，导致 `lt(3,2)` 误判为真。）
    fn lt(&mut self, a: &Bv, b: &Bv) -> Sig {
        if let (Some(x), Some(y)) = (a.konst, b.konst) {
            return self.nl.add_const((x < y) as u8);
        }
        let nb: Vec<Sig> = b.sigs.iter().map(|&s| self.nl.not(s)).collect();
        let one = self.nl.add_const(1);
        let (_, carry) = self.nl.adder_cin(&a.sigs, &nb, one);
        self.nl.not(carry)
    }

    /// OR 归约：值是否非零。
    fn to_bool(&mut self, a: &Bv) -> Sig {
        if let Some(k) = a.konst {
            return self.nl.add_const((k != 0) as u8);
        }
        let mut acc = a.sigs[0];
        for i in 1..BV_W {
            acc = self.nl.or(acc, a.sigs[i]);
        }
        acc
    }

    /// 综合规格表达式。
    fn encode(&mut self, e: &SpecExpr) -> Result<Bv, String> {
        if self.over_budget() {
            return Err("规格综合超出预算（fail-closed）".into());
        }
        match e {
            SpecExpr::Num(n) => Ok(self.konst(*n)),
            SpecExpr::Var(name) => {
                let sigs = self
                    .env
                    .get(name)
                    .cloned()
                    .ok_or_else(|| format!("spec 变量未绑定到任何端口: {name}"))?;
                // 零扩展到 128 位
                let w = sigs.len();
                if w > BV_W {
                    return Err(format!("端口 {name} 位宽 {w} > {BV_W}"));
                }
                let mut out = Vec::with_capacity(BV_W);
                for i in 0..BV_W {
                    if i < w {
                        out.push(sigs[i]);
                    } else {
                        out.push(self.nl.add_const(0));
                    }
                }
                // 1 位端口的值至多为 1：标记 bit 让 `x * port` / `x % port` 走廉价路径
                Ok(Bv { sigs: out, konst: None, bit: w == 1 })
            }
            SpecExpr::Add(x, y) => {
                let a = self.encode(x)?;
                let b = self.encode(y)?;
                Ok(self.add(&a, &b))
            }
            SpecExpr::Sub(x, y) => {
                let a = self.encode(x)?;
                let b = self.encode(y)?;
                Ok(self.sub(&a, &b))
            }
            SpecExpr::Mul(x, y) => {
                let a = self.encode(x)?;
                let b = self.encode(y)?;
                self.mul(&a, &b)
            }
            SpecExpr::Mod(x, y) => {
                let a = self.encode(x)?;
                let b = self.encode(y)?;
                self.modulo(&a, &b)
            }
            SpecExpr::Eq(x, y) => {
                let a = self.encode(x)?;
                let b = self.encode(y)?;
                let s = self.eq(&a, &b);
                Ok(self.bool_bv(s))
            }
            SpecExpr::Ne(x, y) => {
                let a = self.encode(x)?;
                let b = self.encode(y)?;
                let s = self.eq(&a, &b);
                let s = self.nl.not(s);
                Ok(self.bool_bv(s))
            }
            SpecExpr::Lt(x, y) => {
                let a = self.encode(x)?;
                let b = self.encode(y)?;
                let s = self.lt(&a, &b);
                Ok(self.bool_bv(s))
            }
            SpecExpr::Gt(x, y) => {
                let a = self.encode(x)?;
                let b = self.encode(y)?;
                let s = self.lt(&b, &a);
                Ok(self.bool_bv(s))
            }
            SpecExpr::Le(x, y) => {
                let a = self.encode(x)?;
                let b = self.encode(y)?;
                let s = self.lt(&b, &a);
                let s = self.nl.not(s);
                Ok(self.bool_bv(s))
            }
            SpecExpr::Ge(x, y) => {
                let a = self.encode(x)?;
                let b = self.encode(y)?;
                let s = self.lt(&a, &b);
                let s = self.nl.not(s);
                Ok(self.bool_bv(s))
            }
            // 布尔运算：先把操作数归约为 0/1，再按位逻辑
            SpecExpr::And(x, y) => {
                let a = self.encode(x)?;
                let b = self.encode(y)?;
                let ba = self.to_bool(&a);
                let bb = self.to_bool(&b);
                let s = self.nl.and(ba, bb);
                Ok(self.bool_bv(s))
            }
            SpecExpr::Or(x, y) => {
                let a = self.encode(x)?;
                let b = self.encode(y)?;
                let ba = self.to_bool(&a);
                let bb = self.to_bool(&b);
                let s = self.nl.or(ba, bb);
                Ok(self.bool_bv(s))
            }
            SpecExpr::Not(x) => {
                let a = self.encode(x)?;
                let ba = self.to_bool(&a);
                let s = self.nl.not(ba);
                Ok(self.bool_bv(s))
            }
        }
    }
}

/* ============================ 证明编排 ============================ */

/// 构造端口的信号环境：端口名 → 信号（LSB 在前）。
fn port_env(nl: &Netlist, compiled: &Compiled) -> Result<HashMap<String, Vec<Sig>>, String> {
    let (inputs, outputs) = match compiled {
        Compiled::Combinational { inputs, outputs, .. } => (inputs, outputs),
        Compiled::State { .. } => return Err("形式化证明当前仅支持组合电路".into()),
    };
    let mut env = HashMap::new();
    for p in inputs {
        let w = p.width.bits();
        let mut sigs = Vec::with_capacity(w as usize);
        for i in 0..w {
            let key = format!("{}_{}", p.name, i);
            let s = nl.inputs.get(&key).ok_or_else(|| format!("缺少输入位 {key}"))?;
            sigs.push(*s);
        }
        env.insert(p.name.clone(), sigs);
    }
    for p in outputs {
        let w = p.width.bits();
        let mut sigs = Vec::with_capacity(w as usize);
        for i in 0..w {
            let key = format!("{}_{}", p.name, i);
            let s = nl.outputs.get(&key).ok_or_else(|| format!("缺少输出位 {key}"))?;
            sigs.push(*s);
        }
        env.insert(p.name.clone(), sigs);
    }
    Ok(env)
}

/// 把模型中的一段位按 LSB-first 渲染为 `0x…` 十六进制。
///
/// **不做 u128 截断** —— 256 位端口的反例必须完整显示，否则会误导排障。
fn render_bits(
    var_of: impl Fn(u32) -> Option<crate::cnf::Var>,
    model: &[bool],
    width: u32,
) -> String {
    let mut nibbles: Vec<u8> = Vec::new();
    let mut bit = 0u32;
    while bit < width {
        let mut v = 0u8;
        for k in 0..4 {
            if bit + k < width {
                if let Some(var) = var_of(bit + k) {
                    if model.get(var as usize).copied().unwrap_or(false) {
                        v |= 1 << k;
                    }
                }
            }
        }
        nibbles.push(v);
        bit += 4;
    }
    while nibbles.len() > 1 && *nibbles.last().unwrap() == 0 {
        nibbles.pop();
    }
    let mut s = String::from("0x");
    for n in nibbles.iter().rev() {
        s.push(std::char::from_digit(*n as u32, 16).unwrap());
    }
    s
}

/// 从模型还原输入参数值（支持任意位宽）。
fn extract_inputs(
    compiled: &Compiled,
    enc: &crate::cnf::Encoded,
    model: &[bool],
) -> String {
    let params = match compiled {
        Compiled::Combinational { inputs, .. } => inputs,
        _ => return String::new(),
    };
    let mut parts = Vec::new();
    for p in params {
        let hex = render_bits(|i| enc.var_of_input_bit(&p.name, i), model, p.width.bits());
        parts.push(format!("{}={}", p.name, hex));
    }
    parts.join(", ")
}

/// 对一条义务做形式化判定。
///
/// 语义：在 `pre` 成立的全部输入上，`goal` 是否恒真？
/// 编码为 `pre ∧ ¬goal` 的可满足性：UNSAT ⇒ 已证明。
fn discharge(
    compiled: &Compiled,
    kind: &str,
    statement: &str,
    pre: Option<&SpecExpr>,
    goal: &SpecExpr,
) -> Result<Obligation, String> {
    let base_nl = match compiled {
        Compiled::Combinational { netlist, .. } => netlist,
        Compiled::State { .. } => return Err("形式化证明当前仅支持组合电路".into()),
    };
    // 复制网表，把规格综合进去（共享同一组输入信号）
    let mut nl = base_nl.clone();
    let env = port_env(base_nl, compiled)?;
    let mut synth = Synth::new(&mut nl, env);

    let goal_bv = synth.encode(goal)?;
    let goal_sig = synth.to_bool(&goal_bv);
    let pre_sig = match pre {
        Some(p) => {
            let bv = synth.encode(p)?;
            synth.to_bool(&bv)
        }
        None => synth.nl.add_const(1),
    };
    if synth.over_budget() {
        return Ok(Obligation {
            kind: kind.to_string(),
            statement: statement.to_string(),
            verdict: Verdict::Unknown { reason: "规格综合超出预算".into() },
            stats: SatStats::default(),
            cnf_vars: 0,
            cnf_clauses: 0,
        });
    }

    let enc = encode_netlist(&nl);
    let mut cnf = enc.cnf.clone();

    // 前置条件不可满足检查：pre 单独是否可满足？
    {
        let mut c = enc.cnf.clone();
        let pv = enc.var_of(pre_sig).ok_or("pre 信号无对应变量")?;
        c.add_unit(pv);
        let mut s = Solver::new(&c);
        if s.solve() == SolveResult::Unsat {
            return Ok(Obligation {
                kind: kind.to_string(),
                statement: statement.to_string(),
                verdict: Verdict::Unknown { reason: "前置条件恒不成立（输入域为空）".into() },
                stats: s.stats.clone(),
                cnf_vars: c.num_vars,
                cnf_clauses: c.clauses.len(),
            });
        }
    }

    // 主判定：pre ∧ ¬goal
    let pre_v = enc.var_of(pre_sig).ok_or("pre 信号无对应变量")?;
    let goal_v = enc.var_of(goal_sig).ok_or("goal 信号无对应变量")?;
    cnf.add_unit(pre_v);
    cnf.add_unit(-goal_v);
    cnf.validate()?;

    let mut solver = Solver::new(&cnf);
    let res = solver.solve();
    let stats = solver.stats.clone();
    let verdict = match res {
        SolveResult::Unsat => Verdict::Proven,
        SolveResult::Sat => {
            let model = solver.model();
            Verdict::Refuted { input: extract_inputs(compiled, &enc, &model) }
        }
        SolveResult::Unknown => Verdict::Unknown { reason: "求解器触及冲突上限".into() },
    };
    Ok(Obligation {
        kind: kind.to_string(),
        statement: statement.to_string(),
        verdict,
        stats,
        cnf_vars: cnf.num_vars,
        cnf_clauses: cnf.clauses.len(),
    })
}

/// 把规格表达式综合为门级电路（工具与测试用）。
///
/// 返回网表副本；表达式真值以一个 1 位信号给出，并登记为输出 `__spec__`，
/// 便于用 `sim::eval_netlist` 直接求值 —— 这是**综合语义保真**的可验证接口。
pub fn synthesize_expr(compiled: &Compiled, expr: &SpecExpr) -> Result<(Netlist, Sig), String> {
    let base_nl = match compiled {
        Compiled::Combinational { netlist, .. } => netlist,
        Compiled::State { .. } => return Err("仅支持组合电路".into()),
    };
    let mut nl = base_nl.clone();
    let env = port_env(base_nl, compiled)?;
    let mut synth = Synth::new(&mut nl, env);
    let bv = synth.encode(expr)?;
    let sig = synth.to_bool(&bv);
    if synth.over_budget() {
        return Err("规格综合超出预算".into());
    }
    nl.outputs.insert("__spec__".to_string(), sig);
    Ok((nl, sig))
}

/// 对单个 spec 执行形式化证明。
pub fn prove_spec(compiled: &Compiled, spec: &Spec) -> Result<ProveReport, String> {
    let mut rep = ProveReport { circuit: spec.name.clone(), obligations: Vec::new(), pre_unsatisfiable: false };

    // 与 verify.rs 对等：缺少 postcondition 时不得计为"通过"、也不得静默降级
    if spec.post.is_none() {
        return Err(format!("{}: 缺少 postcondition，无可证明内容", spec.name));
    }

    let pre = match &spec.pre {
        Some(p) => Some(parse_spec(p)?),
        None => None,
    };
    // 前置条件可满足性：单独判定一次，避免"输入域为空"被误报为已证明
    if let Some(pe) = &pre {
        let base_nl = match compiled {
            Compiled::Combinational { netlist, .. } => netlist,
            _ => return Err("仅支持组合电路".into()),
        };
        let mut nl = base_nl.clone();
        let env = port_env(base_nl, compiled)?;
        let mut synth = Synth::new(&mut nl, env);
        let pv = synth.encode(pe)?;
        let psig = synth.to_bool(&pv);
        if synth.over_budget() {
            return Err("前置条件综合超出预算".into());
        }
        let enc = encode_netlist(&nl);
        let mut c = enc.cnf.clone();
        let v = enc.var_of(psig).ok_or("pre 信号无变量")?;
        c.add_unit(v);
        let mut s = Solver::new(&c);
        if s.solve() == SolveResult::Unsat {
            rep.pre_unsatisfiable = true;
            return Ok(rep);
        }
    }

    if let Some(post) = &spec.post {
        let e = parse_spec(post)?;
        rep.obligations.push(discharge(compiled, "postcondition", post, pre.as_ref(), &e)?);
    }
    if let Some(inv) = &spec.invariant {
        let e = parse_spec(inv)?;
        // invariant == true 是平凡义务，跳过（与既有语料一致）
        if !matches!(e, SpecExpr::Num(1)) {
            rep.obligations.push(discharge(compiled, "invariant", inv, pre.as_ref(), &e)?);
        }
    }
    Ok(rep)
}

/// 找出与 spec 名字匹配的编译产物（与 verify.rs 的匹配规则一致）。
pub fn find_target<'a>(compiled: &'a [Compiled], spec: &Spec) -> Option<&'a Compiled> {
    let short = spec.name.rsplit('.').next().unwrap_or("");
    compiled
        .iter()
        .find(|c| match c {
            Compiled::Combinational { name, .. } | Compiled::State { name, .. } => {
                name == &spec.name || name.as_str() == short
            }
        })
        .or_else(|| {
            compiled.iter().find(|c| match c {
                Compiled::Combinational { name, .. } | Compiled::State { name, .. } => {
                    name.eq_ignore_ascii_case(short) || name.eq_ignore_ascii_case(&spec.name)
                }
            })
        })
}

/// 对一个文件里的全部 spec 做形式化证明。
pub fn prove_all(decls: &[Decl], compiled: &[Compiled]) -> Vec<Result<ProveReport, String>> {
    decls
        .iter()
        .filter_map(|d| match d {
            Decl::Spec(s) => Some(s),
            _ => None,
        })
        .map(|s| match find_target(compiled, s) {
            Some(t) => prove_spec(t, s),
            None => Err(format!("{}: 未找到对应声明", s.name)),
        })
        .collect()
}

/* ============================ 等价性（miter + SAT） ============================ */

/// 把 `src` 的子句按变量偏移并入 `dst`。
fn merge_with_offset(dst: &mut Cnf, src: &Cnf, offset: i32) -> Result<(), String> {
    for c in &src.clauses {
        let mut nc = Vec::with_capacity(c.len());
        for &l in c {
            let nl = if l > 0 { l + offset } else { l - offset };
            nc.push(nl);
        }
        dst.add_clause(nc);
    }
    // 变量数：dst 的 num_vars 由调用方保证已 ≥ offset + src.num_vars
    Ok(())
}

/// 用 SAT 证明两个组合电路在**全部输入**上等价（miter + UNSAT）。
///
/// 返回 `(是否等价, 反例描述)`。输入位宽不受 20 位穷举上限约束。
pub fn prove_equiv_sat(
    a: &Compiled,
    b: &Compiled,
) -> Result<(bool, Option<String>), String> {
    let (a_name, b_name) = (name_of(a), name_of(b));
    let (a_in, a_out, a_nl) = parts(a)?;
    let (b_in, b_out, b_nl) = parts(b)?;
    // 接口校验（与 equiv.rs 同规则，避免假等价）
    let a_in_ws: Vec<u32> = a_in.iter().map(|p| p.width.bits()).collect();
    let b_in_ws: Vec<u32> = b_in.iter().map(|p| p.width.bits()).collect();
    let a_out_ws: Vec<u32> = a_out.iter().map(|p| p.width.bits()).collect();
    let b_out_ws: Vec<u32> = b_out.iter().map(|p| p.width.bits()).collect();
    if a_in_ws != b_in_ws {
        return Ok((false, Some(format!("输入参数宽度不一致: {a_in_ws:?} vs {b_in_ws:?}"))));
    }
    if a_out_ws != b_out_ws {
        return Ok((false, Some(format!("输出宽度不匹配: {a_name}{a_out_ws:?} vs {b_name}{b_out_ws:?}"))));
    }
    if a_out.is_empty() {
        return Err("电路无输出，等价性无意义".into());
    }

    let ea = encode_netlist(a_nl);
    let eb = encode_netlist(b_nl);
    let mut cnf = Cnf::new();
    // A
    for _ in 0..ea.cnf.num_vars {
        cnf.new_var();
    }
    merge_with_offset(&mut cnf, &ea.cnf, 0)?;
    // B（偏移）
    let off = ea.cnf.num_vars;
    for _ in 0..eb.cnf.num_vars {
        cnf.new_var();
    }
    merge_with_offset(&mut cnf, &eb.cnf, off)?;

    // 输入等价：A 的输入变量 ↔ B 的输入变量
    for (i, p) in a_in.iter().enumerate() {
        let bp = &b_in[i];
        if p.width != bp.width {
            return Ok((false, Some("输入宽度不一致".into())));
        }
        for bit in 0..p.width.bits() {
            let va = ea.var_of_input_bit(&p.name, bit).ok_or("A 输入位缺失")?;
            let vb = eb.var_of_input_bit(&bp.name, bit).ok_or("B 输入位缺失")?;
            let vb = vb + off;
            // 等价：两子句
            cnf.add_clause(vec![-va, vb]);
            cnf.add_clause(vec![va, -vb]);
        }
    }

    // 输出差异：任一输出位不同即可满足 miter
    let mut diffs: Vec<Lit> = Vec::new();
    for (i, p) in a_out.iter().enumerate() {
        let bp = &b_out[i];
        for bit in 0..p.width.bits() {
            let va = ea.var_of_output_bit(&p.name, bit).ok_or("A 输出位缺失")?;
            let vb = eb.var_of_output_bit(&bp.name, bit).ok_or("B 输出位缺失")? + off;
            let d = cnf.new_var();
            // d = XOR(va, vb)
            cnf.add_clause(vec![-va, -vb, -d]);
            cnf.add_clause(vec![va, vb, -d]);
            cnf.add_clause(vec![va, -vb, d]);
            cnf.add_clause(vec![-va, vb, d]);
            diffs.push(d);
        }
    }
    // 「至少一位不同」
    cnf.add_clause(diffs);
    cnf.validate()?;

    let mut solver = Solver::new(&cnf);
    match solver.solve() {
        SolveResult::Unsat => Ok((true, None)),
        SolveResult::Sat => {
            let model = solver.model();
            let mut parts = Vec::new();
            for p in a_in {
                let hex = render_bits(|i| ea.var_of_input_bit(&p.name, i), &model, p.width.bits());
                parts.push(format!("{}={}", p.name, hex));
            }
            Ok((false, Some(format!("反例: 输入 {}", parts.join(", ")))))
        }
        SolveResult::Unknown => Err("等价性判定触及求解器资源上限（未得出结论）".into()),
    }
}

fn name_of(c: &Compiled) -> String {
    match c {
        Compiled::Combinational { name, .. } | Compiled::State { name, .. } => name.clone(),
    }
}

#[allow(clippy::type_complexity)]
fn parts(
    c: &Compiled,
) -> Result<(&Vec<crate::ast::Param>, &Vec<crate::ast::Param>, &Netlist), String> {
    match c {
        Compiled::Combinational { inputs, outputs, netlist, .. } => Ok((inputs, outputs, netlist)),
        Compiled::State { .. } => Err("形式化等价检查当前仅支持组合电路".into()),
    }
}
