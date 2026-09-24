//! 语义保持编译：统一 AST → NAND/LATCH 网表（L1 门级）。
//!
//! 对应白皮书 v2.1 §7.2 编译流程的 "L1 门级 AST → NAND/LATCH 网表" 与
//! §5 类型系统强制区分组合/时序函数。本节实现：
//! - 表达式展开：字面量、引用、门调用（NAND/AND/OR/XOR/NOT）、位索引、切片、
//!   拼接、运算（+ 加法、==/!= 比较、! 取反、& | ^ 位运算）
//! - 语句执行：赋值（含 <- 时序更新）、return（多输出）、if/else（三元选择）
//! - 资源预算：每函数累计 gates/depth/cycles/latches，与声明的 Gates<N>/Depth<N>/Cycles<N>
//!   校验（白皮书 §5、§7.3）
//! - 结构组合：circuit 之间通过编译期引用组合（§8 结构性组合），内联展开。
//! - 语义检查：组合函数中禁止 LATCH 引用；时序函数中 <- 只允许更新已声明 latch。

use std::collections::HashMap;

use crate::ast::*;
use crate::netlist::{Gate, Netlist};
use crate::ty::{Modal, ResourceBudget};

/// 中间值：一位或多位的信号向量。
#[derive(Debug, Clone)]
pub struct Value {
    pub sigs: Vec<usize>, // LSB 在前
    pub width: u32,
}

/// 环境：名字 → 值。
type Env = HashMap<String, Value>;

#[derive(Debug)]
pub struct LowerError {
    pub msg: String,
    pub span: Span,
}

impl LowerError {
    fn new(span: Span, msg: &str) -> Self {
        LowerError { msg: msg.to_string(), span }
    }
}

pub type LowerResult<T> = Result<T, LowerError>;

/// 编译产物：电路（组合）或时序模块（时序）。
#[derive(Debug, Clone)]
pub enum Compiled {
    /// 组合电路：输入参数 -> 输出信号
    Combinational {
        name: String,
        inputs: Vec<Param>,
        outputs: Vec<Param>,
        output_sigs: Vec<usize>,
        netlist: Netlist,
        budget: ResourceBudget,
    },
    /// 时序状态：latches + fn（fn 编译为输入=latch 状态+fn 参数，输出=fn 返回值）
    State {
        name: String,
        latches: Vec<LatchDecl>,
        fns: Vec<CompiledFn>,
    },
}

/// 编译后的时序函数：作为「输入 latch + 参数 → 输出 + 新 latch」的组合网表。
#[derive(Debug, Clone)]
pub struct CompiledFn {
    pub name: String,
    pub params: Vec<Param>,
    pub returns: Vec<Param>,
    pub output_sigs: Vec<usize>,
    pub next_latch_sigs: Vec<(String, Vec<usize>)>, // latch 名 → 新值信号
    pub netlist: Netlist,
    pub budget: ResourceBudget,
}

/// 编译程序：遍历声明，解析引用。
pub struct Compiler<'a> {
    pub decls: &'a [Decl],
    /// 名字 → 已编译产物（供结构组合）
    cache: HashMap<String, Compiled>,
}

impl<'a> Compiler<'a> {
    pub fn new(decls: &'a [Decl]) -> Self {
        Compiler { decls, cache: HashMap::new() }
    }

    /// 编译所有顶层声明（按依赖拓扑自动解析，两次遍历：先收集后编译）。
    pub fn compile_all(&mut self) -> Vec<Compiled> {
        let mut out = Vec::new();
        let names: Vec<String> = self
            .decls
            .iter()
            .filter_map(|d| match d {
                Decl::Circuit(c) => Some(c.name.clone()),
                Decl::State(s) => Some(s.name.clone()),
                Decl::Spec(_) => None,
            })
            .collect();
        for n in names {
            self.compile_named(&n);
        }
        // 依序输出
        for n in self
            .decls
            .iter()
            .filter_map(|d| match d {
                Decl::Circuit(c) => Some(c.name.clone()),
                Decl::State(s) => Some(s.name.clone()),
                Decl::Spec(_) => None,
            })
        {
            if let Some(c) = self.cache.get(&n) {
                out.push(c.clone());
            }
        }
        out
    }

