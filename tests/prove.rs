//! 形式化验证内核（SAT 后端）的集成测试。
//!
//! 最重要的一组测试是**交叉验证**：把新的形式化证明器与既有的穷举验证器
//! （`verify.rs` / `equiv.rs`，经 18 轮独立审计）在同一批输入上对比结论。
//! 两者在可判定范围内必须**完全一致** —— 任何分歧都意味着其中一方有 bug。
//!
//! 这正是本模块声称"证明可信"的证据来源：不是"我们相信 SAT 求解器"，
//! 而是"两条独立实现路径给出同一结论"。

use std::collections::HashMap;

use gatelang::equiv::check_equiv_domain;
use gatelang::lower::Compiler;
use gatelang::parser::parse_program;
use gatelang::prove::{prove_all, prove_equiv_sat, synthesize_expr, Verdict};
use gatelang::sim;
use gatelang::spec::{assert_spec, eval_spec, parse_spec};
use gatelang::verify::verify_all;

fn compile(src: &str) -> (Vec<gatelang::ast::Decl>, Vec<gatelang::lower::Compiled>) {
    let decls = parse_program(src).expect("parse 失败");
    let mut c = Compiler::new(&decls);
    let compiled = c.compile_all().expect("编译失败");
    (decls, compiled)
}

fn find<'a>(
    compiled: &'a [gatelang::lower::Compiled],
    name: &str,
) -> &'a gatelang::lower::Compiled {
    compiled
        .iter()
        .find(|c| match c {
            gatelang::lower::Compiled::Combinational { name: n, .. }
            | gatelang::lower::Compiled::State { name: n, .. } => n == name,
        })
        .unwrap_or_else(|| panic!("未找到电路 {name}"))
}

/// 把 `a=0x5, b=0x3` 形式的反例串解析回输入映射。
fn parse_counterexample(s: &str) -> HashMap<String, u128> {
    let mut m = HashMap::new();
    for part in s.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((k, v)) = part.split_once('=') {
            let val = u128::from_str_radix(v.trim_start_matches("0x"), 16)
                .unwrap_or_else(|_| panic!("反例值解析失败: {part}"));
            m.insert(k.trim().to_string(), val);
        }
    }
    m
}

/* ==================== 交叉验证：SAT vs 穷举 ==================== */

/// 对每个示例文件，逐条 spec 比对"穷举验证器"与"形式化证明器"的结论。
#[test]
fn sat_prover_agrees_with_exhaustive_verifier_on_examples() {
    for f in ["adder4_spec.gat", "equiv.gat", "halfadder.gat", "stdlib_l1.gat"] {
        let path = format!("examples/{f}");
        let src = std::fs::read_to_string(&path).expect("读取示例");
        let (decls, compiled) = compile(&src);

        let brute = verify_all(&decls, &compiled);
        let sat = prove_all(&decls, &compiled);

        assert_eq!(
            sat.len(),
            decls
                .iter()
                .filter(|d| matches!(d, gatelang::ast::Decl::Spec(_)))
                .count(),
            "{f}: spec 条数应一致"
        );

        let sat_all_ok = sat
            .iter()
            .all(|r| r.as_ref().map(|x| x.all_proven()).unwrap_or(false));

        assert_eq!(
            brute.ok(),
            sat_all_ok,
            "{f}: 穷举结论 {:?} 与 SAT 结论不一致\n穷举失败项: {:?}\nSAT: {:?}",
            brute.ok(),
            brute.failed,
            sat.iter()
                .map(|r| match r {
                    Ok(x) => format!(
                        "{} => {:?}",
                        x.circuit,
                        x.obligations.iter().map(|o| &o.verdict).collect::<Vec<_>>()
                    ),
                    Err(e) => format!("ERR {e}"),
                })
                .collect::<Vec<_>>()
        );
    }
}

