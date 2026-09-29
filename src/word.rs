//! 词级（word-level）项重写层 —— 在 bit-blast **之前**用位向量代数判定目标。
//!
//! ## 动机
//!
//! 多项求和的**加法结合律/同余**（`(b0-a)+(b1+a)+b2+b3` 与 `(b0+b1+b2+b3)-a`
//! 的相等）对 resolution 是指数难的 —— SAT 没有同余闭包。实测（见
//! 《GateLang ERC-20 形式化验证报告》§7.2.1）：
//! N=4 在 `Bits<8>` 需 4 秒，`Bits<12>` 已 >120 秒，`Bits<32>` >240 秒。
//! 且**断言（`cut:`）解决不了**：断言 ≠ 重写。
//!
//! 本模块把电路与规格**符号求值成规范形**，若目标在词级化为恒真即直接判证；
//! 否则**回落 SAT**（`prove.rs` 的既有路径，绝不给出错误结论）。
//!
//! ## 可靠性纪律（关键）
//!
//! 规范形只做**保守、单向**的判定：
//!
//! * 规范形 == `Const` ⇒ 该值确实处处相等（可靠）
//! * 两侧规范形**结构相同** ⇒ 二者确实相等（可靠）
//! * 规范形**不同** ⇒ **不作结论**，回落 SAT
//!
//! **绝不用「规范形不同」去证明不等** —— 那是不可靠的（原子被当作独立变元）。
//!
//! 另一条纪律：符号求值必须与 `lower.rs` 的 `lower_expr` 语义**逐条对齐**
//! （`coerce` 零扩展、`need_width` 严格等宽、比较降为 1 位、按位门逐位）。
//! 任何无法精确处理的构造一律**返回 `None`**（回落 SAT），绝不猜测。
//! `tests/word.rs` 用随机电路把词级求值与网表模拟逐条交叉验证。

use std::collections::{BTreeMap, HashMap};

use crate::ast::{BinOp, Circuit, Expr, Stmt, Target};
use crate::spec::SpecExpr;

/// 规格算术的机器宽度（与 `prove.rs::BV_W` 一致）。
pub const SPEC_W: u32 = 128;

/// 位宽门槛：u128 无法表示 `mod 2^w (w>128)` 的系数与常量，故 >128 位一律不判定。
const MAX_WORD_W: u32 = 128;

fn mask(w: u32) -> u128 {
    if w >= 128 {
        u128::MAX
    } else {
        (1u128 << w) - 1
    }
}