    fn compile_named(&mut self, name: &str) -> Compiled {
        if let Some(c) = self.cache.get(name) {
            return c.clone();
        }
        // 查找声明
        for d in self.decls {
            match (d, name) {
                (Decl::Circuit(c), n) if c.name == *n => {
                    let compiled = circuit_lower(c, self, 0, Vec::new())
                        .unwrap_or_else(|e| panic!("lower {} 失败: {} @ {:?}", c.name, e.msg, e.span));
                    self.cache.insert(name.to_string(), compiled.clone());
                    return compiled;
                }
                (Decl::State(s), n) if s.name == *n => {
                    let compiled = state_lower(s, self).unwrap_or_else(|e| panic!("lower {} 失败: {} @ {:?}", s.name, e.msg, e.span));
                    self.cache.insert(name.to_string(), compiled.clone());
                    return compiled;
                }
                _ => {}
            }
        }
        panic!("未找到声明 {name}");
    }

    /// 查找组合电路，供结构组合内联。
    #[allow(dead_code)]

    fn get_combinational(&mut self, name: &str, depth: usize) -> Compiled {
        if depth > 32 {
            panic!("组合递归过深（含循环引用？）:{name}");
        }
        self.compile_named(name)
    }
}

/// 查找声明。
fn find_circuit<'b>(decls: &'b [Decl], name: &str) -> Option<&'b Circuit> {
    decls.iter().find_map(|d| match d {
        Decl::Circuit(c) if c.name == name => Some(c),
        _ => None,
    })
}

#[allow(dead_code)]
fn find_state<'b>(decls: &'b [Decl], name: &str) -> Option<&'b State> {
    decls.iter().find_map(|d| match d {
        Decl::State(s) if s.name == name => Some(s),
        _ => None,
    })
}

