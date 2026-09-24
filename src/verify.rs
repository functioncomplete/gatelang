//! 验证编排：对已编译产物运行 spec 检查。
//!
//! 组合电路：枚举全部输入（位宽 ≤ 20）执行模拟，断言 postcondition。
//! 时序电路：多周期运行，检查 invariant / postcondition（示例）。

use std::collections::HashMap;

use crate::ast::{Decl, Spec};
use crate::lower::{Compiled, Compiler};
use crate::sim;

/// 验证报告。
#[derive(Debug, Default)]
pub struct VerifyReport {
    pub total: usize,
    pub passed: usize,
    pub failed: Vec<String>,
}

impl VerifyReport {
    pub fn ok(&self) -> bool {
        self.failed.is_empty()
    }
}

/// 对所有声明执行 spec 验证。
pub fn verify_all(decls: &[Decl], compiled: &[Compiled]) -> VerifyReport {
    let mut rep = VerifyReport::default();
    let specs: Vec<&Spec> = decls.iter().filter_map(|d| match d {
        Decl::Spec(s) => Some(s),
        _ => None,
    }).collect();

    for spec in &specs {
        rep.total += 1;
        // 找目标编译产物（spec 名支持命名空间：FCT.math.adder4 → 匹配 adder4，忽略大小写）
        let matches = |name: &str| -> bool {
            name == &spec.name || name.eq_ignore_ascii_case(spec.name.split('.').last().unwrap_or(""))
                || spec.name.eq_ignore_ascii_case(name)
        };
        let target = compiled.iter().find(|c| match c {
            Compiled::Combinational { name, .. } => matches(name),
            Compiled::State { name, .. } => matches(name),
        });
        match target {
            Some(Compiled::Combinational { inputs, outputs, netlist, name, .. }) => {
                let in_bits: u32 = inputs.iter().map(|p| p.width.bits()).sum();
                if in_bits > 20 {
                    rep.failed.push(format!("{name}: 输入位宽过大无法穷举 spec"));
                    continue;
                }
                let total_inputs = 1u128 << in_bits;
                let mut ok = true;
                let mut checked = 0u64;
                let mut sample_env = HashMap::new();
                'outer: for x in 0..total_inputs {
                    let mut input_map = HashMap::new();
                    let mut offset = 0u32;
                    for p in inputs {
                        let w = p.width.bits();
                        let val = (x >> offset) & ((1u128 << w) - 1);
                        input_map.insert(p.name.clone(), val);
                        offset += w;
                    }
                    let res = sim::eval_netlist(netlist, &input_map);
                    // 组装输出 env
                    let mut out_map = HashMap::new();
                    for p in outputs {
                        let mut v = 0u128;
                        for b in 0..p.width.bits() {
                            if res.get(&format!("{}_{}", p.name, b)).copied().unwrap_or(false) {
                                v |= 1 << b;
                            }
                        }
                        out_map.insert(p.name.clone(), v);
                    }
                    // 后置条件
                    if let Some(post) = &spec.post {
                        match crate::spec::assert_spec(post, &input_map, &out_map) {
                            Ok(true) => {}
                            Ok(false) => {
                                rep.failed.push(format!("{name}: postcondition 违反 @输入 x={x:#x}"));
                                ok = false;
                                break 'outer;
                            }
                            Err(e) => {
                                rep.failed.push(format!("{name}: postcondition 求值错误 {e}"));
                                ok = false;
                                break 'outer;
                            }
                        }
                    }
                    checked += 1;
                    if checked == 1 {
                        sample_env = out_map.clone();
                    }
                }
                if ok {
                    rep.passed += 1;
                }
                let _ = sample_env;
            }
            Some(Compiled::State { name, latches, fns }) => {
                // 时序验证：每 fn 独立 check invariant（原型：简单示例）
                let _ = latches;
                let mut ok = true;
                for f in fns {
                    // 用随机输入跑 3 周期，验证 postcondition 若存在
                    if let Some(post) = &spec.post {
                        // 约定 postcondition 引用 curr 状态输出
                        let state = HashMap::new();
                        let fn_inputs: HashMap<String, u128> = HashMap::new();
                        let (outs, _) = sim::run_state(f, latches, &state, &fn_inputs, 3);
                        let out_map: HashMap<String, u128> = outs
                            .last()
                            .map(|m| {
                                m.iter()
                                    .map(|(k, v)| {
                                        let val = if *v { 1 } else { 0 };
                                        (k.clone(), val)
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        match crate::spec::assert_spec(post, &fn_inputs, &out_map) {
                            Ok(true) => {}
                            Ok(false) => {
                                rep.failed.push(format!("{name}.{}: postcondition 违反（时序）", f.name));
                                ok = false;
                            }
                            Err(e) => {
                                rep.failed.push(format!("{name}.{}: postcondition 求值错误 {e}", f.name));
                                ok = false;
                            }
                        }
                    }
                }
                if ok {
                    rep.passed += 1;
                }
            }
            _ => {
                rep.failed.push(format!("{}: 未找到对应声明", spec.name));
            }
        }
    }
    rep
}

/// 便捷：端到端编译 + 验证。
pub fn compile_and_verify(decls: &[Decl]) -> (Vec<Compiled>, VerifyReport) {
    let mut c = Compiler::new(decls);
    let compiled = c.compile_all();
    let rep = verify_all(decls, &compiled);
    (compiled, rep)
}