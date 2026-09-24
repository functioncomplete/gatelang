# GateLang 技术开发文档（完全版·模块化开发）

**项目**：GateLang  
**版本**：v2.0  
**发布方**：FunctionComplete  
**日期**：2026年9月  
**文档类型**：技术开发工程文档  
**开发模式**：模块化、分层解耦、可独立测试与集成

## 1. 概述

GateLang 是一门以 NAND 门为唯一组合原语、以 LATCH 为唯一状态原语的门级可验证计算语言。v2.0 引入四层抽象模型（L1 门级、L2 高级语言、L3 领域 DSL、L4 可视化），所有层编译到同一 NAND/LATCH 门级网表。

本文档定义 GateLang 的**模块化开发架构**，包括模块划分、接口契约、依赖关系、开发顺序、测试策略、构建发布与协作规范。目标是让多个团队或个人开发者能够**并行开发、独立测试、按需集成**。

## 2. 总体架构

GateLang 采用**编译器三段式 + 工具链周边 + 标准库**的模块化架构。

```
┌─────────────────────────────────────────────────────────────────────┐  
│                          用户界面层                                   │  
│  gatelangc (CLI)  ·  gatecanvas (可视化)  ·  IDE 插件 (规划中)        │  
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
│  gatelang-backend-sim  ·  gatelang-backend-verilog  ·  gatelang-backend-zk  ·  gatelang-backend-tee │  
└───────────────────────────────┬─────────────────────────────────────┘  
                                │  
┌───────────────────────────────▼─────────────────────────────────────┐  
│                          运行时与工具                                 │  
│  gatesim (模拟器)  ·  gateproof (形式化验证)  ·  gatepm (包管理器)    │  
└───────────────────────────────┬─────────────────────────────────────┘  
                                │  
┌───────────────────────────────▼─────────────────────────────────────┐  
│                          标准库                                       │  
│  stdlib-L1  ·  stdlib-L2  ·  stdlib-L3  ·  领域 DSL 库               │  
└─────────────────────────────────────────────────────────────────────┘
```

## 3. 模块划分

所有模块以 Rust crate 形式实现，通过 Cargo workspace 管理。每个 crate 独立版本化、独立测试、独立发布。

### 3.1 核心编译器模块

| 模块 | Crate 名称 | 职责 | 依赖 |
| - | - | - | - |
| 词法分析 | `gatelang-lexer` | 将源码转为 Token 流，支持 L1-L4 语法 | 无 |
| 语法分析 | `gatelang-parser` | 将 Token 流转为统一 AST | `gatelang-lexer`、`gatelang-ast` |
| AST 定义 | `gatelang-ast` | 定义所有层的 AST 节点 | 无 |
| 类型系统 | `gatelang-types` | 类型检查、状态分离验证 | `gatelang-ast` |
| 资源计算 | `gatelang-resource` | 门数、深度、周期数静态计算 | `gatelang-ast`、`gatelang-types` |
| 规范验证 | `gatelang-spec` | 解析 `spec` 块，调用 Z3/NuSMV | `gatelang-ast`、`gatelang-types` |
| 分层展开 | `gatelang-lower` | L4→L3→L2→L1 逐层展开 | `gatelang-ast`、`gatelang-types` |
| 中间表示 | `gatelang-ir` | NAND/LATCH 网表数据结构 | 无 |
| 编译驱动 | `gatelang-driver` | 串联前端、中端、后端 | 所有核心模块 |


### 3.2 编译器后端模块

| 模块 | Crate 名称 | 职责 | 依赖 |
| - | - | - | - |
| 模拟器后端 | `gatelang-backend-sim` | 生成可执行模拟代码 | `gatelang-ir` |
| Verilog 后端 | `gatelang-backend-verilog` | 生成 Verilog/VHDL | `gatelang-ir` |
| ZK 后端 | `gatelang-backend-zk` | 生成 Circom/Noir | `gatelang-ir` |
| TEE 后端 | `gatelang-backend-tee` | 生成 TEE 可执行代码 | `gatelang-ir` |
| C 后端 | `gatelang-backend-c` | 生成嵌入式 C 代码 | `gatelang-ir` |


### 3.3 工具链模块

| 模块 | Crate 名称 | 职责 | 依赖 |
| - | - | - | - |
| CLI | `gatelang-cli` | 命令行入口 `gatelangc` | `gatelang-driver`、所有后端 |
| 模拟器 | `gatesim` | 门级仿真、波形输出 | `gatelang-ir` |
| 形式化验证 | `gateproof` | 集成 Z3/NuSMV，验证规范 | `gatelang-spec`、`gatelang-ir` |
| 包管理器 | `gatepm` | 函数库发布、依赖解析 | `gatelang-driver` |
| 可视化编辑器 | `gatecanvas` | 拖拽画布、实时仿真、代码生成 | `gatesim`、`gatelang-driver` |


### 3.4 标准库模块

| 模块 | 类型 | 内容 |
| - | - | - |
| `stdlib-L1` | GateLang 源码 | 半加器、全加器、4位加法器、MUX、基本逻辑门 |
| `stdlib-L2` | GateLang 源码 | 安全算术、类型转换、错误处理 |
| `stdlib-L3` | GateLang 源码 | 金融 DSL、AI DSL、游戏 DSL 基础库 |
| `stdlib-verify` | 证明文件 | 每个标准库函数的形式化验证证明 |


## 4. 模块接口契约

每个模块通过 Rust trait 或函数签名定义清晰接口。以下为核心接口示例。

### 4.1 编译驱动接口

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