/// 展开表达式到信号，同时累计资源。环境含参数与已赋值变量。
fn lower_expr(
    expr: &Expr,
    env: &mut Env,
    nl: &mut Netlist,
    budget: &mut ResourceBudget,
    decls: &[Decl],
    _modal: Modal,
    depth: usize,
) -> LowerResult<Value> {
    match expr {
        Expr::Lit(v, w, _sp) => {
            let width = w.bits();
            let mut sigs = Vec::with_capacity(width as usize);
            for i in 0..width {
                let bit = (v >> i) & 1;
                let s = nl.add_const(bit as u8);
                sigs.push(s);
            }
            Ok(Value { sigs, width })
        }
        Expr::Var(name, sp) => {
            // 参数/局部变量，或常量特性名（0/1）
            if let Some(v) = env.get(name) {
                return Ok(v.clone());
            }
            if name == "0" {
                return Ok(Value { sigs: vec![nl.add_const(0)], width: 1 });
            }
            if name == "1" {
                return Ok(Value { sigs: vec![nl.add_const(1)], width: 1 });
            }
            Err(LowerError::new(*sp, &format!("未定义变量/参数: {name}")))
        }
        Expr::Call(callee, args, sp) => {
            // 关键字门：AND/OR/XOR/NOT/NAND
            match callee.as_str() {
                "AND" => {
                    let va = lower_expr(&args[0], env, nl, budget, decls, _modal, depth)?;
                    let vb = lower_expr(&args[1], env, nl, budget, decls, _modal, depth)?;
                    need_width(&va, &vb, *sp)?;
                    let sigs = va.sigs.iter().zip(vb.sigs.iter()).map(|(&a, &b)| nl.and(a, b)).collect();
                    budget.add_gates(2 * va.width);
                    Ok(Value { sigs, width: va.width })
                }
                "OR" => {
                    let va = lower_expr(&args[0], env, nl, budget, decls, _modal, depth)?;
                    let vb = lower_expr(&args[1], env, nl, budget, decls, _modal, depth)?;
                    need_width(&va, &vb, *sp)?;
                    let sigs = va.sigs.iter().zip(vb.sigs.iter()).map(|(&a, &b)| nl.or(a, b)).collect();
                    budget.add_gates(3 * va.width);
                    Ok(Value { sigs, width: va.width })
                }
                "XOR" => {
                    let va = lower_expr(&args[0], env, nl, budget, decls, _modal, depth)?;
                    let vb = lower_expr(&args[1], env, nl, budget, decls, _modal, depth)?;
                    need_width(&va, &vb, *sp)?;
                    let sigs = va.sigs.iter().zip(vb.sigs.iter()).map(|(&a, &b)| nl.xor(a, b)).collect();
                    budget.add_gates(4 * va.width);
                    Ok(Value { sigs, width: va.width })
                }
                "NOT" => {
                    let va = lower_expr(&args[0], env, nl, budget, decls, _modal, depth)?;
                    let sigs = va.sigs.iter().map(|&a| nl.not(a)).collect();
                    budget.add_gates(va.width);
                    Ok(Value { sigs, width: va.width })
                }
                "NAND" => {
                    let va = lower_expr(&args[0], env, nl, budget, decls, _modal, depth)?;
                    let vb = lower_expr(&args[1], env, nl, budget, decls, _modal, depth)?;
                    need_width(&va, &vb, *sp)?;
                    let sigs = va.sigs.iter().zip(vb.sigs.iter()).map(|(&a, &b)| nl.nand(a, b)).collect();
                    budget.add_gates(va.width);
                    Ok(Value { sigs, width: va.width })
                }
                "__UPDATE" => {
                    // 由 <- 语法生成：占位，不在此处理
                    Err(LowerError::new(*sp, "__UPDATE 不应出现在表达式层"))
                }
                _ => {
                    // 结构组合：内联调用其他 circuit（编译期引用）
                    if depth > 32 {
                        return Err(LowerError::new(*sp, "组合递归过深（循环引用？）"));
                    }
                    // 参数求值
                    let mut arg_env = Env::new();
                    let circuit = find_circuit(decls, callee)
                        .ok_or_else(|| LowerError::new(*sp, &format!("未定义调用: {callee}（无此 circuit/state fn）")))?;
                    if args.len() != circuit.params.len() {
                        return Err(LowerError::new(*sp, &format!("{callee} 参数数量不符")));
                    }
                    // 逐参数求值并绑定到子环境
                    for (i, p) in circuit.params.iter().enumerate() {
                        let v = lower_expr(&args[i], env, nl, budget, decls, _modal, depth + 1)?;
                        if v.width != p.width.bits() {
                            return Err(LowerError::new(*sp, &format!("{} 参数 {} 宽度不匹配", callee, p.name)));
                        }
                        arg_env.insert(p.name.clone(), v);
                    }
                    // 递归展开 body
                    let (results, _) = compile_statements(&circuit.body, &mut arg_env, nl, budget, decls, _modal, depth + 1)?;
                    // 多输出：逐输出生成
                    if circuit.returns.len() == 1 {
                        // 单个输出返回值
                        if results.len() != 1 {
                            return Err(LowerError::new(*sp, &format!("{callee} 单输出但 return 返回 {} 个", results.len())));
                        }
                        return Ok(results[0].clone());
                    }
                    // 多输出当作拼接（低位返回在后面）
                    let mut all = Vec::new();
                    for r in results.iter().rev() {
                        all.extend(r.sigs.iter().copied());
                    }
                    let w: u32 = results.iter().map(|r| r.width).sum();
                    Ok(Value { sigs: all, width: w })
                }
            }
        }
        Expr::Index(e, i, sp) => {
            let v = lower_expr(e, env, nl, budget, decls, _modal, depth)?;
            if *i >= v.width {
                return Err(LowerError::new(*sp, "位索引越界"));
            }
            Ok(Value { sigs: vec![v.sigs[*i as usize]], width: 1 })
        }
        Expr::Slice(e, lo, hi, sp) => {
            let v = lower_expr(e, env, nl, budget, decls, _modal, depth)?;
            if *hi >= v.width || lo > hi {
                return Err(LowerError::new(*sp, "切片越界"));
            }
            Ok(Value { sigs: v.sigs[*lo as usize..=*hi as usize].to_vec(), width: hi - lo + 1 })
        }
        Expr::Concat(items, _sp) => {
            // 拼接 [a, b, c]：LSB 语义将第一个加在低位？按真值：concat 先高位
            let mut vals = Vec::new();
            for it in items {
                vals.push(lower_expr(it, env, nl, budget, decls, _modal, depth)?);
            }
            let mut total = 0u32;
            for v in &vals {
                total += v.width;
            }
            // [hi, mid, lo...]：第一个 item 是最高位。LSB-first 存储反转。
            let mut sigs = Vec::new();
            for v in vals.iter().rev() {
                sigs.extend(v.sigs.iter().copied());
            }
            Ok(Value { sigs, width: total })
        }
        Expr::Not(e, _sp) => {
            let v = lower_expr(e, env, nl, budget, decls, _modal, depth)?;
            let sigs: Vec<usize> = v.sigs.iter().map(|&a| nl.not(a)).collect();
            budget.add_gates(v.width);
            Ok(Value { sigs, width: v.width })
        }
        Expr::Bin(op, a, b, _sp) => {
            let va = lower_expr(a, env, nl, budget, decls, _modal, depth)?;
            let vb = lower_expr(b, env, nl, budget, decls, _modal, depth)?;
            let (va, vb) = coerce(nl, &va, &vb);
            match op {
                BinOp::And => {
                    let sigs = va.sigs.iter().zip(vb.sigs.iter()).map(|(&x, &y)| nl.and(x, y)).collect();
                    budget.add_gates(2 * va.width);
                    Ok(Value { sigs, width: va.width })
                }
                BinOp::Or => {
                    let sigs = va.sigs.iter().zip(vb.sigs.iter()).map(|(&x, &y)| nl.or(x, y)).collect();
                    budget.add_gates(3 * va.width);
                    Ok(Value { sigs, width: va.width })
                }
                BinOp::Xor => {
                    let sigs = va.sigs.iter().zip(vb.sigs.iter()).map(|(&x, &y)| nl.xor(x, y)).collect();
                    budget.add_gates(4 * va.width);
                    Ok(Value { sigs, width: va.width })
                }
                BinOp::Add => {
                    // 加法器：逐全加器。FA = 2XOR(8) + 2AND(4) + OR(3) = 15
                    let (sums, _cout) = nl.adder(&va.sigs, &vb.sigs);
                    budget.add_gates(15 * va.width);
                    Ok(Value { sigs: sums, width: va.width })
                }
                BinOp::Eq => {
                    // 等于：逐位 XNOR 再全部 AND。原型：N 位 → 输出 1 位。
                    let mut eq_sig = nl.add_const(1);
                    for i in 0..va.width {
                        let x = nl.xor(va.sigs[i as usize], vb.sigs[i as usize]); // 1 若不同
                        let xn = nl.not(x); // 1 若相同
                        eq_sig = nl.and(eq_sig, xn);
                    }
                    // 门数：每 bit XOR(4)+NOT(1)+AND(2) = 7, 首个 AND 用 const 输入
                    budget.add_gates(7 * va.width);
                    Ok(Value { sigs: vec![eq_sig], width: 1 })
                }
                BinOp::Ne => {
                    let mut eq_sig = nl.add_const(1);
                    for i in 0..va.width {
                        let x = nl.xor(va.sigs[i as usize], vb.sigs[i as usize]);
                        let xn = nl.not(x);
                        eq_sig = nl.and(eq_sig, xn);
                    }
                    budget.add_gates(7 * va.width);
                    let ne = nl.not(eq_sig);
                    budget.add_gates(1);
                    Ok(Value { sigs: vec![ne], width: 1 })
                }
            }
        }
        Expr::Ternary(c, t, e, sp) => {
            let vc = lower_expr(c, env, nl, budget, decls, _modal, depth)?;
            if vc.width != 1 {
                return Err(LowerError::new(*sp, "条件必须为 Bit"));
            }
            let vt = lower_expr(t, env, nl, budget, decls, _modal, depth)?;
            let ve = lower_expr(e, env, nl, budget, decls, _modal, depth)?;
            need_width(&vt, &ve, *sp)?;
            // 选择器：out = (c & t) | (!c & e)  每 bit: AND2+AND2+OR3 = 7 门
            let sigs = vt
                .sigs
                .iter()
                .zip(ve.sigs.iter())
                .map(|(&x, &y)| {
                    let ct = nl.and(vc.sigs[0], x);
                    let nc = nl.not(vc.sigs[0]);
                    let ne = nl.and(nc, y);
                    nl.or(ct, ne)
                })
                .collect::<Vec<_>>();
            budget.add_gates(7 * vt.width);
            Ok(Value { sigs, width: vt.width })
        }
    }
}

