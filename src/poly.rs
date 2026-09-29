//! 多项式算术 + 不等式证书（**整数语义**）—— 审计 AMM 类非线性不变量。
//!
//! ## 为什么需要它
//!
//! 主流 DeFi 的核心是乘除法（Uniswap 的 `x·y=k`、Aave 健康因子、ERC-4626 份额）。
//! 但 `prove.rs` 的规格乘法通用路径**无视实际位宽、永远按 128 位展开**，
//! 于是连 4 位变量的乘法都超出 40 万门预算（实测：`a*b == b*a` 直接
//! `乘法综合超出预算`）。而且乘法器验证本身是 SAT 的著名硬点。
//!
//! 本模块换一条路：**在多项式层面推理**，完全不构造乘法器、也不 bit-blast。
//! 把 `*` 按分配律展开成多项式 `Σ c·M + k`（M 为变量下标的升序多重集），
//! 于是**恒等式**直接判证；**不等式**用 Farkas 风格证书判证。
//!
//! ## 语义与可靠性（关键）
//!
//! 本模块在**整数**上推理（不是 `mod 2ʷ`）。因此必须证明「规格的 u128 回绕
//! 语义 == 整数值」，即**全程不发生回绕**。做法：
//!
//! 1. 变量上界默认取端口位宽（`Bits<N>` → `< 2^N`），precondition 里的
//!    `x <= C` / `x < 2^k` 可收紧。
//! 2. 对**每条比较的两侧**做区间分析（全部用 checked 算术）；任何一步溢出
//!    u128 即 **bail**（回落 SAT）—— 因为那意味着规格的 u128 运算可能回绕。
//! 3. `a - b` 只有在**有 `b <= a` 守卫**时才算安全（守卫来自 precondition 里
//!    直接给出的不等式，或 `a`/`b` 同式）。否则 bail。这保证理想整数语义与
//!    位向量语义一致。
//!
//! ## 证书规则
//!
//! 目标化为 `G >= 0`，假设化为若干 `Dⱼ >= 0`。若存在 `k > 0`、`mⱼ >= 0`
//! 使得 `k·G − Σ mⱼ·Dⱼ` 的**全部系数非负**，则
//! `k·G = (非负多项式) + Σ mⱼ·Dⱼ >= 0`，故 `G >= 0`。
//! （Uniswap 场景：`997·G − D = 3·R0·dOut`，系数全非负。）
//!
//! 与词级层同样的纪律：**只做保守、单向判定**；判不了返回 `None` 回落 SAT，
//! 绝不返回 `Some(false)`（反例由 SAT 给出）。

use std::collections::{BTreeMap, HashMap};

use crate::ast::Param;
use crate::spec::{parse_spec, SpecExpr};

/// 证书搜索：系数放大倍数 `k` 的上限（需覆盖 997 这类费率分母）。
const K_MAX: i128 = 1200;
/// 单个假设的证书重数上限。
const M_MAX: i128 = 4;
/// 多项式项数上限：超过即 bail（避免 `(a+b)*(c+d)*(e+f)*…` 指数爆炸）。
const POLY_TERM_CAP: usize = 4096;

/// 单项式：变量下标的**升序**多重集（`x·x·y` → `[x, x, y]`）。
type Mono = Vec<u32>;

/// 多项式 `Σ c·M + k`（整数系数）。常数项单独存于 `k`。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Poly {
    t: BTreeMap<Mono, i128>,
    k: i128,
}

impl Poly {
    fn zero() -> Poly {
        Poly { t: BTreeMap::new(), k: 0 }
    }

    fn constant(k: i128) -> Poly {
        Poly { t: BTreeMap::new(), k }
    }

    fn var(c: i128, v: u32) -> Poly {
        let mut t = BTreeMap::new();
        if c != 0 {
            t.insert(vec![v], c);
        }
        Poly { t, k: 0 }
    }

    fn is_zero(&self) -> bool {
        self.t.is_empty() && self.k == 0
    }

    fn add(&self, o: &Poly) -> Option<Poly> {
        let mut t = self.t.clone();
        for (m, c) in &o.t {
            let e = t.entry(m.clone()).or_insert(0);
            *e = e.checked_add(*c)?;
        }
        t.retain(|_, c| *c != 0);
        Some(Poly { t, k: self.k.checked_add(o.k)? })
    }

    fn neg(&self) -> Option<Poly> {
        let mut t = BTreeMap::new();
        for (m, c) in &self.t {
            t.insert(m.clone(), c.checked_neg()?);
        }
        Some(Poly { t, k: self.k.checked_neg()? })
    }

