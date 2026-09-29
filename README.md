# GateLang — NAND/LATCH 门级可验证计算语言

[![tests](https://img.shields.io/badge/tests-91%20passing-brightgreen)](tests/integration.rs)
[![rust](https://img.shields.io/badge/rust-1.96-orange)](https://www.rust-lang.org/)
[![dependencies](https://img.shields.io/badge/dependencies-0-blue)](#构建与测试)
[![formal](https://img.shields.io/badge/verification-SAT%20%2F%20UNSAT-purple)](#形式化验证sat-后端)

M5 原型实现（依据《GateLang 技术白皮书 v2.1》与《软件开发文档 v2.1》）。

```text
源码 ──lexer──▶ Token ──parser──▶ AST ──lower(语义保持展开)──▶ NAND/LATCH 网表
  ──resource──▶ 门数/深度/周期 ──sim──▶ 位级模拟 ──spec──▶ 前后置条件验证
  ──equiv──▶ 语义等价检查
  ──cnf(Tseitin)──▶ CNF ──sat(CDCL)──▶ UNSAT=已证明 / SAT=反例   ← 形式化验证
```

## 构建与测试

```bash
cargo build
cargo test          # 91 个测试（45 单元 + 14 集成 + 22 形式化证明 + 5 ERC-20 + 5 词级重写）
                    # ERC-20 的 uint256 慢证明用: cargo test --release -- --ignored
```

## 形式化验证（SAT 后端）

`--verify` 是**穷举仿真**（输入位宽 ≤ 20，即至多 2^20 次模拟）。
`--prove` 是**形式化证明**：规格被综合为门级电路，取反后交给自研 CDCL 求解器。

| 结果 | 含义 |
|------|------|
| `UNSAT` | **对全部输入成立**（不是"测了很多没发现问题"） |
| `SAT` | 反例：具体输入向量，可复现 |
| `Unknown` | 触及资源上限，**不冒充结论**（fail-closed） |

```bash
# 形式化证明 spec（全输入成立，输入位宽无上限）
cargo run --quiet -- examples/adder4_spec.gat --prove

# 形式化等价证明（miter + SAT）
cargo run --quiet -- examples/halfadder.gat --prove-equiv HalfAdder5 HalfAdderNaive
```

### 可信度从哪来

不是"我们相信 SAT 求解器"，而是**两条独立实现路径给出同一结论**：

1. **CNF 编码定义性正确** —— `cnf.rs` 有穷举一致性测试：对每个输入组合，
   网表求值结果必须满足全部子句，反之亦然（编码的模型集合 == 网表语义）。
2. **CDCL 求解器正确** —— `sat.rs` 对 300 组随机 3-SAT + 200 组混合宽度 CNF
   与**穷举参考求解器**逐一比对；另有鸽巢原理 UNSAT 压力用例。
3. **综合语义保真** —— 规格按 `u128` 回绕语义综合，与 `spec::eval_spec` 逐位一致；
   `tests/prove.rs` 把形式化结论与经 18 轮独立审计的穷举验证器
   （`verify.rs` / `equiv.rs`）在同一批语料上**逐条交叉验证**。
4. **反例可靠** —— 被驳倒的规格，其反例会被重新模拟，确认在语义上确实违反。

已知**原型上限**（全部 fail-closed，报错而非给出错误结论）：非常量除数的取模、
超宽乘法（非二次幂/非常量路径）、综合门数预算 400k。

## 已验证割点（`cut:`）—— 引理组合层

`spec` 可声明一条**中间断言**，工具会分两阶段判定：

```gat
spec F {
    precondition: true;
    cut: l <= 15;                       // 阶段 A：先证明 cut 在 pre 下恒真
    postcondition: y == (l + r) % (2^4); // 阶段 B：在 pre ∧ cut 下证明目标
    invariant: true;
}
```

- **可靠性**：只有阶段 A 成立，把 `cut` 当作假设加入阶段 B 才是可靠的。
  若割点被驳倒，报告**整体不得**判定为已证明 ——
  `tests/prove.rs::invalid_cut_cannot_produce_a_false_proof` 专门锁定这一点。
- 割点以独立义务（`kind == "cut"`）出现在报告中，`all_proven()` 要求它也被证明。

**实测的局限（诚实记录）**：割点是"断言"，不是"重写"。
实测它**无法**解决多项求和的**加法结合律/同余**问题
（`(b0-a)+(b1+a)+b2+b3` 与 `(b0+b1+b2+b3)-a`）——
因为 SAT 没有**同余闭包**，而加法结合律对 resolution 是指数难的（文献已知结果）。
真正的解法是**项重写**（让两侧共享信号），不是加断言 —— 见下节。

## 词级重写层（`word.rs`）—— 摆脱加法结合律的指数边界

SAT 后端把一切 bit-blast 成 CNF，于是多项求和的**重结合**必须由 resolution
自行发现 —— 那是**指数难**的。实测扩展性（N=4 账户不变量保持）：

| 位宽 | 纯 SAT | 词级重写层 |
|---|---|---|
| `Bits<8>` | 4 s | **0.006 s** |
| `Bits<12>` | >120 s 未完成 | **瞬间** |
| `Bits<32>` | >280 s 未完成 | **0.2 s**（正确版证明 + 漏洞版反例） |

词级层在 **bit-blast 之前**把电路与规格符号求值成**规范形**
（`Σ cᵢ·atomᵢ + k (mod 2ʷ)`，系数按位宽取模），于是 `(b0-a)+(b1+a)` 与 `b0+b1`
规范化为**同一个形**，比较式随之同形，`AND(inv, NOT(inv))` 被直接判为 `0`。

**可靠性纪律（关键）** —— 规范形只做**保守、单向**判定：

* 规范形 == `Const` ⇒ 该值确实处处相等（可靠）
* 两侧规范形**结构相同** ⇒ 二者确实相等（可靠）
* 规范形**不同** ⇒ **不作结论**，回落 SAT（反例仍由 SAT 给出）

**绝不用「规范形不同」去证明不等**（原子被当作独立变元，那样不可靠）。
另有两道保险：位宽 >128 不判定（u128 表示不了 `mod 2ʷ` 的系数）；任何无法
精确处理的构造（用户电路内联、位索引/切片/拼接、`if` 分支、规格层乘模）
一律返回 `None` 回落 SAT。`tests/word.rs` 用「**有词级层 vs 无词级层（纯 SAT）**」
在同一批语料上逐条对照 —— **词级层绝不改变结论**，只做加速。

## CLI

```bash
# 编译并打印资源（门数/深度/周期）
cargo run --quiet -- examples/adder4.gat

# FCT 后端（《FCT 技术组件白皮书 v1.3》§3.3）：导出门级函数 IR / DSU 描述 / 验证电路 / guest 模板
cargo run --quiet -- examples/stdlib_l1.gat --fct ./fct-out

# spec 验证（穷举输入真值表检查前后置条件）
cargo run --quiet -- examples/adder4_spec.gat --verify

# spec 形式化证明（SAT 后端，对全部输入成立）
cargo run --quiet -- examples/adder4_spec.gat --prove

# 等价性检查（无 --domain 时走 SAT/miter，输入位宽无上限；带 --domain 时穷举）
cargo run --quiet -- examples/halfadder.gat --check-equiv HalfAdder5 HalfAdderNaive
cargo run --quiet -- examples/domain_equiv.gat --check-equiv A B --domain "a==1 && b==1"

# 形式化等价证明（miter + SAT）
cargo run --quiet -- examples/halfadder.gat --prove-equiv HalfAdder5 HalfAdderNaive

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
| `examples/erc20_core.gat` | **ERC-20 纯计算核心（Bits<16>）** | checked add/sub + 余额守恒的形式化证明；含刻意漏洞版本，反例精确命中下溢/溢出 |
| `examples/erc20_uint256.gat` | **ERC-20 算术核心（真实 uint256）** | checked add/sub 的独立判据等价性；余额守恒；含漏洞版本被驳倒 |
| `examples/erc20_invariant.gat` | **全局不变量 Σbalances==totalSupply（uint256）** | 有界地址域（N=2）下的归纳步；含慢证明（约 9 分钟） |
| `examples/erc20_invariant_small.gat` | 同上，N=4 @ Bits<8> | 说明账户数本身不是障碍 |
| `examples/erc20_invariant_n4_32.gat` | **N=4 @ Bits<32> 不变量保持** | 词级重写层对照实验：纯 SAT >280s 无结论，词级 0.2s；含漏洞版反例 |

## 语言要点（原型子集）

- **声明**：`circuit`（组合）/ `state`（时序）/ `spec`（规范）
- **类型**：`Bit` / `Bits<N>`（N ≤ 256，支持真实 `uint256`），资源注解 `gates: Gates<N> depth: Depth<N> cycles: Cycles<N>`
  - 注意：`sim.rs` / `verify.rs` / `equiv.rs` 内部用 `u128` 表示端口值，位宽 > 128 的电路**不能被模拟或穷举**，只能用 `--prove` 做形式化验证
- **原语**：`NAND` 展开 —— NOT=1、AND=2、OR=3、XOR=4 门
- **表达式**：字面量、变量、位索引 `x[i]`、切片 `x[a..b]`、拼接 `[a,b]`、门调用、算术 `+ -`、按位 `& | ^`、比较 `== != < > <= >=`、三元 `c if x else y`
  - 比较运算符**同级左结合**：`a > b == c` 解析为 `((a > b) == c)`，需要 `(a > b) == c` 时请显式加括号
- **spec**：`precondition` / `postcondition` / `invariant` / `edge_cases`，内建求值器（`+ - * %` 比较 `&& || !`、`2^N`、`MAX_UINT`）
  - spec 算术是 **u128 回绕语义**（不是端口位宽模运算）：`Bits<4>` 的 `a + b + cin >= 2` 才能拿到进位
  - spec **没有位运算** `&` / `|` / `^`（只有 `&&` / `||`）；位运算请放进电路
  - `cut:` 是**已验证割点**（见下）
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
| `verify.rs` | spec 验证编排（穷举，≤20 位输入） |
| `equiv.rs` | 穷举等价性检查（含约束域） |
| `cnf.rs` | CNF 表示 + NAND 网表 Tseitin 编码 |
| `sat.rs` | 自研 CDCL SAT 求解器（零依赖） |
| `prove.rs` | **形式化证明**：spec 门级综合 + 反例提取 + miter 等价 |
| `word.rs` | **词级重写层**：bit-blast 前的位向量规范形（加法重结合/同余），判不了则回落 SAT |
| `main.rs` | CLI |

## 形式化验证的实现要点（`prove.rs`）

规格表达式被综合为门级电路，与目标电路拼进同一网表，取反后交给 CDCL：
`pre ∧ ¬goal` 为 UNSAT ⟺ 在全部满足前置条件的输入上 `goal` 恒真。

- **语义保真**：端口值零扩展进 128 位空间运算，与 `spec::eval_spec` 的 `u128`
  回绕语义逐位一致；`&&`/`||`/`!` 先把操作数归约为 0/1。
- **廉价综合路径**：`x * bit` → mux，`x * const` → 移位累加，
  `x % 2^k` → 取低 k 位（因此 `mux2` 的 `a*sel + b*(1-sel)` 是线性的）。
- **比较器必须单条进位链**：`a < b` 用 `a + ~b + 1` 的进位判定；
  若拆成两次加法会丢掉进位，导致 `lt(3,2)` 误判为真（已修复并有回归测试）。
- **fail-closed**：非常量除数的取模、超出预算的乘法等一律**报错**，
  绝不返回一个可能错误的"证明"。

## 参考

- GateLang 技术白皮书 v2.1（本目录）
- GateLang 软件开发文档 v2.1（本目录）
- FCT 双原语开发计划 §5.2/5.3（DSU 运行时 / 共享层）