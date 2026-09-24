# GateLang v2.1 产品需求文档（PRD）

**项目名称**：GateLang  
**版本**：v2.1  
**发布方**：FunctionComplete  
**日期**：2026年9月  
**更新内容**：新增 0/1 二进制基础需求、AI 辅助开发功能需求、AI 强制验证网关需求

## 1. 背景与目标

GateLang 是一门以 **NAND 门**为唯一组合原语、以 **LATCH 触发器**为唯一状态原语的门级可验证计算语言。v2.0 引入四层抽象模型（L1 门级、L2 高级语言、L3 领域 DSL、L4 可视化）。

**v2.1 的核心目标**：

- 将 **0/1 二进制基础**正式纳入语言规范，明确 NAND 与 LATCH 的底层语义。

- 引入 **AI 辅助开发模块**，覆盖代码生成、门级优化、规范生成和教学辅助。

- 建立 **AI 强制验证网关**（`gatelang-ai-gate`），确保 AI 生成的一切代码必须经过类型检查、资源验证、形式化验证和等价性检查。

**v2.1 不包含**：完整 AI 模型训练、云端 AI 服务、IDE 深度集成。这些推迟到后续版本。

## 2. 目标用户

| 用户角色 | 使用层 | 需求 |
| - | - | - |
| 硬件工程师 | L1 | 门级电路描述与 Verilog 编译 |
| ZK 电路开发者 | L1 | 算术电路描述与 Circom/Noir 编译 |
| 形式化验证研究者 | L1 | `spec` 块与自动验证 |
| 软件开发者 | L2 | 函数、类型、控制流语法糖 |
| 金融工程师 | L3 | 金融 DSL，AMM 定价、风控模型 |
| AI 研究者 | L3 | AI DSL，二值化神经网络编译 |
| 游戏设计师 | L3 | 游戏 DSL，伤害计算、路径验证 |
| 教育用户 | L4 | 拖拽搭建电路，观察波形 |
| 产品经理 | L4 | 原型验证逻辑，无需编码 |
| **AI 辅助开发者** | **L1-L4** | **用自然语言生成 GateLang 代码、优化电路、生成规范** |


## 3. 核心功能需求

### 3.1 语言核心（L1 门级层）

| 编号 | 功能 | 优先级 | 说明 |
| - | - | - | - |
| L-01 | NAND 门原语 | P0 | 语言最小组合语义单元 |
| L-02 | LATCH 原语 | P0 | 语言唯一状态语义单元 |
| L-03 | `circuit` 组合函数 | P0 | 无状态、纯组合逻辑 |
| L-04 | `state` 时序结构 | P0 | 包含 LATCH，显式时钟语义 |
| L-05 | `Bit` 类型 | P0 | 0/1 单位信号 |
| L-06 | `Bits\<N\>` 类型 | P0 | N 位宽信号 |
| L-07 | `Latch\<N\>` 类型 | P0 | N 位状态寄存器 |
| L-08 | 资源类型标注 | P1 | `Gates\<G\>`、`Depth\<D\>`、`Cycles\<C\>` |
| L-09 | 类型系统强制状态分离 | P0 | 组合函数不可含 LATCH |
| L-10 | `\<-` LATCH 更新语法 | P0 | 仅在时钟沿更新状态 |
| L-11 | 位运算与逻辑门语法糖 | P1 | `AND`、`OR`、`XOR`、`NOT`，底层展开为 NAND |
| L-12 | **0/1 字面量** | **P0** | **`0`、`1`、`0b0101`、`0xFF`** |
| L-13 | **位宽推导** | **P0** | **编译期自动推导 `Bits\<N\>` 位宽** |
| L-14 | **SR 锁存器标准实现** | **P1** | **两个 NAND 门交叉耦合的标准 SR LATCH** |


### 3.2 L2 高级语言层（预览）

