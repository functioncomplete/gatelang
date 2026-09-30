//! gatelang CLI —— 编译 / 资源报告 / spec 验证 / 等价检查 / 模拟。
//!
//! 用法：
//!   gatelang <file.gat>                    编译并打印资源
//!   gatelang <file.gat> --verify           运行 spec 验证
//!   gatelang <file.gat> --check-equiv A B  检查 A 与 B 组合电路等价
//!   gatelang <file.gat> --check-equiv A B --domain "expr"   在约束输入域内检查等价
//!   gatelang <file.gat> --sim NAME a b     模拟组合电路（十进制输入）
//!   gatelang <file.gat> --fct [DIR]        FCT 后端：导出逻辑原语 IR / DSU 描述 / 验证函数 / guest 模板
//!   gatelang <file.gat> --prove            形式化证明（SAT 后端，全输入而非穷举）
//!   gatelang <file.gat> --prove-equiv A B  形式化等价证明（miter + SAT，输入位宽无上限）

use std::process::ExitCode;

use gatelang::equiv::{check_equiv_domain, sig};
use gatelang::lower::Compiler;
use gatelang::parser::parse_program;
use gatelang::prove::{prove_all, prove_equiv_sat, Verdict};
use gatelang::resource::summarize;
use gatelang::verify::verify_all;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("用法: gatelang <file.gat> [--verify] [--check-equiv A B] [--sim NAME vals...]");
        return ExitCode::from(2);
    }
    let src = match std::fs::read_to_string(&args[1]) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("读取失败: {e}");
            return ExitCode::from(2);
        }
    };

    let decls = match parse_program(&src) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("解析错误: {e}");
            return ExitCode::from(1);
        }
    };

    let mut compiler = Compiler::new(&decls);
    let compiled = match compiler.compile_all() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("编译错误: {e}");
            return ExitCode::from(1);
        }
    };

    // 默认：资源报告
    println!("== 编译产物 ==");
    for c in &compiled {
        println!("{}", summarize(c));
    }
    // 等价值对比
    if let Some(pos) = args.iter().position(|a| a == "--check-equiv") {
        let a_name = args.get(pos + 1).cloned().unwrap_or_default();
        let b_name = args.get(pos + 2).cloned().unwrap_or_default();
        if a_name.is_empty() || b_name.is_empty() {
            println!("--check-equiv 需要两个电路名：--check-equiv NAME_A NAME_B");
            return ExitCode::from(2);
        }
        // 约束域：--domain "expr"（全局选项，可出现在任意位置）
        let domain = args
            .iter()
            .position(|a| a == "--domain")
            .and_then(|p| args.get(p + 1).cloned());
        // 精确匹配电路名（name 后紧跟 '('），避免前缀歧义 / 空名匹配首例
        let a = compiled.iter().find(|c| sig(c).starts_with(&format!("{a_name}(")));
        let b = compiled.iter().find(|c| sig(c).starts_with(&format!("{b_name}(")));
        match (a, b) {
            (Some(a), Some(b)) => {
                // 无约束域：优先 SAT/miter（输入位宽无上限）；SAT 不可用时回退穷举。
                // 带 --domain 时仍走穷举（域过滤器语义与既有实现保持逐位一致）。
                let (res, backend) = if domain.is_none() {
                    match prove_equiv_sat(a, b) {
                        Ok(r) => (Ok(r), "SAT/miter"),
                        Err(e) => (
                            check_equiv_domain(a, b, None)
                                .map_err(|e2| format!("{e}；回退穷举亦失败: {e2}")),
                            "穷举(回退)",
                        ),
                    }
                } else {
                    (check_equiv_domain(a, b, domain.as_deref()), "穷举")
                };
                match res {
                    Ok((true, _)) => {
                        print!("\n== 等价性 ==\n{} 与 {} 语义等价 ✓（{backend}）", a_name, b_name);
                        if let Some(d) = &domain {
                            print!("（约束域: {d}）");
                        }
                        println!();
                    }
                    Ok((false, Some(reason))) => {
                        println!("\n== 等价性 ==\n{} 与 {} 不等价: {reason}（{backend}）", a_name, b_name);
                        return ExitCode::from(1);
                    }
                    Ok((false, None)) => {
                        println!("\n== 等价性 ==\n{} 与 {} 不等价（{backend}）", a_name, b_name);
                        return ExitCode::from(1);
                    }
                    Err(e) => {
                        println!("等价检查错误: {e}");
                        return ExitCode::from(1);
                    }
                }
            }
            _ => {
                println!("缺少可比对电路（--check-equiv NAME_A NAME_B）");
                return ExitCode::from(2);
            }
        }
    }
    // 形式化等价证明（独立入口，语义同 --check-equiv 的 SAT 路径）
    if let Some(pos) = args.iter().position(|a| a == "--prove-equiv") {
        let a_name = args.get(pos + 1).cloned().unwrap_or_default();
        let b_name = args.get(pos + 2).cloned().unwrap_or_default();
        if a_name.is_empty() || b_name.is_empty() {
            println!("--prove-equiv 需要两个电路名：--prove-equiv NAME_A NAME_B");
            return ExitCode::from(2);
        }
        let a = compiled.iter().find(|c| sig(c).starts_with(&format!("{a_name}(")));
        let b = compiled.iter().find(|c| sig(c).starts_with(&format!("{b_name}(")));
        match (a, b) {
            (Some(a), Some(b)) => match prove_equiv_sat(a, b) {
                Ok((true, _)) => println!("\n== 形式化等价 ==\n{} ≡ {} ✓ 对全部输入成立（UNSAT）", a_name, b_name),
                Ok((false, Some(r))) => {
                    println!("\n== 形式化等价 ==\n{} ≢ {}：{r}", a_name, b_name);
                    return ExitCode::from(1);
                }
                Ok((false, None)) => {
                    println!("\n== 形式化等价 ==\n{} ≢ {}", a_name, b_name);
                    return ExitCode::from(1);
                }
                Err(e) => {
                    println!("形式化等价证明错误: {e}");
                    return ExitCode::from(1);
                }
            },
            _ => {
                println!("--prove-equiv 需要两个已定义电路");
                return ExitCode::from(2);
            }
        }
    }
    // 形式化证明（SAT 后端）
    if args.iter().any(|a| a == "--prove") {
        let reports = prove_all(&decls, &compiled);
        println!("\n== 形式化证明（SAT 后端）==");
        if reports.is_empty() {
            println!("  文件中没有 spec 声明");
            return ExitCode::from(2);
        }
        let mut proven = 0usize;
        let mut bad = false;
        for r in &reports {
            match r {
                Ok(rep) => {
                    if rep.pre_unsatisfiable {
                        println!("  ✗ {}: 前置条件恒不成立（输入域为空）", rep.circuit);
                        bad = true;
                        continue;
                    }
                    for ob in &rep.obligations {
                        match &ob.verdict {
                            Verdict::Proven => {
                                proven += 1;
                                println!(
                                    "  ✓ [已证明] {}: {}（CNF {} 变量/{} 子句，冲突 {}）",
                                    rep.circuit, ob.statement, ob.cnf_vars, ob.cnf_clauses, ob.stats.conflicts
                                );
                            }
                            Verdict::Refuted { input } => {
                                bad = true;
                                println!("  ✗ [被驳倒] {}: {} —— 反例 {input}", rep.circuit, ob.statement);
                            }
                            Verdict::Unknown { reason } => {
                                bad = true;
                                println!("  ? [未定] {}: {} —— {reason}", rep.circuit, ob.statement);
                            }
                        }
                    }
                }
                Err(e) => {
                    bad = true;
                    println!("  ✗ {e}");
                }
            }
        }
        if bad {
            return ExitCode::from(1);
        }
        println!("  全部 {proven} 条义务已形式化证明（对全部输入成立，非穷举抽样）");
    }
    // spec 验证
    if args.iter().any(|a| a == "--verify") {
        let rep = verify_all(&decls, &compiled);
        println!("\n== spec 验证 ==");
        println!("通过 {}/{}", rep.passed, rep.total);
        if !rep.failed.is_empty() {
            for f in &rep.failed {
                println!("  ✗ {f}");
            }
            return ExitCode::from(1);
        }
        println!("  ✓ 全部满足规范");
    }
    // FCT 后端（《FCT 技术组件白皮书 v1.4》§3.3）：
    // 输出逻辑原语函数 IR / DSU 描述文件 / 验证函数 / guest 模板 / manifest。
    if let Some(pos) = args.iter().position(|a| a == "--fct") {
        let out = args
            .get(pos + 1)
            .filter(|s| !s.starts_with("--"))
            .cloned()
            .unwrap_or_else(|| "fct-out".to_string());
        if let Err(e) = gatelang::fct::emit(&compiled, &out, &args[1]) {
            eprintln!("FCT 后端失败: {e}");
            return ExitCode::from(1);
        }
    }
    // 模拟
    if let Some(pos) = args.iter().position(|a| a == "--sim") {
        let name = args.get(pos + 1).cloned().unwrap_or_default();
        if name.is_empty() {
            println!("--sim 需要电路名：--sim NAME [vals...]");
            return ExitCode::from(2);
        }
        let c = compiled.iter().find(|c| sig(c).starts_with(&format!("{name}("))).cloned();
        match c {
            Some(gatelang::lower::Compiled::Combinational { inputs, outputs, netlist, .. }) => {
                // 模拟器用 u128 表示端口值：位宽 > 128 时必须拒绝，
                // 否则高位会被静默当作 0（假结果）。
                if let Some(p) = inputs.iter().find(|p| p.width.bits() > 128) {
                    println!(
                        "模拟不支持位宽 > 128 的端口（{}: Bits<{}>）；请用 --prove 做形式化验证",
                        p.name,
                        p.width.bits()
                    );
                    return ExitCode::from(2);
                }
                let rest: &[String] = if pos + 2 <= args.len() { &args[pos + 2..] } else { &[] };
                let vals: Vec<u128> = rest
                    .iter()
                    .filter_map(|s| u128::from_str_radix(s, 10).ok())
                    .collect();
                if vals.len() < inputs.len() {
                    println!("模拟需 {} 个输入，实际 {} 个", inputs.len(), vals.len());
                    return ExitCode::from(2);
                }
                let res = gatelang::sim::sim_combinational(&inputs, &outputs, &netlist, &vals);
                println!("\n== 模拟 {name} ==");
                for (i, o) in outputs.iter().enumerate() {
                    println!("  {} = {}", o.name, res[i]);
                }
            }
            Some(_) => println!("{name} 是时序模块，--sim 仅支持组合电路"),
            None => println!("未找到 {name}"),
        }
    }
    ExitCode::SUCCESS
}