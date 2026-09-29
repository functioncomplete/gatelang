//! 词级重写层（`word.rs`）的集成测试。
//!
//! ## 核心不变量
//!
//! **词级层绝不改变结论。** 它只能把「本来就成立」的命题更快地判为成立；
//! 对任何不成立的命题必须**回落 SAT**（并得到反例）。因此本文件的中心是把
//! 「**有词级层**」与「**无词级层**」（直接调 `prove_spec`，纯 SAT）在同一批
//! 语料上**逐条对照** —— 任何分歧都意味着词级层不可靠。
//!
//! 这正是《GateLang ERC-20 形式化验证报告》§7.2 定下的纪律：
//! 功能要对着动机场景做「有 vs 无」的实测对照，而不是写完就当成功。

use gatelang::ast::Decl;
use gatelang::lower::Compiler;
use gatelang::parser::parse_program;
use gatelang::prove::{find_target, prove_all, prove_spec, Verdict};
use gatelang::spec::parse_spec;
use gatelang::word;

fn compile(src: &str) -> (Vec<Decl>, Vec<gatelang::lower::Compiled>) {
    let decls = parse_program(src).expect("parse 失败");
    let mut c = Compiler::new(&decls);
    let compiled = c.compile_all().expect("编译失败");
    (decls, compiled)
}

fn circ<'a>(d: &'a [Decl], n: &str) -> &'a gatelang::ast::Circuit {
    d.iter()
        .find_map(|x| match x {
            Decl::Circuit(c) if c.name == n => Some(c),
            _ => None,
        })
        .unwrap_or_else(|| panic!("未找到电路 {n}"))
}

/// 结论指纹：电路名 + 每条义务的 (kind, 是否已证)。用于「有/无词级层」对照。
fn fingerprint(rs: &[Result<gatelang::prove::ProveReport, String>]) -> Vec<String> {
    rs.iter()
        .map(|r| match r {
            Ok(rep) => {
                let obs: Vec<String> = rep
                    .obligations
                    .iter()
                    .map(|o| format!("{}={}", o.kind, o.verdict.is_proven()))
                    .collect();
                format!(
                    "{}[pre_unsat={}]{{{}}}",
                    rep.circuit,
                    rep.pre_unsatisfiable,
                    obs.join(",")
                )
            }
            Err(e) => format!("<err>{e}"),
        })
        .collect()
}

/* ===================== 验收场景：N=4 @ Bits<32> ===================== */

/// 对照实验的验收：同一个 `Bits<32>` 多项求和问题，
/// * 正确实现 → 词级层**瞬间**证明（CNF 规模为 0，即未走 bit-blast）
/// * 漏洞实现 → **被驳倒**并给出反例
///
/// 实测（本机）：纯 SAT 路径在同一文件上 >280s 无结论；词级路径 0.2s。
#[test]
fn n4_bits32_proved_and_refuted() {
    let src = include_str!("../examples/erc20_invariant_n4_32.gat");
    let (decls, compiled) = compile(src);
    let rs = prove_all(&decls, &compiled);
    assert_eq!(rs.len(), 2, "应有两条 spec");

    let good = rs[0].as_ref().expect("正确实现不应报错");
    assert!(
        good.all_proven(),
        "正确实现应被证明，实际 {:?}",
        good.obligations
    );
    assert_eq!(
        good.obligations[0].cnf_vars, 0,
        "应走词级重写路径（未构造 CNF）；走 SAT 说明快捷键没生效"
    );

    let bad = rs[1].as_ref().expect("漏洞实现不应报错");
    assert!(!bad.all_proven(), "漏洞实现不得被判为已证明");
    assert!(
        matches!(bad.obligations[0].verdict, Verdict::Refuted { .. }),
        "漏洞实现应给出反例，实际 {:?}",
        bad.obligations[0].verdict
    );
}

/// 负面对照：词级层的判定直接暴露出来，确认它**不会**误证漏洞实现。
#[test]
fn buggy_circuit_is_not_word_proven() {
    let src = include_str!("../examples/erc20_invariant_n4_32.gat");
    let (decls, _) = compile(src);
    let post = parse_spec("violation == 0").unwrap();

    assert_eq!(
        word::word_prove(circ(&decls, "ERC20Step4x32"), None, &post),
        Some(true),
        "正确实现应在词级被证明"
    );
    assert_eq!(
        word::word_prove(circ(&decls, "ERC20Step4x32Buggy"), None, &post),
        None,
        "词级层不得把漏洞实现判为已证明（必须回落 SAT）"
    );
}

/* ===================== 核心验收：N=4 @ 真实 uint256 ===================== */

