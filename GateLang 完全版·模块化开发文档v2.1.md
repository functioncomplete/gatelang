# GateLang 技术开发文档 v2.1（完全版·模块化开发）

**项目**：GateLang  
**版本**：v2.1  
**发布方**：FunctionComplete  
**日期**：2026年9月  
**文档类型**：技术开发工程文档  
**开发模式**：模块化、分层解耦、可独立测试与集成  
**更新内容**：新增 0/1 二进制基础对编译器实现的影响、AI 辅助开发模块（`gatelang-ai`）、AI 生成代码的验证流程、更新后的模块列表与路线图

## 1. 概述

GateLang 是一门以 NAND 门为唯一组合原语、以 LATCH 为唯一状态原语的门级可验证计算语言。v2.1 在 v2.0 四层抽象模型（L1 门级、L2 高级语言、L3 领域 DSL、L4 可视化）基础上，新增两大技术方向：

- **0/1 二进制基础**：明确 NAND 与 LATCH 如何从最基础的 0/1 构建，并将这一认知贯穿到编译器的词法、语法、类型与 IR 设计中。

- **AI 辅助开发**：引入 `gatelang-ai` 模块，定义 AI 在代码生成、优化、形式化规范编写和教学辅助中的角色，并建立 AI 生成代码的强制验证流程。

本文档定义 GateLang v2.1 的**模块化开发架构**，包括模块划分、接口契约、依赖关系、开发顺序、测试策略、构建发布与协作规范。

## 2. 总体架构

GateLang 采用**编译器三段式 + 工具链周边 + 标准库 + AI 辅助层**的模块化架构。

```
┌─────────────────────────────────────────────────────────────────────┐  
│                          用户界面层                                   │  
│  gatelangc (CLI)  ·  gatecanvas (可视化)  ·  IDE 插件 (规划中)        │  
└───────────────────────────────┬─────────────────────────────────────┘  
                                │  
┌───────────────────────────────▼─────────────────────────────────────┐  
│                          AI 辅助层                                    │  
│  gatelang-ai (代码生成 · 优化 · 规范生成 · 教学辅助)                  │  
└───────────────────────────────┬─────────────────────────────────────┘  
                                │  
┌───────────────────────────────▼─────────────────────────────────────┐  
│                          编译器前端                                   │  
│  gatelang-lexer → gatelang-parser → gatelang-ast → gatelang-types    │  
│                                      ↓                               │  
│                              gatelang-resource                        │  
│                                      ↓                               │  
│                               gatelang-spec                           │  
└───────────────────────────────┬─────────────────────────────────────┘  
                                │  
┌───────────────────────────────▼─────────────────────────────────────┐  
│                          编译器中端                                   │  
│  gatelang-lower (L4→L3→L2→L1) → gatelang-ir (NAND/LATCH 网表)        │  
└───────────────────────────────┬─────────────────────────────────────┘  
                                │  
┌───────────────────────────────▼─────────────────────────────────────┐  
│                          编译器后端                                   │  
│  backend-sim · backend-verilog · backend-zk · backend-tee · backend-c │  
└───────────────────────────────┬─────────────────────────────────────┘  
                                │  
┌───────────────────────────────▼─────────────────────────────────────┐  
│                          运行时与工具                                 │  
│  gatesim · gateproof · gatepm · gatecanvas                            │  
└───────────────────────────────┬─────────────────────────────────────┘  
                                │  
┌───────────────────────────────▼─────────────────────────────────────┐  
│                          标准库                                       │  
│  stdlib-L1 · stdlib-L2 · stdlib-L3 · 领域 DSL 库 · 验证证明库         │  
└─────────────────────────────────────────────────────────────────────┘
```

## 3. 模块划分

所有模块以 Rust crate 形式实现，通过 Cargo workspace 管理。每个 crate 独立版本化、独立测试、独立发布。

### 3.1 核心编译器模块

