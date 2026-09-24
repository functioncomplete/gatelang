//! 等价性检查（白皮书 §4.3 / §6.3）：两个电路在指定输入域上行为一致。
//!
//! 原型：对输入位宽 ≤ 20 的组合电路做穷举验证（2^N 输入，N = 总输入位宽）。
//! 支持 `domain` 约束表达式（同 spec 求值器语法）裁剪输入域，例如
//! `a<8 && b<8` 只比较 0..8 × 0..8 的输入。

use crate::ast::Width;
use crate::lower::Compiled;

/// 穷举验证两个组合电路在约束域内等价。返回是否等价，以及反例（若有）。
/// `domain` 为 None 时全输入域；Some(expr) 时用 spec 求值器过滤。
pub fn check_equiv_domain(
    a: &Compiled,
    b: &Compiled,
    domain: Option<&str>,
) -> Result<(bool, Option<String>), String> {
    let (an, ai, ao) = match a {
        Compiled::Combinational { name, inputs, outputs, netlist: _, .. } => {
            let bits: u32 = inputs.iter().map(|p| p.width.bits()).sum();
            (name.clone(), bits, outputs.len())
        }
        _ => return Err("等价检查仅支持组合电路".into()),
    };
    let (bn, bi, bo) = match b {
        Compiled::Combinational { name, inputs, outputs, .. } => {
            let bits: u32 = inputs.iter().map(|p| p.width.bits()).sum();
            (name.clone(), bits, outputs.len())
        }
        _ => return Err("等价检查仅支持组合电路".into()),
    };
    if ai != bi || ao != bo {
        return Ok((false, Some(format!("接口不匹配: {an}(w{ai}o{ao}) vs {bn}(w{bi}o{bo})"))));
    }
    if ai > 20 {
        return Ok((false, Some(format!("输入位宽 {ai} 过大，穷举不可行（原型限制 ≤20）"))));
    }
    // 输出名对齐：按索引取
    let (a_nl, a_in, a_out) = match a {
        Compiled::Combinational { netlist, inputs, outputs, .. } => (netlist, inputs, outputs),
        _ => unreachable!(),
    };
    let (b_nl, b_in, b_out) = match b {
        Compiled::Combinational { netlist, inputs, outputs, .. } => (netlist, inputs, outputs),
        _ => unreachable!(),
    };
    let n = 1u128 << ai;
    // 约束域预解析（解析失败即报错，不静默忽略）
    let domain_expr = match domain {
        Some(d) if !d.trim().is_empty() => Some(crate::spec::parse_spec(d)?),
        _ => None,
    };
    for x in 0..n {
        let mut a_in_map = crate::sim::Inputs::new();
        let mut b_in_map = crate::sim::Inputs::new();
        let mut offset = 0u32;
        for (i, p) in a_in.iter().enumerate() {
            let w = p.width.bits();
            let mask = ((1u128 << w) - 1) as u128;
            let val = (x >> offset) & mask;
            a_in_map.insert(p.name.clone(), val);
            let bw = p.width.bits();
            let bmask = ((1u128 << bw) - 1) as u128;
            let bval = (x >> offset) & bmask;
            b_in_map.insert(b_in[i].name.clone(), bval);
            offset += w;
        }
        // 约束域过滤：不满足 domain 的输入不比较
        if let Some(de) = &domain_expr {
            let v = crate::spec::eval_spec(de, &a_in_map)
                .map_err(|e| format!("domain 求值错误: {e}"))?;
            if v == 0 {
                continue;
            }
        }
        let ra = crate::sim::eval_netlist(a_nl, &a_in_map);
        let rb = crate::sim::eval_netlist(b_nl, &b_in_map);
        // 对比输出
        for (i, p) in a_out.iter().enumerate() {
            let mut va = 0u128;
            let mut vb = 0u128;
            for bit in 0..p.width.bits() {
                let ka = format!("{}_{}", p.name, bit);
                let kb = format!("{}_{}", b_out[i].name, bit);
                if ra.get(&ka).copied().unwrap_or(false) {
                    va |= 1 << bit;
                }
                if rb.get(&kb).copied().unwrap_or(false) {
                    vb |= 1 << bit;
                }
            }
            if va != vb {
                return Ok((false, Some(format!("反例: 输入 x={x:#x}, 输出 {}=0x{va:x} vs {}=0x{vb:x}", p.name, b_out[i].name))));
            }
        }
    }
    Ok((true, None))
}

/// 兼容入口：无约束域。
pub fn check_equiv(a: &Compiled, b: &Compiled) -> Result<(bool, Option<String>), String> {
    check_equiv_domain(a, b, None)
}

/// 便捷：接口描述。
pub fn sig(c: &Compiled) -> String {
    match c {
        Compiled::Combinational { name, inputs, outputs, .. } => {
            let i: Vec<String> = inputs.iter().map(|p| p_desc(&p.name, &p.width)).collect();
            let o: Vec<String> = outputs.iter().map(|p| p_desc(&p.name, &p.width)).collect();
            format!("{name}({})->({})", i.join(","), o.join(","))
        }
        Compiled::State { name, .. } => format!("{name} <state>"),
    }
}

fn p_desc(name: &str, w: &Width) -> String {
    match w {
        Width::Bit => name.to_string(),
        Width::Bits(n) => format!("{name}:Bits<{n}>"),
    }
}