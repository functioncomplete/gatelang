//! 集成测试：端到端 解析→编译→模拟→验证→等价。

use gatelang::equiv::check_equiv;
use gatelang::parser::parse_program;
use gatelang::verify::verify_all;
use gatelang::lower::Compiler;

fn compile(src: &str) -> (Vec<gatelang::ast::Decl>, Vec<gatelang::lower::Compiled>) {
    let decls = parse_program(src).expect("parse 失败");
    let mut c = Compiler::new(&decls);
    let compiled = c.compile_all().expect("编译失败");
    (decls, compiled)
}

#[test]
fn xor_5gate_halfadder_tvb() {
    // 白皮书 v2.2 §4.2/4.3：半加器真值表
    let src = r#"circuit HA(a: Bit, b: Bit) -> (sum: Bit, carry: Bit) {
        t = NAND(a, b);
        carry = NOT(t);
        lt = NAND(a, t);
        rt = NAND(b, t);
        sum = NAND(lt, rt);
        return (sum, carry);
    }
    spec FCT.halfadder.ha {
        precondition: true;
        postcondition: sum == (a + b) % 2 && carry == ((a + b) >= 2);
        invariant: true;
        edge_cases: [a=0,b=0, a=0,b=1, a=1,b=0, a=1,b=1];
    }"#;
    let (decls, compiled) = compile(src);
    // 5 门
    if let gatelang::lower::Compiled::Combinational { name, netlist, .. } = &compiled[0] {
        assert_eq!(name, "HA");
        assert_eq!(netlist.stats().nand_count, 5);
    } else {
        panic!("应为组合");
    }
    let rep = verify_all(&decls, &compiled);
    assert!(rep.ok(), "{:?}", rep.failed);
}

#[test]
fn xor_two_impls_equivalent() {
    let src = r#"circuit A(a: Bit, b: Bit) -> (y: Bit) { y = XOR(a, b); return y; }
    circuit B(a: Bit, b: Bit) -> (y: Bit) {
        n1 = NAND(a, b); n2 = NAND(a, n1); n3 = NAND(b, n1); y = NAND(n2, n3);
        return y;
    }"#;
    let (_, compiled) = compile(src);
    let (a, b) = (&compiled[0], &compiled[1]);
    let (eq, _) = check_equiv(a, b).expect("equiv 检查应运行");
    assert!(eq, "XOR 两实现应语义等价");
}

#[test]
fn adder4_value_and_bound() {
    let src = r#"circuit Adder4(a: Bits<4>, b: Bits<4>) -> (s: Bits<4>)
        gates: Gates<61> depth: Depth<20>
    {
        s = a + b;
        return s;
    }"#;
    let (decls, compiled) = compile(src);
    let rep = verify_all(&decls, &compiled);
    assert!(rep.ok(), "{:?}", rep.failed);
    // 模拟验证：1+2=3, 15+1=0(溢出)
    if let gatelang::lower::Compiled::Combinational { inputs, outputs, netlist, .. } = &compiled[0] {
        let _ = (inputs, outputs);
        // 用 eval_netlist 直接验证
        let mut ins = std::collections::HashMap::new();
        ins.insert("a".into(), 1u128);
        ins.insert("b".into(), 2u128);
        let res = gatelang::sim::eval_netlist(netlist, &ins);
        let mut s = 0u128;
        for b in 0..4 {
            if res.get(&format!("s_{b}")).copied().unwrap_or(false) { s |= 1 << b; }
        }
        assert_eq!(s, 3);
    } else {
        panic!("应为组合");
    }
}

