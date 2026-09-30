//! FCT 后端（《FunctionComplete 技术组件白皮书 v1.4》§3.3）。
//!
//! `gatelangc --fct <dir>` 把编译产物导出为 FCT 兼容工件：
//! - **门级函数 IR**：NAND/LATCH 网表 + 资源元数据（门数/深度/周期/LATCH 数）
//! - **DSU 描述文件**：类别 / 参数 / 成本模型 / 预编译地址（v1.4 §4.2）
//! - **验证电路**：状态根验证 / Merkle 证明验证 / 共识验证（接口描述）
//! - **SP1 / RISC Zero guest program**：可选模板
//! - **manifest.json**：工件清单 + 每个门级函数的 networkHash（SHA-256）
//!
//! 零外部依赖（手写 JSON 序列化与 SHA-256）。

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use crate::ast::{LatchDecl, Param, Width};
use crate::lower::Compiled;
use crate::netlist::{Gate, Netlist};
use crate::ty::ResourceBudget;

/// 入口：把已编译产物写入 `out_dir`。
pub fn emit(compiled: &[Compiled], out_dir: &str, source_path: &str) -> Result<(), String> {
    let base = Path::new(out_dir);
    // 清理上一次的 gate_ir/（仅本工具自管子目录），避免残留旧产物与 manifest 不一致
    let _ = fs::remove_dir_all(base.join("gate_ir"));
    fs::create_dir_all(base.join("gate_ir")).map_err(|e| e.to_string())?;
    fs::create_dir_all(base.join("guest")).map_err(|e| e.to_string())?;

    // (name, kind, gates, depth, networkHash)
    let mut funcs: Vec<(String, &'static str, u32, u32, String)> = Vec::new();

    for c in compiled {
        match c {
            Compiled::Combinational { name, inputs, outputs, netlist, budget, .. } => {
                let ir = gate_ir_json(name, inputs, outputs, &[], &[], netlist, budget);
                let h = sha256_hex(ir.as_bytes());
                fs::write(base.join("gate_ir").join(format!("{}.json", safe_name(name))), &ir)
                    .map_err(|e| e.to_string())?;
                let st = netlist.stats();
                funcs.push((name.clone(), "combinational", st.nand_count, st.max_depth, h));
            }
            Compiled::State { name, latches, fns } => {
                for f in fns {
                    let full = format!("{name}.{}", f.name);
                    let ir = gate_ir_json(&full, &f.params, &f.returns, latches, &f.next_latch_sigs, &f.netlist, &f.budget);
                    let h = sha256_hex(ir.as_bytes());
                    fs::write(base.join("gate_ir").join(format!("{}.json", safe_name(&full))), &ir)
                        .map_err(|e| e.to_string())?;
                    let st = f.netlist.stats();
                    funcs.push((full, "sequential", st.nand_count, st.max_depth, h));
                }
            }
        }
    }

    fs::write(base.join("dsu_descriptor.json"), dsu_descriptor_json())
        .map_err(|e| e.to_string())?;
    fs::write(base.join("verification_circuits.json"), verification_circuits_json())
        .map_err(|e| e.to_string())?;
    fs::write(base.join("guest").join("sp1_main.rs"), SP1_GUEST).map_err(|e| e.to_string())?;
    fs::write(base.join("guest").join("risczero_main.rs"), RISCZERO_GUEST)
        .map_err(|e| e.to_string())?;
    fs::write(base.join("manifest.json"), manifest_json(source_path, &funcs))
        .map_err(|e| e.to_string())?;

    println!(
        "FCT 后端产物 -> {out_dir}/  （gate_ir/ {} 个门级函数 + dsu_descriptor + verification_circuits + guest/ + manifest.json）",
        funcs.len()
    );
    Ok(())
}

/* ============================ 门级函数 IR ============================ */

fn gate_ir_json(
    name: &str,
    inputs: &[Param],
    outputs: &[Param],
    latches: &[LatchDecl],
    next_latch: &[(String, Vec<usize>)],
    nl: &Netlist,
    budget: &ResourceBudget,
) -> String {
    let st = nl.stats();
    let mut s = String::new();
    let _ = write!(s, "{{\n  \"kind\": \"gate_function_ir\",\n  \"spec\": \"FCT v1.4 §3.3\",\n  \"name\": \"{}\",\n", esc(name));
    s.push_str("  \"inputs\": [");
    for (i, p) in inputs.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        let _ = write!(s, "{{\"name\":\"{}\",\"width\":{}}}", esc(&p.name), p.width.bits());
    }
    s.push_str("],\n  \"outputs\": [");
    // 输出信号 ID：外部可据以重放/验证（此前只有 name+width，无法定位信号）
    for (i, p) in outputs.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        let _ = write!(s, "{{\"name\":\"{}\",\"width\":{},\"sigs\":[", esc(&p.name), p.width.bits());
        for j in 0..p.width.bits() {
            if j > 0 {
                s.push(',');
            }
            let sig = nl.outputs.get(&format!("{}_{}", p.name, j)).copied().unwrap_or(usize::MAX);
            let _ = write!(s, "{}", sig);
        }
        s.push_str("]}");
    }
    s.push_str("],\n");
    // LATCH 状态（含初值与 next 信号映射）：时序电路的身份与重放都依赖它
    s.push_str("  \"latches\": [");
    for (i, l) in latches.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        let _ = write!(s, "{{\"name\":\"{}\",\"width\":{},\"init\":{},\"next\":[", esc(&l.name), l.width.bits(), l.init);
        if let Some((_, sigs)) = next_latch.iter().find(|(n, _)| n == &l.name) {
            for (j, sig) in sigs.iter().enumerate() {
                if j > 0 {
                    s.push(',');
                }
                let _ = write!(s, "{}", sig);
            }
        } else {
            // 未赋值的 latch 保持（hold）：next = 当前 latch 输入信号，而非空（空会被重放理解为置 0）
            for j in 0..l.width.bits() {
                if j > 0 {
                    s.push(',');
                }
                let sig = nl
                    .inputs
                    .get(&format!("latch_{}_{}", l.name, j))
                    .copied()
                    .unwrap_or(usize::MAX);
                let _ = write!(s, "{}", sig);
            }
        }
        s.push_str("]}");
    }
    s.push_str("],\n");
    let _ = write!(s, "  \"signal_count\": {},\n  \"gates\": [", nl.max_sig);
    let mut first = true;
    for g in &nl.gates {
        if !first {
            s.push(',');
        }
        first = false;
        match g {
            Gate::Input { out, name } => {
                let _ = write!(s, "{{\"t\":\"input\",\"out\":{},\"name\":\"{}\"}}", out, esc(name));
            }
            Gate::Const { out, v } => {
                let _ = write!(s, "{{\"t\":\"const\",\"out\":{},\"v\":{}}}", out, v);
            }
            Gate::Nand { out, l, r } => {
                let _ = write!(s, "{{\"t\":\"nand\",\"out\":{},\"l\":{},\"r\":{}}}", out, l, r);
            }
            Gate::Latch { out, next, name } => {
                let _ = write!(s, "{{\"t\":\"latch\",\"out\":{},\"next\":{},\"name\":\"{}\"}}", out, next, esc(name));
            }
        }
    }
    s.push_str("],\n");
    // LATCH 数取实际 latch 位宽之和（netlist.stats().latch_count 恒为 0：latch 以输入形式表达）
    let latch_bits: u32 = latches.iter().map(|l| l.width.bits()).sum();
    let _ = write!(
        s,
        "  \"metadata\": {{\"gates\":{},\"depth\":{},\"cycles\":{},\"latches\":{}}}\n}}",
        st.nand_count, st.max_depth, budget.cycles, latch_bits
    );
    s
}

