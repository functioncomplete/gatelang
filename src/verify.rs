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
        // 找目标编译产物（spec 名支持命名空间：FCT.math.adder4 → 匹配 adder4）。
        // 先精确匹配（区分大小写），再回退忽略大小写，避免 Foo/foo 抢匹配导致验错对象。
        let short = spec.name.rsplit('.').next().unwrap_or("");
        let target = compiled
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
            });
        match target {
            Some(Compiled::Combinational { inputs, outputs, netlist, name, .. }) => {
                // 没有 postcondition 就无从验证，绝不能计为通过。
                if spec.post.is_none() {
                    rep.failed.push(format!("{name}: 缺少 postcondition，无法验证"));
                    continue;
                }
                let in_bits: u32 = inputs.iter().map(|p| p.width.bits()).sum();
                if in_bits > 20 {
                    rep.failed.push(format!("{name}: 输入位宽过大无法穷举 spec"));
                    continue;
                }
                // 工作量预算：2^in_bits × (门数 + spec 长度) 过大时不再穷举
                // （spec 长度计入，因其每个输入都要重新解析求值）
                let spec_bytes = spec.pre.as_deref().map_or(0, |s| s.len())
                    + spec.post.as_deref().map_or(0, |s| s.len())
                    + spec.invariant.as_deref().map_or(0, |s| s.len());
                let work = (1u128 << in_bits).saturating_mul(
                    (netlist.gates.len()
                        + spec_bytes
                        + outputs.iter().map(|p| p.width.bits()).sum::<u32>() as usize)
                        as u128,
                );
                if work > 50_000_000 {
                    rep.failed.push(format!(
                        "{name}: 验证规模过大（2^{in_bits} 输入 × {} 门 + {spec_bytes} 规格字节）",
                        netlist.gates.len()
                    ));
                    continue;
                }
                let total_inputs = 1u128 << in_bits;
                let mut ok = true;
                let mut checked = 0u64;
                let mut sample_env = HashMap::new();
                let empty_out: HashMap<String, u128> = HashMap::new();
                'outer: for x in 0..total_inputs {
                    let mut input_map = HashMap::new();
                    let mut offset = 0u32;
                    for p in inputs {
                        let w = p.width.bits();
                        let val = (x >> offset) & ((1u128 << w) - 1);
                        input_map.insert(p.name.clone(), val);
                        offset += w;
                    }
                    // 前置条件过滤（通过后即计入 checked，避免首个输入即失败时误报“无有效输入”）
                    if let Some(pre) = &spec.pre {
                        match crate::spec::assert_spec(pre, &input_map, &empty_out) {
                            Ok(true) => {}
                            Ok(false) => continue,
                            Err(e) => {
                                rep.failed.push(format!("{name}: precondition 求值错误 {e}"));
                                ok = false;
                                break 'outer;
                            }
                        }
                    }
                    checked += 1;
                    let res = sim::eval_netlist(netlist, &input_map);
                    // 组装输出 env
                    let mut out_map = HashMap::new();
                    for p in outputs {
                        let mut v = 0u128;
                        for b in 0..p.width.bits().min(128) {
                            if res.get(&format!("{}_{}", p.name, b)).copied().unwrap_or(false) {
                                v |= 1 << b;
                            }
                        }
                        out_map.insert(p.name.clone(), v);
                    }
                    // 已验证割点：必须在 pre 下恒成立（与 prove.rs 的两阶段语义一致）
                    if let Some(cut) = &spec.cut {
                        match crate::spec::assert_spec(cut, &input_map, &out_map) {
                            Ok(true) => {}
                            Ok(false) => {
                                rep.failed.push(format!("{name}: cut 违反 @输入 x={x:#x}"));
                                ok = false;
                                break 'outer;
                            }
                            Err(e) => {
                                rep.failed.push(format!("{name}: cut 求值错误 {e}"));
                                ok = false;
                                break 'outer;
                            }
                        }
                    }
                    // 后置条件
                    if let Some(post) = &spec.post {
                        match crate::spec::assert_spec(post, &input_map, &out_map) {                            Ok(true) => {}
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
                    // 不变式（若有）同样须在所有输入上成立（此前从不验证 → 假通过）
                    if let Some(inv) = &spec.invariant {
                        match crate::spec::assert_spec(inv, &input_map, &out_map) {
                            Ok(true) => {}
                            Ok(false) => {
                                rep.failed.push(format!("{name}: invariant 违反 @输入 x={x:#x}"));
                                ok = false;
                                break 'outer;
                            }
                            Err(e) => {
                                rep.failed.push(format!("{name}: invariant 求值错误 {e}"));
                                ok = false;
                                break 'outer;
                            }
                        }
                    }
                    if checked == 1 {
                        sample_env = out_map.clone();
                    }
                }
                if checked == 0 {
                    rep.failed.push(format!("{name}: precondition 恒不成立，无可验证输入"));
                    continue;
                }
                if ok {
                    rep.passed += 1;
                }
                let _ = sample_env;
            }
            Some(Compiled::State { name, latches, fns }) => {
                // 时序 spec 同样必须带 postcondition，否则不得计为通过。
                if spec.post.is_none() {
                    rep.failed.push(format!("{name}: 缺少 postcondition，无法验证"));
                    continue;
                }
                if fns.is_empty() {
                    rep.failed.push(format!("{name}: 无 fn 可验证"));
                    continue;
                }
                let mut ok = true;
                for f in fns {
                    let post = spec.post.as_ref().unwrap();
                    // 穷举 fn 参数：此前参数恒为 0，只在 x=0 成立的 postcondition 会被误判为恒真。
                    let in_bits: u32 = f.params.iter().map(|p| p.width.bits()).sum();
                    if in_bits > 20 {
                        rep.failed.push(format!("{name}.{}: 参数位宽过大无法穷举", f.name));
                        ok = false;
                        continue;
                    }
                    let total = 1u128 << in_bits;
                    let out_bits: u32 = f.returns.iter().map(|p| p.width.bits()).sum();
                    // 工作量预算（2^in_bits × 3 周期 × (门数 + 输出位重建)）
                    let work = total
                        .saturating_mul(3)
                        .saturating_mul((f.netlist.gates.len() + out_bits as usize) as u128);
                    if work > 50_000_000 {
                        rep.failed.push(format!("{name}.{}: 验证规模过大", f.name));
                        ok = false;
                        continue;
                    }
                    'param: for x in 0..total {
                        // 以声明的 latch 初值播种状态
                        let mut state: HashMap<String, u128> = HashMap::new();
                        for l in latches {
                            state.insert(l.name.clone(), l.init);
                        }
                        let mut fn_inputs: HashMap<String, u128> = HashMap::new();
                        let mut off = 0u32;
                        for p in &f.params {
                            let w = p.width.bits();
                            fn_inputs.insert(p.name.clone(), (x >> off) & ((1u128 << w) - 1));
                            off += w;
                        }
                        let (outs, final_state) = sim::run_state(f, latches, &state, &fn_inputs, 3);
                        // 先绑定 latch 名（postcondition 常引用 latch 状态）
                        let mut out_map: HashMap<String, u128> = HashMap::new();
                        for (k, sv) in &final_state {
                            out_map.insert(k.clone(), *sv);
                        }
                        // 输出名最后写入，优先于同名 latch（否则 latch 会覆盖输出 → 假通过）
                        if let Some(last) = outs.last() {
                            for p in &f.returns {
                                let mut v = 0u128;
                                for b in 0..p.width.bits().min(128) {
                                    if last.get(&format!("{}_{}", p.name, b)).copied().unwrap_or(false) {
                                        v |= 1u128 << b;
                                    }
                                }
                                out_map.insert(p.name.clone(), v);
                            }
                        }
                        match crate::spec::assert_spec(post, &fn_inputs, &out_map) {
                            Ok(true) => {}
                            Ok(false) => {
                                rep.failed.push(format!("{name}.{}: postcondition 违反 @输入 x={x:#x}", f.name));
                                ok = false;
                                break 'param;
                            }
                            Err(e) => {
                                rep.failed.push(format!("{name}.{}: postcondition 求值错误 {e}", f.name));
                                ok = false;
                                break 'param;
                            }
                        }
                        // 不变式（若有）
                        if let Some(inv) = &spec.invariant {
                            match crate::spec::assert_spec(inv, &fn_inputs, &out_map) {
                                Ok(true) => {}
                                Ok(false) => {
                                    rep.failed.push(format!("{name}.{}: invariant 违反 @输入 x={x:#x}", f.name));
                                    ok = false;
                                    break 'param;
                                }
                                Err(e) => {
                                    rep.failed.push(format!("{name}.{}: invariant 求值错误 {e}", f.name));
                                    ok = false;
                                    break 'param;
                                }
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
    let compiled = c.compile_all().expect("编译失败");
    let rep = verify_all(decls, &compiled);
    (compiled, rep)
}