#[test]
fn counter_state_compiles_and_cycles() {
    let src = r#"state Counter {
        latch value: Bits<4> = 0;
        fn tick() -> Bits<4> {
            value <- value + 1;
            return value;
        }
    }"#;
    let (_, compiled) = compile(src);
    match &compiled[0] {
        gatelang::lower::Compiled::State { name, latches, fns } => {
            assert_eq!(name, "Counter");
            assert_eq!(latches.len(), 1);
            assert_eq!(fns.len(), 1);
        }
        _ => panic!("应为时序"),
    }
}
#[test]
fn domain_equiv_filters_counterexample() {
    // 全域不等价（cin 特例），约束域 a==1&&b==1 等价
    let src = r#"circuit FA(a: Bit, b: Bit, cin: Bit) -> (sum: Bit, cout: Bit) {
        sum = XOR(XOR(a, b), cin);
        cout = OR(AND(a, b), AND(cin, XOR(a, b)));
        return (sum, cout);
    }
    circuit FA2(a: Bit, b: Bit, cin: Bit) -> (sum: Bit, cout: Bit) {
        sum = XOR(XOR(a, b), cin);
        cout = OR(AND(a, b), AND(b, cin));
        return (sum, cout);
    }"#;
    let (_, c) = compile(src);
    let fa = c.iter().find(|x| gatelang::equiv::sig(x).starts_with("FA(")).unwrap();
    let fa2 = c.iter().find(|x| gatelang::equiv::sig(x).starts_with("FA2(")).unwrap();
    // 全域：不等价
    let (ok, ce) = gatelang::equiv::check_equiv_domain(fa, fa2, None).expect("domains");
    assert!(!ok, "全域应不等价");
    assert!(ce.is_some(), "应给出反例");
    // 约束域 a==1 && b==1：等价
    let (ok, _) = gatelang::equiv::check_equiv_domain(fa, fa2, Some("a==1 && b==1")).expect("domain");
    assert!(ok, "约束域内应等价");
}

#[test]
fn full_adder_mux_comparator_specs() {
    // stdlib 模板库：FA / Mux2 spec 全过（Comparator4 由 comparator4_58gates_eq_gt_spec 覆盖）
    let src = r#"circuit FullAdder(a: Bit, b: Bit, cin: Bit) -> (sum: Bit, cout: Bit) {
        ab = XOR(a, b);
        sum = XOR(ab, cin);
        cout = OR(AND(a, b), AND(cin, ab));
        return (sum, cout);
    }
    spec FCT.stdlib.fulladder {
        precondition: true;
        postcondition: sum == (a + b + cin) % 2 && cout == ((a + b + cin) >= 2);
        invariant: true;
        edge_cases: [a=0,b=0,cin=0, a=1,b=1,cin=1, a=1,b=0,cin=1];
    }
    circuit Mux2(a: Bit, b: Bit, sel: Bit) -> Bit {
        out = OR(AND(a, sel), AND(b, NOT(sel)));
        return out;
    }
    spec FCT.stdlib.mux2 {
        precondition: true;
        postcondition: out == (a * sel + b * (1 - sel));
        invariant: true;
        edge_cases: [a=0,b=0,sel=0, a=1,b=0,sel=1, a=0,b=1,sel=0];
    }"#;
    let (decls, c) = compile(src);
    let rep = verify_all(&decls, &c);
    assert_eq!(rep.passed, 2, "两个模板 spec 都应通过: {:?}", rep.failed);
}

#[test]
fn comparator4_58gates_eq_gt_spec() {
    // stdlib 模板库：Comparator4 必须 58 门 / 深度 20，且 eq、gt(a>b) 均正确。
    // 此前 spec 只校验 eq，gt 的错误会静默通过；此处补上 gt 并锁定资源上界。
    let src = r#"circuit Comparator4(a: Bits<4>, b: Bits<4>) -> (eq: Bit, gt: Bit)
        gates: Gates<58> depth: Depth<20>
    {
        d3 = XOR(a[3], b[3]);
        d2 = XOR(a[2], b[2]);
        d1 = XOR(a[1], b[1]);
        d0 = XOR(a[0], b[0]);
        eq = !(d3 | d2 | d1 | d0);
        gt = d3 if a[3] else (d2 if a[2] else (d1 if a[1] else (d0 if a[0] else 0)));
        return (eq, gt);
    }
    spec FCT.stdlib.comparator4 {
        precondition: true;
        postcondition: eq == ((a - b) == 0) && gt == (a > b);
        invariant: true;
        edge_cases: [a=0,b=0, a=15,b=0, a=8,b=8, a=1,b=15, a=5,b=4, a=4,b=5];
    }"#;
    let (decls, compiled) = compile(src);
    match &compiled[0] {
        gatelang::lower::Compiled::Combinational { netlist, .. } => {
            let st = netlist.stats();
            assert_eq!(st.nand_count, 58, "Comparator4 应为 58 门");
            assert_eq!(st.max_depth, 20, "Comparator4 深度应为 20");
        }
        _ => panic!("应为组合"),
    }
    let rep = verify_all(&decls, &compiled);
    assert!(rep.ok(), "comparator4 spec（含 gt）应穷举通过: {:?}", rep.failed);
}

