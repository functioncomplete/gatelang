//! 多项式/不等式层（`poly.rs`）的集成测试 —— Uniswap V2 不变量审计。
//!
//! 与词级层同样的纪律：**只做保守、单向判定**；证明不了就回落 SAT，
//! 绝不假证明。本文件锁定三件事：
//!   1. Uniswap V2 的 k 不下降定理确实被证（且走多项式层，CNF 规模 0）；
//!   2. 放宽约束后**判不了**（负面对照，防假证明）；
//!   3. 前置条件**可满足**（非空域，防"空域上的空洞证明"）；
//!   4. 多项式层不改变既有语料的任何结论（对照纯 SAT）。

use gatelang::ast::{Decl, Param, Span, Width};
use gatelang::lower::Compiler;
use gatelang::parser::parse_program;
use gatelang::poly;
use gatelang::prove::{find_target, prove_all, prove_spec};
use gatelang::spec::eval_spec;
use std::collections::HashMap;

fn compile(src: &str) -> (Vec<Decl>, Vec<gatelang::lower::Compiled>) {
    let decls = parse_program(src).expect("parse 失败");
    let mut c = Compiler::new(&decls);
    let compiled = c.compile_all().expect("编译失败");
    (decls, compiled)
}

fn p32(n: &str) -> Param {
    Param { name: n.to_string(), width: Width::Bits(32), span: Span::new(0, 0) }
}

fn uniswap_ports() -> Vec<Param> {
    vec![p32("R0"), p32("R1"), p32("dIn"), p32("dOut")]
}

/// 验收：三条 Uniswap V2 定理全部被**多项式层**证明（CNF 规模 0 = 未 bit-blast）。
#[test]
fn uniswap_v2_theorems_are_proven_by_polynomial_layer() {
    let src = include_str!("../examples/uniswap_v2_swap.gat");
    let (decls, compiled) = compile(src);
    let rs = prove_all(&decls, &compiled);
    assert_eq!(rs.len(), 3, "应有三条 spec");
    for (r, name) in rs.iter().zip([
        "UniswapV2KNonDecreasing",
        "UniswapV2ReverseDirection",
        "UniswapV2Conservation",
    ]) {
        let rep = r.as_ref().unwrap_or_else(|e| panic!("{name} 报错: {e}"));
        assert!(rep.all_proven(), "{name} 应被证明: {:?}", rep.obligations);
        assert_eq!(
            rep.obligations[0].cnf_vars, 0,
            "{name} 应走多项式层（未构造 CNF）"
        );
    }
}

/// 负面对照：把「安全下单量的上界」**放宽一倍**后必须判不了。
#[test]
fn over_permissive_bound_is_not_proven() {
    let pre = "dOut * (R0 + dIn) <= 2*dIn*R1 && dOut <= R1";
    let post = "(R0 + dIn) * (R1 - dOut) >= R0 * R1";
    assert_eq!(
        poly::prove_text(&uniswap_ports(), Some(pre), post),
        None,
        "约束过宽时不得假证明"
    );
}

/// 前置条件**可满足**的非空域见证：给出一组真实取值同时满足 pre 与 post。
///
/// 没有这一步，"对全部满足 pre 的输入成立"可能因 pre 恒假而**空洞成立**。
#[test]
fn uniswap_precondition_is_satisfiable() {
    // R0=1000, R1=1000, dIn=100, dOut=90
    //   pre:  90*(1000*1000 + 997*100) = 98,973,000 <= 997*100*1000 = 99,700,000 ✓
    //        90 <= 1000 ✓
    //   post: (1100)*(910) = 1,001,000 >= 1,000,000 ✓
    let env: HashMap<String, u128> = [
        ("R0", 1000),
        ("R1", 1000),
        ("dIn", 100),
        ("dOut", 90),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();

    let pre = gatelang::spec::parse_spec("dOut * (1000*R0 + 997*dIn) <= 997*dIn*R1 && dOut <= R1")
        .unwrap();
    let post = gatelang::spec::parse_spec("(R0 + dIn) * (R1 - dOut) >= R0 * R1").unwrap();
    assert_eq!(eval_spec(&pre, &env).unwrap(), 1, "该取值应满足前置");
    assert_eq!(eval_spec(&post, &env).unwrap(), 1, "该取值应满足后置（非空洞）");
}

/// 可靠性：多项式层**绝不改变既有语料的结论**（有 vs 无，逐条对照纯 SAT）。
#[test]
fn poly_layer_never_changes_a_verdict() {
    let files: [(&str, &str); 5] = [
        ("adder4_spec", include_str!("../examples/adder4_spec.gat")),
        ("equiv", include_str!("../examples/equiv.gat")),
        ("stdlib_l1", include_str!("../examples/stdlib_l1.gat")),
        ("erc20_core", include_str!("../examples/erc20_core.gat")),
        (
            "erc20_invariant_small",
            include_str!("../examples/erc20_invariant_small.gat"),
        ),
    ];
    for (label, src) in files {
        let (decls, compiled) = compile(src);
        let with_poly = prove_all(&decls, &compiled);
        let specs: Vec<&gatelang::ast::Spec> = decls
            .iter()
            .filter_map(|d| match d {
                Decl::Spec(s) => Some(s),
                _ => None,
            })
            .collect();
        let sat_only: Vec<Result<gatelang::prove::ProveReport, String>> = specs
            .iter()
            .map(|s| match find_target(&compiled, s) {
                Some(t) => prove_spec(t, s),
                None => Err(format!("{}: 未找到对应声明", s.name)),
            })
            .collect();
        let fp = |rs: &[Result<gatelang::prove::ProveReport, String>]| -> Vec<String> {
            rs.iter()
                .map(|r| match r {
                    Ok(rep) => format!(
                        "{}:{:?}",
                        rep.circuit,
                        rep.obligations
                            .iter()
                            .map(|o| (o.kind.clone(), o.verdict.is_proven()))
                            .collect::<Vec<_>>()
                    ),
                    Err(e) => format!("<err>{e}"),
                })
                .collect()
        };
        assert_eq!(fp(&with_poly), fp(&sat_only), "{label}: 多项式层改变了结论");
    }
}

/// 守卫纪律：`a - b` 没有 `b <= a` 守卫时必须 bail（保证无回绕）。
#[test]
fn subtraction_without_guard_bails() {
    assert_eq!(
        poly::prove_text(&uniswap_ports(), None, "(R1 - dOut) <= R1"),
        None
    );
}

/// 可能回绕时必须 bail（128 位端口的乘积会溢出 u128）。
#[test]
fn potential_wrap_bails() {
    let ports = vec![
        Param { name: "a".into(), width: Width::Bits(128), span: Span::new(0, 0) },
        Param { name: "b".into(), width: Width::Bits(128), span: Span::new(0, 0) },
    ];
    assert_eq!(poly::prove_text(&ports, None, "a * b >= a"), None);
}