/// 被驳倒的 spec，其反例必须在模拟下**确实违反**规格（反例可靠性）。
#[test]
fn refutation_counterexample_is_sound() {
    // 故意写错的 postcondition：加法器声明成减法
    let src = r#"
    circuit Adder4(a: Bits<4>, b: Bits<4>) -> (s: Bits<4>) {
        s = a + b;
        return s;
    }
    spec FCT.math.Adder4 {
        precondition: true;
        postcondition: s == (a - b) % (2^4);
        invariant: true;
    }"#;
    let (decls, compiled) = compile(src);
    let sat = prove_all(&decls, &compiled);
    let rep = sat[0].as_ref().expect("应有报告");
    assert!(!rep.all_proven(), "错误的 postcondition 必须被驳倒");

    let ob = rep
        .obligations
        .iter()
        .find(|o| o.kind == "postcondition")
        .expect("应有 postcondition 义务");
    let input = match &ob.verdict {
        Verdict::Refuted { input } => input.clone(),
        other => panic!("应为 Refuted，实际 {other:?}"),
    };

    // 用反例输入重新模拟，确认规格确实被违反
    let comp = find(&compiled, "Adder4");
    let (inputs, outputs, netlist) = match comp {
        gatelang::lower::Compiled::Combinational { inputs, outputs, netlist, .. } => {
            (inputs, outputs, netlist)
        }
        _ => panic!("应为组合电路"),
    };
    let env = parse_counterexample(&input);
    let res = sim::eval_netlist(netlist, &env);
    let mut out_map = HashMap::new();
    for p in outputs {
        let mut v = 0u128;
        for bit in 0..p.width.bits().min(128) {
            if res.get(&format!("{}_{}", p.name, bit)).copied().unwrap_or(false) {
                v |= 1u128 << bit;
            }
        }
        out_map.insert(p.name.clone(), v);
    }
    let _ = inputs;
    let spec_holds = assert_spec("s == (a - b) % (2^4)", &env, &out_map).expect("求值");
    assert!(
        !spec_holds,
        "反例 {input} 必须在语义上确实违反规格（否则反例不可靠）"
    );
}

/// 正确 spec 必须被证明（不得误报反例）。
#[test]
fn correct_spec_is_proven_not_refuted() {
    let src = r#"
    circuit Adder4(a: Bits<4>, b: Bits<4>) -> (s: Bits<4>) {
        s = a + b;
        return s;
    }
    spec FCT.math.adder4 {
        precondition: a < 2^4 && b < 2^4;
        postcondition: s == (a + b) % (2^4);
        invariant: true;
    }"#;
    let (decls, compiled) = compile(src);
    let sat = prove_all(&decls, &compiled);
    let rep = sat[0].as_ref().expect("应有报告");
    assert!(rep.all_proven(), "正确规格应被证明: {:?}", rep.obligations);
    // UNSAT 意味着零冲突、无反例
    let ob = &rep.obligations[0];
    assert!(matches!(ob.verdict, Verdict::Proven));
    assert!(ob.cnf_clauses > 0, "应有实际编码规模");
}

/* ==================== 等价性：SAT vs 穷举 ==================== */

#[test]
fn sat_equivalence_agrees_with_brute_force_on_examples() {
    let src = std::fs::read_to_string("examples/halfadder.gat").expect("读取");
    let (_, compiled) = compile(&src);
    let a = find(&compiled, "HalfAdder5");
    let b = find(&compiled, "HalfAdderNaive");

    let (brute_eq, _) = check_equiv_domain(a, b, None).expect("穷举");
    let (sat_eq, _) = prove_equiv_sat(a, b).expect("SAT");
    assert_eq!(brute_eq, sat_eq, "半加器两实现的等价性结论必须一致");
    assert!(sat_eq, "HalfAdder5 与 HalfAdderNaive 应等价");
}