| 编号 | 功能 | 优先级 |
| - | - | - |
| L2-01 | `fn` 函数语法 | P1 |
| L2-02 | `struct`/`enum` 类型 | P1 |
| L2-03 | `if/else`/`match` 语法糖 | P1 |
| L2-04 | 算术运算符映射 | P1 |
| L2-05 | `Result\<T,E\>` 错误处理 | P2 |


### 3.3 L3 领域 DSL 层（原型）

| 编号 | 功能 | 优先级 |
| - | - | - |
| L3-01 | 金融 DSL（AMM 定价） | P2 |
| L3-02 | AI DSL（二值化 MLP） | P2 |
| L3-03 | 游戏 DSL（伤害计算） | P2 |


### 3.4 L4 可视化层（原型）

| 编号 | 功能 | 优先级 |
| - | - | - |
| L4-01 | 拖拽式画布 | P2 |
| L4-02 | 波形仿真 | P2 |
| L4-03 | 自动生成 L1 代码 | P2 |
| L4-04 | **AI 教学助手** | **P1** |


### 3.5 形式化规范

| 编号 | 功能 | 优先级 |
| - | - | - |
| S-01 | `spec` 块语法 | P0 |
| S-02 | 规范验证接口（Z3/NuSMV） | P1 |
| S-03 | 验证结果报告 | P1 |
| S-04 | 规范与代码绑定 | P0 |
| S-05 | **分层展开等价性检查** | **P1** |


### 3.6 编译器

| 编号 | 功能 | 优先级 |
| - | - | - |
| C-01 | 词法/语法分析（L1-L4） | P0 |
| C-02 | 类型检查与资源验证 | P0 |
| C-03 | 分层展开（L4→L3→L2→L1） | P0 |
| C-04 | 资源计算 | P0 |
| C-05 | NAND/LATCH 网表输出 | P0 |
| C-06 | 软件模拟器后端 | P0 |
| C-07 | FPGA/ASIC 网表后端 | P1 |
| C-08 | ZK 电路后端 | P2 |
| C-09 | TEE 后端 | P2 |
| C-10 | 编译日志与资源报告 | P1 |


### 3.7 AI 辅助开发（v2.1 新增）

| 编号 | 功能 | 优先级 | 说明 |
| - | - | - | - |
| AI-01 | AI 代码生成 | P1 | 从自然语言/伪代码生成候选 GateLang 代码 |
| AI-02 | AI 门级优化 | P2 | 分析 NAND 网表，提出门数/深度优化方案 |
| AI-03 | AI 规范生成 | P1 | 从函数签名和行为描述生成 `spec` 块 |
| AI-04 | AI 教学助手 | P1 | 可视化层教学辅助、错误解释、练习题生成 |
| AI-05 | AI 验证网关 | P0 | 强制 AI 生成代码经过类型检查、资源验证、形式化验证 |
| AI-06 | AI 优化等价性检查 | P1 | AI 优化后的网表必须通过形式化等价性检查 |
| AI-07 | AI 多后端接口 | P2 | 支持多个 LLM 后端，避免单一供应商锁定 |


### 3.8 工具链

| 编号 | 功能 | 优先级 |
| - | - | - |
| T-01 | CLI 编译器 `gatelangc` | P0 |
| T-02 | 门级模拟器 `gatesim` | P0 |
| T-03 | 形式化验证工具 `gateproof` | P1 |
| T-04 | 包管理器 `gatepm` | P1 |
| T-05 | 可视化编辑器 `gatecanvas` | P2 |
| T-06 | 项目配置文件 `gatelang.toml` | P1 |
| T-07 | 基础标准库 | P1 |


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
| N-08 | **分层编译一致性** | **L1/L2/L3/L4 编译后门级网表一致** |
| N-09 | **AI 验证强制性** | **AI 生成代码 100% 经过验证网关** |
| N-10 | **AI 失败安全** | **AI 模块失败时不产生未验证代码** |


## 5. 用户故事

