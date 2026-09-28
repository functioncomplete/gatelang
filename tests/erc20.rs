//! ERC-20 纯计算核心的形式化验证回归测试。
//!
//! 这些契约对应 OpenZeppelin ERC-20 中**可在 GateLang 纯计算模型内表达**的
//! 算术核心。输入是 Bits<16>，`ERC20Transfer` 有 48 位输入 ——
//! 穷举验证器（`verify.rs`）对此**必须拒绝**（2^48 不可枚举），
//! 而形式化证明器必须给出确定结论。
//!
//! 测试同时锁定两类结论：
//!   - 正确实现的安全属性 → `Proven`
//!   - 刻意构造的漏洞实现 → `Refuted`，且反例必须落在真实漏洞场景上

use std::collections::HashMap;

use gatelang::lower::Compiler;
use gatelang::parser::parse_program;
use gatelang::prove::{prove_all, Verdict};
use gatelang::sim;
use gatelang::spec::assert_spec;
use gatelang::verify::verify_all;

fn compile(src: &str) -> (Vec<gatelang::ast::Decl>, Vec<gatelang::lower::Compiled>) {
    let decls = parse_program(src).expect("parse 失败");
    let mut c = Compiler::new(&decls);
    let compiled = c.compile_all().expect("编译失败");
    (decls, compiled)
}

fn erc20_src() -> String {
    std::fs::read_to_string("examples/erc20_core.gat").expect("读取 erc20_core.gat")
}

fn report_for<'a>(
    reports: &'a [Result<gatelang::prove::ProveReport, String>],
    circuit: &str,
) -> &'a gatelang::prove::ProveReport {
    reports
        .iter()
        .filter_map(|r| r.as_ref().ok())
        .find(|r| r.circuit.ends_with(circuit))
        .unwrap_or_else(|| panic!("未找到 {circuit} 的证明报告"))
}

fn verdict_of<'a>(rep: &'a gatelang::prove::ProveReport, kind: &str) -> &'a Verdict {
    &rep.obligations
        .iter()
        .find(|o| o.kind == kind)
        .unwrap_or_else(|| panic!("{} 缺少 {kind} 义务", rep.circuit))
        .verdict
}

/// 穷举验证器对 ERC-20 核心**完全无能为力** —— 这正是不做形式化就没法验证的场景。
#[test]
fn exhaustive_verifier_cannot_handle_erc20_core() {
    let (decls, compiled) = compile(&erc20_src());
    let brute = verify_all(&decls, &compiled);
    assert!(!brute.ok(), "穷举验证器应当拒绝全部 ERC-20 规格");
    assert!(
        brute.failed.iter().all(|f| f.contains("位宽过大")),
        "拒绝原因应是位宽过大（不可枚举），实际: {:?}",
        brute.failed
    );
}

/// 正确实现的核心安全属性必须被**形式化证明**（对全部 2^48 个输入成立）。
#[test]
fn erc20_correct_implementations_are_formally_proven() {
    let (decls, compiled) = compile(&erc20_src());
    let reports = prove_all(&decls, &compiled);

    // C1: checked 减法恰好在下溢时失败
    let r = report_for(&reports, "ERC20SafeSub");
    assert!(
        matches!(verdict_of(r, "postcondition"), Verdict::Proven),
        "ERC20SafeSub 的下溢检查应被证明: {:?}",
        verdict_of(r, "postcondition")
    );
    assert!(
        matches!(verdict_of(r, "invariant"), Verdict::Proven),
        "ERC20SafeSub 的差值语义应被证明"
    );

    // C3: checked 加法恰好在上溢时失败
    let r = report_for(&reports, "ERC20SafeAdd");
    assert!(matches!(verdict_of(r, "postcondition"), Verdict::Proven));
    assert!(matches!(verdict_of(r, "invariant"), Verdict::Proven));

    // C4: 余额守恒（ERC-20 最核心的安全不变量）
    let r = report_for(&reports, "ERC20Transfer");
    assert!(
        matches!(verdict_of(r, "postcondition"), Verdict::Proven),
        "余额守恒必须对任意输入成立: {:?}",
        verdict_of(r, "postcondition")
    );
    assert!(
        matches!(verdict_of(r, "invariant"), Verdict::Proven),
        "条件式余额保证必须成立: {:?}",
        verdict_of(r, "invariant")
    );
}