#[test]
fn verifier_rejects_malformed_or_missing_postcondition() {
    // C1: 不支持的运算符/尾部 token 必须报错，不能静默削弱 postcondition 成恒真。
    assert!(gatelang::spec::parse_spec("y == a ^ b").is_err(), "尾部 token 应报错");
    assert!(gatelang::spec::parse_spec("y == a garbage").is_err(), "多余 token 应报错");
    // 合法的仍应解析成功
    assert!(gatelang::spec::parse_spec("y == a").is_ok());
    assert!(gatelang::spec::parse_spec("sum == (a + b) % 2").is_ok());

    // C2: 没有 postcondition 的 spec 不得计为通过。
    let src = r#"circuit C(a: Bit) -> (y: Bit) { y = a; return y; }
    spec C { precondition: true; invariant: true; }"#;
    let (decls, compiled) = compile(src);
    let rep = verify_all(&decls, &compiled);
    assert!(!rep.ok(), "缺少 postcondition 不应通过: {:?}", rep.failed);
}

#[test]
fn equiv_rejects_empty_domain_and_width_mismatch() {
    // C3: 空约束域不得判定为"等价"（此前 vacuous 返回 true）。
    let src = r#"circuit A(a: Bit) -> (y: Bit) { y = a; return y; }
    circuit B(a: Bit) -> (y: Bit) { y = NOT(a); return y; }"#;
    let (_, c) = compile(src);
    let r = gatelang::equiv::check_equiv_domain(&c[0], &c[1], Some("0"));
    assert!(r.is_err(), "空约束域应报错而非等价: {r:?}");

    // C4: 输出宽度不同不得判定为"等价"（Bit vs Bits<2>）。
    let src2 = r#"circuit A(a: Bit) -> (y: Bit) { y = a; return y; }
    circuit B(a: Bit) -> (y: Bits<2>) { y = [a, a]; return y; }"#;
    let (_, c2) = compile(src2);
    let (ok, _) = gatelang::equiv::check_equiv(&c2[0], &c2[1]).unwrap();
    assert!(!ok, "输出宽度不同不应等价");
}

#[test]
fn malformed_input_errors_not_panics() {
    // H1：门元数错误应返回 Err（而非 args[i] 越界 panic）
    let d = parse_program("circuit C(a: Bit) -> Bit { y = NAND(a); return y; }").unwrap();
    let mut comp = Compiler::new(&d);
    assert!(comp.compile_all().is_err(), "NAND(a) 应返回编译错误");

    // H4：超深嵌套应返回解析错误（而非栈溢出 abort）
    let deep = format!(
        "circuit C(a: Bit) -> Bit {{ y = {}a{}; return y; }}",
        "(".repeat(800),
        ")".repeat(800)
    );
    assert!(parse_program(&deep).is_err(), "超深嵌套应报错");

    // H6：无记忆化内联的组合爆炸应干净报错（而非 OOM/挂死）
    let mut src = String::from("circuit L0(a: Bit) -> (y: Bit) { y = a; return y; }\n");
    for k in 1..20u32 {
        src.push_str(&format!(
            "circuit L{k}(a: Bit) -> (y: Bit) {{ p = L{k1}(a); q = L{k1}(a); y = AND(p,q); return y; }}\n",
            k = k,
            k1 = k - 1
        ));
    }
    let d2 = parse_program(&src).unwrap();
    let mut comp2 = Compiler::new(&d2);
    assert!(comp2.compile_all().is_err(), "组合爆炸应报错");

    // 状态赋值的越界/宽度不匹配应返回 Err 而非 panic
    for bad in [
        "state S { latch x: Bits<4> = 0; fn f() -> Bit { x[0] = []; return 0; } }",
        "state S { latch x: Bits<4> = 0; fn f() -> Bit { x[2..3] = [1,1,1,1]; return 0; } }",
    ] {
        let dd = parse_program(bad).unwrap();
        let mut cc = Compiler::new(&dd);
        assert!(cc.compile_all().is_err(), "应报错: {bad}");
    }

    // 十进制字面量 >8 位应解析报错（不再静默截断）
    assert!(parse_program("circuit A(a: Bits<8>) -> (r: Bits<8>) { r = a + 256; return r; }").is_err());
    // Bits<0> 应被拒绝
    assert!(parse_program("circuit A(a: Bits<0>) -> (r: Bit) { r = a[0]; return r; }").is_err());

    // invariant 必须被验证（此前从不检查）
    let (di, ci) = compile(
        "circuit F(a: Bit) -> (y: Bit) { y = a; return y; }\nspec F { invariant: a == 0; postcondition: 1; }",
    );
    assert!(!verify_all(&di, &ci).ok(), "invariant a==0 应被检出违反");
}