#[test]
fn sat_equivalence_detects_inequivalence() {
    let src = r#"
    circuit Eq1(a: Bits<4>, b: Bits<4>) -> (s: Bits<4>) { s = a + b; return s; }
    circuit Eq2(a: Bits<4>, b: Bits<4>) -> (s: Bits<4>) { s = a + b + 1; return s; }
    "#;
    let (_, compiled) = compile(src);
    let a = find(&compiled, "Eq1");
    let b = find(&compiled, "Eq2");
    let (eq, cex) = prove_equiv_sat(a, b).expect("SAT");
    assert!(!eq, "a+b 与 a+b+1 不应等价");
    assert!(cex.is_some(), "不等价必须给出反例");
}

/* ==================== 关键能力：突破穷举上限 ==================== */

/// 32 位加法器自等价：穷举因 64 位输入拒绝，SAT 必须证明等价。
/// 这是"从测试到证明"的核心证据 —— 旧上限是 20 位输入。
#[test]
fn sat_equivalence_beyond_exhaustive_limit() {
    let src = r#"
    circuit Add32A(a: Bits<32>, b: Bits<32>) -> (s: Bits<32>) { s = a + b; return s; }
    circuit Add32B(a: Bits<32>, b: Bits<32>) -> (s: Bits<32>) { s = a + b; return s; }
    "#;
    let (_, compiled) = compile(src);
    let a = find(&compiled, "Add32A");
    let b = find(&compiled, "Add32B");

    // 穷举路径必须因位宽拒绝（证明旧上限确实存在）
    let brute = check_equiv_domain(a, b, None).expect("穷举应返回 Err 或拒绝结果");
    assert!(!brute.0, "穷举应因 64 位输入无法穷举而拒绝: {brute:?}");

    // SAT 路径必须证明等价
    let (eq, cex) = prove_equiv_sat(a, b).expect("SAT 不应失败");
    assert!(eq, "32 位加法器自等价应被形式化证明（反例 {cex:?}）");
}

/// 32 位加法器规格：穷举拒绝，SAT 证明。
#[test]
fn sat_proves_wide_spec_beyond_brute_force() {
    let src = r#"
    circuit Add32(a: Bits<32>, b: Bits<32>) -> (s: Bits<32>) { s = a + b; return s; }
    spec FCT.math.add32 {
        precondition: true;
        postcondition: s == (a + b) % (2^32);
        invariant: true;
    }"#;
    let (decls, compiled) = compile(src);

    // 穷举验证器应拒绝
    let brute = verify_all(&decls, &compiled);
    assert!(!brute.ok(), "穷举应因位宽过大而拒绝: {:?}", brute.failed);

    // SAT 应证明
    let sat = prove_all(&decls, &compiled);
    let rep = sat[0].as_ref().expect("应有报告");
    assert!(
        rep.all_proven(),
        "32 位加法器规格应被形式化证明: {:?}",
        rep.obligations.iter().map(|o| (&o.kind, &o.verdict)).collect::<Vec<_>>()
    );
}

/// 综合器必须支持 `*` / `%` 的**廉价路径**（常量乘数、二次幂取模）。
/// 注意：GateLang 的**电路表达式**不支持 `*`/`%`，它们只出现在 `spec` 一侧。
#[test]
fn sat_handles_mul_and_mod_synthesis() {
    // 电路用 a+a+a 表达 3a（mod 2^4）；spec 用 `a * 3` 与 `% (2^4)`
    let src = r#"
    circuit Mul3(a: Bits<4>) -> (y: Bits<4>) {
        y = a + a + a;
        return y;
    }
    spec FCT.math.Mul3 {
        precondition: true;
        postcondition: y == (a * 3) % (2^4);
        invariant: true;
    }"#;
    let (decls, compiled) = compile(src);
    let brute = verify_all(&decls, &compiled);
    let sat = prove_all(&decls, &compiled);
    let rep = sat[0].as_ref().expect("报告");
    assert_eq!(
        brute.ok(),
        rep.all_proven(),
        "乘法/取模综合结论必须与穷举一致：穷举 {:?}，SAT {:?}",
        brute.failed,
        rep.obligations.iter().map(|o| &o.verdict).collect::<Vec<_>>()
    );
    assert!(rep.all_proven(), "3a == (a*3) % 16 应被证明");
}