| 模块 | Crate 名称 | 职责 | 依赖 |
| - | - | - | - |
| 词法分析 | `gatelang-lexer` | 将源码转为 Token 流，支持 L1-L4 语法，识别 0/1 字面量 | 无 |
| 语法分析 | `gatelang-parser` | 将 Token 流转为统一 AST | `gatelang-lexer`、`gatelang-ast` |
| AST 定义 | `gatelang-ast` | 定义所有层的 AST 节点，含 `Bit`/`Bits\<N\>` 字面量 | 无 |
| 类型系统 | `gatelang-types` | 类型检查、状态分离验证、0/1 位宽推导 | `gatelang-ast` |
| 资源计算 | `gatelang-resource` | 门数、深度、周期数静态计算 | `gatelang-ast`、`gatelang-types` |
| 规范验证 | `gatelang-spec` | 解析 `spec` 块，调用 Z3/NuSMV | `gatelang-ast`、`gatelang-types` |
| 分层展开 | `gatelang-lower` | L4→L3→L2→L1 逐层展开 | `gatelang-ast`、`gatelang-types` |
| 中间表示 | `gatelang-ir` | NAND/LATCH 网表数据结构，0/1 信号线表示 | 无 |
| 编译驱动 | `gatelang-driver` | 串联前端、中端、后端 | 所有核心模块 |


### 3.2 AI 辅助模块（v2.1 新增）

| 模块 | Crate 名称 | 职责 | 依赖 |
| - | - | - | - |
| AI 代码生成 | `gatelang-ai-gen` | 从自然语言/伪代码生成候选 GateLang 代码 | `gatelang-driver`、外部 LLM API |
| AI 门级优化 | `gatelang-ai-opt` | 分析 NAND 网表，提出门数/深度优化方案 | `gatelang-ir`、`gatelang-driver` |
| AI 规范生成 | `gatelang-ai-spec` | 从函数签名和行为描述生成 `spec` 块 | `gatelang-ast`、外部 LLM API |
| AI 教学助手 | `gatelang-ai-tutor` | 可视化层教学辅助、错误解释、练习题生成 | `gatelang-ast`、`gatesim` |
| AI 验证网关 | `gatelang-ai-gate` | 强制 AI 生成代码经过类型检查、资源验证、形式化验证 | `gatelang-driver`、`gateproof` |


### 3.3 编译器后端模块

| 模块 | Crate 名称 | 职责 | 依赖 |
| - | - | - | - |
| 模拟器后端 | `gatelang-backend-sim` | 生成可执行模拟代码 | `gatelang-ir` |
| Verilog 后端 | `gatelang-backend-verilog` | 生成 Verilog/VHDL | `gatelang-ir` |
| ZK 后端 | `gatelang-backend-zk` | 生成 Circom/Noir | `gatelang-ir` |
| TEE 后端 | `gatelang-backend-tee` | 生成 TEE 可执行代码 | `gatelang-ir` |
| C 后端 | `gatelang-backend-c` | 生成嵌入式 C 代码 | `gatelang-ir` |


### 3.4 工具链模块

| 模块 | Crate 名称 | 职责 | 依赖 |
| - | - | - | - |
| CLI | `gatelang-cli` | 命令行入口 `gatelangc` | `gatelang-driver`、所有后端 |
| 模拟器 | `gatesim` | 门级仿真、波形输出 | `gatelang-ir` |
| 形式化验证 | `gateproof` | 集成 Z3/NuSMV，验证规范 | `gatelang-spec`、`gatelang-ir` |
| 包管理器 | `gatepm` | 函数库发布、依赖解析 | `gatelang-driver` |
| 可视化编辑器 | `gatecanvas` | 拖拽画布、实时仿真、代码生成 | `gatesim`、`gatelang-driver`、`gatelang-ai-tutor` |


### 3.5 标准库模块

