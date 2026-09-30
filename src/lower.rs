//! 语义保持编译：统一 AST → NAND/LATCH 网表（L1 门级）。
//!
//! 对应白皮书 v2.2 §7.2 编译流程的 "L1 门级 AST → NAND/LATCH 网表" 与
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

/// 单个电路内联展开的网表规模上限（防组合爆炸导致 OOM/挂死）。
/// 模板库最大仅 60 门，100k 为极宽裕的原型上限。
const MAX_NETLIST_GATES: usize = 100_000;

/// 全部声明累计门数上限（防"多声明 × 大模板"的跨声明总量 OOM）。
const MAX_TOTAL_GATES: usize = 500_000;

/// 单个端口/latch 的位宽上限。
///
/// 网表本身是**逐位**的（`Bits<N>` = N 个信号），因此对宽度没有结构性限制；
/// 这个上限只用于阻止 `Bits<100000>` 这类把网表撑爆的声明。
/// 设为 256 以支持真实 `uint256`。
///
/// **注意**：`sim.rs` / `verify.rs` / `equiv.rs` 内部用 `u128` 表示端口值，
/// 因此位宽 > 128 的电路**不能**被模拟或穷举（形式化证明走 CNF，不受此限）。
const MAX_WIDTH: u32 = 256;

fn compiled_gates(c: &Compiled) -> usize {
    match c {
        Compiled::Combinational { netlist, .. } => netlist.gates.len(),
        Compiled::State { fns, .. } => fns.iter().map(|f| f.netlist.gates.len()).sum(),
    }
}

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
    pub fn compile_all(&mut self) -> Result<Vec<Compiled>, String> {
        let mut out = Vec::new();
        let mut total_gates: usize = 0;
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
            let c = self.compile_named(&n)?;
            total_gates += compiled_gates(&c);
            if total_gates > MAX_TOTAL_GATES {
                return Err(format!("全部声明累计门数超限（> {MAX_TOTAL_GATES}）"));
            }
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
        Ok(out)
    }

    fn compile_named(&mut self, name: &str) -> Result<Compiled, String> {
        if let Some(c) = self.cache.get(name) {
            return Ok(c.clone());
        }
        // 查找声明
        for d in self.decls {
            match (d, name) {
                (Decl::Circuit(c), n) if c.name == *n => {
                    let compiled = circuit_lower(c, self, 0, Vec::new())
                        .map_err(|e| format!("lower {} 失败: {} @ {:?}", c.name, e.msg, e.span))?;
                    self.cache.insert(name.to_string(), compiled.clone());
                    return Ok(compiled);
                }
                (Decl::State(s), n) if s.name == *n => {
                    let compiled = state_lower(s, self)
                        .map_err(|e| format!("lower {} 失败: {} @ {:?}", s.name, e.msg, e.span))?;
                    self.cache.insert(name.to_string(), compiled.clone());
                    return Ok(compiled);
                }
                _ => {}
            }
        }
        Err(format!("未找到声明 {name}"))
    }

    /// 查找组合电路，供结构组合内联。
    #[allow(dead_code)]

    fn get_combinational(&mut self, name: &str, depth: usize) -> Compiled {
        if depth > 32 {
            panic!("组合递归过深（含循环引用？）:{name}");
        }
        self.compile_named(name).unwrap_or_else(|e| panic!("{e}"))
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
    decls: &[Decl],
    _modal: Modal,
    depth: usize,
    edepth: usize,
) -> LowerResult<Value> {
    if edepth > 512 {
        return Err(LowerError::new(Span::new(0, 0), "表达式嵌套过深（求值）"));
    }
    // 每次表达式求值入口检查网表规模：单条 giant 表达式（如巨型 return）也受限于此。
    if nl.gates.len() > MAX_NETLIST_GATES {
        return Err(LowerError::new(Span::new(0, 0), "网表规模超限（表达式过大）"));
    }
    match expr {
        Expr::Lit(v, w, sp) => {
            let width = w.bits();
            if width > 128 {
                return Err(LowerError::new(*sp, "字面量位宽 > 128 不支持"));
            }
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
            // 门原语元数校验：避免 args[i] 越界 panic（如 NAND(a)）
            match callee.as_str() {
                "AND" | "OR" | "XOR" | "NAND" if args.len() != 2 => {
                    return Err(LowerError::new(*sp, &format!("{callee} 需要 2 个参数，实际 {}", args.len())));
                }
                "NOT" if args.len() != 1 => {
                    return Err(LowerError::new(*sp, &format!("NOT 需要 1 个参数，实际 {}", args.len())));
                }
                _ => {}
            }
            // 关键字门：AND/OR/XOR/NOT/NAND
            match callee.as_str() {
                "AND" => {
                    let va = lower_expr(&args[0], env, nl, decls, _modal, depth, edepth + 1)?;
                    let vb = lower_expr(&args[1], env, nl, decls, _modal, depth, edepth + 1)?;
                    need_width(&va, &vb, *sp)?;
                    let sigs = va.sigs.iter().zip(vb.sigs.iter()).map(|(&a, &b)| nl.and(a, b)).collect();
                    Ok(Value { sigs, width: va.width })
                }
                "OR" => {
                    let va = lower_expr(&args[0], env, nl, decls, _modal, depth, edepth + 1)?;
                    let vb = lower_expr(&args[1], env, nl, decls, _modal, depth, edepth + 1)?;
                    need_width(&va, &vb, *sp)?;
                    let sigs = va.sigs.iter().zip(vb.sigs.iter()).map(|(&a, &b)| nl.or(a, b)).collect();
                    Ok(Value { sigs, width: va.width })
                }
                "XOR" => {
                    let va = lower_expr(&args[0], env, nl, decls, _modal, depth, edepth + 1)?;
                    let vb = lower_expr(&args[1], env, nl, decls, _modal, depth, edepth + 1)?;
                    need_width(&va, &vb, *sp)?;
                    let sigs = va.sigs.iter().zip(vb.sigs.iter()).map(|(&a, &b)| nl.xor(a, b)).collect();
                    Ok(Value { sigs, width: va.width })
                }
                "NOT" => {
                    let va = lower_expr(&args[0], env, nl, decls, _modal, depth, edepth + 1)?;
                    let sigs = va.sigs.iter().map(|&a| nl.not(a)).collect();
                    Ok(Value { sigs, width: va.width })
                }
                "NAND" => {
                    let va = lower_expr(&args[0], env, nl, decls, _modal, depth, edepth + 1)?;
                    let vb = lower_expr(&args[1], env, nl, decls, _modal, depth, edepth + 1)?;
                    need_width(&va, &vb, *sp)?;
                    let sigs = va.sigs.iter().zip(vb.sigs.iter()).map(|(&a, &b)| nl.nand(a, b)).collect();
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
                    // 网表规模上限：内联无记忆化，重复引用会导致 2^k 指数爆炸；
                    // 超限时干净报错，而非 OOM / 挂死。并计入展开次数（纯透传链不增门/信号）。
                    nl.expansions += 1;
                    if nl.expansions > 200_000 || nl.gates.len() > MAX_NETLIST_GATES {
                        return Err(LowerError::new(*sp, "网表规模超限（组合爆炸或循环引用？）"));
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
                        let v = lower_expr(&args[i], env, nl, decls, _modal, depth + 1, edepth + 1)?;
                        if v.width != p.width.bits() {
                            return Err(LowerError::new(*sp, &format!("{} 参数 {} 宽度不匹配", callee, p.name)));
                        }
                        arg_env.insert(p.name.clone(), v);
                    }
                    // 递归展开 body
                    let (results, _) = compile_statements(&circuit.body, &mut arg_env, nl, decls, _modal, depth + 1)?;
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
            let v = lower_expr(e, env, nl, decls, _modal, depth, edepth + 1)?;
            if *i >= v.width {
                return Err(LowerError::new(*sp, "位索引越界"));
            }
            Ok(Value { sigs: vec![v.sigs[*i as usize]], width: 1 })
        }
        Expr::Slice(e, lo, hi, sp) => {
            let v = lower_expr(e, env, nl, decls, _modal, depth, edepth + 1)?;
            if *hi >= v.width || lo > hi {
                return Err(LowerError::new(*sp, "切片越界"));
            }
            Ok(Value { sigs: v.sigs[*lo as usize..=*hi as usize].to_vec(), width: hi - lo + 1 })
        }
        Expr::Concat(items, _sp) => {
            // 拼接 [a, b, c]：LSB 语义将第一个加在低位？按真值：concat 先高位
            let mut vals = Vec::new();
            for it in items {
                vals.push(lower_expr(it, env, nl, decls, _modal, depth, edepth + 1)?);
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
            let v = lower_expr(e, env, nl, decls, _modal, depth, edepth + 1)?;
            let sigs: Vec<usize> = v.sigs.iter().map(|&a| nl.not(a)).collect();
            Ok(Value { sigs, width: v.width })
        }
        Expr::Bin(op, a, b, _sp) => {
            let va = lower_expr(a, env, nl, decls, _modal, depth, edepth + 1)?;
            let vb = lower_expr(b, env, nl, decls, _modal, depth, edepth + 1)?;
            let (va, vb) = coerce(nl, &va, &vb);
            match op {
                BinOp::And => {
                    let sigs = va.sigs.iter().zip(vb.sigs.iter()).map(|(&x, &y)| nl.and(x, y)).collect();
                    Ok(Value { sigs, width: va.width })
                }
                BinOp::Or => {
                    let sigs = va.sigs.iter().zip(vb.sigs.iter()).map(|(&x, &y)| nl.or(x, y)).collect();
                    Ok(Value { sigs, width: va.width })
                }
                BinOp::Xor => {
                    let sigs = va.sigs.iter().zip(vb.sigs.iter()).map(|(&x, &y)| nl.xor(x, y)).collect();
                    Ok(Value { sigs, width: va.width })
                }
                BinOp::Add => {
                    // 加法器：逐全加器。FA = 2XOR(8) + 2AND(4) + OR(3) = 15
                    let (sums, _cout) = nl.adder(&va.sigs, &vb.sigs);
                    Ok(Value { sigs: sums, width: va.width })
                }
                BinOp::Sub => {
                    // 减法：a + ~b + 1（**单条进位链**，结果模 2^N）
                    let nb: Vec<usize> = vb.sigs.iter().map(|&s| nl.not(s)).collect();
                    let one = nl.add_const(1);
                    let (sums, _cout) = nl.adder_cin(&va.sigs, &nb, one);
                    Ok(Value { sigs: sums, width: va.width })
                }
                BinOp::Lt => {
                    let s = lower_lt(nl, &va, &vb);
                    Ok(Value { sigs: vec![s], width: 1 })
                }
                BinOp::Gt => {
                    let s = lower_lt(nl, &vb, &va);
                    Ok(Value { sigs: vec![s], width: 1 })
                }
                BinOp::Le => {
                    let s = lower_lt(nl, &vb, &va);
                    let s = nl.not(s);
                    Ok(Value { sigs: vec![s], width: 1 })
                }
                BinOp::Ge => {
                    let s = lower_lt(nl, &va, &vb);
                    let s = nl.not(s);
                    Ok(Value { sigs: vec![s], width: 1 })
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
                    Ok(Value { sigs: vec![eq_sig], width: 1 })
                }
                BinOp::Ne => {
                    let mut eq_sig = nl.add_const(1);
                    for i in 0..va.width {
                        let x = nl.xor(va.sigs[i as usize], vb.sigs[i as usize]);
                        let xn = nl.not(x);
                        eq_sig = nl.and(eq_sig, xn);
                    }
                    let ne = nl.not(eq_sig);
                    Ok(Value { sigs: vec![ne], width: 1 })
                }
            }
        }
        Expr::Ternary(c, t, e, sp) => {
            let vc = lower_expr(c, env, nl, decls, _modal, depth, edepth + 1)?;
            if vc.width != 1 {
                return Err(LowerError::new(*sp, "条件必须为 Bit"));
            }
            let vt = lower_expr(t, env, nl, decls, _modal, depth, edepth + 1)?;
            let ve = lower_expr(e, env, nl, decls, _modal, depth, edepth + 1)?;
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
            Ok(Value { sigs, width: vt.width })
        }
    }
}

/// 无符号比较 `a < b`（1 位）：单条进位链 `a + ~b + 1` 的进位取反。
///
/// 必须用单条进位链：把 `a + ~b + 1` 拆成两次加法会丢掉第一次的进位，
/// 使借位判断错误（比较结果会错）。
fn lower_lt(nl: &mut Netlist, a: &Value, b: &Value) -> usize {
    let nb: Vec<usize> = b.sigs.iter().map(|&s| nl.not(s)).collect();
    let one = nl.add_const(1);
    let (_, carry) = nl.adder_cin(&a.sigs, &nb, one);
    nl.not(carry)
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
        // 累计门数上限：内联之外的语句体也可生成海量门（如数千条 128 位加法）→ 防 OOM
        if nl.gates.len() > MAX_NETLIST_GATES {
            return Err(LowerError::new(stmt_span_safe(), "网表规模超限（累计门数过多）"));
        }
        match stmt {
            Stmt::Assign(a) => {
                // 时序更新 <-：语义检查在调用者做。这里只记录结果（调用者注册到 next_latch）
                // 展开 value；target 是 latch 或变量。
                let v = lower_expr(&a.value, env, nl, decls, modal, depth, 0)?;
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
                            if lo > hi || *hi >= base.width as usize {
                                return Err(LowerError::new(*sp, "切片赋值越界"));
                            }
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
                                if off + w > v.sigs.len() {
                                    return Err(LowerError::new(*sp, "多目标赋值超出右值宽度"));
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
                    vals.push(lower_expr(e, env, nl, decls, modal, depth, 0)?);
                }
                returned = Some((vals, stmt_span_safe()));
            }
            Stmt::If(if_stmt) => {
                let vc = lower_expr(&if_stmt.cond, env, nl, decls, modal, depth, 0)?;
                if vc.width != 1 {
                    return Err(LowerError::new(if_stmt.span, "if 条件必须为 Bit"));
                }
                // 分支环境快照
                let mut then_env = env.clone();
                let mut else_env = env.clone();
                // 注意：if 内的 return 不处理（原型限制：若有 return 抛出说明）
                let (_, then_ret) = compile_statements(&if_stmt.then_body, &mut then_env, nl, decls, modal, depth + 1)?;
                let (_, else_ret) = compile_statements(&if_stmt.else_body, &mut else_env, nl, decls, modal, depth + 1)?;
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
    // 原型以 u128 表示信号：输入/输出位宽一律 ≤128，否则仿真/验证会静默截断（假通过）。
    for p in &c.params {
        if p.width.bits() > MAX_WIDTH {
            return Err(LowerError::new(c.span, &format!("输入位宽 > {MAX_WIDTH} 不支持")));
        }
    }
    for p in &c.returns {
        if p.width.bits() > MAX_WIDTH {
            return Err(LowerError::new(c.span, &format!("输出位宽 > {MAX_WIDTH} 不支持")));
        }
    }
    // 重复输出名会使 nl.outputs 覆盖 → 验证/等价读到同一信号，拒绝。
    for (i, p) in c.returns.iter().enumerate() {
        for q in c.returns.iter().skip(i + 1) {
            if p.name == q.name {
                return Err(LowerError::new(c.span, "重复的输出参数名"));
            }
        }
    }
    // 重复输入名会产生重复信号 + 冲突的 input 名（FCT IR 会丢输入），拒绝。
    for (i, p) in c.params.iter().enumerate() {
        for q in c.params.iter().skip(i + 1) {
            if p.name == q.name {
                return Err(LowerError::new(c.span, "重复的输入参数名"));
            }
        }
    }
    // 参数 → 输入信号
    for p in &c.params {
        if nl.gates.len() + p.width.bits() as usize > MAX_NETLIST_GATES {
            return Err(LowerError::new(c.span, "输入位宽超限（Bits<N> 过大）"));
        }
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
    let (results, _) = compile_statements(&c.body, &mut env, &mut nl, compiler.decls, Modal::Combinational, depth)?;
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
    // 重复 latch 名 / fn 名会产生冲突信号与重复 FCT 产物条目，拒绝。
    for (i, l) in s.latches.iter().enumerate() {
        for q in s.latches.iter().skip(i + 1) {
            if l.name == q.name {
                return Err(LowerError::new(s.span, "重复的 latch 名"));
            }
        }
    }
    for (i, f) in s.fns.iter().enumerate() {
        for g in s.fns.iter().skip(i + 1) {
            if f.name == g.name {
                return Err(LowerError::new(s.span, "重复的 fn 名"));
            }
        }
    }
    let mut fns = Vec::new();
    for f in &s.fns {
        let mut nl = Netlist::default();
        let mut budget = ResourceBudget::default();
        let mut env = Env::new();
        // 位宽 ≤128（同组合电路）
        for l in &s.latches {
            if l.width.bits() > MAX_WIDTH {
                return Err(LowerError::new(s.span, &format!("latch 位宽 > {MAX_WIDTH} 不支持")));
            }
        }
        for p in &f.params {
            if p.width.bits() > MAX_WIDTH {
                return Err(LowerError::new(f.span, &format!("参数位宽 > {MAX_WIDTH} 不支持")));
            }
        }
        for p in &f.returns {
            if p.width.bits() > MAX_WIDTH {
                return Err(LowerError::new(f.span, &format!("输出位宽 > {MAX_WIDTH} 不支持")));
            }
        }
        for (i, p) in f.params.iter().enumerate() {
            for q in f.params.iter().skip(i + 1) {
                if p.name == q.name {
                    return Err(LowerError::new(f.span, "重复的参数名"));
                }
            }
        }
        for (i, p) in f.returns.iter().enumerate() {
            for q in f.returns.iter().skip(i + 1) {
                if p.name == q.name {
                    return Err(LowerError::new(f.span, "重复的输出名"));
                }
            }
        }
        // latch 状态作为输入
        for l in &s.latches {
            if nl.gates.len() + l.width.bits() as usize > MAX_NETLIST_GATES {
                return Err(LowerError::new(s.span, "latch 位宽超限"));
            }
            let sigs: Vec<usize> = (0..l.width.bits()).map(|_| nl.new_sig()).collect();
            for (i, sig) in sigs.iter().enumerate() {
                let g = Gate::Input { out: *sig, name: format!("latch_{}_{}", l.name, i) };
                nl.gates.push(g);
                nl.inputs.insert(format!("latch_{}_{}", l.name, i), *sig);
                nl.depths.insert(*sig, 0);
            }
            env.insert(l.name.clone(), Value { sigs: sigs.clone(), width: l.width.bits() });
        }
        // 参数名不得与 latch 输入前缀冲突（否则参数信号会覆盖 latch 信号 → 时序验证假通过）
        for p in &f.params {
            for l in &s.latches {
                if p.name == format!("latch_{}", l.name) {
                    return Err(LowerError::new(p.span, "参数名与保留前缀 latch_ 冲突"));
                }
            }
        }
        // fn 参数
        for p in &f.params {
            if nl.gates.len() + p.width.bits() as usize > MAX_NETLIST_GATES {
                return Err(LowerError::new(f.span, "参数位宽超限"));
            }
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
        let (results, _) = compile_fn_body_with_updates(&f.body, &mut env, &mut nl, compiler.decls, &mut next_latch, &s.latches)?;
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
    decls: &[Decl],
    next_latch: &mut Vec<(String, Vec<usize>)>,
    latches: &[LatchDecl],
) -> LowerResult<(Vec<Value>, Option<Span>)> {
    let mut returned: Option<(Vec<Value>, Span)> = None;
    for stmt in body {
        if returned.is_some() {
            break;
        }
        if nl.gates.len() > MAX_NETLIST_GATES {
            return Err(LowerError::new(stmt_span_safe(), "网表规模超限（累计门数过多）"));
        }
        match stmt {
            Stmt::Assign(a) => {
                // <- 更新：目标为 latch
                if let Expr::Call(callee, args, _) = &a.value {
                    if callee == "__UPDATE" {
                        if args.len() != 1 {
                            return Err(LowerError::new(a.span, "__UPDATE 需要且仅需 1 个参数"));
                        }
                        let v = lower_expr(&args[0], env, nl, decls, Modal::Sequential, 0, 0)?;
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
                let v = lower_expr(&a.value, env, nl, decls, Modal::Sequential, 0, 0)?;
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
                            if lo > hi || *hi >= base.width as usize {
                                return Err(LowerError::new(*sp, "切片赋值越界"));
                            }
                            let _hold = (hi - lo) as u32 + 1;
                            if v.width != _hold {
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
                    let mut off = 0usize;
                    for t in &a.targets {
                        match t {
                            Target::Var(name, sp) => {
                                let w = env.get(name).map(|s| s.width as usize).unwrap_or(0);
                                if w == 0 {
                                    return Err(LowerError::new(*sp, "多目标赋值宽度未知"));
                                }
                                if off + w > v.sigs.len() {
                                    return Err(LowerError::new(*sp, "多目标赋值超出右值宽度"));
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
                    vals.push(lower_expr(e, env, nl, decls, Modal::Sequential, 0, 0)?);
                }
                returned = Some((vals, stmt_span_safe()));
            }
            Stmt::If(if_stmt) => {
                // 简化：if 内不支持 latch 更新（原型），但支持组合选择
                let vc = lower_expr(&if_stmt.cond, env, nl, decls, Modal::Sequential, 0, 0)?;
                if vc.width != 1 {
                    return Err(LowerError::new(if_stmt.span, "if 条件必须为 Bit"));
                }
                let mut then_env = env.clone();
                let mut else_env = env.clone();
                // 分支内的 <- 必须显式拒绝：直接写入 next_latch 会无视条件（静默错误时序）。
                let mut then_upd: Vec<(String, Vec<usize>)> = Vec::new();
                let mut else_upd: Vec<(String, Vec<usize>)> = Vec::new();
                let (_, then_ret) = compile_fn_body_with_updates(&if_stmt.then_body, &mut then_env, nl, decls, &mut then_upd, latches)?;
                let (_, else_ret) = compile_fn_body_with_updates(&if_stmt.else_body, &mut else_env, nl, decls, &mut else_upd, latches)?;
                if then_ret.is_some() || else_ret.is_some() {
                    return Err(LowerError::new(if_stmt.span, "if 内 return 暂不支持"));
                }
                if !then_upd.is_empty() || !else_upd.is_empty() {
                    return Err(LowerError::new(if_stmt.span, "if 分支内 <- 更新暂不支持（原型）"));
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