/// 按位宽取模（仅对 w ≤ 128 有意义；调用方有门槛）。
fn norm(w: u32, v: u128) -> u128 {
    v & mask(w)
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum BitOp {
    And,
    Or,
    Xor,
    Not,
    /// 逻辑归约：值是否非零，结果 1 位
    ToBool,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}

/// 词级规范形。每个节点自带位宽。
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum C {
    /// 位宽 + 值（已按位宽掩码）
    Const(u32, u128),
    /// 位宽 + 名字（端口/输入）
    Var(u32, String),
    /// `Σ coeffᵢ·atomᵢ + k`（mod 2^width）：atom 已排序去重、系数非零
    Sum(u32, Vec<(u128, C)>, u128),
    /// 位运算（`And`/`Or`/`Xor` 已展平去重；`Not` 一元；`ToBool` 归约）
    Bits(u32, BitOp, Vec<C>),
    /// 比较，结果恒 1 位（第一个字段是**操作数**位宽）
    Cmp(u32, CmpOp, Box<C>, Box<C>),
    /// 三目
    Ite(u32, Box<C>, Box<C>, Box<C>),
    /// 零扩展到 `to` 位（原子；不可与算术重组交换）
    Zext(u32, Box<C>),
}

impl C {
    pub fn width(&self) -> u32 {
        match self {
            C::Const(w, _) | C::Var(w, _) | C::Sum(w, ..) | C::Bits(w, ..)
            | C::Ite(w, ..) => *w,
            // 比较的结果恒为 1 位（字段是第一操作数的位宽，用于常量判定）
            C::Cmp(..) => 1,
            C::Zext(to, _) => *to,
        }
    }
}

pub fn width(c: &C) -> u32 {
    c.width()
}

/* ============================ 构造子（语义对齐 lower.rs） ============================ */

fn cst(w: u32, v: u128) -> C {
    C::Const(w, norm(w, v))
}

/// 加法：`Σ + k`，展平嵌套 `Sum`、合并同类项、系数按位宽取模。
fn add_all(w: u32, parts: Vec<(u128, C)>, k: u128) -> C {
    let mut acc: BTreeMap<C, u128> = BTreeMap::new();
    let mut kk = norm(w, k);
    for (coef, t) in parts {
        let coef = norm(w, coef);
        if coef == 0 {
            continue;
        }
        match t {
            C::Const(tw, v) if tw == w => {
                kk = norm(w, kk.wrapping_add(coef.wrapping_mul(v)));
            }
            C::Sum(tw, subs, sk) if tw == w => {
                kk = norm(w, kk.wrapping_add(coef.wrapping_mul(sk)));
                for (c2, a2) in subs {
                    let e = acc.entry(a2).or_insert(0);
                    *e = norm(w, e.wrapping_add(coef.wrapping_mul(c2)));
                }
            }
            other => {
                let e = acc.entry(other).or_insert(0);
                *e = norm(w, e.wrapping_add(coef));
            }
        }
    }
    acc.retain(|_, c| *c != 0);
    if acc.is_empty() {
        return C::Const(w, kk);
    }
    if acc.len() == 1 && kk == 0 {
        if let Some((a, &c)) = acc.iter().next() {
            if c == 1 {
                return a.clone();
            }
        }
    }
    C::Sum(w, acc.into_iter().map(|(a, c)| (c, a)).collect(), kk)
}

fn add(w: u32, a: C, b: C) -> C {
    add_all(w, vec![(1, a), (1, b)], 0)
}

fn sub(w: u32, a: C, b: C) -> C {
    add_all(w, vec![(1, a), (mask(w), b)], 0)
}

/// 零扩展到 `to` 位。
fn zext(to: u32, x: C) -> C {
    if x.width() == to {
        return x;
    }
    match x {
        C::Const(w, v) => C::Const(to, norm(w, v)),
        other => C::Zext(to, Box::new(other)),
    }
}

/// `rest` 中是否存在互补对（`x` 与 `~x`）。`x & ~x = 0`、`x | ~x = all-ones`
/// 对**按位**补码在任意位宽都成立。
fn has_complement(rest: &[C], w: u32) -> bool {
    for i in 0..rest.len() {
        let nx = bits(w, BitOp::Not, vec![rest[i].clone()]);
        for j in 0..rest.len() {
            if i != j && rest[j] == nx {
                return true;
            }
        }
    }
    false
}

fn bits(w: u32, op: BitOp, ops: Vec<C>) -> C {
    match op {
        BitOp::Not => {
            let x = match ops.into_iter().next() {
                Some(x) => x,
                None => return C::Const(w, 0),
            };
            if let C::Const(tw, v) = &x {
                if *tw == w {
                    return C::Const(w, norm(w, !*v));
                }
            }
            if let C::Bits(tw, BitOp::Not, inner) = &x {
                if *tw == w && inner.len() == 1 {
                    return inner[0].clone();
                }
            }
            C::Bits(w, BitOp::Not, vec![x])
        }
        BitOp::ToBool => {
            let x = match ops.into_iter().next() {
                Some(x) => x,
                None => return C::Const(1, 0),
            };
            match &x {
                C::Const(tw, v) => C::Const(1, (norm(*tw, *v) != 0) as u128),
                C::Cmp(..) => x,
                _ if x.width() == 1 => x,
                _ => C::Bits(1, BitOp::ToBool, vec![x]),
            }
        }
        BitOp::Xor => {
            // 展平 + 奇偶消去（x ^ x = 0）
            let mut stack = ops;
            let mut parity: BTreeMap<C, bool> = BTreeMap::new();
            let mut k: Option<u128> = None;
            while let Some(o) = stack.pop() {
                match o {
                    C::Bits(w2, BitOp::Xor, inner) if w2 == w => stack.extend(inner),
                    C::Const(tw, v) if tw == w => k = Some(k.map_or(v, |a| a ^ v)),
                    other => {
                        let e = parity.entry(other).or_insert(false);
                        *e = !*e;
                    }
                }
            }
            let mut rest: Vec<C> = parity
                .into_iter()
                .filter(|(_, p)| *p)
                .map(|(c, _)| c)
                .collect();
            rest.sort();
            if let Some(kv) = k {
                let kv = norm(w, kv);
                if kv != 0 {
                    rest.push(C::Const(w, kv));
                }
            }
            rest.sort();
            if rest.is_empty() {
                return C::Const(w, 0);
            }
            if rest.len() == 1 {
                return rest.pop().unwrap();
            }
            C::Bits(w, BitOp::Xor, rest)
        }
        BitOp::And | BitOp::Or => {
            let ident = if op == BitOp::And { mask(w) } else { 0 };
            let mut flat: Vec<C> = Vec::new();
            for o in ops {
                match o {
                    C::Bits(w2, op2, inner) if w2 == w && op2 == op => flat.extend(inner),
                    other => flat.push(other),
                }
            }
            let mut konst: Option<u128> = None;
            let mut rest: Vec<C> = Vec::new();
            for o in flat {
                match &o {
                    C::Const(tw, v) if *tw == w => {
                        konst = Some(match (konst, op) {
                            (None, _) => *v,
                            (Some(k), BitOp::And) => k & *v,
                            (Some(k), BitOp::Or) => k | *v,
                            (Some(k), _) => k,
                        });
                    }
                    _ => rest.push(o),
                }
            }
            rest.sort();
            rest.dedup();
            if op == BitOp::And {
                if konst == Some(0) {
                    return C::Const(w, 0);
                }
                if has_complement(&rest, w) {
                    return C::Const(w, 0);
                }
            } else {
                if konst == Some(mask(w)) {
                    return C::Const(w, mask(w));
                }
                if has_complement(&rest, w) {
                    return C::Const(w, mask(w));
                }
            }
            if let Some(kv) = konst {
                if kv != ident {
                    rest.push(C::Const(w, kv));
                    rest.sort();
                }
            }
            if rest.is_empty() {
                return C::Const(w, ident);
            }
            if rest.len() == 1 {
                return rest.pop().unwrap();
            }
            C::Bits(w, op, rest)
        }
    }
}

fn not1(a: C) -> C {
    bits(1, BitOp::Not, vec![a])
}

fn and1(a: C, b: C) -> C {
    bits(1, BitOp::And, vec![a, b])
}

fn or1(a: C, b: C) -> C {
    bits(1, BitOp::Or, vec![a, b])
}

fn to_bool(a: C) -> C {
    bits(1, BitOp::ToBool, vec![a])
}

fn const_of(a: &C, b: &C) -> Option<(u128, u128)> {
    match (a, b) {
        (C::Const(w1, v1), C::Const(w2, v2)) if w1 == w2 => Some((*v1, *v2)),
        _ => None,
    }
}

/// 小于（`Cmp`，结果 1 位）。语义对齐 `lower.rs::lower_lt`。
fn lt_of(w: u32, a: C, b: C) -> C {
    if a == b {
        return C::Const(1, 0);
    }
    if let Some((x, y)) = const_of(&a, &b) {
        return C::Const(1, (x < y) as u128);
    }
    C::Cmp(w, CmpOp::Lt, Box::new(a), Box::new(b))
}

fn eq_of(w: u32, a: C, b: C) -> C {
    if a == b {
        return C::Const(1, 1);
    }
    if let Some((x, y)) = const_of(&a, &b) {
        return C::Const(1, (x == y) as u128);
    }
    // 对称：排序使 `a==b` 与 `b==a` 规范形一致
    let (x, y) = if a <= b { (a, b) } else { (b, a) };
    C::Cmp(w, CmpOp::Eq, Box::new(x), Box::new(y))
}

/// 二元比较，语义对齐 `lower.rs` 的 `BinOp::Lt/Gt/Le/Ge/Eq/Ne` 分派。
fn cmp_bin(w: u32, op: BinOp, a: C, b: C) -> C {
    match op {
        BinOp::Eq => eq_of(w, a, b),
        BinOp::Ne => not1(eq_of(w, a, b)),
        BinOp::Lt => lt_of(w, a, b),
        BinOp::Gt => lt_of(w, b, a),
        BinOp::Le => not1(lt_of(w, b, a)),
        BinOp::Ge => not1(lt_of(w, a, b)),
        _ => unreachable!("cmp_bin 仅处理比较"),
    }
}

fn ite(w: u32, c: C, t: C, e: C) -> C {
    match truth(&c) {
        Some(true) => t,
        Some(false) => e,
        None => {
            if t == e {
                t
            } else {
                C::Ite(w, Box::new(c), Box::new(t), Box::new(e))
            }
        }
    }
}

/* ============================ 常量判定 ============================ */

/// 若该 1 位项的取值被规范形唯一确定，返回 `Some(bool)`。
///
/// **只用于「确定为真/假」**；返回 `None` 表示无法判定（须回落 SAT）。
pub fn truth(c: &C) -> Option<bool> {
    match c {
        C::Const(w, v) => {
            if *w == 1 {
                Some(*v != 0)
            } else {
                None
            }
        }
        C::Cmp(_, op, a, b) => {
            if a == b {
                return match op {
                    CmpOp::Eq | CmpOp::Le | CmpOp::Ge => Some(true),
                    CmpOp::Ne | CmpOp::Lt | CmpOp::Gt => Some(false),
                };
            }
            if let Some((x, y)) = const_of(a, b) {
                return Some(match op {
                    CmpOp::Eq => x == y,
                    CmpOp::Ne => x != y,
                    CmpOp::Lt => x < y,
                    CmpOp::Gt => x > y,
                    CmpOp::Le => x <= y,
                    CmpOp::Ge => x >= y,
                });
            }
            None
        }
        C::Bits(1, BitOp::Not, ops) if ops.len() == 1 => truth(&ops[0]).map(|v| !v),
        C::Bits(1, BitOp::ToBool, ops) if ops.len() == 1 => truth(&ops[0]),
        C::Bits(1, BitOp::And, ops) => {
            if ops.iter().any(|o| truth(o) == Some(false)) {
                return Some(false);
            }
            if ops.iter().all(|o| truth(o) == Some(true)) {
                return Some(true);
            }
            None
        }
        C::Bits(1, BitOp::Or, ops) => {
            if ops.iter().any(|o| truth(o) == Some(true)) {
                return Some(true);
            }
            if ops.iter().all(|o| truth(o) == Some(false)) {
                return Some(false);
            }
            None
        }
        C::Bits(1, BitOp::Xor, ops) => {
            let mut acc = false;
            for o in ops {
                acc ^= truth(o)?;
            }
            Some(acc)
        }
        C::Ite(1, c, t, e) => {
            if let Some(b) = truth(c) {
                return truth(if b { t } else { e });
            }
            if t == e {
                return truth(t);
            }
            None
        }
        _ => None,
    }
}

/* ============================ 符号求值：电路 AST ============================ */

/// 符号求值电路表达式。`None` = 不支持该构造（回落 SAT）。
fn eval_expr(e: &Expr, env: &HashMap<String, C>) -> Option<C> {
    match e {
        Expr::Lit(v, w, _) => Some(cst(w.bits(), *v)),
        Expr::Var(name, _) => {
            if let Some(c) = env.get(name) {
                return Some(c.clone());
            }
            match name.as_str() {
                "0" => Some(C::Const(1, 0)),
                "1" => Some(C::Const(1, 1)),
                _ => None,
            }
        }
        Expr::Not(x, _) => {
            let a = eval_expr(x, env)?;
            let w = a.width();
            Some(bits(w, BitOp::Not, vec![a]))
        }
        Expr::Call(callee, args, _) => {
            match callee.as_str() {
                // 门原语：与 lower.rs 一致 —— AND/OR/XOR/NAND 用 need_width（严格等宽）
                "AND" | "OR" | "XOR" | "NAND" => {
                    if args.len() != 2 {
                        return None;
                    }
                    let a = eval_expr(&args[0], env)?;
                    let b = eval_expr(&args[1], env)?;
                    if a.width() != b.width() {
                        return None; // need_width 会报错 → 不判定
                    }
                    let w = a.width();
                    Some(match callee.as_str() {
                        "AND" => bits(w, BitOp::And, vec![a, b]),
                        "OR" => bits(w, BitOp::Or, vec![a, b]),
                        "XOR" => bits(w, BitOp::Xor, vec![a, b]),
                        _ => bits(w, BitOp::Not, vec![bits(w, BitOp::And, vec![a, b])]),
                    })
                }
                "NOT" => {
                    if args.len() != 1 {
                        return None;
                    }
                    let a = eval_expr(&args[0], env)?;
                    let w = a.width();
                    Some(bits(w, BitOp::Not, vec![a]))
                }
                // 结构组合（内联其他 circuit）：不在此层处理，保守回落 SAT
                _ => None,
            }
        }
        Expr::Bin(op, a, b, _) => {
            let mut x = eval_expr(a, env)?;
            let mut y = eval_expr(b, env)?;
            // coerce：零扩展到较宽者（与 lower.rs 一致）
            let (wx, wy) = (x.width(), y.width());
            if wx < wy {
                x = zext(wy, x);
            } else if wy < wx {
                y = zext(wx, y);
            }
            let w = x.width();
            match op {
                BinOp::And => Some(bits(w, BitOp::And, vec![x, y])),
                BinOp::Or => Some(bits(w, BitOp::Or, vec![x, y])),
                BinOp::Xor => Some(bits(w, BitOp::Xor, vec![x, y])),
                BinOp::Add => Some(add(w, x, y)),
                BinOp::Sub => Some(sub(w, x, y)),
                BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => {
                    Some(cmp_bin(w, *op, x, y))
                }
            }
        }
        Expr::Ternary(c, t, e, _) => {
            let cc = eval_expr(c, env)?;
            if cc.width() != 1 {
                return None;
            }
            let tt = eval_expr(t, env)?;
            let ee = eval_expr(e, env)?;
            if tt.width() != ee.width() {
                return None;
            }
            let w = tt.width();
            Some(ite(w, cc, tt, ee))
        }
        // 位索引/切片/拼接：保守不处理
        Expr::Index(..) | Expr::Slice(..) | Expr::Concat(..) => None,
    }
}

/// 符号执行电路体。镜像 `lower.rs::compile_statements` 的可支持子集。
fn exec_body(
    body: &[Stmt],
    env: &mut HashMap<String, C>,
    ret: &mut Option<Vec<C>>,
) -> Option<()> {
    for s in body {
        if ret.is_some() {
            break;
        }
        match s {
            Stmt::Assign(a) => {
                if a.targets.len() != 1 {
                    return None;
                }
                let v = eval_expr(&a.value, env)?;
                match &a.targets[0] {
                    Target::Var(name, _) => {
                        env.insert(name.clone(), v);
                    }
                    // 位/切片赋值：不处理
                    _ => return None,
                }
            }
            Stmt::Return(exprs) => {
                let mut vs = Vec::with_capacity(exprs.len());
                for e in exprs {
                    vs.push(eval_expr(e, env)?);
                }
                *ret = Some(vs);
            }
            Stmt::AssertResource(..) => {}
            // 分支：不处理（保守）
            Stmt::If(_) => return None,
        }
    }
    Some(())
}

/* ============================ 符号求值：规格表达式 ============================ */

/// 规格表达式按 `SPEC_W`（128 位）语义符号求值；端口零扩展。
fn eval_spec(e: &SpecExpr, env: &HashMap<String, C>) -> Option<C> {
    const W: u32 = SPEC_W;
    match e {
        SpecExpr::Num(n) => Some(cst(W, *n)),
        SpecExpr::Var(name) => {
            let c = env.get(name)?;
            if c.width() > SPEC_W {
                return None; // 与 Synth 的端口 >128 位报错一致：不判定
            }
            Some(zext(W, c.clone()))
        }
        SpecExpr::Add(a, b) => Some(add(W, eval_spec(a, env)?, eval_spec(b, env)?)),
        SpecExpr::Sub(a, b) => Some(sub(W, eval_spec(a, env)?, eval_spec(b, env)?)),
        // 规格层乘/模：词级不实现（保守回落 SAT）
        SpecExpr::Mul(..) | SpecExpr::Mod(..) => None,
        SpecExpr::Eq(a, b) => Some(eq_of(W, eval_spec(a, env)?, eval_spec(b, env)?)),
        SpecExpr::Ne(a, b) => Some(not1(eq_of(W, eval_spec(a, env)?, eval_spec(b, env)?))),
        SpecExpr::Lt(a, b) => Some(lt_of(W, eval_spec(a, env)?, eval_spec(b, env)?)),
        SpecExpr::Gt(a, b) => Some(lt_of(W, eval_spec(b, env)?, eval_spec(a, env)?)),
        SpecExpr::Le(a, b) => Some(not1(lt_of(W, eval_spec(b, env)?, eval_spec(a, env)?))),
        SpecExpr::Ge(a, b) => Some(not1(lt_of(W, eval_spec(a, env)?, eval_spec(b, env)?))),
        SpecExpr::And(a, b) => Some(and1(
            to_bool(eval_spec(a, env)?),
            to_bool(eval_spec(b, env)?),
        )),
        SpecExpr::Or(a, b) => Some(or1(
            to_bool(eval_spec(a, env)?),
            to_bool(eval_spec(b, env)?),
        )),
        SpecExpr::Not(a) => Some(not1(to_bool(eval_spec(a, env)?))),
    }
}

/* ============================ 判定入口 ============================ */

/// 尝试在词级证明 `pre ⟹ post`。
///
/// 返回 `Some(true)` 表示**已证**（对全部输入成立）；`None` 表示**无法判定**
/// （调用方必须回落 SAT）。本函数**不会**返回 `Some(false)` —— 反例由 SAT 路径给出。
pub fn word_prove(circuit: &Circuit, pre: Option<&SpecExpr>, post: &SpecExpr) -> Option<bool> {
    // 位宽门槛：>128 位无法用 u128 表示 mod 2^w 的系数/常量 → 不判定
    let maxw = circuit
        .params
        .iter()
        .chain(circuit.returns.iter())
        .map(|p| p.width.bits())
        .max()
        .unwrap_or(0);
    if maxw > MAX_WORD_W {
        return None;
    }

    let mut env: HashMap<String, C> = HashMap::new();
    for p in &circuit.params {
        env.insert(p.name.clone(), C::Var(p.width.bits(), p.name.clone()));
    }
    let mut ret: Option<Vec<C>> = None;
    exec_body(&circuit.body, &mut env, &mut ret)?;
    let ret = ret?;
    if ret.len() != circuit.returns.len() {
        return None;
    }
    for (p, v) in circuit.returns.iter().zip(ret.iter()) {
        if v.width() != p.width.bits() {
            return None;
        }
        env.insert(p.name.clone(), v.clone());
    }

    let pre_c = match pre {
        Some(p) => eval_spec(p, &env)?,
        None => cst(SPEC_W, 1),
    };
    let post_c = eval_spec(post, &env)?;
    let pre_b = to_bool(pre_c);
    let post_b = to_bool(post_c);

    // 前置条件恒假：属「输入域为空」，交回 SAT 路径按既有约定报告（保持行为一致）
    if truth(&pre_b) == Some(false) {
        return None;
    }

    let imp = or1(not1(pre_b), post_b);
    if truth(&imp) == Some(true) {
        Some(true)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(n: &str, w: u32) -> C {
        C::Var(w, n.to_string())
    }

    #[test]
    fn cancellation_mod_2w() {
        // (x - a) + (y + a) == x + y   （mod 2^8）
        let w = 8;
        let lhs = add(w, sub(w, v("x", w), v("a", w)), add(w, v("y", w), v("a", w)));
        let rhs = add(w, v("x", w), v("y", w));
        assert_eq!(lhs, rhs, "(x-a)+(y+a) 应规范化为 x+y");
    }

    #[test]
    fn multi_term_reassociation() {
        // (b0-a)+(b1+a)+b2+b3 == b0+b1+b2+b3   （mod 2^32，多项重结合）
        let w = 32;
        let mut lhs = add(
            w,
            sub(w, v("b0", w), v("amount", w)),
            add(w, v("b1", w), v("amount", w)),
        );
        lhs = add(w, lhs, v("b2", w));
        lhs = add(w, lhs, v("b3", w));
        let mut rhs = add(w, v("b0", w), v("b1", w));
        rhs = add(w, rhs, v("b2", w));
        rhs = add(w, rhs, v("b3", w));
        assert_eq!(lhs, rhs, "多项求和应规范化为同一形");
        // 且比较式同形
        let c1 = eq_of(w, lhs, v("total", w));
        let c2 = eq_of(w, rhs, v("total", w));
        assert_eq!(c1, c2);
    }

    #[test]
    fn complement_detection() {
        // AND(x, NOT(x)) == 0 ； 对任意位宽
        for w in [1u32, 8, 32] {
            let x = v("x", w);
            let nx = bits(w, BitOp::Not, vec![x.clone()]);
            assert_eq!(bits(w, BitOp::And, vec![x.clone(), nx.clone()]), C::Const(w, 0));
            assert_eq!(
                bits(w, BitOp::Or, vec![x.clone(), nx]),
                C::Const(w, mask(w))
            );
        }
    }

    #[test]
    fn xor_parity() {
        let w = 8;
        let x = v("x", w);
        assert_eq!(bits(w, BitOp::Xor, vec![x.clone(), x.clone()]), C::Const(w, 0));
    }

    #[test]
    fn identity_and_absorption() {
        let w = 8;
        let x = v("x", w);
        // x & all-ones == x ; x | 0 == x ; x ^ 0 == x
        assert_eq!(bits(w, BitOp::And, vec![x.clone(), C::Const(w, mask(w))]), x);
        assert_eq!(bits(w, BitOp::Or, vec![x.clone(), C::Const(w, 0)]), x.clone());
        assert_eq!(bits(w, BitOp::Xor, vec![x.clone(), C::Const(w, 0)]), x.clone());
        // x & 0 == 0
        assert_eq!(bits(w, BitOp::And, vec![x.clone(), C::Const(w, 0)]), C::Const(w, 0));
    }

    #[test]
    fn truth_on_basic_booleans() {
        assert_eq!(truth(&C::Const(1, 1)), Some(true));
        assert_eq!(truth(&C::Const(1, 0)), Some(false));
        // x == x  => true
        let x = v("x", 8);
        assert_eq!(truth(&eq_of(8, x.clone(), x.clone())), Some(true));
        // x < x  => false
        assert_eq!(truth(&lt_of(8, x.clone(), x)), Some(false));
    }

    #[test]
    fn does_not_prove_inequality() {
        // 规范形不同 **不得** 被判为真：a+b vs a+b+1 不相等，truth 必须为 None
        let w = 8;
        let s1 = add(w, v("a", w), v("b", w));
        let s2 = add(w, add(w, v("a", w), v("b", w)), cst(w, 1));
        let e = eq_of(w, s1, s2);
        assert_eq!(truth(&e), None, "不同规范形不得判真（也不得判假）");
        assert_ne!(e, C::Const(1, 1));
    }

    #[test]
    fn sub_by_self_is_zero() {
        let w = 16;
        assert!(matches!(
            sub(w, v("x", w), v("x", w)),
            C::Const(_, 0)
        ));
    }

    #[test]
    fn coefficient_wraps_at_width() {
        // 256 次自加 mod 2^8 => 0
        let w = 8;
        let mut acc = cst(w, 0);
        for _ in 0..256 {
            acc = add(w, acc, v("x", w));
        }
        assert_eq!(acc, C::Const(w, 0));
    }

    #[test]
    fn zext_of_const_folds() {
        assert_eq!(zext(128, C::Const(1, 0)), C::Const(128, 0));
        assert_eq!(zext(128, C::Const(8, 200)), C::Const(128, 200));
        // 非 const 保持为原子
        assert!(matches!(zext(128, v("x", 8)), C::Zext(128, _)));
    }
}
