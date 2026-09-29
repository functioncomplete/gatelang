//! GateLang — 基于 NAND/LATCH 的门级可验证计算语言。
//!
//! M5 原型实现（依据《GateLang 技术白皮书 v2.1》与《软件开发文档 v2.1》）：
//!
//! ```text
//! 源码 ──lexer──▶ Token ──parser──▶ AST ──type-check──▶ 类型化 AST
//!   ──lower(语义保持展开)──▶ NAND/LATCH 网表 IR ──resource──▶ 门数/深度/周期
//!   ──sim──▶ 位级模拟结果 ──spec──▶ 前/后置条件验证 ──equiv──▶ 语义等价检查
//! ```
//!
//! 核心形态：单 crate 多模块（对应《软件开发文档 v2.1》模块划分的合并原型；
//! 独立 crate 拆分见工作区结构文档）。所有代码零外部依赖。

pub mod ast;
pub mod cnf;
pub mod equiv;
pub mod fct;
pub mod lexer;
pub mod lower;
pub mod netlist;
pub mod parser;
pub mod prove;
pub mod resource;
pub mod sat;
pub mod sim;
pub mod spec;
pub mod ty;
pub mod u256;
pub mod verify;
pub mod word;

pub use parser::parse_program;