/// 高代价综合必须 **fail-closed**：非常量、非二次幂的取模必须报错，
/// 而不是给出一个可能错误的结论。
#[test]
fn expensive_synthesis_fails_closed() {
    // 除数 3 不是二次幂 → 取模综合代价过高，必须明确报错
    let src = r#"
    circuit F(a: Bits<4>) -> (y: Bits<4>) { y = a; return y; }
    spec F {
        precondition: true;
        postcondition: y == a % 3;
        invariant: true;
    }"#;
    let (decls, compiled) = compile(src);
    let sat = prove_all(&decls, &compiled);
    assert!(
        sat[0].is_err(),
        "非二次幂取模应 fail-closed 报错，绝不能给出结论: {:?}",
        sat[0]
    );
}

/// 前置条件为空域时必须报"未定"，不能算作已证明。
#[test]
fn unsat_precondition_is_not_proven() {
    let src = r#"
    circuit F(a: Bit) -> (y: Bit) { y = a; return y; }
    spec F {
        precondition: a > 1;
        postcondition: y == a;
        invariant: true;
    }"#;
    let (decls, compiled) = compile(src);
    let sat = prove_all(&decls, &compiled);
    let rep = sat[0].as_ref().expect("报告");
    assert!(rep.pre_unsatisfiable, "a > 1 对 1 位输入恒不成立，应标记空域");
    assert!(!rep.all_proven(), "空输入域不得算作已证明");
}

/// spec 引用不存在的端口必须报错，绝不静默忽略。
#[test]
fn unknown_spec_variable_is_error() {
    let src = r#"
    circuit F(a: Bit) -> (y: Bit) { y = a; return y; }
    spec F { precondition: true; postcondition: y == zzz; invariant: true; }"#;
    let (decls, compiled) = compile(src);
    let sat = prove_all(&decls, &compiled);
    assert!(sat[0].is_err(), "引用未绑定变量应报错: {:?}", sat[0]);
}

/// 形式化证明不得依赖"穷举可能恰好没覆盖到"——构造只在单一输入上失败的性质。
/// `y == (a + b) % 16` 对 4 位加法器成立；但 `y == a` 只在 a+b 不进位时成立，
/// 反例数极少（如 a=0,b=1），仍必须被找出。
#[test]
fn refutes_property_holding_on_most_inputs() {
    let src = r#"
    circuit Adder4(a: Bits<4>, b: Bits<4>) -> (s: Bits<4>) { s = a + b; return s; }
    spec FCT.math.Adder4 {
        precondition: true;
        postcondition: s == a;
        invariant: true;
    }"#;
    let (decls, compiled) = compile(src);
    let sat = prove_all(&decls, &compiled);
    let rep = sat[0].as_ref().expect("报告");
    assert!(!rep.all_proven(), "s == a 对加法器不成立");
    let ob = &rep.obligations[0];
    match &ob.verdict {
        Verdict::Refuted { input } => {
            let env = parse_counterexample(input);
            // 反例必须满足 b == 0 或 a 的加法不进位；至少 b 应使 s != a
            assert!(env.contains_key("a") && env.contains_key("b"), "反例应含全部输入");
            let a = env["a"];
            let b = env["b"];
            assert_ne!((a + b) % 16, a, "反例必须真的违反 s == a");
        }
        other => panic!("应为 Refuted，实际 {other:?}"),
    }
}