| 编号 | 用户故事 | 验收标准 |
| - | - | - |
| US-01 | 作为硬件工程师，我想用 GateLang 描述一个 4 位加法器，并编译为 Verilog | 编译成功，生成 Verilog，门数与预期一致 |
| US-02 | 作为 ZK 开发者，我想将 SHA-256 单轮编译为 Circom 电路 | 生成 Circom 代码，约束数可报告 |
| US-03 | 作为形式化验证研究者，我想为加法器编写规范并自动验证 | `gateproof` 返回验证通过，或给出反例 |
| US-04 | 作为教育者，我想在模拟器中运行交通灯状态机 | 波形正确显示状态转换 |
| US-05 | 作为开发者，我想在编译时看到每个函数的门数和深度 | 编译报告列出资源使用 |
| US-06 | 作为软件开发者，我想用 `fn` 编写转账逻辑，编译为门级电路 | 编译成功，资源报告显示门数 |
| US-07 | 作为金融工程师，我想用金融 DSL 描述 AMM 定价 | 展开为 L2，再展开为门级，形式化验证通过 |
| US-08 | 作为学生，我想在画布上拖出半加器并查看波形 | 波形正确，生成 L1 代码 |
| US-09 | **作为 AI 辅助开发者，我想用自然语言描述一个 8 位加法器，AI 生成 GateLang 代码** | **AI 生成候选代码，经 `gatelang-ai-gate` 验证通过** |
| US-10 | **作为 AI 辅助开发者，我想让 AI 优化我的网表，减少门数** | **AI 提出优化方案，经等价性检查通过，门数减少** |
| US-11 | **作为 AI 辅助开发者，我想让 AI 为我的函数生成 `spec` 块** | **AI 生成规范，经 `gateproof` 验证通过** |
| US-12 | **作为学生，我想让 AI 教学助手解释我的电路为什么不对** | **AI 给出错误原因和修改建议** |


## 6. 验收标准

- 语言核心：`circuit` 和 `state` 语法正确解析，类型系统拒绝非法状态混合，0/1 字面量和位宽推导正确。

- 编译器：能将 4 位加法器、交通灯状态机、SHA-256 单轮编译为 NAND/LATCH 网表。

- 模拟器：能仿真上述电路，输出正确波形。

- 形式化验证：支持 `spec` 块解析，调用 Z3 验证简单后置条件，支持分层展开等价性检查。

- 后端：至少完成软件模拟器后端和 NAND/LATCH 网表输出。

- **AI 辅助**：AI 代码生成、规范生成、教学助手可用，所有 AI 生成代码经过强制验证网关。

- 文档：语言规范、CLI 使用手册、示例代码库、AI 辅助使用指南。

## 7. 里程碑

| 阶段 | 时间 | 交付物 |
| - | - | - |
| M1 | 第 1-2 周 | 语言语法定义（含 0/1）、AST、解析器 |
| M2 | 第 3-4 周 | 类型检查器、资源计算、NAND 展开 |
| M3 | 第 5-6 周 | 模拟器后端、CLI `gatelangc` |
| M4 | 第 7-8 周 | 形式化规范解析、Z3 集成原型 |
| M5 | 第 9-10 周 | 基础标准库、示例集、文档 |
| M6 | 第 11-12 周 | 内部测试、Bug 修复、v0.1 发布 |
| M7 | 第 13-16 周 | L2 高级语言层、Verilog 后端 |
| M8 | 第 17-20 周 | L3 领域 DSL、ZK 后端 |
| M9 | 第 21-24 周 | AI 代码生成、AI 规范生成、AI 验证网关 |
| M10 | 第 25-28 周 | AI 门级优化、AI 教学助手、v2.1 发布 |


# GateLang v2.1 实体关系设计（ERD）

GateLang v2.1 的 ERD 描述工具链元模型，新增 **0/1 二进制基础**和 **AI 辅助开发**相关实体。

## 1. 核心 ERD