fn need_width(a: &Value, b: &Value, sp: Span) -> LowerResult<()> {
    if a.width != b.width {
        Err(LowerError::new(sp, &format!("宽度不匹配: {} vs {}", a.width, b.width)))
    } else {
        Ok(())
    }
}

/// 零扩展：将窄的 Value 高位补 0，使两边等宽（用于字面量/位宽不同的操作数）。
/// 返回新的 Value；若已等宽则原样返回。
fn coerce(nl: &mut crate::netlist::Netlist, a: &Value, b: &Value) -> (Value, Value) {
    if a.width == b.width {
        return (a.clone(), b.clone());
    }
    if a.width < b.width {
        let mut na = a.clone();
        while na.width < b.width {
            na.sigs.push(nl.add_const(0));
            na.width += 1;
        }
        return (na, b.clone());
    }
    let mut nb = b.clone();
    while nb.width < a.width {
        nb.sigs.push(nl.add_const(0));
        nb.width += 1;
    }
    (a.clone(), nb)
}

/// 编译语句块。返回最后 return 的值（可能多输出），以及环境副作用。
fn compile_statements(
    body: &[Stmt],
    env: &mut Env,
    nl: &mut Netlist,
    budget: &mut ResourceBudget,
    decls: &[Decl],
    modal: Modal,
    depth: usize,
) -> LowerResult<(Vec<Value>, Option<Span>)> {
    let mut returned: Option<(Vec<Value>, Span)> = None;
    for stmt in body {
        if returned.is_some() {
            // 已 return 之后语句不执行（保持语义）
            break;
        }
        match stmt {
            Stmt::Assign(a) => {
                // 时序更新 <-：语义检查在调用者做。这里只记录结果（调用者注册到 next_latch）
                // 展开 value；target 是 latch 或变量。
                let v = lower_expr(&a.value, env, nl, budget, decls, modal, depth)?;
                if a.targets.len() == 1 {
                    match &a.targets[0] {
                        Target::Var(name, sp) => {
                            if let Some(prev) = env.get(name) {
                                if prev.width != v.width {
                                    return Err(LowerError::new(*sp, &format!("{name} 宽度变化：{} -> {}", prev.width, v.width)));
                                }
                            }
                            // 若目标为 pin 区（参数），直接覆盖环境
                            env.insert(name.clone(), v);
                        }
                        Target::Index(inner, i, sp) => {
                            // 更新变量的第 i 位
                            let base = match inner.as_ref() {
                                Target::Var(n, _) => env.get(n).cloned().ok_or_else(|| LowerError::new(*sp, "索引目标未定义"))?,
                                _ => return Err(LowerError::new(*sp, "不支持嵌套索引赋值")),
                            };
                            if *i >= base.width as usize {
                                return Err(LowerError::new(*sp, "索引赋值越界"));
                            }
                            if v.width != 1 {
                                return Err(LowerError::new(*sp, "位赋值右值必须为 Bit"));
                            }
                            let mut new = base.sigs.clone();
                            new[*i] = v.sigs[0];
                            env.insert(inner_name(inner), Value { sigs: new, width: base.width });
                        }
                        Target::Slice(inner, lo, hi, sp) => {
                            let base = match inner.as_ref() {
                                Target::Var(n, _) => env.get(n).cloned().ok_or_else(|| LowerError::new(*sp, "切片目标未定义"))?,
                                _ => return Err(LowerError::new(*sp, "不支持嵌套切片赋值")),
                            };
                            let hold = (hi - lo) as u32 + 1;
                            if v.width != hold {
                                return Err(LowerError::new(*sp, "切片赋值宽度不匹配"));
                            }
                            let mut new = base.sigs.clone();
                            for (i, s) in v.sigs.iter().enumerate() {
                                new[lo + i] = *s;
                            }
                            env.insert(inner_name(inner), Value { sigs: new, width: base.width });
                        }
                    }
                } else {
                    // 多目标：map 到 targets 的 value 展平（从左到右，LSB-first）
                    let mut off = 0usize;
                    for t in &a.targets {
                        match t {
                            Target::Var(name, sp) => {
                                let w = env.get(name).map(|s| s.width as usize).unwrap_or(0);
                                if w == 0 {
                                    return Err(LowerError::new(*sp, "多目标赋值宽度未知"));
                                }
                                let slice: Vec<usize> = v.sigs[off..off + w].to_vec();
                                env.insert(name.clone(), Value { sigs: slice, width: w as u32 });
                                off += w;
                            }
                            _ => return Err(LowerError::new(a.span, "多目标只支持变量")),
                        }
                    }
                }
            }
            Stmt::Return(exprs) => {
                let mut vals = Vec::new();
                for e in exprs {
                    vals.push(lower_expr(e, env, nl, budget, decls, modal, depth)?);
                }
                returned = Some((vals, stmt_span_safe()));
            }
            Stmt::If(if_stmt) => {
                let vc = lower_expr(&if_stmt.cond, env, nl, budget, decls, modal, depth)?;
                if vc.width != 1 {
                    return Err(LowerError::new(if_stmt.span, "if 条件必须为 Bit"));
                }
                // 分支环境快照
                let mut then_env = env.clone();
                let mut else_env = env.clone();
                // 注意：if 内的 return 不处理（原型限制：若有 return 抛出说明）
                let (_, then_ret) = compile_statements(&if_stmt.then_body, &mut then_env, nl, budget, decls, modal, depth + 1)?;
                let (_, else_ret) = compile_statements(&if_stmt.else_body, &mut else_env, nl, budget, decls, modal, depth + 1)?;
                if then_ret.is_some() || else_ret.is_some() {
                    return Err(LowerError::new(if_stmt.span, "if 内 return 暂不支持（原型）"));
                }
                // 合并环境：每变量 mux
                let all_keys: Vec<String> = {
                    let mut ks: Vec<String> = then_env.keys().cloned().collect();
                    for k in else_env.keys() {
                        if !ks.contains(k) {
                            ks.push(k.clone());
                        }
                    }
                    ks
                };
                for k in all_keys {
                    let tv = then_env.get(&k);
                    let ev = else_env.get(&k);
                    match (tv, ev) {
                        (Some(t), Some(e)) => {
                            if t.width != e.width {
                                return Err(LowerError::new(if_stmt.span, &format!("{k} 分支宽度不一致")));
                            }
                            let sigs = t
                                .sigs
                                .iter()
                                .zip(e.sigs.iter())
                                .map(|(&x, &y)| {
                                    let ct = nl.and(vc.sigs[0], x);
                                    let nc = nl.not(vc.sigs[0]);
                                    let ne = nl.and(nc, y);
                                    nl.or(ct, ne)
                                })
                                .collect::<Vec<_>>();
                            budget.add_gates(8 * t.width);
                            env.insert(k, Value { sigs, width: t.width });
                        }
                        _ => {
                            // 单分支才有的变量：保持（不合并），原型宽松处理
                        }
                    }
                }
            }
            Stmt::AssertResource(_, _, _) => { /* 已在编译后统一校验 */ }
        }
    }
    match returned {
        Some((v, _)) => Ok((v, Some(if_stmt_default_span()))),
        None => Ok((Vec::new(), None)),
    }
}