| 模块 | 类型 | 内容 |
| - | - | - |
| `stdlib-L1` | GateLang 源码 | 半加器、全加器、4位加法器、MUX、基本逻辑门 |
| `stdlib-L2` | GateLang 源码 | 安全算术、类型转换、错误处理 |
| `stdlib-L3` | GateLang 源码 | 金融 DSL、AI DSL、游戏 DSL 基础库 |
| `stdlib-verify` | 证明文件 | 每个标准库函数的形式化验证证明 |


## 4. 0/1 二进制基础对编译器实现的影响（v2.1 新增）

### 4.1 词法层

`gatelang-lexer` 需识别 0/1 字面量及其扩展形式：

| 字面量 | 含义 | 示例 |
| - | - | - |
| `0`、`1` | 单位 `Bit` | `latch q: Bit = 0` |
| `0b0101` | 二进制 `Bits\<N\>` | `latch value: Bits\<4\> = 0b0101` |
| `0x1F` | 十六进制 `Bits\<N\>` | `latch mask: Bits\<8\> = 0xFF` |


### 4.2 语法层

`gatelang-parser` 需支持位宽推导语法：

```
latch q: Bit = 0  
latch value: Bits\<8\> = 0  
latch pattern: Bits\<4\> = 0b1010
```

以及 NAND 和 LATCH 的原语语法：

```
circuit NAND(a: Bit, b: Bit) -\> Bit \{  
    return NOT(AND(a, b))  
\}  
  
state SRLatch \{  
    latch q: Bit = 0  
    latch qbar: Bit = 1  
    fn update(s: Bit, r: Bit) -\> (Bit, Bit) \{  
        next\_q = NAND(s, qbar)  
        next\_qbar = NAND(r, q)  
        q \<- next\_q  
        qbar \<- next\_qbar  
        return (next\_q, next\_qbar)  
    \}  
\}
```

### 4.3 类型层

`gatelang-types` 需实现位宽推导与 0/1 常量折叠：

- `Bit` 是 `Bits\<1\>` 的别名。

- `Bits\<N\>` 的常量在编译期折叠为 0/1 序列。

- 位宽不匹配时编译错误。

### 4.4 IR 层

`gatelang-ir` 的网表以 0/1 信号线为基础：

```
pub struct Signal \{  
    pub id: SignalId,  
    pub width: usize,        // 位宽，1 表示 Bit  
    pub kind: SignalKind,    // Input / Output / Internal / Latch  
\}  
  
pub struct NandGate \{  
    pub input\_a: SignalId,  
    pub input\_b: SignalId,  
    pub output: SignalId,  
\}  
  
pub struct Latch \{  
    pub data\_in: SignalId,  
    pub data\_out: SignalId,  
    pub enable: SignalId,  
    pub initial\_value: BitVector,  // 0/1 序列  
\}
```

### 4.5 模拟器层

`gatesim` 以 0/1 向量表示信号状态，逐周期仿真：

```
pub struct SimulationState \{  
    pub signals: Vec\<BitVector\>,   // 每条线一个 0/1 向量  
    pub cycle: u64,  
\}
```

### 4.6 验证层

`gateproof` 对 NAND/LATCH 网表进行形式化验证时，将所有信号建模为布尔变量，0/1 是其唯一取值域。SMT 求解器（Z3）天然支持布尔逻辑，因此验证是精确的。

## 5. AI 辅助开发模块详细设计（v2.1 新增）

### 5.1 AI 代码生成（`gatelang-ai-gen`）

**输入**：自然语言描述或伪代码  
**输出**：候选 GateLang 代码（L1/L2/L3）  
**流程**：

```
用户描述 → LLM 生成候选代码 → gatelang-ai-gate 验证 → 通过/拒绝
```

**接口**：

```
pub trait AiCodeGenerator \{  
    fn generate(&self, description: &str, target\_layer: Layer) -\> Result\<Vec\<CandidateCode\>, AiError\>;  
\}  
  
pub struct CandidateCode \{  
    pub source: String,  
    pub confidence: f32,  
    pub reasoning: String,  
\}
```