```
erDiagram  
    Project ||--o\{ SourceFile : contains  
    SourceFile ||--o\{ Module : defines  
    Module ||--o\{ Circuit : defines  
    Module ||--o\{ State : defines  
    Module ||--o\{ Function : defines  
    Module ||--o\{ DomainBlock : defines  
    Layer ||--o\{ Module : belongs\_to  
    Circuit ||--o\{ Signal : contains  
    State ||--o\{ Latch : contains  
    State ||--o\{ Function : contains  
    Function ||--o| Spec : has  
    Function ||--o\{ Type : uses  
    Function ||--o\{ Resource : consumes  
    Function ||--o\{ Function : composes  
    Signal ||--|| BitVector : represents  
    Latch ||--|| BitVector : stores  
    Compilation ||--|| Project : compiles  
    Compilation ||--o\{ Layer : processes  
    Compilation ||--o\{ Backend : uses  
    Compilation ||--o\{ Resource : produces  
    Compilation ||--o\{ Proof : generates  
    Backend ||--o\{ Target : generates  
    LibraryPackage ||--o\{ Circuit : includes  
    LibraryPackage ||--o\{ State : includes  
    Proof ||--|| Function : verifies  
    AiRequest ||--o\{ AiCandidate : generates  
    AiCandidate ||--o| ValidatedCode : becomes  
    ValidatedCode ||--|| Compilation : triggers  
    AiGate ||--o\{ ValidatedCode : produces  
    AiGate ||--o\{ Proof : requires  
    AiModule ||--o\{ AiRequest : handles
```

## 2. 实体说明

### 2.1 核心实体

| 实体 | 说明 |
| - | - |
| Project | 一个 GateLang 项目，包含多个源文件 |
| SourceFile | 源文件，包含电路和状态定义 |
| Module | 模块，属于某一层 |
| Layer | 抽象层，L1/L2/L3/L4 |
| Circuit | 组合逻辑模块，无状态 |
| State | 时序模块，包含 LATCH |
| Latch | 状态寄存器，声明宽度和初值 |
| Function | 函数，可组合或时序 |
| DomainBlock | 领域 DSL 块，如 `finance AMMPool` |
| Spec | 形式化规范，绑定到函数 |
| Type | 数据类型，如 `Bit`、`Bits\<8\>` |
| Signal | 信号线，承载 0/1 向量 |
| BitVector | 0/1 序列，信号的底层表示 |
| Resource | 资源消耗，门数/深度/周期/LATCH 数 |
| Compilation | 一次编译过程 |
| Backend | 编译后端 |
| Target | 生成的目标代码 |
| Proof | 形式化验证证明 |
| LibraryPackage | 标准库包 |


### 2.2 AI 辅助实体（v2.1 新增）

| 实体 | 说明 |
| - | - |
| AiModule | AI 辅助模块，如代码生成、优化、规范生成 |
| AiRequest | 一次 AI 请求，包含输入描述、目标层、约束条件 |
| AiCandidate | AI 生成的候选代码，包含源码、置信度、推理说明 |
| ValidatedCode | 通过 `gatelang-ai-gate` 验证的代码 |
| AiGate | AI 验证网关，强制执行类型检查、资源验证、形式化验证 |


## 3. 实体属性

### 3.1 Signal 与 BitVector

| 实体 | 属性 | 类型 | 说明 |
| - | - | - | - |
| Signal | id | SignalId | 唯一标识 |
| Signal | width | usize | 位宽，1 表示 `Bit` |
| Signal | kind | SignalKind | Input/Output/Internal/Latch |
| BitVector | bits | Vec | 0/1 序列 |
| BitVector | width | usize | 位宽 |


### 3.2 AiRequest 与 AiCandidate

| 实体 | 属性 | 类型 | 说明 |
| - | - | - | - |
| AiRequest | id | RequestId | 唯一标识 |
| AiRequest | description | String | 自然语言描述 |
| AiRequest | target\_layer | Layer | 目标抽象层 |
| AiRequest | constraints | Vec | 资源约束、规范约束 |
| AiCandidate | id | CandidateId | 唯一标识 |
| AiCandidate | source | String | 候选 GateLang 代码 |
| AiCandidate | confidence | f32 | 置信度 |
| AiCandidate | reasoning | String | 推理说明 |
| AiCandidate | status | CandidateStatus | Pending/Validated/Rejected |
| ValidatedCode | source | String | 验证通过的代码 |
| ValidatedCode | resources | ResourceReport | 资源报告 |
| ValidatedCode | proof | Option | 形式化验证证明 |