### 4.2 分层展开接口

```
pub trait LoweringPass \{  
    fn lower(&self, ast: &Ast) -\> Result\<Ast, LoweringError\>;  
\}  
  
pub struct L4ToL3Lowering;  
pub struct L3ToL2Lowering;  
pub struct L2ToL1Lowering;  
pub struct L1ToIRLowering;
```

### 4.3 后端接口

```
pub trait Backend \{  
    fn name(&self) -\> &str;  
    fn generate(&self, ir: &NandLatchNetlist) -\> Result\<TargetCode, BackendError\>;  
\}
```

### 4.4 形式化验证接口

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

## 5. 依赖关系与开发顺序

### 5.1 依赖图

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
┌───────────────┬───────────────┬───────────────┐  
│ backend-sim   │ backend-verilog│ backend-zk    │ backend-tee  
└───────────────┴───────────────┴───────────────┘  
    ↓  
gatelang-driver  
    ↓  
gatelang-cli / gatesim / gateproof / gatepm / gatecanvas
```

### 5.2 开发顺序

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


## 6. 模块化测试策略

每个模块独立测试，集成测试在 driver 层进行。

| 模块 | 单元测试 | 集成测试 | 属性测试 | 形式化验证 |
| - | - | - | - | - |
| `gatelang-lexer` | Token 流正确性 | 与 parser 集成 | — | — |
| `gatelang-parser` | AST 结构 | 与 types 集成 | — | — |
| `gatelang-types` | 类型检查规则 | 与 resource 集成 | 类型系统不变式 | — |
| `gatelang-resource` | 门数/深度计算 | 与 lower 集成 | 资源上界准确性 | — |
| `gatelang-spec` | 规范解析 | 与 gateproof 集成 | — | Z3 验证 |
| `gatelang-lower` | 逐层展开 | 与 IR 集成 | 分层一致性 | 等价性检查 |
| `gatelang-ir` | 网表结构 | 与后端集成 | — | — |
| `backend-\*` | 代码生成 | 端到端编译 | — | — |
| `gatesim` | 仿真正确性 | 示例电路 | — | — |
| `gateproof` | 验证器 | 标准库证明 | — | — |
| `gatecanvas` | UI 组件 | 完整工作流 | — | — |


## 7. 构建与发布

### 7.1 Cargo Workspace 结构

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

### 7.2 构建命令

| 命令 | 作用 |
| - | - |
| `cargo build --release` | 构建所有模块 |
| `cargo test` | 运行所有单元测试 |
| `cargo test --workspace` | 运行全工作区测试 |
| `cargo clippy` | 静态检查 |
| `cargo fmt` | 格式化 |
| `mdbook build` | 构建文档 |


### 7.3 发布流程

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
打标签 v0.1.0  
    ↓  
发布 GitHub Release  
    ├── gatelangc (Linux/macOS/Windows)  
    ├── gatesim  
    ├── gateproof  
    └── gatecanvas
```

## 8. 模块化协作规范

### 8.1 版本管理

- 每个 crate 独立语义化版本。

- 主版本号变更表示不兼容接口变更。

- 工作区统一版本号用于发布。

### 8.2 接口稳定性

- 核心接口（`CompilerDriver`、`Backend`、`SpecVerifier`）变更需 RFC。

- 内部模块接口可快速迭代，但需保持测试覆盖。

### 8.3 代码规范

- 使用 `rustfmt` 统一格式。

- 使用 `clippy` 静态检查。

- 公共 API 必须有文档注释。

- 错误处理使用 `Result`，避免 `panic`。

- 模块间依赖通过 trait 注入，避免硬编码。

### 8.4 文档

- 每个 crate 包含 `README.md` 和 API 文档。

- 语言规范使用 mdBook。

- 示例代码位于 `examples/`。

- 开发指南位于 `docs/`。

## 9. 风险与缓解

| 风险 | 影响 | 缓解 |
| - | - | - |
| 模块接口不稳定 | 并行开发受阻 | 早期定义核心 trait，RFC 流程 |
| 分层展开语义偏差 | 核心价值受损 | 形式化等价性检查，逐层验证 |
| 后端开发工作量大 | 延迟发布 | 按优先级：sim → verilog → zk → tee |
| 形式化验证集成复杂 | 延迟 | Z3 作为可选依赖，先支持简单规范 |
| 资源计算跨层不准确 | 失去可预测性 | 属性测试覆盖各层展开 |
| 可视化编辑器复杂 | 范围蔓延 | v0.1 仅原型，后续迭代 |


## 10. 里程碑

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


## 11. 附录

### 11.1 术语表

| 术语 | 定义 |
| - | - |
| NAND | 与非门，逻辑完备的基础门 |
| LATCH | 触发器，保存一位状态 |
| L1-L4 | 四层抽象：门级、高级、领域 DSL、可视化 |
| IR | 中间表示，NAND/LATCH 网表 |
| 分层展开 | L4→L3→L2→L1 的逐层降低抽象 |
| 资源报告 | 门数、深度、周期数、LATCH 数 |
| 形式化规范 | `spec` 块，编译期自动验证 |


### 11.2 参考

- GateLang 技术白皮书 v2.0

- GateLang PRD v2.0

- GateLang ERD v2.0

- Rust API 指南

- Z3 SMT Solver 文档

- mdBook 文档工具


**FunctionComplete · GateLang 技术开发文档 · 模块化开发 · 2026**

*NAND 是唯一的组合原语。LATCH 是唯一的状态原语。模块化让所有人皆可参与构建。*