### 5.2 AI 门级优化（`gatelang-ai-opt`）

**输入**：NAND/LATCH 网表  
**输出**：优化后的网表 + 等价性证明  
**流程**：

```
原始网表 → AI 提出优化方案 → 生成新网表 → 形式化等价性检查 → 接受/拒绝
```

**接口**：

```
pub trait AiOptimizer \{  
    fn optimize(&self, ir: &NandLatchNetlist) -\> Result\<OptimizationResult, AiError\>;  
\}  
  
pub struct OptimizationResult \{  
    pub optimized\_ir: NandLatchNetlist,  
    pub gates\_saved: usize,  
    pub depth\_reduced: usize,  
    pub equivalence\_proof: Proof,  
\}
```

### 5.3 AI 规范生成（`gatelang-ai-spec`）

**输入**：函数签名 + 行为描述  
**输出**：`spec` 块  
**流程**：

```
函数签名 + 描述 → LLM 生成 spec → gatelang-spec 解析 → gateproof 验证
```

### 5.4 AI 教学助手（`gatelang-ai-tutor`）

**输入**：用户操作、电路状态、错误信息  
**输出**：解释、建议、练习题  
**集成**：`gatecanvas` 可视化层

### 5.5 AI 验证网关（`gatelang-ai-gate`）

**核心原则**：**AI 生成的一切代码必须经过强制验证，才能被接受。**

| 验证步骤 | 工具 | 失败处理 |
| - | - | - |
| 语法检查 | `gatelang-parser` | 拒绝，返回错误 |
| 类型检查 | `gatelang-types` | 拒绝，返回错误 |
| 资源验证 | `gatelang-resource` | 拒绝，返回错误 |
| 规范验证 | `gateproof` | 拒绝，返回反例 |
| 等价性检查（优化时） | `gateproof` | 拒绝，返回差异 |


**接口**：

```
pub trait AiGate \{  
    fn validate(&self, candidate: &CandidateCode) -\> Result\<ValidatedCode, GateError\>;  
\}  
  
pub struct ValidatedCode \{  
    pub source: String,  
    pub resources: ResourceReport,  
    pub proof: Option\<Proof\>,  
\}
```

## 6. 模块接口契约

### 6.1 编译驱动接口

```
pub trait CompilerDriver \{  
    fn compile(&self, source: &str, target: Backend) -\> Result\<CompilationResult, CompileError\>;  
    fn compile\_project(&self, project: &Project, target: Backend) -\> Result\<CompilationResult, CompileError\>;  
\}  
  
pub struct CompilationResult \{  
    pub ir: NandLatchNetlist,  
    pub resources: ResourceReport,  
    pub proofs: Vec\<Proof\>,  
    pub warnings: Vec\<Warning\>,  
\}
```

### 6.2 分层展开接口

```
pub trait LoweringPass \{  
    fn lower(&self, ast: &Ast) -\> Result\<Ast, LoweringError\>;  
\}  
  
pub struct L4ToL3Lowering;  
pub struct L3ToL2Lowering;  
pub struct L2ToL1Lowering;  
pub struct L1ToIRLowering;
```

### 6.3 后端接口

```
pub trait Backend \{  
    fn name(&self) -\> &str;  
    fn generate(&self, ir: &NandLatchNetlist) -\> Result\<TargetCode, BackendError\>;  
\}
```

### 6.4 形式化验证接口

```
pub trait SpecVerifier \{  
    fn verify(&self, spec: &Spec, ir: &NandLatchNetlist) -\> VerificationResult;  
\}  
  
pub enum VerificationResult \{  
    Passed \{ proof\_hash: String \},  
    Failed \{ counterexample: String \},  
    Unknown \{ reason: String \},  
\}
```

### 6.5 AI 辅助接口（v2.1 新增）