### 3.3 AiGate 验证流程

| 步骤 | 工具 | 失败处理 |
| - | - | - |
| 语法检查 | `gatelang-parser` | 拒绝，返回错误 |
| 类型检查 | `gatelang-types` | 拒绝，返回错误 |
| 资源验证 | `gatelang-resource` | 拒绝，返回错误 |
| 规范验证 | `gateproof` | 拒绝，返回反例 |
| 等价性检查（优化时） | `gateproof` | 拒绝，返回差异 |


# GateLang v2.1 软件开发工程文档（完全版）

**项目**：GateLang  
**版本**：v2.1  
**发布方**：FunctionComplete  
**日期**：2026年9月  
**开发模式**：模块化、分层解耦、可独立测试与集成

## 1. 概述

GateLang 是一门以 NAND 门为唯一组合原语、以 LATCH 为唯一状态原语的门级可验证计算语言。v2.1 在 v2.0 四层抽象模型基础上，新增 **0/1 二进制基础**和 **AI 辅助开发**两大技术方向。

本文档定义 GateLang v2.1 的模块化开发架构，包括模块划分、接口契约、依赖关系、开发顺序、测试策略、构建发布与协作规范。

## 2. 总体架构

```
┌─────────────────────────────────────────────────────────────────────┐  
│                          用户界面层                                   │  
│  gatelangc (CLI)  ·  gatecanvas (可视化)  ·  IDE 插件 (规划中)        │  
└───────────────────────────────┬─────────────────────────────────────┘  
                                │  
┌───────────────────────────────▼─────────────────────────────────────┐  
│                          AI 辅助层                                    │  
│  gatelang-ai-gen · gatelang-ai-opt · gatelang-ai-spec                │  
│  gatelang-ai-tutor · gatelang-ai-gate                                 │  
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

### 3.1 核心编译器模块

| 模块 | Crate 名称 | 职责 | 依赖 |
| - | - | - | - |
| 词法分析 | `gatelang-lexer` | Token 流，识别 0/1 字面量 | 无 |
| 语法分析 | `gatelang-parser` | 统一 AST | `gatelang-lexer`、`gatelang-ast` |
| AST 定义 | `gatelang-ast` | AST 节点，含 `Bit`/`Bits\<N\>` 字面量 | 无 |
| 类型系统 | `gatelang-types` | 类型检查、状态分离、位宽推导 | `gatelang-ast` |
| 资源计算 | `gatelang-resource` | 门数、深度、周期数 | `gatelang-ast`、`gatelang-types` |
| 规范验证 | `gatelang-spec` | `spec` 块解析，Z3/NuSMV | `gatelang-ast`、`gatelang-types` |
| 分层展开 | `gatelang-lower` | L4→L3→L2→L1 | `gatelang-ast`、`gatelang-types` |
| 中间表示 | `gatelang-ir` | NAND/LATCH 网表，0/1 信号线 | 无 |
| 编译驱动 | `gatelang-driver` | 串联前端、中端、后端 | 所有核心模块 |


### 3.2 AI 辅助模块

| 模块 | Crate 名称 | 职责 | 依赖 |
| - | - | - | - |
| AI 代码生成 | `gatelang-ai-gen` | 自然语言→候选 GateLang 代码 | `gatelang-driver`、LLM API |
| AI 门级优化 | `gatelang-ai-opt` | NAND 网表优化 | `gatelang-ir`、`gatelang-driver` |
| AI 规范生成 | `gatelang-ai-spec` | 生成 `spec` 块 | `gatelang-ast`、LLM API |
| AI 教学助手 | `gatelang-ai-tutor` | 教学辅助、错误解释 | `gatelang-ast`、`gatesim` |
| AI 验证网关 | `gatelang-ai-gate` | 强制验证 AI 生成代码 | `gatelang-driver`、`gateproof` |


### 3.3 编译器后端模块

| 模块 | Crate 名称 | 职责 | 依赖 |
| - | - | - | - |
| 模拟器后端 | `gatelang-backend-sim` | 可执行模拟代码 | `gatelang-ir` |
| Verilog 后端 | `gatelang-backend-verilog` | Verilog/VHDL | `gatelang-ir` |
| ZK 后端 | `gatelang-backend-zk` | Circom/Noir | `gatelang-ir` |
| TEE 后端 | `gatelang-backend-tee` | TEE 可执行代码 | `gatelang-ir` |
| C 后端 | `gatelang-backend-c` | 嵌入式 C 代码 | `gatelang-ir` |


### 3.4 工具链模块

| 模块 | Crate 名称 | 职责 | 依赖 |
| - | - | - | - |
| CLI | `gatelang-cli` | 命令行入口 | `gatelang-driver`、所有后端 |
| 模拟器 | `gatesim` | 门级仿真、波形输出 | `gatelang-ir` |
| 形式化验证 | `gateproof` | Z3/NuSMV 集成 | `gatelang-spec`、`gatelang-ir` |
| 包管理器 | `gatepm` | 函数库发布、依赖解析 | `gatelang-driver` |
| 可视化编辑器 | `gatecanvas` | 拖拽画布、实时仿真 | `gatesim`、`gatelang-driver`、`gatelang-ai-tutor` |


### 3.5 标准库模块

| 模块 | 类型 | 内容 |
| - | - | - |
| `stdlib-L1` | GateLang 源码 | 半加器、全加器、4位加法器、MUX、基本逻辑门、SR LATCH |
| `stdlib-L2` | GateLang 源码 | 安全算术、类型转换、错误处理 |
| `stdlib-L3` | GateLang 源码 | 金融 DSL、AI DSL、游戏 DSL 基础库 |
| `stdlib-verify` | 证明文件 | 每个标准库函数的形式化验证证明 |


## 4. 0/1 二进制基础对编译器实现的影响

### 4.1 词法层

`gatelang-lexer` 识别 0/1 字面量：

| 字面量 | 含义 | 示例 |
| - | - | - |
| `0`、`1` | 单位 `Bit` | `latch q: Bit = 0` |
| `0b0101` | 二进制 `Bits\<N\>` | `latch value: Bits\<4\> = 0b0101` |
| `0x1F` | 十六进制 `Bits\<N\>` | `latch mask: Bits\<8\> = 0xFF` |


### 4.2 语法层

支持位宽推导语法：

```
latch q: Bit = 0  
latch value: Bits\<8\> = 0  
latch pattern: Bits\<4\> = 0b1010
```

支持 NAND 和 LATCH 原语语法：

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

`gatelang-types` 实现位宽推导与 0/1 常量折叠：

- `Bit` 是 `Bits\<1\>` 的别名。

- `Bits\<N\>` 的常量在编译期折叠为 0/1 序列。

- 位宽不匹配时编译错误。

### 4.4 IR 层

`gatelang-ir` 的网表以 0/1 信号线为基础：

```
pub struct Signal \{  
    pub id: SignalId,  
    pub width: usize,  
    pub kind: SignalKind,  
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
    pub initial\_value: BitVector,  
\}
```

### 4.5 模拟器层

`gatesim` 以 0/1 向量表示信号状态：

```
pub struct SimulationState \{  
    pub signals: Vec\<BitVector\>,  
    pub cycle: u64,  
\}
```

### 4.6 验证层

`gateproof` 将 NAND/LATCH 网表建模为布尔变量，0/1 是唯一取值域。Z3 天然支持布尔逻辑，验证精确。

## 5. AI 辅助开发模块详细设计

### 5.1 AI 代码生成（`gatelang-ai-gen`）

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

**流程**：

```
原始网表 → AI 优化方案 → 新网表 → 形式化等价性检查 → 接受/拒绝
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

