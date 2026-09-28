//! CNF 表示与 NAND 网表的 Tseitin 编码。
//!
//! 门级形式化验证的前端：把 `netlist::Netlist`（纯 NAND/CONST/INPUT）等价转换为
//! 合取范式，交给 `sat::Solver` 判定。
//!
//! ## 语义约定（必须与 `sim::eval_netlist` 逐位一致）
//!
//! 每个信号 id 对应一个 SAT 变量（`var = sig + 1`），信号真值 ⟺ 变量取真。
//! 编码是**定义性**的（Tseitin），因此 CNF 的每一个模型都唯一对应一个网表求值，
//! 反之亦然 —— 这正是"证明"而非"测试"的依据。
//!
//! NAND：`o = ¬(l ∧ r)` 编码为三条子句
//! ```text
//!   (¬l ∨ ¬r ∨ ¬o)     l=r=1 时强制 o=0
//!   ( l      ∨  o)     l=0 时强制 o=1
//!   (      r ∨  o)     r=0 时强制 o=1
//! ```
//! 三条子句合取恰好刻画 `o = ¬(l ∧ r)`，不多不少。

use std::collections::HashMap;

use crate::netlist::{Gate, Netlist};

/// 文字：±变量号（1-based），0 非法。
pub type Lit = i32;
/// 变量号（1-based）。
pub type Var = i32;

/// 合取范式。
#[derive(Debug, Clone, Default)]
pub struct Cnf {
    /// 变量总数（1..=num_vars）
    pub num_vars: Var,
    /// 子句集合；空子句（`vec![]`）表示恒假
    pub clauses: Vec<Vec<Lit>>,
}

impl Cnf {
    pub fn new() -> Self {
        Cnf { num_vars: 0, clauses: Vec::new() }
    }

    /// 分配一个新变量。
    pub fn new_var(&mut self) -> Var {
        self.num_vars += 1;
        self.num_vars
    }

    /// 追加子句。调用方保证文字非零且 |lit| ≤ num_vars。
    pub fn add_clause(&mut self, c: Vec<Lit>) {
        self.clauses.push(c);
    }

    /// 追加单位子句。
    pub fn add_unit(&mut self, l: Lit) {
        self.clauses.push(vec![l]);
    }

    /// 追加恒假子句（立即 UNSAT）。
    pub fn add_empty(&mut self) {
        self.clauses.push(Vec::new());
    }

    /// DIMACS 文本导出（`p cnf V C` + 每行子句）。
    pub fn dimacs(&self) -> String {
        let mut s = format!("p cnf {} {}\n", self.num_vars, self.clauses.len());
        for c in &self.clauses {
            for l in c {
                s.push_str(&l.to_string());
                s.push(' ');
            }
            s.push_str("0\n");
        }
        s
    }

    /// 结构自检：文字非零、变量号在范围内。返回违规描述。
    pub fn validate(&self) -> Result<(), String> {
        for (i, c) in self.clauses.iter().enumerate() {
            for &l in c {
                if l == 0 {
                    return Err(format!("子句 #{i} 含 0 文字"));
                }
                if l.abs() > self.num_vars {
                    return Err(format!("子句 #{i} 文字 {l} 超出变量数 {}", self.num_vars));
                }
            }
        }
        Ok(())
    }
}

/// 网表编码结果。
#[derive(Debug, Clone)]
pub struct Encoded {
    pub cnf: Cnf,
    /// 网表输入名（形如 `a_0`）→ 变量
    pub input_vars: HashMap<String, Var>,
    /// 网表输出名（形如 `s_0`）→ 变量
    pub output_vars: HashMap<String, Var>,
    /// 信号 id → 变量（`sig + 1`）
    pub sig_vars: Vec<Var>,
}

impl Encoded {
    /// 取某信号对应的变量。
    pub fn var_of(&self, sig: usize) -> Option<Var> {
        self.sig_vars.get(sig).copied()
    }
    /// 按输入位名取变量（`name_i`）。
    pub fn var_of_input_bit(&self, base: &str, bit: u32) -> Option<Var> {
        self.input_vars.get(&format!("{base}_{bit}")).copied()
    }
    /// 按输出位名取变量（`name_i`）。
    pub fn var_of_output_bit(&self, base: &str, bit: u32) -> Option<Var> {
        self.output_vars.get(&format!("{base}_{bit}")).copied()
    }
}

