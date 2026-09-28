//! 递归下降解析器：Token 流 → AST。
//!
//! 语法子集（对应白皮书 v2.1 §4 抽象模型 + §5 类型）：
//! - `circuit Name(p: T, ...) -> (r: T, ...) bound* { stmt* }`
//!   其中 T = Bit | Bits<N>；bound = gates: Gates<N> / depth: Depth<N> / cycles: Cycles<N>
//! - `state Name { latch v: T = init; fn f(...) -> (...) { stmt* } }`
//! - `spec Name { precondition: ...; postcondition: ...; invariant: ...; edge_cases: [...] }`
//! - 语句：赋值（含 <- 时序更新）、return、if/else
//! - 表达式：字面量、变量、位索引 x[i]、切片 x[a..b]、拼接 [a,b]、门调用 NAND(a,b)、
//!   NOT/AND/OR/XOR/NAND 关键字函数、算术 +、比较 == != 、二元逻辑、三元 if。

use crate::ast::*;
use crate::lexer::{tokenize, Tok, Token};

pub struct Parser {
    toks: Vec<Token>,
    pos: usize,
    depth: usize,
}

type PResult<T> = Result<T, String>;

impl Parser {
    pub fn new(src: &str) -> Result<Self, String> {
        Ok(Parser { toks: tokenize(src)?, pos: 0, depth: 0 })
    }

    fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }

    #[allow(dead_code)]

    fn peek_at(&self, n: usize) -> &Tok {
        let i = (self.pos + n).min(self.toks.len() - 1);
        &self.toks[i].tok
    }

    fn span(&self) -> Span {
        self.toks[self.pos].span
    }

    fn bump(&mut self) -> Tok {
        let t = self.toks[self.pos].tok.clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn eat(&mut self, t: &Tok) -> bool {
        if self.peek() == t {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, t: &Tok, what: &str) -> PResult<()> {
        if self.eat(t) {
            Ok(())
        } else {
            Err(format!("{:?}: 期望 {what}，实际 {:?}", self.span(), self.peek()))
        }
    }

    fn expect_ident(&mut self) -> PResult<(String, Span)> {
        match self.peek().clone() {
            Tok::Ident(s) => {
                let sp = self.span();
                self.bump();
                Ok((s, sp))
            }
            other => Err(format!("{:?}: 期望 标识符，实际 {other:?}", self.span())),
        }
    }

    // ---- 顶层 ----

    pub fn parse_program(src: &str) -> PResult<Vec<Decl>> {
        let mut p = Parser::new(src)?;
        // 源文件 token 上限：AST 规模（进而递归 Drop / eval 深度）有界，
        // 防止超深 AST 在 drop 时栈溢出（不可捕获的 abort）。
        if p.toks.len() > 8_000 {
            return Err(format!("源文件过大（token {} > 8000）", p.toks.len()));
        }
        let mut decls = Vec::new();
        while *p.peek() != Tok::Eof {
            decls.push(p.parse_decl()?);
        }
        Ok(decls)
    }

    fn parse_decl(&mut self) -> PResult<Decl> {
        match self.peek().clone() {
            Tok::Circuit => {
                self.bump();
                Ok(Decl::Circuit(self.parse_circuit()?))
            }
            Tok::State => {
                self.bump();
                Ok(Decl::State(self.parse_state()?))
            }
            Tok::Spec => {
                self.bump();
                Ok(Decl::Spec(self.parse_spec()?))
            }
            other => Err(format!("{:?}: 顶层只能有 circuit/state/spec，实际 {other:?}", self.span())),
        }
    }

    // ---- circuit ----

    fn parse_width(&mut self) -> PResult<Width> {
        let sp = self.span();
        match self.expect_ident()?.0.as_str() {
            "Bit" => Ok(Width::Bit),
            "Bits" => {
                self.expect(&Tok::Lt, "<")?;
                let n = match self.bump() {
                    Tok::Uint(v, _) => {
                        if v > u32::MAX as u128 {
                            return Err(format!("{sp:?}: Bits<N> 宽度超出 u32: {v}"));
                        }
                        if v == 0 {
                            return Err(format!("{sp:?}: Bits<0> 无意义"));
                        }
                        v as u32
                    }
                    other => return Err(format!("{sp:?}: Bits<N> 期望宽度数字，实际 {other:?}")),
                };
                self.expect(&Tok::Gt, ">")?;
                Ok(Width::Bits(n))
            }
            other => Err(format!("{sp:?}: 未知类型 {other}（支持 Bit / Bits<N>）")),
        }
    }

    fn parse_param(&mut self) -> PResult<Param> {
        let sp = self.span();
        let (name, _) = self.expect_ident()?;
        self.expect(&Tok::Colon, ":")?;
        let width = self.parse_width()?;
        Ok(Param { name, width, span: sp })
    }

    fn parse_params(&mut self) -> PResult<Vec<Param>> {
        self.expect(&Tok::LParen, "(")?;
        let mut ps = Vec::new();
        if !self.eat(&Tok::RParen) {
            loop {
                ps.push(self.parse_param()?);
                if !self.eat(&Tok::Comma) {
                    break;
                }
            }
            self.expect(&Tok::RParen, ")")?;
        }
        Ok(ps)
    }

    fn parse_bounds(&mut self) -> PResult<Vec<ResourceBound>> {
        let mut bs = Vec::new();
        loop {
            match self.peek().clone() {
                Tok::Ident(s) => match s.as_str() {
"gates" => {
                                self.bump();
                                self.expect(&Tok::Colon, ":")?;
                                // Gates<
                                match self.expect_ident()?.0.as_str() {
                                    "Gates" => {}
                                    other => return Err(format!("{:?}: 期望 Gates，实际 {other}", self.span())),
                                }
                                self.expect(&Tok::Lt, "<")?;
                                let n = match self.bump() {
                                    Tok::Uint(v, _) => {
                                        if v > u32::MAX as u128 {
                                            return Err(format!("{:?}: 资源上界超出 u32: {v}", self.span()));
                                        }
                                        v as u32
                                    }
                                    other => return Err(format!("{:?}: Gates<N> 期望数字，实际 {other:?}", self.span())),
                                };
                                self.expect(&Tok::Gt, ">")?;
                                bs.push(ResourceBound::Gates(n));
                            }
                            "depth" => {
                                self.bump();
                                self.expect(&Tok::Colon, ":")?;
                                match self.expect_ident()?.0.as_str() {
                                    "Depth" => {}
                                    other => return Err(format!("{:?}: 期望 Depth，实际 {other}", self.span())),
                                }
                                self.expect(&Tok::Lt, "<")?;
                                let n = match self.bump() {
                                    Tok::Uint(v, _) => {
                                        if v > u32::MAX as u128 {
                                            return Err(format!("{:?}: 资源上界超出 u32: {v}", self.span()));
                                        }
                                        v as u32
                                    }
                                    other => return Err(format!("{:?}: Depth<N> 期望数字，实际 {other:?}", self.span())),
                                };
                                self.expect(&Tok::Gt, ">")?;
                                bs.push(ResourceBound::Depth(n));
                            }
                            "cycles" => {
                                self.bump();
                                self.expect(&Tok::Colon, ":")?;
                                match self.expect_ident()?.0.as_str() {
                                    "Cycles" => {}
                                    other => return Err(format!("{:?}: 期望 Cycles，实际 {other}", self.span())),
                                }
                                self.expect(&Tok::Lt, "<")?;
                                let n = match self.bump() {
                                    Tok::Uint(v, _) => {
                                        if v > u32::MAX as u128 {
                                            return Err(format!("{:?}: 资源上界超出 u32: {v}", self.span()));
                                        }
                                        v as u32
                                    }
                                    other => return Err(format!("{:?}: Cycles<N> 期望数字，实际 {other:?}", self.span())),
                                };
                                self.expect(&Tok::Gt, ">")?;
                                bs.push(ResourceBound::Cycles(n));
                            }
                    _ => break,
                },
                Tok::LCurly => break,
                _ => break,
            }
        }
        Ok(bs)
    }

    fn parse_returns(&mut self) -> PResult<Vec<Param>> {
        // 支持 -> (r: T, ...) 或 -> T（无名单返回）
        if *self.peek() == Tok::LParen {
            return self.parse_params();
        }
        // 基值判定：Bit / Bits<N> 是无名返回；ident: 是命名返回
        if let Some(t) = self.toks.get(self.pos + 1) {
            if t.tok == Tok::Colon {
                return Ok(vec![self.parse_param()?]);
            }
        }
        let sp = self.span();
        let width = self.parse_width()?;
        Ok(vec![Param { name: "out".into(), width, span: sp }])
    }

    fn parse_circuit(&mut self) -> PResult<Circuit> {
        let sp = self.span();
        let (name, _) = self.expect_ident()?;
        let params = self.parse_params()?;
        self.expect(&Tok::Arrow, "->")?;
        let returns = self.parse_returns()?;
        let bounds = self.parse_bounds()?;
        let body = self.parse_block()?;
        Ok(Circuit { name, params, returns, bounds, body, span: sp })
    }

    fn parse_state(&mut self) -> PResult<State> {
        let sp = self.span();
        let (name, _) = self.expect_ident()?;
        self.expect(&Tok::LCurly, "{")?;
        let mut latches = Vec::new();
        let mut fns = Vec::new();
        loop {
            match self.peek().clone() {
                Tok::RCurly => {
                    self.bump();
                    break;
                }
                Tok::Latch => {
                    self.bump();
                    latches.push(self.parse_latch()?);
                }
                Tok::Fn => {
                    self.bump();
                    fns.push(self.parse_state_fn()?);
                }
                other => {
                    return Err(format!("{:?}: state 体内只允许 latch/fn，实际 {other:?}", self.span()));
                }
            }
        }
        Ok(State { name, latches, fns, span: sp })
    }

    fn parse_latch(&mut self) -> PResult<LatchDecl> {
        let sp = self.span();
        let (name, _) = self.expect_ident()?;
        self.expect(&Tok::Colon, ":")?;
        let width = self.parse_width()?;
        self.expect(&Tok::Assign, "=")?;
        let init = match self.bump() {
            Tok::Uint(v, _) => v,
            other => return Err(format!("{sp:?}: latch 初始值期望数字，实际 {other:?}")),
        };
        self.expect(&Tok::Semi, ";")?;
        Ok(LatchDecl { name, width, init, span: sp })
    }

    fn parse_state_fn(&mut self) -> PResult<StateFn> {
        let sp = self.span();
        let (name, _) = self.expect_ident()?;
        let params = self.parse_params()?;
        self.expect(&Tok::Arrow, "->")?;
        let returns = self.parse_returns()?;
        let body = self.parse_block()?;
        Ok(StateFn { name, params, returns, body, span: sp })
    }

    fn parse_block(&mut self) -> PResult<Vec<Stmt>> {
        self.expect(&Tok::LCurly, "{")?;
        let mut stmts = Vec::new();
        while *self.peek() != Tok::RCurly && *self.peek() != Tok::Eof {
            stmts.push(self.parse_stmt()?);
        }
        self.expect(&Tok::RCurly, "}")?;
        Ok(stmts)
    }

    fn parse_stmt(&mut self) -> PResult<Stmt> {
        match self.peek().clone() {
            Tok::Return => {
                self.bump();
                let e = if self.peek() == &Tok::LParen {
                    // return (e1, e2, ...) 多输出
                    self.bump();
                    let mut items = Vec::new();
                    items.push(self.parse_expr()?);
                    while self.eat(&Tok::Comma) {
                        items.push(self.parse_expr()?);
                    }
                    self.expect(&Tok::RParen, ")")?;
                    items
                } else {
                    vec![self.parse_expr()?]
                };
                self.expect(&Tok::Semi, ";")?;
                Ok(Stmt::Return(e))
            }
            Tok::If => self.parse_if_stmt(),
            _ => self.parse_assign(),
        }
    }

    fn parse_if_stmt(&mut self) -> PResult<Stmt> {
        // if 也是递归入口（块内可再嵌 if），须计入深度。
        self.depth += 1;
        if self.depth > 128 {
            self.depth -= 1;
            return Err(format!("{:?}: if 嵌套过深（>128）", self.span()));
        }
        let sp = self.span();
        self.bump(); // if
        let cond = self.parse_expr()?;
        let then_body = self.parse_block()?;
        let mut else_body = Vec::new();
        if self.eat(&Tok::Else) {
            else_body = self.parse_block()?;
        }
        self.depth -= 1;
        Ok(Stmt::If(Box::new(IfStmt { cond, then_body, else_body, span: sp })))
    }

    fn parse_assign(&mut self) -> PResult<Stmt> {
        let sp = self.span();
        // 目标列表：要么单个 Target，要么 (t1, t2, ...)
        let mut targets = Vec::new();
        if self.eat(&Tok::LParen) {
            loop {
                targets.push(self.parse_target()?);
                if !self.eat(&Tok::Comma) {
                    break;
                }
            }
            self.expect(&Tok::RParen, ")")?;
        } else {
            targets.push(self.parse_target()?);
        }
        // 运算符：= 或 <-（时序更新）
        let is_update = match self.peek().clone() {
            Tok::Assign => {
                self.bump();
                false
            }
            Tok::Update => {
                self.bump();
                true
            }
            other => return Err(format!("{sp:?}: 赋值期望 = 或 <-，实际 {other:?}")),
        };
        let value = self.parse_expr()?;
        self.expect(&Tok::Semi, ";")?;
        if is_update {
            // <- 只允许在 state fn 中发生（组合函数里禁止，由 lower 校验）
            Ok(Stmt::Assign(Assign {
                targets: targets.iter().map(|t| t.clone()).collect(),
                value: Expr::Call(
                    "__UPDATE".into(),
                    vec![value],
                    sp,
                ),
                span: sp,
            }))
        } else {
            Ok(Stmt::Assign(Assign { targets: targets.clone(), value, span: sp }))
        }
    }

    fn parse_target(&mut self) -> PResult<Target> {
        let sp = self.span();
        match self.peek().clone() {
            Tok::Ident(s) => {
                self.bump();
                // 检查 [i] 或 [a..b]
                if self.eat(&Tok::LBracket) {
                    if let Tok::Uint(a, _) = self.bump() {
                        if self.eat(&Tok::DotDot) {
                            let b = match self.bump() {
                                Tok::Uint(v, _) => v as usize,
                                other => return Err(format!("{sp:?}: 期望切片上界，实际 {other:?}")),
                            };
                            self.expect(&Tok::RBracket, "]")?;
                            return Ok(Target::Slice(Box::new(Target::Var(s, sp)), a as usize, b, sp));
                        }
                        self.expect(&Tok::RBracket, "]")?;
                        return Ok(Target::Index(Box::new(Target::Var(s, sp)), a as usize, sp));
                    }
                    return Err(format!("{sp:?}: 期望索引数字"));
                }
                Ok(Target::Var(s, sp))
            }
            other => Err(format!("{sp:?}: 期望赋值目标，实际 {other:?}")),
        }
    }

    // ---- spec ----

    fn parse_spec(&mut self) -> PResult<Spec> {
        let sp = self.span();
        // 名称支持点号命名空间如 FCT.Math.add
        let mut name = String::new();
        let (first, _) = self.expect_ident()?;
        name.push_str(&first);
        while self.eat(&Tok::Dot) {
            let (seg, _) = self.expect_ident()?;
            name.push('.');
            name.push_str(&seg);
        }
        self.expect(&Tok::LCurly, "{")?;
        let mut pre = None;
        let mut post = None;
        let mut invariant = None;
        let mut edge_cases = Vec::new();
        loop {
            match self.peek().clone() {
                Tok::RCurly => {
                    self.bump();
                    break;
                }
                Tok::Pre => {
                    self.bump();
                    self.expect(&Tok::Colon, ":")?;
                    pre = Some(self.parse_expr_to_semi()?);
                }
                Tok::Post => {
                    self.bump();
                    self.expect(&Tok::Colon, ":")?;
                    post = Some(self.parse_expr_to_semi()?);
                }
                Tok::Invariant => {
                    self.bump();
                    self.expect(&Tok::Colon, ":")?;
                    invariant = Some(self.parse_expr_to_semi()?);
                }
                Tok::EdgeCases => {
                    self.bump();
                    self.expect(&Tok::Colon, ":")?;
                    self.expect(&Tok::LBracket, "[")?;
                    while *self.peek() != Tok::RBracket && *self.peek() != Tok::Eof {
                        edge_cases.push(self.parse_expr_to_comma()?);
                    }
                    self.expect(&Tok::RBracket, "]")?;
                    self.eat(&Tok::Semi);
                }
                other => return Err(format!("{:?}: spec 体内只允许 precondition/postcondition/invariant/edge_cases，实际 {other:?}", self.span())),
            }
        }
        Ok(Spec { name, pre, post, invariant, edge_cases, span: sp })
    }

    /// 解析到分号的表达式，返回其源码文本（spec 以文本保存，供验证器求值）。
    fn parse_expr_to_semi(&mut self) -> PResult<String> {
        let start = self.pos;
        let mut depth = 0i32;
        loop {
            match self.peek().clone() {
                Tok::Semi if depth == 0 => break,
                Tok::LParen | Tok::LBracket | Tok::LCurly => depth += 1,
                Tok::RParen | Tok::RBracket | Tok::RCurly => depth -= 1,
                Tok::Eof => break,
                _ => {}
            }
            self.bump();
        }
        self.expect(&Tok::Semi, ";")?;
        Ok(render_tokens(&self.toks[start..self.pos - 1]))
    }

    fn parse_expr_to_comma(&mut self) -> PResult<String> {
        let start = self.pos;
        let mut depth = 0i32;
        loop {
            match self.peek().clone() {
                Tok::Comma if depth == 0 => break,
                Tok::RBracket if depth == 0 => break,
                Tok::LParen | Tok::LBracket | Tok::LCurly => depth += 1,
                Tok::RParen | Tok::RBracket | Tok::RCurly => depth -= 1,
                Tok::Eof => break,
                _ => {}
            }
            self.bump();
        }
        let render = render_tokens(&self.toks[start..self.pos]);
        // 允许最后一个元素后无逗号（直接 ]）
        if !self.eat(&Tok::Comma) {
            if self.peek() == &Tok::RBracket {
                return Ok(render);
            }
            return Err(format!("{:?}: spec 列表期望逗号或 ]", self.span()));
        }
        Ok(render)
    }

    // ---- 表达式 ----

    fn parse_expr(&mut self) -> PResult<Expr> {
        // 递归深度上限：防止畸形输入（如数千层括号）导致栈溢出（不可捕获的 abort）。
        // 每层括号约耗 9 个栈帧，128 层对 2MB 线程栈安全，且远超真实表达式嵌套。
        self.depth += 1;
        if self.depth > 128 {
            self.depth -= 1;
            return Err(format!("{:?}: 表达式嵌套过深（>128）", self.span()));
        }
        let r = self.parse_ternary();
        self.depth -= 1;
        r
    }

    fn parse_ternary(&mut self) -> PResult<Expr> {
        let sp = self.span();
        let cond = self.parse_or()?;
        if self.eat(&Tok::If) {
            // if c { a } else { b }：三元？支持 if c { a } else { b } 作为表达式
            let then_body = self.parse_expr()?;
            self.expect(&Tok::Else, "else")?;
            let else_body = self.parse_expr()?;
            return Ok(Expr::Ternary(Box::new(cond), Box::new(then_body), Box::new(else_body), sp));
        }
        Ok(cond)
    }

    fn parse_or(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_xor()?;
        let mut n = 0u32;
        while self.eat(&Tok::Pipe) {
            n += 1;
            if n > 256 {
                return Err(format!("{:?}: 表达式运算项过多（>256）", self.span()));
            }
            let sp = self.span();
            let rhs = self.parse_xor()?;
            lhs = Expr::Bin(BinOp::Or, Box::new(lhs), Box::new(rhs), sp);
        }
        Ok(lhs)
    }

    /// 按位异或：优先级介于 `|` 与 `&` 之间（与 C 一致）。
    fn parse_xor(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_and()?;
        let mut n = 0u32;
        while self.eat(&Tok::Caret) {
            n += 1;
            if n > 256 {
                return Err(format!("{:?}: 表达式运算项过多（>256）", self.span()));
            }
            let sp = self.span();
            let rhs = self.parse_and()?;
            lhs = Expr::Bin(BinOp::Xor, Box::new(lhs), Box::new(rhs), sp);
        }
        Ok(lhs)
    }

    fn parse_and(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_cmp()?;
        let mut n = 0u32;
        while self.eat(&Tok::Amp) {
            n += 1;
            if n > 256 {
                return Err(format!("{:?}: 表达式运算项过多（>256）", self.span()));
            }
            let sp = self.span();
            let rhs = self.parse_cmp()?;
            lhs = Expr::Bin(BinOp::And, Box::new(lhs), Box::new(rhs), sp);
        }
        Ok(lhs)
    }

    fn parse_cmp(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_add()?;
        let mut n = 0u32;
        loop {
            let (op, sp) = match self.peek().clone() {
                Tok::EqEq => (BinOp::Eq, self.span()),
                Tok::Ne => (BinOp::Ne, self.span()),
                Tok::Lt => (BinOp::Lt, self.span()),
                Tok::Gt => (BinOp::Gt, self.span()),
                Tok::Le => (BinOp::Le, self.span()),
                Tok::Ge => (BinOp::Ge, self.span()),
                _ => break,
            };
            n += 1;
            if n > 256 {
                return Err(format!("{:?}: 表达式运算项过多（>256）", self.span()));
            }
            self.bump();
            let rhs = self.parse_add()?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs), sp);
        }
        Ok(lhs)
    }

    fn parse_add(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_unary()?;
        let mut n = 0u32;
        loop {
            let sp = self.span();
            let op = if self.eat(&Tok::Plus) {
                BinOp::Add
            } else if self.eat(&Tok::Minus) {
                BinOp::Sub
            } else {
                break;
            };
            n += 1;
            if n > 256 {
                return Err(format!("{:?}: 表达式运算项过多（>256）", self.span()));
            }
            let rhs = self.parse_unary()?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs), sp);
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> PResult<Expr> {
        if self.eat(&Tok::Bang) {
            // 一元 ! 也是递归入口，须计入深度（否则数千层 ! 会栈溢出 abort）。
            self.depth += 1;
            if self.depth > 128 {
                self.depth -= 1;
                return Err(format!("{:?}: 表达式嵌套过深（>128）", self.span()));
            }
            let sp = self.span();
            let e = self.parse_unary()?;
            self.depth -= 1;
            return Ok(Expr::Not(Box::new(e), sp));
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> PResult<Expr> {
        let mut e = self.parse_primary()?;
        let mut n = 0u32;
        loop {
            n += 1;
            if n > 256 {
                return Err(format!("{:?}: 下标/切片链过长（>256）", self.span()));
            }
            match self.peek().clone() {
                Tok::LBracket => {
                    self.bump();
                    if let Tok::Uint(i, _) = self.bump() {
                        if self.eat(&Tok::DotDot) {
                            let hi = match self.bump() {
                                Tok::Uint(v, _) => v as u32,
                                other => return Err(format!("{:?}: 切片期望上界，实际 {other:?}", self.span())),
                            };
                            self.expect(&Tok::RBracket, "]")?;
                            e = Expr::Slice(Box::new(e), i as u32, hi, self.span());
                        } else {
                            self.expect(&Tok::RBracket, "]")?;
                            e = Expr::Index(Box::new(e), i as u32, self.span());
                        }
                    } else {
                        return Err(format!("{:?}: 下标期望数字", self.span()));
                    }
                }
                _ => break,
            }
        }
        Ok(e)
    }

    fn parse_primary(&mut self) -> PResult<Expr> {
        let sp = self.span();
        match self.peek().clone() {
            Tok::Uint(v, raw) => {
                self.bump();
                // 字面量溢出 u128 时 lexer 会静默给 0 —— 在此显式报错。
                let checked = if raw.starts_with("0b") {
                    u128::from_str_radix(raw.trim_start_matches("0b"), 2)
                } else {
                    raw.parse::<u128>()
                };
                if checked.is_err() {
                    return Err(format!("{sp:?}: 数值字面量超出 u128: {raw}"));
                }
                // 0b101 → Bits<3>；整数 → 视上下文，这里标记 Bits（宽度由 raw 决定位数或 1 位）
                let w = if raw.starts_with("0b") {
                    Width::Bits(raw.len() as u32 - 2)
                } else if v <= 1 {
                    Width::Bit
                } else if v <= 0xFF {
                    // 十进制整数按 8 位（原型）
                    Width::Bits(8)
                } else {
                    // 超出 8 位会被静默截断（如 256→0），导致错误编译/假验证 → 明确报错
                    return Err(format!("{sp:?}: 十进制字面量 {raw} 超出 8 位；请用 0b… 指定位宽"));
                };
                Ok(Expr::Lit(v, w, sp))
            }
            Tok::Ident(s) => {
                self.bump();
                // 类似函数调用
                if self.eat(&Tok::LParen) {
                    let mut args = Vec::new();
                    if !self.eat(&Tok::RParen) {
                        loop {
                            args.push(self.parse_expr()?);
                            if !self.eat(&Tok::Comma) {
                                break;
                            }
                        }
                        self.expect(&Tok::RParen, ")")?;
                    }
                    Ok(Expr::Call(s, args, sp))
                } else {
                    Ok(Expr::Var(s, sp))
                }
            }
            Tok::LBracket => {
                self.bump();
                let mut items = Vec::new();
                if !self.eat(&Tok::RBracket) {
                    loop {
                        items.push(self.parse_expr()?);
                        if !self.eat(&Tok::Comma) {
                            break;
                        }
                    }
                    self.expect(&Tok::RBracket, "]")?;
                }
                Ok(Expr::Concat(items, sp))
            }
            Tok::LParen => {
                self.bump();
                let e = self.parse_expr()?;
                self.expect(&Tok::RParen, ")")?;
                Ok(e)
            }
            other => Err(format!("{sp:?}: 表达式起始意外 token {other:?}")),
        }
    }
}

/// 将 token 区间渲染为文本（spec 表达式保存用）。
fn render_tokens(toks: &[Token]) -> String {
    let mut s = String::new();
    for t in toks {
        match &t.tok {
            Tok::Ident(x) => s.push_str(x),
            Tok::Uint(_v, raw) => s.push_str(raw),
            Tok::Plus => s.push('+'),
            Tok::Minus => s.push('-'),
            Tok::Star => s.push('*'),
            Tok::Percent => s.push('%'),
            Tok::Amp => s.push('&'),
            Tok::Pipe => s.push('|'),
            Tok::AmpAmp => s.push_str("&&"),
            Tok::PipePipe => s.push_str("||"),
            Tok::Le => s.push_str("<="),
            Tok::Ge => s.push_str(">="),
            Tok::Caret => s.push('^'),
            Tok::Bang => s.push('!'),
            Tok::EqEq => s.push_str("=="),
            Tok::Ne => s.push_str("!="),
            Tok::Lt => s.push('<'),
            Tok::Gt => s.push('>'),
            Tok::LParen => s.push('('),
            Tok::RParen => s.push(')'),
            Tok::LBracket => s.push('['),
            Tok::RBracket => s.push(']'),
            Tok::Comma => s.push(','),
            Tok::Colon => s.push(':'),
            Tok::Semi => s.push(';'),
            Tok::Assign => s.push('='),
            Tok::Dot => s.push('.'),
            Tok::DotDot => s.push_str(".."),
            _ => s.push(' '),
        }
    }
    s
}

/// 便捷入口：源码 → 声明列表。
pub fn parse_program(src: &str) -> Result<Vec<Decl>, String> {
    Parser::parse_program(src)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_circuit() {
        let src = r#"circuit XOR(a: Bit, b: Bit) -> Bit {
            y = NAND(a, b);
            return y;
        }"#;
        let decls = parse_program(src).unwrap();
        assert_eq!(decls.len(), 1);
        match &decls[0] {
            Decl::Circuit(c) => {
                assert_eq!(c.name, "XOR");
                assert_eq!(c.params[0].name, "a");
                assert_eq!(c.params[0].width, Width::Bit);
                assert_eq!(c.returns.len(), 1);
            }
            _ => panic!("期望 circuit"),
        }
    }

    #[test]
    fn parse_bits_width_and_bounds() {
        let src = r#"circuit Adder8(a: Bits<8>, b: Bits<8>) -> (s: Bits<8>, c: Bit)
            gates: Gates<200> depth: Depth<20> cycles: Cycles<1>
        {
            s = a + b;
            return s;
        }"#;
        let decls = parse_program(src).unwrap();
        match &decls[0] {
            Decl::Circuit(c) => {
                assert_eq!(c.params[0].width, Width::Bits(8));
                assert_eq!(c.bounds.len(), 3);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn parse_state_with_latch() {
        let src = r#"state Counter {
            latch value: Bits<8> = 0;
            fn increment() -> Bits<8> {
                value <- value + 1;
                return value;
            }
        }"#;
        let decls = parse_program(src).unwrap();
        match &decls[0] {
            Decl::State(s) => {
                assert_eq!(s.latches.len(), 1);
                assert_eq!(s.latches[0].name, "value");
                assert_eq!(s.latches[0].init, 0);
                assert_eq!(s.fns.len(), 1);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn parse_spec() {
        let src = r#"spec FCT.Math.add {
            precondition: a + b <= 2^256 - 1;
            postcondition: result == a + b;
            invariant: no overflow;
            edge_cases: [a=0, b=0, a=MAX_UINT, b=MAX_UINT];
        }"#;
        let decls = parse_program(src).unwrap();
        match &decls[0] {
            Decl::Spec(s) => {
                assert!(s.pre.is_some());
                assert!(s.post.is_some());
                assert_eq!(s.edge_cases.len(), 4);
            }
            _ => panic!(),
        }
    }
}