/// 布尔运算语义保真：`&&`/`||`/`!` 必须先把操作数归约为 0/1（与 eval_spec 一致）。
#[test]
fn boolean_operators_match_eval_semantics() {
    // 规格用非 0/1 的操作数：`1 && 2` 在 eval_spec 语义下为 1（两者非零）
    let src = r#"
    circuit F(a: Bits<4>) -> (y: Bits<4>) { y = a; return y; }
    spec F {
        precondition: true;
        postcondition: y == a && (1 && 2);
        invariant: true;
    }"#;
    let (decls, compiled) = compile(src);
    let brute = verify_all(&decls, &compiled);
    let sat = prove_all(&decls, &compiled);
    let rep = sat[0].as_ref().expect("报告");
    assert_eq!(
        brute.ok(),
        rep.all_proven(),
        "布尔运算语义不一致：穷举 {:?} SAT {:?}",
        brute.failed,
        rep.obligations.iter().map(|o| &o.verdict).collect::<Vec<_>>()
    );
    assert!(rep.all_proven(), "y == a && (1 && 2) 应被证明（与穷举一致）");
}

/// 规格综合器必须覆盖 spec 语法的全部运算符，且对每条表达式给出稳定判定。
/// `E == E` 恒真 —— 若综合器对某个运算符处理错误（如比较器进位、布尔归约），
/// 就会在这里被驳倒。
#[test]
fn synthesis_covers_all_spec_operators() {
    let exprs = [
        "(a + b) % 2",
        "a > b",
        "a >= b",
        "a < b",
        "a <= b",
        "(a - b) == 0",
        "a != b",
        "!(a == b)",
        "(a + a + a) % 4",
        "(a * 3) % 8",
        "(a * b) % 8",
        "a <= b && b <= a",
        "a > 0 || b > 0",
        "(2^4 - 1) - a",
        "MAX_UINT - a == MAX_UINT - a",
    ];
    for e in exprs {
        // 注意：spec 的比较运算符同级左结合，故必须显式加括号写成 `(E) == (E)`，
        // 否则 `a > b == a > b` 会被解析成 `(((a > b) == a) > b)`。
        let src = [
            "circuit C(a: Bits<4>, b: Bits<4>) -> (y: Bit) { y = a == a; return y; }\n",
            "spec C { precondition: true; postcondition: (",
            e,
            ") == (",
            e,
            "); invariant: true; }",
        ]
        .concat();
        let (decls, compiled) = compile(&src);
        let sat = prove_all(&decls, &compiled);
        let rep = match sat[0].as_ref() {
            Ok(r) => r,
            // 高代价综合必须 fail-closed（报错），不得给出结论
            Err(err) => {
                assert!(
                    err.contains("fail-closed") || err.contains("超出预算") || err.contains("除法器"),
                    "表达式 `{e}` 的失败必须是明确的 fail-closed，实际: {err}"
                );
                continue;
            }
        };
        assert!(
            rep.all_proven(),
            "`{e} == {e}` 应恒真却被驳倒，综合器有误: {:?}",
            rep.obligations.iter().map(|o| (&o.kind, &o.verdict)).collect::<Vec<_>>()
        );
        // 与穷举验证器交叉核对（可穷举时）
        let brute = verify_all(&decls, &compiled);
        assert_eq!(brute.ok(), rep.all_proven(), "`{e}` 穷举与 SAT 结论不一致");
        // 解析器必须接受该表达式
        parse_spec(e).unwrap_or_else(|err| panic!("表达式 `{e}` 解析失败: {err}"));
    }
}

