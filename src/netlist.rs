//! NAND/LATCH 网表 IR。
//!
//! 语义保持编译的最终形态（白皮书 v2.1 §7.2）：所有高级结构
//! 展开为 NAND 门（组合）与 LATCH 触发器（时序）的网表。
//! 本模块同时提供 NAND 原语库：每个基本逻辑结构 → NAND 门的展开，
//! 展开恒以真实 NAND 计数（NOT=1, AND=2, OR=3, XOR=4, 半加器=5, 全加器=15）。

use std::collections::HashMap;

/// 信号标识：每个网表节点一个 id。
pub type Sig = usize;

/// 门
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Gate {
    /// out = NAND(l, r)
    Nand { out: Sig, l: Sig, r: Sig },
    /// out = LATCH(next, clk)：clk 上升沿把 next 存入状态（时序）
    Latch { out: Sig, next: Sig, name: String },
    /// out = CONST(v)：常量 0/1
    Const { out: Sig, v: u8 },
    /// out = INPUT，输入引线
    Input { out: Sig, name: String },
}

/// 一个已展开的组合/时序单元网表。
#[derive(Debug, Clone, Default)]
pub struct Netlist {
    pub gates: Vec<Gate>,
    /// 输入名 → 信号
    pub inputs: HashMap<String, Sig>,
    /// 输出名 → 信号
    pub outputs: HashMap<String, Sig>,
    /// 信号数量（含隐性中间信号）
    pub max_sig: usize,
    /// 追踪每个信号的最深组合路径（用于 depth 计算）
    pub depths: HashMap<Sig, u32>,
    /// 结构内联展开次数（用于封顶纯透传链的 2^n 爆炸：它不增加门/信号）
    pub expansions: usize,
}

impl Netlist {
    pub fn new_sig(&mut self) -> Sig {
        let s = self.max_sig;
        self.max_sig += 1;
        s
    }

    /// 添加输入。
    pub fn add_input(&mut self, name: &str) -> Sig {
        let s = self.new_sig();
        self.gates.push(Gate::Input { out: s, name: name.to_string() });
        self.inputs.insert(name.to_string(), s);
        self.depths.insert(s, 0);
        s
    }

    /// 添加常量（0/1）。
    pub fn add_const(&mut self, v: u8) -> Sig {
        let s = self.new_sig();
        self.gates.push(Gate::Const { out: s, v });
        self.depths.insert(s, 0);
        s
    }

    /// NAND：out = NAND(l, r)，depth = max(dl, dr)+1。
    pub fn nand(&mut self, l: Sig, r: Sig) -> Sig {
        let out = self.new_sig();
        let d = self.depths.get(&l).copied().unwrap_or(0).max(self.depths.get(&r).copied().unwrap_or(0)) + 1;
        self.gates.push(Gate::Nand { out, l, r });
        self.depths.insert(out, d);
        out
    }

    /* ---------- 派生门（全部用 NAND 展开） ---------- */

    /// NOT(x) = NAND(x, x)，1 门。
    pub fn not(&mut self, x: Sig) -> Sig {
        self.nand(x, x)
    }

    /// AND(a,b) = NOT(NAND(a,b))，2 门。
    pub fn and(&mut self, a: Sig, b: Sig) -> Sig {
        let n = self.nand(a, b);
        self.not(n)
    }

    /// OR(a,b) = NAND(NAND(a,a), NAND(b,b))：NAND(a,a)=1 + NAND(b,b)=1 + NAND=1 = 3 门。
    pub fn or(&mut self, a: Sig, b: Sig) -> Sig {
        let na = self.nand(a, a);
        let nb = self.nand(b, b);
        self.nand(na, nb)
    }

    /// XOR(a,b)，4 门（t=NAND(a,b)；NAND(a,t)+NAND(b,t)+NAND(·,·)）。
    pub fn xor(&mut self, a: Sig, b: Sig) -> Sig {
        let t = self.nand(a, b);
        let lt = self.nand(a, t);
        let rt = self.nand(b, t);
        self.nand(lt, rt)
    }

    /// 位向量 NOT（宽度 n）。
    pub fn not_vec(&mut self, v: &[Sig]) -> Vec<Sig> {
        v.iter().map(|&x| self.not(x)).collect()
    }

    /// 位向量 AND（同宽）。
    pub fn and_vec(&mut self, a: &[Sig], b: &[Sig]) -> Vec<Sig> {
        a.iter().zip(b).map(|(&x, &y)| self.and(x, y)).collect()
    }

    /// 位向量 OR。
    pub fn or_vec(&mut self, a: &[Sig], b: &[Sig]) -> Vec<Sig> {
        a.iter().zip(b).map(|(&x, &y)| self.or(x, y)).collect()
    }

    /// 位向量 XOR。
    pub fn xor_vec(&mut self, a: &[Sig], b: &[Sig]) -> Vec<Sig> {
        a.iter().zip(b).map(|(&x, &y)| self.xor(x, y)).collect()
    }

    /// 全加器：输入 (a, b, cin)，输出 (sum, cout)。
    /// sum = a^b^cin；cout = (a&b) | (cin & (a^b))。
    /// 门数：2×XOR(4) + 2×AND(2) + 1×OR(3) = 15（未合并共享）。
    pub fn full_adder(&mut self, a: Sig, b: Sig, cin: Sig) -> (Sig, Sig) {
        let ab = self.xor(a, b);
        let sum = self.xor(ab, cin);
        let ab_and = self.and(a, b);
        let cin_ab = self.and(cin, ab);
        let cout = self.or(ab_and, cin_ab);
        (sum, cout)
    }

