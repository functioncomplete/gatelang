//! 位级模拟器。对 NAND/LATCH 网表求值（白皮书 §10.2 模拟器）。
//! 组合电路：一次性拓扑求值（信号按创建序引用前置，单趟即可）。
//! 时序电路：latch 状态作为输入，一周期 = 一次求值 + latch 更新。

use std::collections::HashMap;

use crate::ast::{LatchDecl, Param};
use crate::lower::CompiledFn;
use crate::netlist::{Gate, Netlist};

/// 模拟结果：输出信号 → 布尔值。
pub type SimResult = HashMap<String, bool>;

/// 输入赋值：名字 → u128（按 LSB-first 位模式）。
pub type Inputs = HashMap<String, u128>;

/// 组合网表求值。输入通过 env 提供（名字 → 值）。返回输出名 → bool。
pub fn eval_netlist(nl: &Netlist, inputs: &HashMap<String, u128>) -> HashMap<String, bool> {
    let vals = eval_all(nl, inputs);
    // 输出
    let mut out = HashMap::new();
    for (name, sig) in &nl.outputs {
        if let Some(v) = vals.get(sig) {
            out.insert(name.clone(), *v);
        }
    }
    out
}

/// 求值所有信号（含内部信号），供 run_state 读 next-latch 值。
fn eval_all(nl: &Netlist, inputs: &HashMap<String, u128>) -> HashMap<usize, bool> {
    // 已解析的信号值
    let mut vals: HashMap<usize, bool> = HashMap::new();
    // 输入：按位展开
    for (name, v) in inputs {
        for i in 0..64u32 {
            let key = format!("{name}_{i}");
            if nl.inputs.contains_key(&key) {
                vals.insert(nl.inputs[&key], ((v >> i) & 1) == 1);
            }
        }
    }
    // 常量与门：按序求值（信号创建序保证输入已就绪）
    for g in &nl.gates {
        match g {
            Gate::Nand { out, l, r } => {
                let lv = vals.get(l).copied().unwrap_or(false);
                let rv = vals.get(r).copied().unwrap_or(false);
                vals.insert(*out, !(lv && rv));
            }
            Gate::Const { out, v } => {
                vals.insert(*out, *v == 1);
            }
            Gate::Latch { out, next, .. } => {
                // 原型：时序模拟由 state 处理。必要时直接接通 next
                let nv = vals.get(next).copied().unwrap_or(false);
                vals.insert(*out, nv);
            }
            Gate::Input { .. } => {}
        }
    }
    vals
}

/// 时序模块模拟：多周期执行。
/// 输入: latches 初始值 + fn 参数。返回每周期 output 序列与最终 latch 状态。
pub fn run_state(
    f: &CompiledFn,
    _latches: &[LatchDecl],
    latch_inputs: &HashMap<String, u128>,
    fn_inputs: &Inputs,
    cycles: usize,
) -> (Vec<HashMap<String, bool>>, HashMap<String, u128>) {
    let mut state = latch_inputs.clone();
    let mut outputs = Vec::new();
    for _ in 0..cycles {
        // 装配输入：latch 状态 + fn 参数
        let mut full: HashMap<String, u128> = HashMap::new();
        for (k, v) in &state {
            for i in 0..64u32 {
                if f.netlist.inputs.contains_key(&format!("latch_{k}_{i}")) {
                    full.insert(format!("latch_{k}_{i}"), (*v >> i) & 1);
                }
            }
        }
        for (k, v) in fn_inputs {
            for i in 0..64u32 {
                if f.netlist.inputs.contains_key(&format!("{k}_{i}")) {
                    full.insert(format!("{k}_{i}"), (*v >> i) & 1);
                }
            }
        }
        let vals = eval_all(&f.netlist, &full);
        // 收集输出（从内部信号值重建输出名 → bool）
        let mut out: HashMap<String, bool> = HashMap::new();
        for (name, sig) in &f.netlist.outputs {
            if let Some(v) = vals.get(sig) {
                out.insert(name.clone(), *v);
            }
        }
        outputs.push(out);
        // latch 更新：从 next_latch_sigs 收集新值（直接查内部信号）
        for (lname, sigs) in &f.next_latch_sigs {
            let mut v = 0u128;
            for (i, sig) in sigs.iter().enumerate() {
                if vals.get(sig).copied().unwrap_or(false) {
                    v |= 1 << i;
                }
            }
            state.insert(lname.clone(), v);
        }
    }
    (outputs, state)
}

/// 便捷：组合电路模拟（按参数顺序给值）。
pub fn sim_combinational(
    params: &[Param],
    outputs: &[Param],
    nl: &Netlist,
    input_vals: &[u128],
) -> Vec<u128> {
    let mut inputs = HashMap::new();
    for (i, p) in params.iter().enumerate() {
        inputs.insert(p.name.clone(), input_vals[i]);
    }
    let res = eval_netlist(nl, &inputs);
    // 输出收集：每个输出参数按其位宽重组
    let mut out = Vec::new();
    for (i, p) in outputs.iter().enumerate() {
        let mut v = 0u128;
        for b in 0..p.width.bits() {
            let key = format!("{}_{}", p.name, b);
            if res.get(&key).copied().unwrap_or(false) {
                v |= 1 << b;
            }
        }
        if p.width == crate::ast::Width::Bit && outputs.len() == 1 && params.is_empty() {
            // 特例：无参数时序？忽略
        }
        let _ = i;
        out.push(v);
    }
    out
}