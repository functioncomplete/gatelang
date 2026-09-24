# GateLang v0.1 产品需求文档（PRD）、实体关系设计（ERD）与软件开发工程文档

**项目名称**：GateLang  
**版本**：v0.1  
**发布方**：FunctionComplete  
**日期**：2026年9月

# 第一部分：产品需求文档（PRD）

## 1. 背景与目标

GateLang 是一门以 **NAND 门** 为唯一组合原语、以 **LATCH 触发器** 为唯一状态原语的门级可验证计算语言。它不绑定任何特定执行环境，可编译为 FPGA/ASIC 网表、ZK 电路、TEE 可执行代码或软件模拟器。

**v0.1 的核心目标**：交付一个可用的语言核心与工具链原型，验证“门级最小语义 + 类型级资源预测 + 原生形式化规范”这一设计理念的可行性。

**v0.1 不包含**：完整标准库、包管理器、生产级优化器、多后端全覆盖。这些推迟到后续版本。

## 2. 目标用户

| 用户角色 | 需求 |
| - | - |
| **硬件工程师** | 用高级语法描述电路，编译为可综合的 FPGA/ASIC 网表 |
| **ZK 电路开发者** | 用统一语言描述算术电路，编译为 Circom/Noir |
| **TEE 开发者** | 编写纯计算逻辑，编译为 TEE 可执行代码 |
| **形式化验证研究者** | 在语言层面编写规范，自动验证函数正确性 |
| **计算机体系结构教育者** | 从半加器到 CPU 的渐进式教学工具 |


## 3. 核心功能需求

### 3.1 语言核心

| 编号 | 功能 | 优先级 | 说明 |
| - | - | - | - |
| L-01 | NAND 门原语 | P0 | 语言最小组合语义单元 |
| L-02 | LATCH 原语 | P0 | 语言唯一状态语义单元 |
| L-03 | `circuit` 组合函数 | P0 | 无状态、纯组合逻辑 |
| L-04 | `state` 时序结构 | P0 | 包含 LATCH，显式时钟语义 |
| L-05 | `Bits\<N\>` 类型 | P0 | N 位宽信号 |
| L-06 | `Latch\<N\>` 类型 | P0 | N 位状态寄存器 |
| L-07 | 资源类型标注 | P1 | `Gates\<G\>`、`Depth\<D\>`、`Cycles\<C\>` |
| L-08 | 类型系统强制状态分离 | P0 | 组合函数不可含 LATCH；时序函数不可隐式修改外部状态 |
| L-09 | `\<-` LATCH 更新语法 | P0 | 仅在时钟沿更新状态 |
| L-10 | 基本位运算与逻辑门语法糖 | P1 | `AND`、`OR`、`XOR`、`NOT` 等，底层展开为 NAND |


### 3.2 形式化规范

| 编号 | 功能 | 优先级 | 说明 |
| - | - | - | - |
| S-01 | `spec` 块语法 | P0 | 支持 `precondition`、`postcondition`、`invariant` |
| S-02 | 规范验证接口 | P1 | 调用外部求解器（Z3、NuSMV）进行验证 |
| S-03 | 验证结果报告 | P1 | 输出通过/失败及反例 |
| S-04 | 规范与代码绑定 | P0 | 规范随函数一同编译，不可分离 |


### 3.3 编译器

| 编号 | 功能 | 优先级 | 说明 |
| - | - | - | - |
| C-01 | 词法/语法分析 | P0 | 解析 GateLang 源码 |
| C-02 | 类型检查 | P0 | 验证类型正确性、资源上界 |
| C-03 | 门级展开 | P0 | 将高级结构展开为 NAND/LATCH 网表 |
| C-04 | 资源计算 | P0 | 静态计算门数、深度、周期数 |
| C-05 | NAND/LATCH 网表输出 | P0 | 生成中间表示（IR） |
| C-06 | 软件模拟器后端 | P0 | 生成可执行模拟代码 |
| C-07 | FPGA/ASIC 网表后端 | P1 | 生成 Verilog/VHDL |
| C-08 | ZK 电路后端 | P2 | 生成 Circom/Noir |
| C-09 | 编译日志与报告 | P1 | 输出资源使用、验证结果 |


### 3.4 工具链