fn stmt_span_safe() -> Span {
    Span::new(0, 0)
}
fn if_stmt_default_span() -> Span {
    Span::new(0, 0)
}
fn inner_name(t: &Target) -> String {
    match t {
        Target::Var(n, _) => n.clone(),
        Target::Index(i, _, _) | Target::Slice(i, _, _, _) => inner_name(i),
    }
}

/// 编译组合电路。
fn circuit_lower(c: &Circuit, compiler: &mut Compiler, depth: usize, stack: Vec<String>) -> LowerResult<Compiled> {
    let mut nl = Netlist::default();
    let mut budget = ResourceBudget::default();
    let mut env = Env::new();
    // 参数 → 输入信号
    for p in &c.params {
        let sigs: Vec<usize> = (0..p.width.bits()).map(|_| nl.new_sig()).collect();
        let mut gates = Vec::new();
        for (i, s) in sigs.iter().enumerate() {
            gates.push(Gate::Input { out: *s, name: format!("{}_{}", p.name, i) });
            nl.inputs.insert(format!("{}_{}", p.name, i), *s);
            nl.depths.insert(*s, 0);
        }
        nl.gates.extend(gates);
        env.insert(p.name.clone(), Value { sigs: sigs.clone(), width: p.width.bits() });
    }
    // 编译 body
    let (results, _) = compile_statements(&c.body, &mut env, &mut nl, &mut budget, compiler.decls, Modal::Combinational, depth)?;
    // 输出
    let mut output_sigs = Vec::new();
    for (i, p) in c.returns.iter().enumerate() {
        let v = results.get(i).ok_or_else(|| LowerError::new(c.span, &format!("{} 缺少 return 值 #{}", c.name, i)))?;
        if v.width != p.width.bits() {
            return Err(LowerError::new(c.span, &format!("{} 输出 {} 宽度不匹配 {} vs {}", c.name, p.name, v.width, p.width.bits())));
        }
        // 输出信号（LSB-first），按返回声明顺序
        output_sigs.extend(v.sigs.iter().copied());
        for (j, s) in v.sigs.iter().enumerate() {
            nl.outputs.insert(format!("{}_{}", p.name, j), *s);
        }
    }
    // 资源校验：从网表统计
    let stats = nl.stats();
    budget.gates = stats.nand_count;
    budget.depth = stats.max_depth;
    budget.latches = stats.latch_count;
    // 与声明校验
    let errs = budget.check_bounds(&c.bounds);
    if !errs.is_empty() {
        let msg = errs.join("; ");
        return Err(LowerError::new(c.span, &msg));
    }
    let _ = stack;
    Ok(Compiled::Combinational {
        name: c.name.clone(),
        inputs: c.params.clone(),
        outputs: c.returns.clone(),
        output_sigs,
        netlist: nl,
        budget,
    })
}