/// 报告 §7.2.1 判定「纯 SAT 完全不可达」的场景：
/// **N=4 账户、真实 uint256（256 位）** 的不变量保持。
///
/// 纯 SAT 在此没有可行路径（多项重结合 + 256 位）；词级重写层应在
/// bit-blast 之前规范化判证（CNF 规模 0）。
#[test]
fn n4_uint256_is_word_proven() {
    let src = include_str!("../examples/erc20_invariant_n4_256.gat");
    let (decls, compiled) = compile(src);
    let rs = prove_all(&decls, &compiled);
    let rep = rs[0].as_ref().expect("不应报错");
    assert!(
        rep.all_proven(),
        "N=4 @ uint256 应被证明，实际 {:?}",
        rep.obligations
    );
    assert_eq!(rep.obligations[0].cnf_vars, 0, "应走词级重写路径");
}

/// 256 位下的**负面对照**：漏洞实现（to 侧误用减法）不得被词级层判证。
///
/// 用词级接口直接断言，避免在 256 位下让 SAT 去找反例（那本身就可能是长时间搜索）。
#[test]
fn buggy_uint256_is_not_word_proven() {
    let src = r#"
circuit Buggy256(
    b0: Bits<256>, b1: Bits<256>, b2: Bits<256>, b3: Bits<256>,
    total: Bits<256>, amount: Bits<256>
) -> (violation: Bit) {
    invBefore = (b0 + b1 + b2 + b3) == total;
    invAfter = (((b0 - amount) + (b1 - amount)) + b2 + b3) == total;
    violation = AND(invBefore, NOT(invAfter));
    return violation;
}

spec T.Buggy256 {
    precondition: true;
    postcondition: violation == 0;
    invariant: true;
}
"#;
    let (decls, _) = compile(src);
    let post = parse_spec("violation == 0").unwrap();
    assert_eq!(
        word::word_prove(circ(&decls, "Buggy256"), None, &post),
        None,
        "256 位漏洞实现不得被词级层判真（必须回落 SAT）"
    );
}

/* ===================== 可靠性：词级层绝不改变结论 ===================== */

/// 对既有语料逐条对照「有词级层」与「无词级层（纯 SAT）」的结论。
///
/// 这些例子的位宽 ≤128 且构造受支持，词级层**会**介入 ——
/// 因此这正是检验它没有改变任何结论的地方。
#[test]
fn word_layer_never_changes_a_verdict() {
    let files: [(&str, &str); 6] = [
        ("adder4_spec", include_str!("../examples/adder4_spec.gat")),
        ("equiv", include_str!("../examples/equiv.gat")),
        ("domain_equiv", include_str!("../examples/domain_equiv.gat")),
        ("stdlib_l1", include_str!("../examples/stdlib_l1.gat")),
        ("erc20_core", include_str!("../examples/erc20_core.gat")),
        (
            "erc20_invariant_small",
            include_str!("../examples/erc20_invariant_small.gat"),
        ),
    ];

    for (label, src) in files {
        let (decls, compiled) = compile(src);
        let with_word = prove_all(&decls, &compiled);

        // 无词级层：直接走 prove_spec（纯 SAT）
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

        assert_eq!(
            fingerprint(&with_word),
            fingerprint(&sat_only),
            "{label}: 词级层改变了结论（应为纯加速，不得改变判定）"
        );
    }
}

/// 词级层介入的**证据**：至少有一条义务在启用词级层后未构造 CNF
/// （否则本文件其余测试可能在"根本没生效"的情况下空过）。
#[test]
fn word_layer_actually_engages() {
    let src = include_str!("../examples/erc20_invariant_small.gat");
    let (decls, compiled) = compile(src);
    let rs = prove_all(&decls, &compiled);
    let engaged = rs
        .iter()
        .filter_map(|r| r.as_ref().ok())
        .flat_map(|rep| rep.obligations.iter())
        .any(|o| o.cnf_vars == 0 && o.verdict.is_proven());
    assert!(
        engaged,
        "词级层应至少在 erc20_invariant_small 上介入（CNF 规模 0 的已证义务）"
    );
}

/* ===================== 词级代数的单元级性质 ===================== */

/// 重结合恒等式在词级成立，且**不做**不可靠的反向判定。
#[test]
fn reassociation_is_proven_but_inequality_is_not() {
    // (b0-a)+(b1+a)+b2+b3 == b0+b1+b2+b3   （32 位）
    let src = include_str!("../examples/erc20_invariant_n4_32.gat");
    let (decls, _) = compile(src);
    let c = circ(&decls, "ERC20Step4x32");

    // 恒真：应证明
    let ok = parse_spec("violation == 0").unwrap();
    assert_eq!(word::word_prove(c, None, &ok), Some(true));

    // 恒假 / 不成立：**不得**判真（返回 None，交回 SAT）
    let buggy = circ(&decls, "ERC20Step4x32Buggy");
    let never = parse_spec("violation == 1").unwrap();
    assert_eq!(
        word::word_prove(buggy, None, &never),
        None,
        "不成立的命题不得被词级层判真"
    );
}