/// 综合器对**语义关键**的表达式必须与穷举验证器逐输入一致。
/// 这里用电路真值来校验：`y` 就是电路输出，spec 用各种表达式描述它。
#[test]
fn synthesis_matches_brute_force_on_expression_semantics() {
    // 4 位加法器 + 一组覆盖各运算符的 postcondition
    let posts = [
        "s == (a + b) % (2^4)",
        "(s - a) % (2^4) == b",
        "s == (a + b) - ((a + b) >= 2^4) * (2^4)",
        "(a + b) >= 2^4 || s == (a + b)",
        "!(a + b >= 2^4) || s == (a + b) - 2^4",
        "a <= s || b == 0",
    ];
    for post in posts {
        let src = [
            "circuit A4(a: Bits<4>, b: Bits<4>) -> (s: Bits<4>) { s = a + b; return s; }\n",
            "spec FCT.math.A4 { precondition: true; postcondition: ",
            post,
            "; invariant: true; }",
        ]
        .concat();
        let (decls, compiled) = compile(&src);
        let brute = verify_all(&decls, &compiled);
        let sat = prove_all(&decls, &compiled);
        let rep = match sat[0].as_ref() {
            Ok(r) => r,
            Err(e) => panic!("`{post}` 综合失败（应可支持）: {e}"),
        };
        assert_eq!(
            brute.ok(),
            rep.all_proven(),
            "`{post}` 穷举({}) 与 SAT({}) 结论不一致：穷举失败 {:?}，SAT {:?}",
            brute.ok(),
            rep.all_proven(),
            brute.failed,
            rep.obligations.iter().map(|o| &o.verdict).collect::<Vec<_>>()
        );
    }
}

/* ==================== 决定性测试：综合语义保真 ==================== */

/// **综合保真（决定性）**：把每个规格表达式综合成门级电路，
/// 穷举全部 4 位输入，把「门级模拟结果」与「`spec::eval_spec` 的 u128 参考求值」
/// 逐位比对。任何综合语义偏差都会在这里暴露。
///
/// 这个测试如果能早点存在，`lt` 的进位 bug（`lt(3,2)` 误判为真）会立刻被抓到。
#[test]
fn synthesised_circuit_matches_eval_spec_exhaustively() {
    let exprs = [
        "a + b",
        "a - b",
        "(a + b) % 2",
        "(a + b) % 4",
        "(a + b) % (2^4)",
        "(a + a + a) % 16",
        "(a * 3) % 16",
        "(a * b) % 16",
        "a > b",
        "a >= b",
        "a < b",
        "a <= b",
        "a == b",
        "a != b",
        "!(a == b)",
        "(a - b) == 0",
        "a <= b && b <= a",
        "a > 0 || b > 0",
        "1 && 2",
        "0 || 3",
        "MAX_UINT - a",
        "(2^4 - 1) - a",
        "a + b - a",
        "((a + b) >= 2^4) * (2^4)",
        "a > b || a == b",
    ];
    let src = "circuit C(a: Bits<4>, b: Bits<4>) -> (y: Bit) { y = a == a; return y; }";
    let (_, compiled) = compile(src);
    let c = find(&compiled, "C");

    for e in exprs {
        let parsed = parse_spec(e).unwrap_or_else(|err| panic!("`{e}` 解析失败: {err}"));
        let (nl, _sig) = synthesize_expr(c, &parsed)
            .unwrap_or_else(|err| panic!("`{e}` 综合失败（应可支持）: {err}"));
        for a in 0u128..16 {
            for b in 0u128..16 {
                let mut env: HashMap<String, u128> = HashMap::new();
                env.insert("a".to_string(), a);
                env.insert("b".to_string(), b);
                // 门级综合电路的求值
                let res = sim::eval_netlist(&nl, &env);
                let synth_val = res.get("__spec__").copied().unwrap_or(false);
                // 参考求值器（经 18 轮审计的 u128 语义）
                let ref_val = eval_spec(&parsed, &env).expect("参考求值") != 0;
                assert_eq!(
                    synth_val, ref_val,
                    "表达式 `{e}` 在 a={a} b={b} 时：综合电路给出 {synth_val}，参考求值器给出 {ref_val}"
                );
            }
        }
    }
}