/* ============================ DSU 描述文件 ============================ */

fn dsu_descriptor_json() -> String {
    r#"{
  "kind": "dsu_descriptor",
  "spec": "FCT v1.4 §4.2",
  "categories": [
    {"name": "HASH",          "scenarios": "Poseidon / Blake3 / SHA-256", "cost_model": "per-hash + per-byte",          "precompile": "0x0000000000000000000000000000000000000000"},
    {"name": "SIGN",          "scenarios": "EdDSA / BLS / secp256k1",      "cost_model": "per-signature + batch discount", "precompile": "0x0000000000000000000000000000000000000000"},
    {"name": "ARITH",         "scenarios": "finite field / bigint / EC",   "cost_model": "per-field-op",                  "precompile": "0x0000000000000000000000000000000000000000"},
    {"name": "STATE_MACHINE", "scenarios": "branch / loop / jump",         "cost_model": "per-step",                      "precompile": "0x0000000000000000000000000000000000000000"},
    {"name": "ML",            "scenarios": "quantized NN forward pass",    "cost_model": "per-layer FLOPs (quantized)",   "precompile": "0x0000000000000000000000000000000000000000"}
  ],
  "note": "precompile 地址与 params/costModel 由目标链部署方 / L3 DSL 编译产物填入；此处为模板。"
}"#
    .to_string()
}

/* ============================ 验证电路 ============================ */

fn verification_circuits_json() -> String {
    r#"{
  "kind": "verification_circuits",
  "spec": "FCT v1.4 §3.3",
  "circuits": [
    {"name": "state_root_verify",        "purpose": "重算压缩状态承诺根",             "inputs": ["leaf", "index", "siblings[]"],                 "output": "root"},
    {"name": "merkle_inclusion_verify",  "purpose": "二进制 Merkle 包含证明",         "inputs": ["leaf", "index", "siblings[]", "root"],         "output": "ok"},
    {"name": "consensus_verify",         "purpose": "Ethercoin 状态根共识验证 (>2/3)", "inputs": ["root", "signatures[]", "validatorSet"],        "output": "ok"}
  ],
  "note": "链上实现由 FCT 组件（CSC / 清算层）提供；此处为接口描述。"
}"#
    .to_string()
}