| 编号 | 功能 | 优先级 | 说明 |
| - | - | - | - |
| T-01 | CLI 编译器 `gatelangc` | P0 | 命令行编译入口 |
| T-02 | 门级模拟器 `gatesim` | P0 | 组合/时序仿真、波形输出 |
| T-03 | 形式化验证工具 `gateproof` | P1 | 集成 Z3/NuSMV |
| T-04 | 项目配置文件 | P1 | `gatelang.toml` 定义项目结构 |
| T-05 | 基础标准库 | P1 | 半加器、全加器、4位加法器、MUX、基本逻辑门 |


## 4. 非功能需求

| 编号 | 需求 | 指标 |
| - | - | - |
| N-01 | 编译速度 | 1000 行源码编译 \< 2 秒（不含形式化验证） |
| N-02 | 资源计算准确性 | 门数/深度/周期计算误差 0% |
| N-03 | 模拟器性能 | 10 万门网络仿真 \> 10 万周期/秒 |
| N-04 | 跨平台 | Linux/macOS/Windows 支持 |
| N-05 | 可扩展性 | 后端插件化，新增后端无需修改核心 |
| N-06 | 错误信息 | 编译错误包含源码位置、原因、建议 |
| N-07 | 确定性 | 相同输入始终产生相同输出 |


## 5. 用户故事

| 编号 | 用户故事 | 验收标准 |
| - | - | - |
| US-01 | 作为硬件工程师，我想用 GateLang 描述一个 4 位加法器，并编译为 Verilog | 编译成功，生成 Verilog 文件，门数与预期一致 |
| US-02 | 作为 ZK 开发者，我想将 SHA-256 单轮编译为 Circom 电路 | 生成 Circom 代码，约束数可报告 |
| US-03 | 作为形式化验证研究者，我想为加法器编写规范并自动验证 | `gateproof` 返回验证通过，或给出反例 |
| US-04 | 作为教育者，我想在模拟器中运行交通灯状态机 | 波形正确显示状态转换 |
| US-05 | 作为开发者，我想在编译时看到每个函数的门数和深度 | 编译报告列出资源使用 |


## 6. 验收标准

- 语言核心：`circuit` 和 `state` 语法正确解析，类型系统拒绝非法状态混合。

- 编译器：能将 4 位加法器、交通灯状态机、SHA-256 单轮编译为 NAND/LATCH 网表。

- 模拟器：能仿真上述电路，输出正确波形。

- 形式化验证：至少支持 `spec` 块的语法解析，并能调用 Z3 验证简单后置条件。

- 后端：至少完成软件模拟器后端和 NAND/LATCH 网表输出。

- 文档：语言规范、CLI 使用手册、示例代码库。

## 7. 里程碑

| 阶段 | 时间 | 交付物 |
| - | - | - |
| M1 | 第 1-2 周 | 语言语法定义、AST 设计、解析器原型 |
| M2 | 第 3-4 周 | 类型检查器、资源计算、NAND 展开 |
| M3 | 第 5-6 周 | 软件模拟器后端、CLI `gatelangc` |
| M4 | 第 7-8 周 | 形式化规范解析、Z3 集成原型 |
| M5 | 第 9-10 周 | 基础标准库、示例集、文档 |
| M6 | 第 11-12 周 | 内部测试、Bug 修复、v0.1 发布 |


# 第二部分：实体关系设计（ERD）

GateLang v0.1 的 ERD 描述的是**工具链元模型**，即编译器、项目、函数、规范、资源、编译结果等实体及其关系。不涉及数据库存储，而是用于指导内部数据结构和模块设计。