/// 综合保真（宽度交叉）：不同端口位宽组合下也必须一致。
#[test]
fn synthesis_fidelity_across_port_widths() {
    let src = r#"
    circuit W(a: Bit, b: Bits<3>, c: Bits<8>) -> (y: Bit) { y = a == a; return y; }
    "#;
    let (_, compiled) = compile(src);
    let c = find(&compiled, "W");
    let exprs = [
        "a + b",
        "b + c",
        "c - b",
        "(a + c) % 8",
        "a * b",
        "b * c",
        "c > b",
        "(a + b + c) % 2",
        "a && b",
        "!(a) || c",
    ];
    for e in exprs {
        let parsed = parse_spec(e).unwrap_or_else(|err| panic!("`{e}` 解析失败: {err}"));
        let (nl, _) = synthesize_expr(c, &parsed)
            .unwrap_or_else(|err| panic!("`{e}` 综合失败（应可支持）: {err}"));
        for a in 0u128..2 {
            for b in 0u128..8 {
                for cc in [0u128, 1, 7, 8, 127, 128, 255] {
                    let mut env: HashMap<String, u128> = HashMap::new();
                    env.insert("a".to_string(), a);
                    env.insert("b".to_string(), b);
                    env.insert("c".to_string(), cc);
                    let res = sim::eval_netlist(&nl, &env);
                    let synth_val = res.get("__spec__").copied().unwrap_or(false);
                    let ref_val = eval_spec(&parsed, &env).expect("参考求值") != 0;
                    assert_eq!(
                        synth_val, ref_val,
                        "表达式 `{e}` 在 a={a} b={b} c={cc} 时不一致：综合 {synth_val} vs 参考 {ref_val}"
                    );
                }
            }
        }
    }
}

/* ==================== 假证明猎杀：随机规格模糊测试 ==================== */

struct Rng(u64);
impl Rng {
    fn next(&mut self, m: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % m
    }
}

/// 随机生成一个**全括号化**的规格表达式（广度受限于 depth，保证可综合）。
fn gen_expr(r: &mut Rng, depth: usize, vars: &[&str]) -> String {
    let ops = ["+", "-", "%", "*", ">", ">=", "<", "<=", "==", "!=", "&&", "||"];
    let consts = ["0", "1", "2", "3", "15", "16", "MAX_UINT", "true", "false"];
    if depth == 0 || r.next(3) == 0 {
        if r.next(3) == 0 {
            let c = consts[r.next(consts.len() as u64) as usize];
            return c.to_string();
        }
        let v = vars[r.next(vars.len() as u64) as usize];
        return v.to_string();
    }
    let op = ops[r.next(ops.len() as u64) as usize];
    let a = gen_expr(r, depth - 1, vars);
    let b = gen_expr(r, depth - 1, vars);
    format!("({a} {op} {b})")
}

fn is_fail_closed(err: &str) -> bool {
    err.contains("超出预算")
        || err.contains("fail-closed")
        || err.contains("除法器")
        || err.contains("代价过高")
}

