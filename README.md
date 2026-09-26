# GateLang — NAND/LATCH 门级可验证计算语言

M5 原型实现（依据《GateLang 技术白皮书 v2.1》与《软件开发文档 v2.1》）。

```text
源码 ──lexer──▶ Token ──parser──▶ AST ──lower(语义保持展开)──▶ NAND/LATCH 网表
  ──resource──▶ 门数/深度/周期 ──sim──▶ 位级模拟 ──spec──▶ 前后置条件验证
  ──equiv──▶ 语义等价检查
```

## 构建与测试

```bash
cargo build
cargo test          # 19 个测试（12 单元 + 7 集成）
```

## CLI

```bash
# 编译并打印资源（门数/深度/周期）
cargo run --quiet -- examples/adder4.gat

# FCT 后端（《FCT 技术组件白皮书 v1.3》§3.3）：导出门级函数 IR / DSU 描述 / 验证电路 / guest 模板
cargo run --quiet -- examples/stdlib_l1.gat --fct ./fct-out

# spec 验证（穷举输入真值表检查前后置条件）
cargo run --quiet -- examples/adder4_spec.gat --verify

# 等价性检查（两个电路穷举输入对比）
cargo run --quiet -- examples/halfadder.gat --check-equiv HalfAdder5 HalfAdderNaive

# 组合电路模拟
cargo run --quiet -- examples/adder4_spec.gat --sim Adder4 15 1
```

## 示例

| 文件 | 内容 | 亮点 |
|------|------|------|
| `examples/halfadder.gat` | 半加器两实现 | 5 门共享 vs 6 门 naive,`--check-equiv` 证明等价 |
| `examples/adder4.gat` | 半加器 + 4 位加法器 | 60 门、深度 19 |
| `examples/adder4_spec.gat` | 加法器 + spec | `--verify` 穷举 256 输入验证 post 条件 |
| `examples/equiv.gat` | XOR 两实现 | 门级 vs 显式 NAND 展开,等价 + 真值表 spec |
| `examples/domain_equiv.gat` | 约束域等价 | 全域不等价、约束域 `a==1&&b==1` 等价并给反例 |
| `examples/stdlib_l1.gat` | 模板库：全加器 / Mux2 / Comparator4 | 15 / 8 / 58 门，`Gates<>` 上界强制 + spec 穷举（含 gt） |
| `examples/srlatch.gat` | SR Latch + 计数器 | 时序状态与 latch 更新 |

## 语言要点（原型子集）

- **声明**：`circuit`（组合）/ `state`（时序）/ `spec`（规范）
- **类型**：`Bit` / `Bits<N>`，资源注解 `gates: Gates<N> depth: Depth<N> cycles: Cycles<N>`
- **原语**：`NAND` 展开 —— NOT=1、AND=2、OR=3、XOR=4 门
- **表达式**：字面量、变量、位索引 `x[i]`、切片 `x[a..b]`、拼接 `[a,b]`、门调用、算术 `+`、比较、三元
- **spec**：`precondition` / `postcondition` / `invariant` / `edge_cases`，内建求值器（`+ - * %` 比较 `&& || !`、`2^N`、`MAX_UINT`）
- **门数与白皮书叙事一致**：半加器共享实现 = 5 个 NAND 门（v2.1 §4.2）

## 模块（单 crate 多模块）

| 模块 | 职责 |
|------|------|
| `lexer.rs` | Token 化 |
| `parser.rs` | 递归下降解析 → AST |
| `ast.rs` | AST / 类型 / 资源声明 |
| `ty.rs` | 类型系统（Bit/Bits） |
| `netlist.rs` | NAND/LATCH 网表 IR + 门原语库 |
| `lower.rs` | 语义保持编译（结构内联展开 / latch 状态） |
| `resource.rs` | 门数/深度/周期统计与校验 |
| `sim.rs` | 位级模拟器（组合 + 时序多周期） |
| `spec.rs` | 规范表达式解析与求值 |
| `verify.rs` | spec 验证编排 |
| `equiv.rs` | 穷举等价性检查 |
| `main.rs` | CLI |

## 参考

- GateLang 技术白皮书 v2.1（本目录）
- GateLang 软件开发文档 v2.1（本目录）
- FCT 双原语开发计划 §5.2/5.3（DSU 运行时 / 共享层）