```
erDiagram  
    Project ||--o\{ SourceFile : contains  
    SourceFile ||--o\{ Circuit : defines  
    SourceFile ||--o\{ State : defines  
    Circuit ||--o\{ Function : contains  
    State ||--o\{ Latch : contains  
    State ||--o\{ Function : contains  
    Function ||--o| Spec : has  
    Function ||--o\{ Type : uses  
    Function ||--o\{ Resource : consumes  
    Function ||--o\{ Function : composes  
    Compilation ||--|| Project : compiles  
    Compilation ||--o\{ Backend : uses  
    Compilation ||--o\{ Resource : produces  
    Compilation ||--o\{ Proof : generates  
    Backend ||--o\{ Target : generates  
    LibraryPackage ||--o\{ Circuit : includes  
    LibraryPackage ||--o\{ State : includes  
    Proof ||--|| Function : verifies  
  
    Project \{  
        string name  
        string version  
        string root\_path  
    \}  
    SourceFile \{  
        string path  
        string content  
    \}  
    Circuit \{  
        string name  
        TypeSignature signature  
        ResourceBound bound  
    \}  
    State \{  
        string name  
        Latch\[\] latches  
    \}  
    Latch \{  
        string name  
        int width  
        int initial\_value  
    \}  
    Function \{  
        string name  
        TypeSignature signature  
        bool is\_combinational  
    \}  
    Spec \{  
        string precondition  
        string postcondition  
        string\[\] invariants  
    \}  
    Type \{  
        string kind  
        int width  
    \}  
    Resource \{  
        int gates  
        int depth  
        int cycles  
        int latches  
    \}  
    Compilation \{  
        string id  
        datetime timestamp  
        string status  
    \}  
    Backend \{  
        string name  
        string version  
    \}  
    Target \{  
        string language  
        string output\_path  
    \}  
    Proof \{  
        string hash  
        string status  
        string solver  
    \}  
    LibraryPackage \{  
        string name  
        string version  
    \}
```

**实体说明**：

| 实体 | 说明 |
| - | - |
| Project | 一个 GateLang 项目，包含多个源文件 |
| SourceFile | 源文件，包含电路和状态定义 |
| Circuit | 组合逻辑模块，无状态 |
| State | 时序模块，包含 LATCH |
| Latch | 状态寄存器，声明宽度和初值 |
| Function | 函数，可组合或时序 |
| Spec | 形式化规范，绑定到函数 |
| Type | 数据类型，如 `Bits\<8\>` |
| Resource | 资源消耗，门数/深度/周期/LATCH 数 |
| Compilation | 一次编译过程 |
| Backend | 编译后端，如模拟器、Verilog |
| Target | 生成的目标代码 |
| Proof | 形式化验证证明 |
| LibraryPackage | 标准库包 |


# 第三部分：软件开发工程文档

## 1. 系统架构

GateLang v0.1 采用经典的**编译器三段式架构**，外加工具链周边模块。

```
┌─────────────────────────────────────────────────────────┐  
│                       CLI 层                            │  
│  gatelangc · gatesim · gateproof · gatepm (规划中)      │  
└─────────────────────┬───────────────────────────────────┘  
                      │  
┌─────────────────────▼───────────────────────────────────┐  
│                    编译器前端                            │  
│  词法分析 · 语法分析 · AST 构建 · 类型检查 · 资源验证    │  
└─────────────────────┬───────────────────────────────────┘  
                      │  
┌─────────────────────▼───────────────────────────────────┐  
│                    编译器中端                            │  
│  门级展开 · 资源计算 · 规范提取 · 中间表示（IR）         │  
└─────────────────────┬───────────────────────────────────┘  
                      │  
┌─────────────────────▼───────────────────────────────────┐  
│                    编译器后端                            │  
│  模拟器后端 · Verilog 后端 · ZK 后端（规划中）           │  
└─────────────────────────────────────────────────────────┘
```

**周边模块**：

- 形式化验证接口：调用 Z3/NuSMV。

- 模拟器运行时：执行门级网表。

- 标准库：预定义基础电路。

## 2. 模块划分

| 模块 | 职责 | 技术选型 |
| - | - | - |
| `gatelang-lexer` | 词法分析 | Rust + logos |
| `gatelang-parser` | 语法分析，生成 AST | Rust + pest 或手写递归下降 |
| `gatelang-ast` | AST 数据结构定义 | Rust |
| `gatelang-types` | 类型系统与类型检查 | Rust |
| `gatelang-resource` | 资源计算与验证 | Rust |
| `gatelang-spec` | 规范解析与验证接口 | Rust + Z3 binding |
| `gatelang-ir` | 中间表示（NAND/LATCH 网表） | Rust |
| `gatelang-backend-sim` | 软件模拟器后端 | Rust + C 运行时 |
| `gatelang-backend-verilog` | Verilog 后端 | Rust |
| `gatelang-cli` | 命令行接口 | Rust + clap |
| `gatesim` | 门级模拟器 | Rust + 波形输出 |
| `gateproof` | 形式化验证工具 | Rust + Z3 |
| `stdlib` | 标准库 | GateLang 源码 |