/// **随机规格模糊测试**：在多个电路上随机生成 postcondition（绝大多数是错的），
/// 逐条比对形式化证明器与穷举验证器的结论。
///
/// 任何一次"证明器说已证明、穷举却找到反例"（或反向）都说明有 bug。
/// 这是对假证明/假反例最直接的猎杀。
#[test]
fn fuzz_prover_agrees_with_verifier() {
    let circuits = [
        "circuit C(a: Bits<4>, b: Bits<4>) -> (y: Bits<4>) { y = a + b; return y; }",
        "circuit C(a: Bits<4>, b: Bits<4>) -> (y: Bits<4>) { y = a & b; return y; }",
        "circuit C(a: Bits<4>, b: Bits<4>) -> (y: Bits<4>) { y = a + a; return y; }",
        "circuit C(a: Bits<4>, b: Bits<4>) -> (y: Bits<4>) { y = a | b; return y; }",
        "circuit C(a: Bits<4>, b: Bits<4>) -> (y: Bit) { y = a == b; return y; }",
        "circuit C(a: Bits<4>, b: Bits<4>) -> (y: Bit) { y = a > b; return y; }",
        "circuit C(a: Bits<4>, b: Bits<4>) -> (y: Bits<4>) { y = b; return y; }",
    ];
    let mut r = Rng(0xC0FFEE1234567);
    let mut compared = 0usize;
    let mut fail_closed = 0usize;
    for i in 0..400 {
        let circ = circuits[i % circuits.len()];
        let post = gen_expr(&mut r, 3, &["a", "b", "y"]);
        let src = [
            circ,
            "\nspec C { precondition: true; postcondition: ",
            &post,
            "; invariant: true; }",
        ]
        .concat();
        let (decls, compiled) = match std::panic::catch_unwind(|| compile(&src)) {
            Ok(x) => x,
            Err(_) => continue, // 生成器偶发产生非法源码，跳过
        };
        let brute = verify_all(&decls, &compiled);
        let sat = prove_all(&decls, &compiled);
        match sat[0].as_ref() {
            Ok(rep) => {
                compared += 1;
                assert_eq!(
                    brute.ok(),
                    rep.all_proven(),
                    "post `{post}`（电路 #{}) 结论不一致：穷举={} 失败项{:?}；SAT={} {:?}",
                    i % circuits.len(),
                    brute.ok(),
                    brute.failed,
                    rep.all_proven(),
                    rep.obligations.iter().map(|o| (&o.kind, &o.verdict)).collect::<Vec<_>>()
                );
            }
            Err(e) => {
                // 不支持的表达式必须 fail-closed，绝不能给出结论
                assert!(
                    is_fail_closed(e),
                    "post `{post}` 的失败必须是明确的 fail-closed，实际: {e}"
                );
                fail_closed += 1;
            }
        }
    }
    assert!(compared >= 200, "可比对用例太少（{compared}），模糊测试覆盖不足");
    // 记录但不强制：fail-closed 比例反映综合器的能力边界
    println!("模糊测试：比对 {compared} 例，fail-closed {fail_closed} 例");
}

/// **等价性模糊测试**：随机电路对，比对 SAT/miter 与穷举的等价性结论。
#[test]
fn fuzz_equivalence_agrees_with_brute_force() {
    let bodies = [
        "y = a + b; return y;",
        "y = a & b; return y;",
        "y = a | b; return y;",
        "y = a + a; return y;",
        "y = a + a + b; return y;",
        "y = b; return y;",
        "y = a; return y;",
    ];
    let mut r = Rng(0xBADC0DE999);
    let mut eqs = 0usize;
    let mut neqs = 0usize;
    for i in 0..120 {
        let b1 = bodies[i % bodies.len()];
        let b2 = bodies[r.next(bodies.len() as u64) as usize];
        let src = [
            "circuit P(a: Bits<4>, b: Bits<4>) -> (y: Bits<4>) { ",
            b1,
            " }\ncircuit Q(a: Bits<4>, b: Bits<4>) -> (y: Bits<4>) { ",
            b2,
            " }",
        ]
        .concat();
        let (_, compiled) = compile(&src);
        let p = find(&compiled, "P");
        let q = find(&compiled, "Q");
        let (brute_eq, _) = check_equiv_domain(p, q, None).expect("穷举");
        let (sat_eq, cex) = prove_equiv_sat(p, q).expect("SAT");
        assert_eq!(
            brute_eq, sat_eq,
            "等价性结论不一致：P=`{b1}` Q=`{b2}`；穷举={brute_eq} SAT={sat_eq} 反例{cex:?}"
        );
        if sat_eq { eqs += 1 } else { neqs += 1 }
        // 不等价时必须给出反例
        if !sat_eq {
            assert!(cex.is_some(), "不等价必须给出反例");
        }
    }
    assert!(eqs > 0 && neqs > 0, "模糊测试应同时覆盖等价与不等价（eq={eqs} neq={neqs}）");
}