```
pub trait AiAssistant \{  
    fn generate\_code(&self, description: &str, layer: Layer) -\> Result\<ValidatedCode, AiError\>;  
    fn optimize(&self, ir: &NandLatchNetlist) -\> Result\<OptimizationResult, AiError\>;  
    fn generate\_spec(&self, signature: &Signature, description: &str) -\> Result\<Spec, AiError\>;  
    fn explain(&self, ir: &NandLatchNetlist, signal: SignalId) -\> Result\<String, AiError\>;  
\}
```

## 7. 依赖关系与开发顺序

### 7.1 依赖图

```
gatelang-lexer  
    ↓  
gatelang-parser → gatelang-ast  
    ↓               ↓  
gatelang-types ←────┘  
    ↓  
gatelang-resource  
    ↓  
gatelang-spec  
    ↓  
gatelang-lower  
    ↓  
gatelang-ir  
    ↓  
┌───────────────┬───────────────┬───────────────┬───────────────┐  
│ backend-sim   │ backend-verilog│ backend-zk    │ backend-tee   │ backend-c  
└───────────────┴───────────────┴───────────────┴───────────────┘  
    ↓  
gatelang-driver  
    ↓  
┌───────────────┬───────────────┬───────────────┬───────────────┐  
│ gatelang-ai-gen│ gatelang-ai-opt│ gatelang-ai-spec│ gatelang-ai-tutor  
└───────────────┴───────────────┴───────────────┴───────────────┘  
    ↓  
gatelang-ai-gate  
    ↓  
gatelang-cli / gatesim / gateproof / gatepm / gatecanvas
```

### 7.2 开发顺序

| 阶段 | 开发模块 | 可并行 |
| - | - | - |
| 第 1 阶段 | `gatelang-lexer`、`gatelang-ast`、`gatelang-parser` | 是 |
| 第 2 阶段 | `gatelang-types`、`gatelang-resource` | 是 |
| 第 3 阶段 | `gatelang-spec`、`gatelang-lower` | 是 |
| 第 4 阶段 | `gatelang-ir`、`gatelang-backend-sim` | 是 |
| 第 5 阶段 | `gatelang-driver`、`gatelang-cli` | 否 |
| 第 6 阶段 | `gatesim`、`gateproof` | 是 |
| 第 7 阶段 | `gatelang-backend-verilog`、`stdlib-L1` | 是 |
| 第 8 阶段 | `gatelang-backend-zk`、`stdlib-L2` | 是 |
| 第 9 阶段 | `gatelang-backend-tee`、`gatepm` | 是 |
| 第 10 阶段 | `gatecanvas`、`stdlib-L3` | 是 |
| 第 11 阶段 | `gatelang-ai-gen`、`gatelang-ai-spec` | 是 |
| 第 12 阶段 | `gatelang-ai-opt`、`gatelang-ai-tutor` | 是 |
| 第 13 阶段 | `gatelang-ai-gate`（集成） | 否 |


## 8. 模块化测试策略

| 模块 | 单元测试 | 集成测试 | 属性测试 | 形式化验证 |
| - | - | - | - | - |
| `gatelang-lexer` | Token 流正确性、0/1 字面量 | 与 parser 集成 | — | — |
| `gatelang-parser` | AST 结构 | 与 types 集成 | — | — |
| `gatelang-types` | 类型检查规则、位宽推导 | 与 resource 集成 | 类型系统不变式 | — |
| `gatelang-resource` | 门数/深度计算 | 与 lower 集成 | 资源上界准确性 | — |
| `gatelang-spec` | 规范解析 | 与 gateproof 集成 | — | Z3 验证 |
| `gatelang-lower` | 逐层展开 | 与 IR 集成 | 分层一致性 | 等价性检查 |
| `gatelang-ir` | 网表结构、0/1 信号 | 与后端集成 | — | — |
| `backend-\*` | 代码生成 | 端到端编译 | — | — |
| `gatesim` | 仿真正确性 | 示例电路 | — | — |
| `gateproof` | 验证器 | 标准库证明 | — | — |
| `gatelang-ai-gen` | 生成接口 | 与 ai-gate 集成 | 生成代码类型正确率 | — |
| `gatelang-ai-opt` | 优化接口 | 与等价性检查集成 | 优化后资源改善 | **等价性检查** |
| `gatelang-ai-spec` | 规范生成接口 | 与 gateproof 集成 | — | Z3 验证 |
| `gatelang-ai-tutor` | 教学接口 | 与 gatecanvas 集成 | — | — |
| `gatelang-ai-gate` | 验证流程 | 全链路集成 | — | 强制验证 |
| `gatecanvas` | UI 组件 | 完整工作流 | — | — |