/// 编译 state：每个 fn 独立编译为一个组合网表（输入 = latch 状态 + fn 参数）。
fn state_lower(s: &State, compiler: &mut Compiler) -> LowerResult<Compiled> {
    let mut fns = Vec::new();
    for f in &s.fns {
        let mut nl = Netlist::default();
        let mut budget = ResourceBudget::default();
        let mut env = Env::new();
        // latch 状态作为输入
        for l in &s.latches {
            let sigs: Vec<usize> = (0..l.width.bits()).map(|_| nl.new_sig()).collect();
            for (i, sig) in sigs.iter().enumerate() {
                let g = Gate::Input { out: *sig, name: format!("latch_{}_{}", l.name, i) };
                nl.gates.push(g);
                nl.inputs.insert(format!("latch_{}_{}", l.name, i), *sig);
                nl.depths.insert(*sig, 0);
            }
            env.insert(l.name.clone(), Value { sigs: sigs.clone(), width: l.width.bits() });
        }
        // fn 参数
        for p in &f.params {
            let sigs: Vec<usize> = (0..p.width.bits()).map(|_| nl.new_sig()).collect();
            for (i, sig) in sigs.iter().enumerate() {
                let g = Gate::Input { out: *sig, name: format!("{}_{}", p.name, i) };
                nl.gates.push(g);
                nl.inputs.insert(format!("{}_{}", p.name, i), *sig);
                nl.depths.insert(*sig, 0);
            }
            env.insert(p.name.clone(), Value { sigs: sigs.clone(), width: p.width.bits() });
        }
        // 编译 body，追踪 latch 更新
        let mut next_latch: Vec<(String, Vec<usize>)> = Vec::new();
        // 捕获 <-：这里简化：赋值给 latch 名的记录下来
        let (results, _) = compile_fn_body_with_updates(&f.body, &mut env, &mut nl, &mut budget, compiler.decls, &mut next_latch, &s.latches)?;
        // 输出
        let mut output_sigs = Vec::new();
        for (i, p) in f.returns.iter().enumerate() {
            let v = results.get(i).ok_or_else(|| LowerError::new(f.span, &format!("{} fn {} 缺少 return 值 #{}", s.name, f.name, i)))?;
            output_sigs.extend(v.sigs.iter().copied());
            for (j, sig) in v.sigs.iter().enumerate() {
                nl.outputs.insert(format!("{}_{}", p.name, j), *sig);
            }
        }
        let stats = nl.stats();
        budget.gates = stats.nand_count;
        budget.depth = stats.max_depth;
        budget.latches = s.latches.iter().map(|l| l.width.bits()).sum();
        budget.cycles = 1;
        // 检查 latch 更新目标
        for (lname, _) in &next_latch {
            if !s.latches.iter().any(|l| &l.name == lname) {
                return Err(LowerError::new(f.span, &format!("未声明 latch: {lname}")));
            }
        }
        fns.push(CompiledFn {
            name: f.name.clone(),
            params: f.params.clone(),
            returns: f.returns.clone(),
            output_sigs,
            next_latch_sigs: next_latch,
            netlist: nl,
            budget,
        });
    }
    Ok(Compiled::State { name: s.name.clone(), latches: s.latches.clone(), fns })
}

