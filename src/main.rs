//! gatelang CLI —— 编译 / 资源报告 / spec 验证 / 等价检查 / 模拟。
//!
//! 用法：
//!   gatelang <file.gat>                    编译并打印资源
//!   gatelang <file.gat> --verify           运行 spec 验证
//!   gatelang <file.gat> --check-equiv A B  检查 A 与 B 组合电路等价
//!   gatelang <file.gat> --check-equiv A B --domain "expr"   在约束输入域内检查等价
//!   gatelang <file.gat> --sim NAME a b     模拟组合电路（十进制输入）
//!   gatelang <file.gat> --fct [DIR]        FCT 后端：导出门级 IR / DSU 描述 / 验证电路 / guest 模板

use std::process::ExitCode;

use gatelang::equiv::{check_equiv_domain, sig};
use gatelang::lower::Compiler;
use gatelang::parser::parse_program;
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
            (Some(a), Some(b)) => match check_equiv_domain(a, b, domain.as_deref()) {
                Ok((true, _)) => {
                    print!("\n== 等价性 ==\n{} 与 {} 语义等价 ✓", a_name, b_name);
                    if let Some(d) = &domain {
                        print!("（约束域: {d}）");
                    }
                    println!();
                }
                Ok((false, Some(reason))) => {
                    println!("\n== 等价性 ==\n{} 与 {} 不等价: {reason}", a_name, b_name);
                    return ExitCode::from(1);
                }
                Ok((false, None)) => {
                    println!("\n== 等价性 ==\n{} 与 {} 不等价", a_name, b_name);
                    return ExitCode::from(1);
                }
                Err(e) => {
                    println!("等价检查错误: {e}");
                    return ExitCode::from(1);
                }
            },
            _ => {
                println!("缺少可比对电路（--check-equiv NAME_A NAME_B）");
                return ExitCode::from(2);
            }
        }
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
    // FCT 后端（《FCT 技术组件白皮书 v1.3》§3.3）：
    // 输出门级函数 IR / DSU 描述文件 / 验证电路 / guest 模板 / manifest。
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