#[test]
fn fct_backend_emits_artifacts() {
    // FCT 后端（v1.4 §3.3）：应输出门级 IR / DSU 描述 / 验证电路 / guest / manifest
    let (_, c) = compile("circuit Adder4(a: Bits<4>, b: Bits<4>) -> (s: Bits<4>) { s = a + b; return s; }");
    let dir = std::env::temp_dir().join(format!("fct_it_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    gatelang::fct::emit(&c, dir.to_str().unwrap(), "inline").expect("fct emit");
    for f in [
        "manifest.json",
        "dsu_descriptor.json",
        "verification_circuits.json",
        "gate_ir/Adder4.json",
        "guest/sp1_main.rs",
        "guest/risczero_main.rs",
    ] {
        assert!(dir.join(f).exists(), "缺少产物 {f}");
    }
    let m = std::fs::read_to_string(dir.join("manifest.json")).unwrap();
    assert!(m.contains("\"networkHash\":\"0x"), "manifest 应含 networkHash");
    assert!(m.contains("Adder4"));
    let ir = std::fs::read_to_string(dir.join("gate_ir/Adder4.json")).unwrap();
    assert!(ir.contains("\"t\":\"nand\"") && ir.contains("\"gates\":60"), "IR 应含 nand 门与门数");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fct_network_hash_covers_latch_init() {
    // 时序电路：latch 初值必须进入身份哈希（否则行为不同的机器产生相同 networkHash）
    let mk = |init: u32| format!("state M {{ latch q: Bit = {init}; fn f() -> Bit {{ return q; }} }}");
    let (_, a) = compile(&mk(0));
    let (_, b) = compile(&mk(1));
    let da = std::env::temp_dir().join(format!("fct_h0_{}", std::process::id()));
    let db = std::env::temp_dir().join(format!("fct_h1_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&da);
    let _ = std::fs::remove_dir_all(&db);
    gatelang::fct::emit(&a, da.to_str().unwrap(), "x").unwrap();
    gatelang::fct::emit(&b, db.to_str().unwrap(), "x").unwrap();
    let ma = std::fs::read_to_string(da.join("manifest.json")).unwrap();
    let mb = std::fs::read_to_string(db.join("manifest.json")).unwrap();
    assert_ne!(ma, mb, "latch 初值不同应产生不同 networkHash");
    let _ = std::fs::remove_dir_all(&da);
    let _ = std::fs::remove_dir_all(&db);
}

#[test]
fn fct_rerun_clears_stale_gate_ir() {
    let dir = std::env::temp_dir().join(format!("fct_stale_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (_, c1) = compile("circuit A1(a: Bit) -> (y: Bit) { y = a; return y; }");
    gatelang::fct::emit(&c1, dir.to_str().unwrap(), "s").unwrap();
    assert!(dir.join("gate_ir/A1.json").exists());
    let (_, c2) = compile("circuit B2(a: Bit) -> (y: Bit) { y = NOT(a); return y; }");
    gatelang::fct::emit(&c2, dir.to_str().unwrap(), "s").unwrap();
    assert!(!dir.join("gate_ir/A1.json").exists(), "旧 IR 应被清理");
    assert!(dir.join("gate_ir/B2.json").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn state_verifier_respects_latch_init_and_trajectory() {
    // 回归：state 验证须以 latch 初值为种子并模拟真实轨迹
    // （此前从 0 开始且不展开 latch 输入 → 任意 postcondition 都“通过”）。
    let bad = r#"state T { latch v: Bit = 1; fn f() -> Bit { v <- NOT(v); return v; } }
    spec T { postcondition: v == 1; }"#;
    let (d, c) = compile(bad);
    assert!(!verify_all(&d, &c).ok(), "init=1 经 3 周期到 0，v==1 应为假");

    let good = r#"state T { latch v: Bit = 1; fn f() -> Bit { v <- NOT(v); return v; } }
    spec T { postcondition: v == 0; }"#;
    let (d2, c2) = compile(good);
    assert!(verify_all(&d2, &c2).ok(), "init=1 经 3 周期到 0，v==0 应为真");
}