**AI 模块的特殊测试要求**：

- AI 生成的每一份代码必须经过 `gatelang-ai-gate` 的完整验证。

- AI 优化后的网表必须通过形式化等价性检查。

- AI 生成的 `spec` 必须通过 `gateproof` 验证。

- 测试集包含对抗性输入，验证 AI 不会绕过验证流程。

## 9. 构建与发布

### 9.1 Cargo Workspace 结构

```
gatelang/  
├── Cargo.toml (workspace)  
├── crates/  
│   ├── gatelang-lexer/  
│   ├── gatelang-parser/  
│   ├── gatelang-ast/  
│   ├── gatelang-types/  
│   ├── gatelang-resource/  
│   ├── gatelang-spec/  
│   ├── gatelang-lower/  
│   ├── gatelang-ir/  
│   ├── gatelang-driver/  
│   ├── gatelang-backend-sim/  
│   ├── gatelang-backend-verilog/  
│   ├── gatelang-backend-zk/  
│   ├── gatelang-backend-tee/  
│   ├── gatelang-backend-c/  
│   ├── gatelang-ai-gen/  
│   ├── gatelang-ai-opt/  
│   ├── gatelang-ai-spec/  
│   ├── gatelang-ai-tutor/  
│   ├── gatelang-ai-gate/  
│   ├── gatelang-cli/  
│   ├── gatesim/  
│   ├── gateproof/  
│   ├── gatepm/  
│   └── gatecanvas/  
├── stdlib/  
│   ├── L1/  
│   ├── L2/  
│   ├── L3/  
│   └── proofs/  
├── examples/  
├── docs/  
└── tests/
```

### 9.2 构建命令

| 命令 | 作用 |
| - | - |
| `cargo build --release` | 构建所有模块 |
| `cargo test --workspace` | 运行全工作区测试 |
| `cargo clippy` | 静态检查 |
| `cargo fmt` | 格式化 |
| `mdbook build` | 构建文档 |


### 9.3 发布流程

```
Push / PR  
    ↓  
GitHub Actions  
    ├── cargo fmt --check  
    ├── cargo clippy  
    ├── cargo test --workspace  
    ├── cargo build --release  
    └── mdbook build  
    ↓  
合并到 main  
    ↓  
打标签 v2.1.0  
    ↓  
发布 GitHub Release  
    ├── gatelangc (Linux/macOS/Windows)  
    ├── gatesim  
    ├── gateproof  
    ├── gatecanvas  
    └── AI 辅助插件（可选）
```

## 10. 模块化协作规范

### 10.1 版本管理

- 每个 crate 独立语义化版本。

- 主版本号变更表示不兼容接口变更。

- 工作区统一版本号用于发布。

### 10.2 接口稳定性

- 核心接口（`CompilerDriver`、`Backend`、`SpecVerifier`、`AiAssistant`）变更需 RFC。

- 内部模块接口可快速迭代，但需保持测试覆盖。

### 10.3 代码规范

- 使用 `rustfmt` 统一格式。

- 使用 `clippy` 静态检查。

- 公共 API 必须有文档注释。

- 错误处理使用 `Result`，避免 `panic`。

- 模块间依赖通过 trait 注入，避免硬编码。

### 10.4 AI 模块的特殊规范