## 3. 技术选型

| 维度 | 选择 | 理由 |
| - | - | - |
| **实现语言** | Rust | 内存安全、高性能、优秀的枚举与模式匹配，适合编译器开发 |
| **解析器** | 手写递归下降 | 对语言语法有完全控制，错误信息友好 |
| **中间表示** | 自定义 IR | NAND/LATCH 网表需要精确控制 |
| **形式化验证** | Z3（SMT） | 成熟、跨平台、Rust binding 可用 |
| **模拟器** | Rust + 事件驱动 | 高性能，支持大规模网表 |
| **构建系统** | Cargo | Rust 原生，依赖管理简单 |
| **测试框架** | Rust 内置 test + proptest | 单元测试 + 属性测试 |
| **CI/CD** | GitHub Actions | 跨平台构建与测试 |
| **文档** | mdBook | 语言规范与用户手册 |


## 4. 开发环境

- **操作系统**：Linux (Ubuntu 22.04+), macOS (12+), Windows (WSL2)

- **Rust 版本**：1.75+

- **依赖**：

  - `clap`：CLI 参数解析

  - `logos`：词法分析（可选）

  - `z3`：形式化验证

  - `serde`：配置与序列化

  - `petgraph`：电路图分析

  - `plotters`：波形输出

## 5. 测试策略

| 测试类型 | 范围 | 工具 |
| - | - | - |
| **单元测试** | 词法、语法、类型检查、资源计算 | Rust `\#\[test\]` |
| **集成测试** | 从源码到网表、模拟器执行 | Rust 测试框架 |
| **属性测试** | 资源计算准确性、类型系统不变式 | `proptest` |
| **形式化验证测试** | 规范验证的正确性 | Z3 + 测试用例 |
| **回归测试** | 示例电路编译与仿真 | 快照测试 |
| **性能测试** | 编译速度、模拟器吞吐 | `criterion` |


## 6. 构建与发布

- **构建**：`cargo build --release`

- **测试**：`cargo test`

- **文档**：`mdbook build`

- **发布**：GitHub Releases，提供 Linux/macOS/Windows 二进制

- **版本号**：语义化版本 `0.1.0`

## 7. 代码规范

- 使用 `rustfmt` 统一格式。

- 使用 `clippy` 进行静态检查。

- 公共 API 必须有文档注释。

- 错误处理使用 `Result`，避免 `panic`。

- 模块划分清晰，避免循环依赖。

## 8. CI/CD 流程

```
Push / PR  
    ↓  
GitHub Actions  
    ├── cargo fmt --check  
    ├── cargo clippy  
    ├── cargo test  
    ├── cargo build --release  
    └── mdbook build  
    ↓  
合并到 main  
    ↓  
打标签 v0.1.0  
    ↓  
发布 GitHub Release
```

## 9. 风险与缓解

| 风险 | 影响 | 缓解措施 |
| - | - | - |
| 形式化验证集成复杂 | 延迟发布 | 先支持简单后置条件，Z3 集成作为可选特性 |
| 门级展开规模爆炸 | 编译缓慢 | 设置资源上界，编译期拒绝超大电路 |
| 多后端开发工作量大 | 范围蔓延 | v0.1 只做模拟器后端和 IR，Verilog 后端作为 P1 |
| 类型系统设计缺陷 | 后期重构 | 早期形式化类型规则，写类型检查测试 |
| 资源计算不准确 | 失去核心价值 | 属性测试覆盖，与手工计算对比 |


## 10. 交付物清单

- [ ] 语言规范文档（mdBook）

- [ ] `gatelangc` 编译器 v0.1

- [ ] `gatesim` 模拟器 v0.1

- [ ] `gateproof` 形式化验证工具原型

- [ ] 基础标准库（半加器、全加器、4位加法器、MUX、基本逻辑门）

- [ ] 示例项目（交通灯、SHA-256 单轮）

- [ ] 用户手册

- [ ] 测试报告

- [ ] GitHub Release v0.1.0

**FunctionComplete · GateLang v0.1 工程文档 · 2026**

*NAND 是唯一的组合原语。LATCH 是唯一的状态原语。其余一切皆由此推导。*

