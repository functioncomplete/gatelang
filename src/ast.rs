//! GateLang 抽象语法树（AST）。

/// 来源位置
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub line: usize,
    pub col: usize,
}

impl Span {
    pub fn new(line: usize, col: usize) -> Self {
        Span { line, col }
    }
}

/// 宽类型（位宽），对应白皮书 §5 类型系统
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Width {
    Bit,
    Bits(u32),
}

impl Width {
    pub fn bits(&self) -> u32 {
        match self {
            Width::Bit => 1,
            Width::Bits(n) => *n,
        }
    }
}

/// 资源上界类型：Gates<G> / Depth<D> / Cycles<C>（白皮书 §5）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceBound {
    Gates(u32),
    Depth(u32),
    Cycles(u32),
}

/// 顶层声明
#[derive(Debug, Clone)]
pub enum Decl {
    /// 组合电路：circuit Name(...) -> (...) bound... { ... }
    Circuit(Circuit),
    /// 时序逻辑：state Name { latch ...; fn ... }
    State(State),
    /// 规范块：spec Name { ... }
    Spec(Spec),
}

/// circuit 声明（组合函数，无 LATCH）
#[derive(Debug, Clone)]
pub struct Circuit {
    pub name: String,
    pub params: Vec<Param>,
    pub returns: Vec<Param>, // 输出参数（命名 + 宽）
    pub bounds: Vec<ResourceBound>,
    pub body: Vec<Stmt>,
    pub span: Span,
}

/// state 声明（时序函数）
#[derive(Debug, Clone)]
pub struct State {
    pub name: String,
    pub latches: Vec<LatchDecl>,
    pub fns: Vec<StateFn>,
    pub span: Span,
}

/// latch 状态寄存器声明
#[derive(Debug, Clone)]
pub struct LatchDecl {
    pub name: String,
    pub width: Width,
    pub init: u128, // 初始值（bit 模式）
    pub span: Span,
}

/// state 内部函数（可读/写 latch）
#[derive(Debug, Clone)]
pub struct StateFn {
    pub name: String,
    pub params: Vec<Param>,
    pub returns: Vec<Param>,
    pub body: Vec<Stmt>,
    pub span: Span,
}

/// 参数（含名字与宽）
#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub width: Width,
    pub span: Span,
}

/// 语句
#[derive(Debug, Clone)]
pub enum Stmt {
    /// 赋值：lhs = expr（lhs 可为变量、索引、元组）
    Assign(Assign),
    /// 返回：return e | return (e1, e2, ...)（多输出）
    Return(Vec<Expr>),
    /// 声明的资源上界验证（Gates<G> 等，带真实值）
    AssertResource(String, ResourceBound, u32),
    /// 条件：if c { ... } else { ... }
    If(Box<IfStmt>),
}

/// 赋值目标
#[derive(Debug, Clone)]
pub struct Assign {
    pub targets: Vec<Target>, // 单目标或多目标（元组）
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum Target {
    Var(String, Span),
    /// 位索引：x[i]
    Index(Box<Target>, usize, Span),
    /// 位切片：x[a..b]（含 b）
    Slice(Box<Target>, usize, usize, Span),
}

#[derive(Debug, Clone)]
pub struct IfStmt {
    pub cond: Expr,
    pub then_body: Vec<Stmt>,
    pub else_body: Vec<Stmt>,
    pub span: Span,
}

/// 表达式
#[derive(Debug, Clone)]
pub enum Expr {
    /// 位字面量 / 位向量字面量：0b101 或 0 / 1
    Lit(u128, Width, Span),
    /// 变量引用
    Var(String, Span),
    /// 门调用 / 函数调用：Name(args)
    /// 支持：NAND/AND/OR/XOR/NOT（原语门）、用户 circuit / state fn 引用
    Call(String, Vec<Expr>, Span),
    /// 位索引 x[i]
    Index(Box<Expr>, u32, Span),
    /// 位切片 x[a..=b]
    Slice(Box<Expr>, u32, u32, Span),
    /// 拼接 [a, b, c]（高位在前）
    Concat(Vec<Expr>, Span),
    /// 一元 NOT：!e
    Not(Box<Expr>, Span),
    /// 二元：& | ^ NAND 语义可由基础门组合；算术 + 由加法器统一下沉
    Bin(BinOp, Box<Expr>, Box<Expr>, Span),
    /// 条件表达式 if c { a } else { b }
    // 复用 IfStmt？为简化在表达式层用 Ternary
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>, Span),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    And,
    Or,
    Xor,
    Add,
    Sub, // 减法（模 2^N，由单条进位链 a + ~b + 1 实现）
    Eq, // 比较（== 返回 Bit，可展开为 XNOR + AND）
    Ne,
    /// 无符号比较，均返回 1 位（Bit）
    Lt,
    Gt,
    Le,
    Ge,
}

/// 规范块：spec Name { precondition:... postcondition:... edge_cases:[...] invariant:... }
#[derive(Debug, Clone)]
pub struct Spec {
    pub name: String,
    pub pre: Option<String>,
    pub post: Option<String>,
    pub invariant: Option<String>,
    pub edge_cases: Vec<String>,
    pub span: Span,
}