/* ============================ guest program（可选） ============================ */

const SP1_GUEST: &str = r#"//! FCT Prover guest program 模板（SP1，可选，v1.4 §3.3）。
//! 用途：将 DSU 组合执行 / 门级函数重放编译为 zkEVM 可证明程序。
//!
//! 用法（示意）：
//!   let mut stdin = sp1_zkvm::io::read::<Input>();
//!   let out = dsu_execute(&stdin);
//!   sp1_zkvm::io::commit(&out);
#![no_main]
sp1_zkvm::entrypoint!(main);

pub fn main() {
    // TODO: 读入输入 → 执行 DSU/门级重放 → commit 输出与状态根
    let input: Vec<u8> = sp1_zkvm::io::read();
    sp1_zkvm::io::commit(&input);
}
"#;

const RISCZERO_GUEST: &str = r#"//! FCT Prover guest program 模板（RISC Zero，可选，v1.4 §3.3）。
//! 用途：将 DSU 组合执行 / 门级函数重放编译为 RISC-V zkVM 可证明程序。
#![no_main]
#![no_std]
risc0_zkvm::guest::entry!(main);

pub fn main() {
    // TODO: 读入输入 → 执行 DSU/门级重放 → commit 输出与状态根
    let input: Vec<u8> = risc0_zkvm::guest::env::read();
    risc0_zkvm::guest::env::commit(&input);
}
"#;

/* ============================ manifest ============================ */

fn manifest_json(source: &str, funcs: &[(String, &'static str, u32, u32, String)]) -> String {
    let mut s = String::new();
    s.push_str("{\n  \"kind\": \"fct_backend_manifest\",\n  \"spec\": \"FCT v1.4 §3.3\",\n");
    let _ = write!(s, "  \"source\": \"{}\",\n", esc(source));
    s.push_str("  \"compiler\": \"gatelangc (gatelang prototype)\",\n");
    s.push_str("  \"artifacts\": [\"gate_ir/*.json\", \"dsu_descriptor.json\", \"verification_circuits.json\", \"guest/sp1_main.rs\", \"guest/risczero_main.rs\"],\n");
    s.push_str("  \"gate_functions\": [\n");
    for (i, (name, kind, g, d, h)) in funcs.iter().enumerate() {
        if i > 0 {
            s.push_str(",\n");
        }
        // networkHash = SHA-256(门级函数 IR)：跨链身份锚（FCT v1.4 §3.4/§8.2）
        let _ = write!(
            s,
            "    {{\"name\":\"{}\",\"kind\":\"{}\",\"gates\":{},\"depth\":{},\"networkHash\":\"0x{}\"}}",
            esc(name), kind, g, d, h
        );
    }
    s.push_str("\n  ]\n}");
    s
}

/* ============================ 工具 ============================ */

/// 产物文件名净化：只保留 `[A-Za-z0-9_.]`，杜绝目录穿越（纵深防御；
/// 语言本身已把标识符限制在该字符集内）。
fn safe_name(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '.' { c } else { '_' })
        .collect()
}

/// JSON 字符串转义。
fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(o, "\\u{:04x}", c as u32);
            }
            c => o.push(c),
        }
    }
    o
}

/// 纯 Rust SHA-256（FIPS 180-4）。用于门级函数 IR 的 networkHash（跨链身份锚）。
pub fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    let bitlen = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bitlen.to_be_bytes());

    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[4 * i], chunk[4 * i + 1], chunk[4 * i + 2], chunk[4 * i + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut out = String::with_capacity(64);
    for x in h {
        let _ = write!(out, "{:08x}", x);
    }
    out
}

/// 便于外部查询宽度描述的辅助。
#[allow(dead_code)]
pub fn width_bits(w: &Width) -> u32 {
    w.bits()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_vectors() {
        // FIPS 180-4 已知向量
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn esc_handles_quotes_and_control() {
        assert_eq!(esc("a\"b\\c\n"), "a\\\"b\\\\c\\n");
    }

    #[test]
    fn dsu_and_verification_jsons_are_wellformed() {
        // 粗略校验：括号配平且包含关键字段
        for s in [dsu_descriptor_json(), verification_circuits_json()] {
            assert!(s.starts_with('{') && s.trim_end().ends_with('}'));
            assert_eq!(s.matches('{').count(), s.matches('}').count());
        }
        assert!(dsu_descriptor_json().contains("\"STATE_MACHINE\""));
        assert!(verification_circuits_json().contains("merkle_inclusion_verify"));
    }
}