**流程**：

```
函数签名 + 描述 → LLM 生成 spec → gatelang-spec 解析 → gateproof 验证
```

### 5.4 AI 教学助手（`gatelang-ai-tutor`）

**集成**：`gatecanvas` 可视化层，提供错误解释、修改建议、练习题。

### 5.5 AI 验证网关（`gatelang-ai-gate`）

**核心原则**：AI 生成的一切代码必须经过强制验证。

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

### 6.5 AI 辅助接口

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
| `gatelang-lexer` | Token 流、0/1 字面量 | 与 parser 集成 | — | — |
| `gatelang-parser` | AST 结构 | 与 types 集成 | — | — |
| `gatelang-types` | 类型检查、位宽推导 | 与 resource 集成 | 类型系统不变式 | — |
| `gatelang-resource` | 门数/深度计算 | 与 lower 集成 | 资源上界准确性 | — |
| `gatelang-spec` | 规范解析 | 与 gateproof 集成 | — | Z3 验证 |
| `gatelang-lower` | 逐层展开 | 与 IR 集成 | 分层一致性 | 等价性检查 |
| `gatelang-ir` | 网表结构、0/1 信号 | 与后端集成 | — | — |
| `backend-\*` | 代码生成 | 端到端编译 | — | — |
| `gatesim` | 仿真正确性 | 示例电路 | — | — |
| `gateproof` | 验证器 | 标准库证明 | — | — |
| `gatelang-ai-gen` | 生成接口 | 与 ai-gate 集成 | 生成代码类型正确率 | — |
| `gatelang-ai-opt` | 优化接口 | 与等价性检查集成 | 优化后资源改善 | 等价性检查 |
| `gatelang-ai-spec` | 规范生成接口 | 与 gateproof 集成 | — | Z3 验证 |
| `gatelang-ai-tutor` | 教学接口 | 与 gatecanvas 集成 | — | — |
| `gatelang-ai-gate` | 验证流程 | 全链路集成 | — | 强制验证 |
| `gatecanvas` | UI 组件 | 完整工作流 | — | — |