- AI 模块不得直接修改编译器核心数据，必须通过 `gatelang-ai-gate` 验证。

- AI 生成的所有代码必须附带置信度和推理说明。

- AI 优化必须提供等价性证明，否则拒绝。

- AI 模块的失败必须是安全的（fail-safe），不得产生未验证的代码。

## 11. 风险与缓解

| 风险 | 影响 | 缓解 |
| - | - | - |
| 模块接口不稳定 | 并行开发受阻 | 早期定义核心 trait，RFC 流程 |
| 分层展开语义偏差 | 核心价值受损 | 形式化等价性检查，逐层验证 |
| 后端开发工作量大 | 延迟发布 | 按优先级：sim → verilog → zk → tee |
| 形式化验证集成复杂 | 延迟 | Z3 作为可选依赖，先支持简单规范 |
| 资源计算跨层不准确 | 失去可预测性 | 属性测试覆盖各层展开 |
| 可视化编辑器复杂 | 范围蔓延 | v0.1 仅原型，后续迭代 |
| **AI 生成错误代码** | 安全风险 | **强制 `gatelang-ai-gate` 验证** |
| **AI 优化破坏语义** | 核心价值受损 | **强制等价性检查** |
| **AI 规范生成不完整** | 验证漏洞 | **gateproof 兜底，人工复核** |
| **AI 模块依赖外部 LLM** | 供应链风险 | 抽象接口，支持多 LLM 后端 |


## 12. 里程碑

| 阶段 | 时间 | 交付物 |
| - | - | - |
| M1 | 第 1-2 周 | `gatelang-lexer`、`gatelang-ast`、`gatelang-parser` |
| M2 | 第 3-4 周 | `gatelang-types`、`gatelang-resource` |
| M3 | 第 5-6 周 | `gatelang-spec`、`gatelang-lower`、`gatelang-ir` |
| M4 | 第 7-8 周 | `gatelang-backend-sim`、`gatelang-driver`、`gatelang-cli` |
| M5 | 第 9-10 周 | `gatesim`、`gateproof`、`stdlib-L1` |
| M6 | 第 11-12 周 | `gatelang-backend-verilog`、文档、v0.1 发布 |
| M7 | 第 13-16 周 | `gatelang-backend-zk`、`stdlib-L2`、`gatepm` |
| M8 | 第 17-20 周 | `gatelang-backend-tee`、`gatecanvas`、`stdlib-L3` |
| M9 | 第 21-24 周 | `gatelang-ai-gen`、`gatelang-ai-spec`、`gatelang-ai-gate` |
| M10 | 第 25-28 周 | `gatelang-ai-opt`、`gatelang-ai-tutor`、v2.1 发布 |


## 13. 附录

### 13.1 术语表

| 术语 | 定义 |
| - | - |
| NAND | 与非门，逻辑完备的基础门 |
| LATCH | 触发器，保存一位状态 |
| Bit | 0/1 单位信号 |
| Bits | N 位宽信号 |
| L1-L4 | 四层抽象：门级、高级、领域 DSL、可视化 |
| IR | 中间表示，NAND/LATCH 网表 |
| 分层展开 | L4→L3→L2→L1 的逐层降低抽象 |
| 资源报告 | 门数、深度、周期数、LATCH 数 |
| 形式化规范 | `spec` 块，编译期自动验证 |
| AI Gate | AI 生成代码的强制验证网关 |


### 13.2 参考

- GateLang 技术白皮书 v2.1

- GateLang PRD v2.0

- GateLang ERD v2.0

- Rust API 指南

- Z3 SMT Solver 文档

- mdBook 文档工具


**FunctionComplete · GateLang 技术开发文档 v2.1 · 模块化开发 · 2026**

*NAND 是唯一的组合原语。LATCH 是唯一的状态原语。0/1 是唯一的底层语言。AI 是加速器，形式化验证是保证者。模块化让所有人皆可参与构建。*

