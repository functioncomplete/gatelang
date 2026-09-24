//! 静态资源计算（白皮书 §7.3）。门数 / 深度 / 周期 / LATCH 数。
//! 本模块从网表统计 + 预算校验错误格式化。

use crate::lower::Compiled;

/// 展示编译产物的资源摘要。
pub fn summarize(c: &Compiled) -> String {
    match c {
        Compiled::Combinational { name, budget, .. } => {
            format!("[组合] {name}: {}（声明校验：隐含在 lower）", budget.summary())
        }
        Compiled::State { name, latches, fns } => {
            let mut s = format!("[时序] {name}: latches={}", latches.len());
            for f in fns {
                s.push_str(&format!("\n    fn {}: {}", f.name, f.budget.summary()));
            }
            s
        }
    }
}

/// 资源校验总入口：返回每个组合模块的 (名字, 违规列表)。
pub fn check_resources(compiled: &[Compiled]) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    for c in compiled {
        match c {
            Compiled::Combinational { name, budget, .. } => {
                // 上界校验在 lower 编译时已执行；这里额外输出摘要
                out.push((name.clone(), budget.check_bounds(&[])));
            }
            Compiled::State { name, fns, .. } => {
                for f in fns {
                    out.push((format!("{name}.{}", f.name), f.budget.check_bounds(&[])));
                }
            }
        }
    }
    out
}