**AI 模块特殊测试要求**：

- AI 生成的每一份代码必须经过 `gatelang-ai-gate` 完整验证。

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

### 10.4 AI 模块特殊规范

- AI 模块不得直接修改编译器核心数据，必须通过 `gatelang-ai-gate` 验证。

- AI 生成的所有代码必须附带置信度和推理说明。

- AI 优化必须提供等价性证明，否则拒绝。

- AI 模块的失败必须是安全的（fail-safe），不得产生未验证的代码。

- AI 模块支持多 LLM 后端，避免单一供应商锁定。

## 11. 风险与缓解

| 风险 | 影响 | 缓解 |
| - | - | - |
| 模块接口不稳定 | 并行开发受阻 | 早期定义核心 trait，RFC 流程 |
| 分层展开语义偏差 | 核心价值受损 | 形式化等价性检查，逐层验证 |
| 后端开发工作量大 | 延迟发布 | 按优先级：sim → verilog → zk → tee |
| 形式化验证集成复杂 | 延迟 | Z3 作为可选依赖，先支持简单规范 |
| 资源计算跨层不准确 | 失去可预测性 | 属性测试覆盖各层展开 |
| 可视化编辑器复杂 | 范围蔓延 | v0.1 仅原型，后续迭代 |
| AI 生成错误代码 | 安全风险 | 强制 `gatelang-ai-gate` 验证 |
| AI 优化破坏语义 | 核心价值受损 | 强制等价性检查 |
| AI 规范生成不完整 | 验证漏洞 | `gateproof` 兜底，人工复核 |
| AI 模块依赖外部 LLM | 供应链风险 | 抽象接口，支持多 LLM 后端 |


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

- Rust API 指南

- Z3 SMT Solver 文档

- mdBook 文档工具


**FunctionComplete · GateLang v2.1 PRD · ERD · 软件开发工程文档 · 2026**

*NAND 是唯一的组合原语。LATCH 是唯一的状态原语。0/1 是唯一的底层语言。AI 是加速器，形式化验证是保证者。模块化让所有人皆可参与构建。*