    fn sub(&self, o: &Poly) -> Option<Poly> {
        self.add(&o.neg()?)
    }

    fn scale(&self, c: i128) -> Option<Poly> {
        if c == 0 {
            return Some(Poly::zero());
        }
        let mut t = BTreeMap::new();
        for (m, v) in &self.t {
            t.insert(m.clone(), v.checked_mul(c)?);
        }
        Some(Poly { t, k: self.k.checked_mul(c)? })
    }

    fn mul(&self, o: &Poly) -> Option<Poly> {
        let na = self.t.len() + 1;
        let nb = o.t.len() + 1;
        if na.saturating_mul(nb) > POLY_TERM_CAP {
            return None;
        }
        let mut t: BTreeMap<Mono, i128> = BTreeMap::new();
        for (m1, c1) in &self.t {
            for (m2, c2) in &o.t {
                let mut m = m1.clone();
                m.extend(m2.iter().copied());
                m.sort_unstable();
                let e = t.entry(m).or_insert(0);
                *e = e.checked_add(c1.checked_mul(*c2)?)?;
            }
        }
        for (m, c) in &o.t {
            let e = t.entry(m.clone()).or_insert(0);
            *e = e.checked_add(self.k.checked_mul(*c)?)?;
        }
        for (m, c) in &self.t {
            let e = t.entry(m.clone()).or_insert(0);
            *e = e.checked_add(o.k.checked_mul(*c)?)?;
        }
        let k = self.k.checked_mul(o.k)?;
        t.retain(|_, c| *c != 0);
        Some(Poly { t, k })
    }

    /// 全部系数（含常数项）非负。
    fn all_nonneg(&self) -> bool {
        self.k >= 0 && self.t.values().all(|c| *c >= 0)
    }

    fn as_const(&self) -> Option<u128> {
        if self.t.is_empty() && self.k >= 0 {
            Some(self.k as u128)
        } else {
            None
        }
    }
}

/* ============================ 变量表与上界 ============================ */

struct Vars {
    idx: HashMap<String, u32>,
    names: Vec<String>,
    /// 上界（含）
    bound: Vec<u128>,
}

impl Vars {
    fn new(ports: &[Param]) -> Vars {
        let mut v = Vars { idx: HashMap::new(), names: Vec::new(), bound: Vec::new() };
        for p in ports {
            v.declare(&p.name, p.width.bits());
        }
        v
    }

    fn declare(&mut self, name: &str, w: u32) {
        if self.idx.contains_key(name) {
            return;
        }
        let i = self.names.len() as u32;
        self.idx.insert(name.to_string(), i);
        self.names.push(name.to_string());
        self.bound.push(bound_of_width(w));
    }

    fn get(&mut self, name: &str) -> u32 {
        if let Some(i) = self.idx.get(name) {
            return *i;
        }
        let i = self.names.len() as u32;
        self.idx.insert(name.to_string(), i);
        self.names.push(name.to_string());
        self.bound.push(u128::MAX); // 未知变量：上界无穷 → 区间必失败 → bail
        i
    }

    fn tighten_name(&mut self, i: u32, hi: u128) {
        let k = i as usize;
        if hi < self.bound[k] {
            self.bound[k] = hi;
        }
    }
}

fn bound_of_width(w: u32) -> u128 {
    if w >= 128 {
        u128::MAX
    } else {
        (1u128 << w) - 1
    }
}

/* ============================ SpecExpr -> Poly ============================ */