    /// 半加器：sum = XOR(a,b)，carry = AND(a,b)。
    /// 共享 t=NAND(a,b)：t(1) + NOT t → carry(1) + XOR 借 t 的 3 门 = 5 门。
    /// 与白皮书 v2.1「整个半加器由 5 个 NAND 门构成」一致。
    pub fn half_adder(&mut self, a: Sig, b: Sig) -> (Sig, Sig) {
        let t = self.nand(a, b);         // 1 门
        let carry = self.not(t);          // 2 门
        let lt = self.nand(a, t);         // 3 门
        let rt = self.nand(b, t);         // 4 门
        let sum = self.nand(lt, rt);      // 5 门
        (sum, carry)
    }

    /// N 位加法器（a+b，进位输出丢弃时忽略 cout）。
    pub fn adder(&mut self, a: &[Sig], b: &[Sig]) -> (Vec<Sig>, Sig) {
        let cin = self.add_const(0);
        self.adder_cin(a, b, cin)
    }

    /// N 位加法器，带进位输入。
    ///
    /// **必须用单条进位链**：把 `a + ~b + 1` 拆成两次加法会丢掉第一次的进位，
    /// 使"有无借位"判断错误（比较器会给出错误结果）。
    pub fn adder_cin(&mut self, a: &[Sig], b: &[Sig], cin: Sig) -> (Vec<Sig>, Sig) {
        let mut sums = Vec::with_capacity(a.len());
        let mut carry = cin;
        for i in 0..a.len() {
            let (s, c) = self.full_adder(a[i], b[i], carry);
            sums.push(s);
            carry = c;
        }
        (sums, carry)
    }
}

/// 统计信息。
#[derive(Debug, Clone, Default)]
pub struct Stats {
    pub nand_count: u32,
    pub latch_count: u32,
    pub max_depth: u32,
}

impl Netlist {
    pub fn stats(&self) -> Stats {
        let mut nand = 0;
        let mut latch = 0;
        for g in &self.gates {
            match g {
                Gate::Nand { .. } => nand += 1,
                Gate::Latch { .. } => latch += 1,
                _ => {}
            }
        }
        let max_depth = self.depths.values().copied().max().unwrap_or(0);
        Stats { nand_count: nand, latch_count: latch, max_depth }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nand_primitives_count() {
        let mut nl = Netlist::default();
        let a = nl.add_input("a");
        let b = nl.add_input("b");
        // 标准 NAND 实现 XOR = 4 门
        let x = nl.xor(a, b);
        let s = nl.stats();
        assert_eq!(s.nand_count, 4);
        // AND = 2, 叠加 OR = +3 → 7
        let mut nl2 = Netlist::default();
        let a2 = nl2.add_input("a");
        let b2 = nl2.add_input("b");
        nl2.and(a2, b2);
        assert_eq!(nl2.stats().nand_count, 2);
        nl2.or(a2, b2);
        assert_eq!(nl2.stats().nand_count, 5);
        assert_eq!(x, 5); // XOR 输出信号 id：a=0, b=1, t=2, lt=3, rt=4, out=5
        // 深度
        assert_eq!(nl.depths[&x], 3); // XOR 深度 3
    }

    #[test]
    fn half_adder_5gates() {
        // 白皮书 v2.1：「整个半加器由 5 个 NAND 门构成」
        let mut nl = Netlist::default();
        let a = nl.add_input("a");
        let b = nl.add_input("b");
        let (sum, carry) = nl.half_adder(a, b);
        assert_eq!(nl.stats().nand_count, 5);
        let _ = (sum, carry);
    }

    #[test]
    fn adder_4bit_count() {
        let mut nl = Netlist::default();
        let a: Vec<Sig> = (0..4).map(|i| nl.add_input(&format!("a{i}"))).collect();
        let b: Vec<Sig> = (0..4).map(|i| nl.add_input(&format!("b{i}"))).collect();
        let (sums, cout) = nl.adder(&a, &b);
        assert_eq!(sums.len(), 4);
        // FA = 2×XOR(4) + 2×AND(2) + 1×OR(3) = 15 门；4×15 = 60
        let s = nl.stats();
        assert_eq!(s.nand_count, 60);
        let _ = cout;
    }

    #[test]
    fn vector_ops_match_scalar_counts() {
        // 位向量原语（not/and/or/xor_vec）逐位展开，门数应等于标量原语 × 位宽。
        let mut nl = Netlist::default();
        let a: Vec<Sig> = (0..2).map(|i| nl.add_input(&format!("a{i}"))).collect();
        let b: Vec<Sig> = (0..2).map(|i| nl.add_input(&format!("b{i}"))).collect();
        assert_eq!(nl.not_vec(&a).len(), 2);
        assert_eq!(nl.and_vec(&a, &b).len(), 2);
        assert_eq!(nl.or_vec(&a, &b).len(), 2);
        assert_eq!(nl.xor_vec(&a, &b).len(), 2);
        // 2×(NOT1 + AND2 + OR3 + XOR4) = 2×10 = 20
        assert_eq!(nl.stats().nand_count, 20);
    }
}