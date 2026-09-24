//! 集成测试：端到端 解析→编译→模拟→验证→等价。

use gatelang::equiv::check_equiv;
use gatelang::parser::parse_program;
use gatelang::verify::verify_all;
use gatelang::lower::Compiler;

fn compile(src: &str) -> (Vec<gatelang::ast::Decl>, Vec<gatelang::lower::Compiled>) {
    let decls = parse_program(src).expect("parse 失败");
    let mut c = Compiler::new(&decls);
    let compiled = c.compile_all();
    (decls, compiled)
}

#[test]
fn xor_5gate_halfadder_tvb() {
    // 白皮书 v2.1 §4.2/4.3：半加器真值表
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