fn to_poly(e: &SpecExpr, vars: &mut Vars) -> Option<Poly> {
    match e {
        SpecExpr::Num(n) => {
            if *n > i128::MAX as u128 {
                return None;
            }
            Some(Poly::constant(*n as i128))
        }
        SpecExpr::Var(name) => Some(Poly::var(1, vars.get(name))),
        SpecExpr::Add(a, b) => to_poly(a, vars)?.add(&to_poly(b, vars)?),
        SpecExpr::Sub(a, b) => to_poly(a, vars)?.sub(&to_poly(b, vars)?),
        SpecExpr::Mul(a, b) => to_poly(a, vars)?.mul(&to_poly(b, vars)?),
        _ => None, // 取模/比较/布尔不在多项式层
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Rel {
    Ge,
    Gt,
    Le,
    Lt,
    Eq,
    Ne,
}

/// 一条比较（保留原始子表达式，供区间分析使用）。
#[derive(Clone)]
struct Cmp {
    l: SpecExpr,
    rel: Rel,
    r: SpecExpr,
}

/// 把表达式拆为合取下的比较列表；`||`/`!`/非比较 → `None`。
fn conjuncts(e: &SpecExpr) -> Option<Vec<Cmp>> {
    match e {
        SpecExpr::And(a, b) => {
            let mut v = conjuncts(a)?;
            v.extend(conjuncts(b)?);
            Some(v)
        }
        SpecExpr::Num(1) => Some(Vec::new()),
        SpecExpr::Num(0) => None, // 恒假：交 SAT 路径按「输入域为空」处理
        SpecExpr::Eq(a, b)
        | SpecExpr::Ne(a, b)
        | SpecExpr::Lt(a, b)
        | SpecExpr::Gt(a, b)
        | SpecExpr::Le(a, b)
        | SpecExpr::Ge(a, b) => {
            let rel = match e {
                SpecExpr::Eq(..) => Rel::Eq,
                SpecExpr::Ne(..) => Rel::Ne,
                SpecExpr::Lt(..) => Rel::Lt,
                SpecExpr::Gt(..) => Rel::Gt,
                SpecExpr::Le(..) => Rel::Le,
                _ => Rel::Ge,
            };
            Some(vec![Cmp { l: (**a).clone(), rel, r: (**b).clone() }])
        }
        _ => None,
    }
}

/* ============================ 区间分析（无回绕保证） ============================ */

type Iv = (u128, u128);

fn iv_add(a: Iv, b: Iv) -> Option<Iv> {
    Some((a.0.checked_add(b.0)?, a.1.checked_add(b.1)?))
}

fn iv_mul(a: Iv, b: Iv) -> Option<Iv> {
    let ps = [
        a.0.checked_mul(b.0)?,
        a.0.checked_mul(b.1)?,
        a.1.checked_mul(b.0)?,
        a.1.checked_mul(b.1)?,
    ];
    Some((*ps.iter().min()?, *ps.iter().max()?))
}

/// 区间分析。任何一步 checked 溢出（= 可能回绕）→ `None`。
///
/// `guards` 是「已证非负」的多项式集合，用于放行 `a - b`。
fn interval(e: &SpecExpr, vars: &mut Vars, guards: &[Poly]) -> Option<Iv> {
    match e {
        SpecExpr::Num(n) => Some((*n, *n)),
        SpecExpr::Var(name) => {
            let i = vars.get(name) as usize;
            Some((0, vars.bound[i]))
        }
        SpecExpr::Add(a, b) => iv_add(interval(a, vars, guards)?, interval(b, vars, guards)?),
        SpecExpr::Mul(a, b) => iv_mul(interval(a, vars, guards)?, interval(b, vars, guards)?),
        SpecExpr::Sub(a, b) => {
            let (_, ahi) = interval(a, vars, guards)?;
            let _ = interval(b, vars, guards)?;
            // 必须存在守卫证明 a - b >= 0
            let d = to_poly(a, vars)?.sub(&to_poly(b, vars)?)?;
            let ok = d.is_zero() || guards.iter().any(|g| *g == d);
            if !ok {
                return None;
            }
            // b >= 0 ⇒ a - b <= a
            Some((0, ahi))
        }
        _ => None,
    }
}

/// 校验一条比较两侧的区间都能算出（即规格的 u128 运算全程无回绕）。
fn check_nowrap(c: &Cmp, vars: &mut Vars, guards: &[Poly]) -> Option<()> {
    interval(&c.l, vars, guards)?;
    interval(&c.r, vars, guards)?;
    Some(())
}

/* ============================ 判定 ============================ */

/// `A REL B` → 一组 `Gᵢ >= 0`（**等式给出两个方向**）。`!=` 返回 `None`。
fn goals_of(c: &Cmp, vars: &mut Vars) -> Option<Vec<Poly>> {
    let d = to_poly(&c.l, vars)?.sub(&to_poly(&c.r, vars)?)?;
    Some(match c.rel {
        Rel::Ge => vec![d],
        Rel::Gt => vec![d.sub(&Poly::constant(1))?],
        Rel::Le => vec![d.neg()?],
        Rel::Lt => vec![d.neg()?.sub(&Poly::constant(1))?],
        // 等式必须**双向**：只证 `>=` 是不足的
        Rel::Eq => {
            let n = d.neg()?;
            vec![d, n]
        }
        Rel::Ne => return None,
    })
}

/// 用 Farkas 风格证书证明 `G >= 0`：找 `k > 0`、`mⱼ >= 0` 使
/// `k·G − Σ mⱼ·Dⱼ` 全系数非负。
fn prove_ge0(goal: &Poly, hyps: &[Poly]) -> Option<bool> {
    if goal.is_zero() || goal.all_nonneg() {
        return Some(true);
    }
    for k in 1..=K_MAX {
        let kg = goal.scale(k)?;
        if kg.all_nonneg() {
            return Some(true);
        }
        for d in hyps {
            for m in 1..=M_MAX {
                if let Some(diff) = kg.sub(&d.scale(m)?) {
                    if diff.all_nonneg() {
                        return Some(true);
                    }
                }
            }
        }
        for i in 0..hyps.len() {
            for j in (i + 1)..hyps.len() {
                for mi in 1..=M_MAX {
                    for mj in 1..=M_MAX {
                        if let Some(diff) = kg.sub(&hyps[i].scale(mi)?)?.sub(&hyps[j].scale(mj)?) {
                            if diff.all_nonneg() {
                                return Some(true);
                            }
                        }
                    }
                }
            }
        }
    }
    Some(false)
}

/// 尝试用多项式证书证明 `pre ⟹ post`。`Some(true)` = 已证；`None` = 无法判定。
pub fn prove_inequality(ports: &[Param], pre: Option<&SpecExpr>, post: &SpecExpr) -> Option<bool> {
    let mut vars = Vars::new(ports);

    // 1) 前置：提取变量上界 + 假设 + 守卫
    let mut hyps: Vec<Poly> = Vec::new();
    let mut guards: Vec<Poly> = Vec::new();
    if let Some(p) = pre {
        for c in conjuncts(p)? {
            tighten_from_cmp(&c, &mut vars);
            let gs = goals_of(&c, &mut vars)?;
            for g in &gs {
                guards.push(g.clone());
            }
            hyps.extend(gs);
        }
    }

    // 2) 无回绕校验：前置与后置的每条比较两侧都必须在 [0, 2^128)
    if let Some(p) = pre {
        for c in conjuncts(p)? {
            check_nowrap(&c, &mut vars, &guards)?;
        }
    }
    let post_cs = conjuncts(post)?;
    if post_cs.is_empty() {
        return None;
    }
    for c in &post_cs {
        check_nowrap(c, &mut vars, &guards)?;
    }

    // 3) 证明后置的每个合取支（等式双向）
    for c in &post_cs {
        for g in goals_of(c, &mut vars)? {
            if !prove_ge0(&g, &hyps)? {
                return None;
            }
        }
    }
    Some(true)
}

/// 从 `x <= C` / `x < C` 收紧变量上界（`2^k` 在词法层已是 `Num`）。
fn tighten_from_cmp(c: &Cmp, vars: &mut Vars) {
    if let SpecExpr::Var(name) = &c.l {
        if let Some(hi) = c.r.as_const_for_bound() {
            let vi = vars.get(name);
            match c.rel {
                Rel::Le => vars.tighten_name(vi, hi),
                Rel::Lt => vars.tighten_name(vi, hi.saturating_sub(1)),
                _ => {}
            }
        }
    }
}

/// 供上界收紧使用：右侧是否为非负常量多项式。
trait BoundConst {
    fn as_const_for_bound(&self) -> Option<u128>;
}

impl BoundConst for SpecExpr {
    fn as_const_for_bound(&self) -> Option<u128> {
        let mut v = Vars { idx: HashMap::new(), names: Vec::new(), bound: Vec::new() };
        to_poly(self, &mut v)?.as_const()
    }
}

/// 一键：解析 `pre`/`post` 文本后判定。
pub fn prove_text(ports: &[Param], pre: Option<&str>, post: &str) -> Option<bool> {
    let pre_e = match pre {
        Some(t) => Some(parse_spec(t).ok()?),
        None => None,
    };
    let post_e = parse_spec(post).ok()?;
    prove_inequality(ports, pre_e.as_ref(), &post_e)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Param, Span, Width};

    fn p32(n: &str) -> Param {
        Param { name: n.to_string(), width: Width::Bits(32), span: Span::new(0, 0) }
    }

    fn ports4() -> Vec<Param> {
        vec![p32("R0"), p32("R1"), p32("dIn"), p32("dOut")]
    }

    fn vx() -> Vars {
        let mut v = Vars::new(&[]);
        v.declare("x", 32);
        v.declare("y", 32);
        v
    }

    #[test]
    fn poly_commutative() {
        let mut v = vx();
        let a = to_poly(&parse_spec("x*y").unwrap(), &mut v).unwrap();
        let b = to_poly(&parse_spec("y*x").unwrap(), &mut v).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn poly_distributive() {
        let mut v = vx();
        let a = to_poly(&parse_spec("(x + y) * (x + y)").unwrap(), &mut v).unwrap();
        let b = to_poly(&parse_spec("x*x + 2*x*y + y*y").unwrap(), &mut v).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn poly_cancels() {
        let mut v = vx();
        let e = to_poly(&parse_spec("x*y - y*x").unwrap(), &mut v).unwrap();
        assert!(e.is_zero());
    }

    #[test]
    fn poly_squares() {
        let mut v = vx();
        let a = to_poly(&parse_spec("x*x").unwrap(), &mut v).unwrap();
        let b = to_poly(&parse_spec("x * x").unwrap(), &mut v).unwrap();
        assert_eq!(a, b);
    }

    /// Uniswap V2：k 不下降（0.3% 费）—— 核心验收
    #[test]
    fn uniswap_k_non_decreasing_certificate() {
        let pre = "dOut * (1000*R0 + 997*dIn) <= 997*dIn*R1 && dOut <= R1";
        let post = "(R0 + dIn) * (R1 - dOut) >= R0 * R1";
        assert_eq!(prove_text(&ports4(), Some(pre), post), Some(true));
    }

    /// 证书是 `997·G − D = 3·R0·dOut`，全非负 —— 直接验证这条代数恒等式
    #[test]
    fn certificate_polynomial_is_nonneg() {
        let ports = ports4();
        let mut vars = Vars::new(&ports);
        let g = to_poly(&parse_spec("(R0 + dIn) * (R1 - dOut) - R0 * R1").unwrap(), &mut vars)
            .unwrap();
        let d = to_poly(
            &parse_spec("997*dIn*R1 - dOut*(1000*R0 + 997*dIn)").unwrap(),
            &mut vars,
        )
        .unwrap();
        let cert = g.scale(997).unwrap().sub(&d).unwrap();
        assert!(cert.all_nonneg(), "997·G − D 应全系数非负: {cert:?}");
        assert!(!cert.is_zero());
    }

    /// 负面对照：约束**放宽一倍**（允许两倍的安全下单量）时必须判不了。
    ///
    /// 注意不能用「无费（997→1000）」当反例 —— 无费时 k **恰好保持**，
    /// 命题仍成立（证书 k=1000, m=1 给出恒 0）。这正是容易想错的地方。
    #[test]
    fn over_permissive_variant_is_not_proved() {
        let pre = "dOut * (R0 + dIn) <= 2*dIn*R1 && dOut <= R1";
        let post = "(R0 + dIn) * (R1 - dOut) >= R0 * R1";
        assert_eq!(
            prove_text(&ports4(), Some(pre), post),
            None,
            "约束过宽时必须判不了（不得假证明）"
        );
    }

    /// 无费变体确实可证（k 恰好保持），锁住上一条注释的结论
    #[test]
    fn no_fee_preserves_k_and_is_provable() {
        let pre = "dOut * (1000*R0 + 1000*dIn) <= 1000*dIn*R1 && dOut <= R1";
        let post = "(R0 + dIn) * (R1 - dOut) >= R0 * R1";
        assert_eq!(prove_text(&ports4(), Some(pre), post), Some(true));
    }

    /// 守卫缺失时必须 bail（否则整数语义与位向量语义不一致）
    #[test]
    fn unguarded_subtraction_bails() {
        let post = "(R1 - dOut) <= R1";
        assert_eq!(prove_text(&ports4(), None, post), None, "无 dOut<=R1 守卫应 bail");
    }

    /// 回绕风险必须 bail：把端口放宽到 128 位后乘积会溢出 u128
    #[test]
    fn wide_ports_bail_on_potential_wrap() {
        let ports = vec![
            Param { name: "a".into(), width: Width::Bits(128), span: Span::new(0, 0) },
            Param { name: "b".into(), width: Width::Bits(128), span: Span::new(0, 0) },
        ];
        assert_eq!(prove_text(&ports, None, "a * b >= a"), None, "可能回绕应 bail");
    }

    #[test]
    fn plain_identity_is_proved() {
        let ports = vec![p32("a"), p32("b")];
        assert_eq!(prove_text(&ports, None, "a * b == b * a"), Some(true));
        assert_eq!(
            prove_text(&ports, None, "(a + b) * (a + b) == a*a + 2*a*b + b*b"),
            Some(true)
        );
    }

    #[test]
    fn does_not_prove_false_inequality() {
        let ports = vec![p32("a"), p32("b")];
        assert_eq!(prove_text(&ports, None, "a * b >= a * b + 1"), None);
    }
}