/// 漏洞实现必须被**驳倒**，且反例必须真的落在漏洞场景上。
#[test]
fn erc20_buggy_implementations_are_refuted_with_sound_counterexamples() {
    let (decls, compiled) = compile(&erc20_src());
    let reports = prove_all(&decls, &compiled);

    // C2: 漏掉下溢检查
    let r = report_for(&reports, "ERC20SubBuggy");
    let input = match verdict_of(r, "postcondition") {
        Verdict::Refuted { input } => input.clone(),
        other => panic!("漏洞实现应被驳倒，实际 {other:?}"),
    };
    let env = parse_cex(&input);
    let (a, b) = (env["a"], env["b"]);
    assert!(a < b, "下溢反例必须满足 a < b（真实下溢场景），实际 a={a} b={b}");
    // 用该输入模拟漏洞电路：确认它确实返回了 ok=1（即漏报）
    let comp = find(&compiled, "ERC20SubBuggy");
    let out = eval(comp, &env);
    assert_eq!(out["ok"], 1, "漏洞电路在反例输入下确实错误地返回 ok=1");

    // C5: to 侧溢出漏检查
    let r = report_for(&reports, "ERC20TransferBuggy");
    let input = match verdict_of(r, "postcondition") {
        Verdict::Refuted { input } => input.clone(),
        other => panic!("漏洞实现应被驳倒，实际 {other:?}"),
    };
    let env = parse_cex(&input);
    let (from, to, amt) = (env["fromBal"], env["toBal"], env["amount"]);
    let comp = find(&compiled, "ERC20TransferBuggy");
    let out = eval(comp, &env);
    assert_eq!(out["ok"], 1, "漏洞电路在反例输入下错误地报告成功");
    assert!(
        from >= amt,
        "反例必须让 from 侧检查通过（否则不是'本应成功却出错'的场景）"
    );
    assert!(
        to + amt >= (1u128 << 16),
        "反例必须让 to 侧加法真的溢出（to={to} amt={amt}）"
    );
    assert!(out["newTo"] < to, "溢出后接收方余额反而变小（凭空销毁）");
    let _ = assert_spec; // 保持导入语义清晰
}

/* ---------------- 辅助 ---------------- */

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

fn parse_cex(s: &str) -> HashMap<String, u128> {
    let mut m = HashMap::new();
    for part in s.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((k, v)) = part.split_once('=') {
            let val = u128::from_str_radix(v.trim().trim_start_matches("0x"), 16)
                .unwrap_or_else(|_| panic!("反例值解析失败: {part}"));
            m.insert(k.trim().to_string(), val);
        }
    }
    m
}

/// 用给定输入模拟组合电路，返回输出名 → 值。
fn eval(comp: &gatelang::lower::Compiled, env: &HashMap<String, u128>) -> HashMap<String, u128> {
    let (outputs, netlist) = match comp {
        gatelang::lower::Compiled::Combinational { outputs, netlist, .. } => (outputs, netlist),
        _ => panic!("应为组合电路"),
    };
    let res = sim::eval_netlist(netlist, env);
    let mut out = HashMap::new();
    for p in outputs {
        let mut v = 0u128;
        for bit in 0..p.width.bits().min(128) {
            if res.get(&format!("{}_{}", p.name, bit)).copied().unwrap_or(false) {
                v |= 1u128 << bit;
            }
        }
        out.insert(p.name.clone(), v);
    }
    out
}