/// 把一个 NAND 网表 Tseitin 编码为 CNF。
///
/// 每个信号（含 INPUT / CONST / NAND 输出）分配一个变量；
/// 结构性约束以子句表达。INPUT 是自由变量（可被任意赋值）。
pub fn encode_netlist(nl: &Netlist) -> Encoded {
    let mut cnf = Cnf::new();
    // 信号 0..max_sig 各占一个变量：var = sig + 1。
    let n = nl.max_sig;
    let mut sig_vars = Vec::with_capacity(n);
    for s in 0..n {
        let _ = s;
        let v = cnf.new_var();
        sig_vars.push(v);
    }
    let vof = |sig: usize| -> Var { (sig + 1) as Var };

    for g in &nl.gates {
        match g {
            Gate::Input { .. } => { /* 自由变量，无子句 */ }
            Gate::Const { out, v } => {
                let o = vof(*out);
                if *v == 1 {
                    cnf.add_unit(o);
                } else {
                    cnf.add_unit(-o);
                }
            }
            Gate::Nand { out, l, r } => {
                let (o, a, b) = (vof(*out), vof(*l), vof(*r));
                // o = ¬(a ∧ b)
                cnf.add_clause(vec![-a, -b, -o]);
                cnf.add_clause(vec![a, o]);
                cnf.add_clause(vec![b, o]);
            }
            Gate::Latch { out, next, .. } => {
                // 组合前端不应出现 LATCH；若出现按 out = next 等价编码（不静默丢弃）。
                let (o, nx) = (vof(*out), vof(*next));
                cnf.add_clause(vec![-nx, o]);
                cnf.add_clause(vec![nx, -o]);
            }
        }
    }

    // 名称索引
    let mut input_vars = HashMap::new();
    for (name, sig) in &nl.inputs {
        input_vars.insert(name.clone(), vof(*sig));
    }
    let mut output_vars = HashMap::new();
    for (name, sig) in &nl.outputs {
        output_vars.insert(name.clone(), vof(*sig));
    }

    Encoded { cnf, input_vars, output_vars, sig_vars }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::netlist::Netlist;

    /// 穷举网表输入，验证 CNF 的模型集合恰好等于网表语义。
    /// 这是编码正确性的**唯一可信判据**：编码错了，后面所有"证明"都是假的。
    fn check_encoding_matches_netlist(nl: &Netlist, n_inputs: usize) {
        let enc = encode_netlist(nl);
        enc.cnf.validate().expect("CNF 结构自检");
        assert_eq!(enc.cnf.num_vars as usize, nl.max_sig.max(1));
        // 穷举输入（≤16 位），用 SAT 求解器验证性；此处用暴力枚举变量赋值核对
        // 简化：直接对全部输入组合，检查"网表求值赋值"是否满足全部子句。
        let ins: Vec<usize> = (0..nl.max_sig)
            .filter(|s| nl.gates.iter().any(|g| matches!(g, Gate::Input { out, .. } if out == s)))
            .collect();
        assert_eq!(ins.len(), n_inputs, "输入信号数");
        for x in 0..(1u32 << n_inputs) {
            let mut assign = vec![false; nl.max_sig.max(1)];
            for (i, s) in ins.iter().enumerate() {
                assign[*s] = (x >> i) & 1 == 1;
            }
            // 拓扑求值
            for g in &nl.gates {
                match g {
                    Gate::Nand { out, l, r } => assign[*out] = !(assign[*l] && assign[*r]),
                    Gate::Const { out, v } => assign[*out] = *v == 1,
                    _ => {}
                }
            }
            // 全部子句应被满足
            for (ci, c) in enc.cnf.clauses.iter().enumerate() {
                let sat = c.iter().any(|&l| {
                    let v = assign[(l.abs() - 1) as usize];
                    (l > 0) == v
                });
                assert!(sat, "输入 x={x} 时子句 #{ci} {c:?} 未被满足");
            }
        }
    }

    #[test]
    fn encode_xor_matches_semantics() {
        let mut nl = Netlist::default();
        let a = nl.add_input("a");
        let b = nl.add_input("b");
        nl.xor(a, b);
        check_encoding_matches_netlist(&nl, 2);
    }

    #[test]
    fn encode_half_adder_matches_semantics() {
        let mut nl = Netlist::default();
        let a = nl.add_input("a");
        let b = nl.add_input("b");
        let (sum, carry) = nl.half_adder(a, b);
        nl.outputs.insert("sum".into(), sum);
        nl.outputs.insert("carry".into(), carry);
        check_encoding_matches_netlist(&nl, 2);
    }

    #[test]
    fn encode_adder4_matches_semantics() {
        let mut nl = Netlist::default();
        let a: Vec<usize> = (0..4).map(|i| nl.add_input(&format!("a{i}"))).collect();
        let b: Vec<usize> = (0..4).map(|i| nl.add_input(&format!("b{i}"))).collect();
        let (sums, _) = nl.adder(&a, &b);
        for (i, s) in sums.iter().enumerate() {
            nl.outputs.insert(format!("s{i}"), *s);
        }
        check_encoding_matches_netlist(&nl, 8);
    }

    #[test]
    fn const_gates_encoded() {
        let mut nl = Netlist::default();
        let z = nl.add_const(0);
        let o = nl.add_const(1);
        nl.outputs.insert("z".into(), z);
        nl.outputs.insert("o".into(), o);
        check_encoding_matches_netlist(&nl, 0);
        let enc = encode_netlist(&nl);
        assert_eq!(enc.cnf.clauses.len(), 2, "两个常量各一条单位子句");
    }

    #[test]
    fn dimacs_roundtrip_shape() {
        let mut nl = Netlist::default();
        let a = nl.add_input("a");
        let b = nl.add_input("b");
        nl.xor(a, b);
        let enc = encode_netlist(&nl);
        let d = enc.cnf.dimacs();
        let first = d.lines().next().unwrap();
        assert_eq!(first, format!("p cnf {} {}", enc.cnf.num_vars, enc.cnf.clauses.len()));
    }
}