/// 类似 compile_statements，但追踪 <- 更新到 latch。
fn compile_fn_body_with_updates(
    body: &[Stmt],
    env: &mut Env,
    nl: &mut Netlist,
    budget: &mut ResourceBudget,
    decls: &[Decl],
    next_latch: &mut Vec<(String, Vec<usize>)>,
    latches: &[LatchDecl],
) -> LowerResult<(Vec<Value>, Option<Span>)> {
    let mut returned: Option<(Vec<Value>, Span)> = None;
    for stmt in body {
        if returned.is_some() {
            break;
        }
        match stmt {
            Stmt::Assign(a) => {
                // <- 更新：目标为 latch
                if let Expr::Call(callee, args, _) = &a.value {
                    if callee == "__UPDATE" {
                        let v = lower_expr(&args[0], env, nl, budget, decls, Modal::Sequential, 0)?;
                        if a.targets.len() != 1 {
                            return Err(LowerError::new(a.span, "<- 只能单目标"));
                        }
                        match &a.targets[0] {
                            Target::Var(name, sp) => {
                                // 必须是 latch
                                let lw = latches.iter().find(|l| &l.name == name).map(|l| l.width.bits());
                                match lw {
                                    Some(w) if w == v.width => {
                                        next_latch.push((name.clone(), v.sigs.clone()));
                                        // env 同步更新
                                        env.insert(name.clone(), v);
                                    }
                                    Some(_) => return Err(LowerError::new(*sp, &format!("latch {name} 宽度不匹配"))),
                                    None => return Err(LowerError::new(*sp, &format!("<- 目标不是 latch: {name}"))),
                                }
                            }
                            _ => return Err(LowerError::new(a.span, "<- 目标必须为 latch 变量")),
                        }
                        continue;
                    }
                }
                // = 赋值：普通
                let v = lower_expr(&a.value, env, nl, budget, decls, Modal::Sequential, 0)?;
                if a.targets.len() == 1 {
                    match &a.targets[0] {
                        Target::Var(name, sp) => {
                            if let Some(prev) = env.get(name) {
                                if prev.width != v.width {
                                    return Err(LowerError::new(*sp, &format!("{name} 宽度变化")));
                                }
                            }
                            env.insert(name.clone(), v);
                        }
                        Target::Index(inner, i, sp) => {
                            let base = match inner.as_ref() {
                                Target::Var(n, _) => env.get(n).cloned().ok_or_else(|| LowerError::new(*sp, "索引目标未定义"))?,
                                _ => return Err(LowerError::new(*sp, "不支持嵌套索引赋值")),
                            };
                            if *i >= base.width as usize {
                                return Err(LowerError::new(*sp, "索引赋值越界"));
                            }
                            let mut new = base.sigs.clone();
                            new[*i] = v.sigs[0];
                            env.insert(inner_name(inner), Value { sigs: new, width: base.width });
                        }
                        Target::Slice(inner, lo, hi, sp) => {
                            let base = match inner.as_ref() {
                                Target::Var(n, _) => env.get(n).cloned().ok_or_else(|| LowerError::new(*sp, "切片目标未定义"))?,
                                _ => return Err(LowerError::new(*sp, "不支持嵌套切片赋值")),
                            };
                            let _hold = (hi - lo) as u32 + 1;
                            let mut new = base.sigs.clone();
                            for (i, s) in v.sigs.iter().enumerate() {
                                new[lo + i] = *s;
                            }
                            env.insert(inner_name(inner), Value { sigs: new, width: base.width });
                        }
                    }
                } else {
                    let mut off = 0usize;
                    for t in &a.targets {
                        match t {
                            Target::Var(name, sp) => {
                                let w = env.get(name).map(|s| s.width as usize).unwrap_or(0);
                                if w == 0 {
                                    return Err(LowerError::new(*sp, "多目标赋值宽度未知"));
                                }
                                let slice: Vec<usize> = v.sigs[off..off + w].to_vec();
                                env.insert(name.clone(), Value { sigs: slice, width: w as u32 });
                                off += w;
                            }
                            _ => return Err(LowerError::new(a.span, "多目标只支持变量")),
                        }
                    }
                }
            }
            Stmt::Return(exprs) => {
                let mut vals = Vec::new();
                for e in exprs {
                    vals.push(lower_expr(e, env, nl, budget, decls, Modal::Sequential, 0)?);
                }
                returned = Some((vals, stmt_span_safe()));
            }
            Stmt::If(if_stmt) => {
                // 简化：if 内不支持 latch 更新（原型），但支持组合选择
                let vc = lower_expr(&if_stmt.cond, env, nl, budget, decls, Modal::Sequential, 0)?;
                let mut then_env = env.clone();
                let mut else_env = env.clone();
                let (_, then_ret) = compile_fn_body_with_updates(&if_stmt.then_body, &mut then_env, nl, budget, decls, next_latch, latches)?;
                let (_, else_ret) = compile_fn_body_with_updates(&if_stmt.else_body, &mut else_env, nl, budget, decls, next_latch, latches)?;
                if then_ret.is_some() || else_ret.is_some() {
                    return Err(LowerError::new(if_stmt.span, "if 内 return 暂不支持"));
                }
                let all_keys: Vec<String> = {
                    let mut ks: Vec<String> = then_env.keys().cloned().collect();
                    for k in else_env.keys() {
                        if !ks.contains(k) {
                            ks.push(k.clone());
                        }
                    }
                    ks
                };
                for k in all_keys {
                    let tv = then_env.get(&k);
                    let ev = else_env.get(&k);
                    match (tv, ev) {
                        (Some(t), Some(e)) => {
                            if t.width != e.width {
                                return Err(LowerError::new(if_stmt.span, &format!("{k} 分支宽度不一致")));
                            }
                            let sigs = t
                                .sigs
                                .iter()
                                .zip(e.sigs.iter())
                                .map(|(&x, &y)| {
                                    let ct = nl.and(vc.sigs[0], x);
                                    let nc = nl.not(vc.sigs[0]);
                                    let ne = nl.and(nc, y);
                                    nl.or(ct, ne)
                                })
                                .collect::<Vec<_>>();
                            budget.add_gates(8 * t.width);
                            env.insert(k, Value { sigs, width: t.width });
                        }
                        _ => {}
                    }
                }
            }
            Stmt::AssertResource(_, _, _) => {}
        }
    }
    Ok(returned.map(|(v, sp)| (v, Some(sp))).unwrap_or_default())
}