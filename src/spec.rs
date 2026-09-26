//! spec 形式化语义支持（白皮书 §6.1）。编译后由验证器按表达式检查。
//!
//! 原型实现：内置一个小型布尔/算术表达式求值器，支持
//!  precondition / postcondition / invariant / edge_cases 文本。
//! 表达式语法：数字、标识符、+ - * <= >= == != < > && || ! ( )。
//! MAX_UINT 与 2^256 常量预绑定。

use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum SpecExpr {
    Num(u128),
    Var(String),
    Add(Box<SpecExpr>, Box<SpecExpr>),
    Sub(Box<SpecExpr>, Box<SpecExpr>),
    Mul(Box<SpecExpr>, Box<SpecExpr>),
    Mod(Box<SpecExpr>, Box<SpecExpr>),
    Le(Box<SpecExpr>, Box<SpecExpr>),
    Ge(Box<SpecExpr>, Box<SpecExpr>),
    Lt(Box<SpecExpr>, Box<SpecExpr>),
    Gt(Box<SpecExpr>, Box<SpecExpr>),
    Eq(Box<SpecExpr>, Box<SpecExpr>),
    Ne(Box<SpecExpr>, Box<SpecExpr>),
    And(Box<SpecExpr>, Box<SpecExpr>),
    Or(Box<SpecExpr>, Box<SpecExpr>),
    Not(Box<SpecExpr>),
}

/// 解析 spec 表达式。
pub fn parse_spec(text: &str) -> Result<SpecExpr, String> {
    let toks = tokenize_spec(text)?;
    // token 总数上限：既防止超长表达式，也保证 AST 规模有界
    // （否则深层 Box 链在 eval / drop 时递归溢出栈）。
    if toks.len() > 2000 {
        return Err(format!("spec: 表达式过长（token {} > 2000）", toks.len()));
    }
    let mut p = SpecParser { toks, pos: 0, depth: 0 };
    let e = p.parse_or()?;
    // 必须消费全部 token：否则不支持的运算符/字面量会被静默丢弃，
    // 导致 postcondition 被削弱成恒真（例如 `y == a ^ b` 被截成 `y == a`）。
    if p.pos != p.toks.len() {
        return Err(format!(
            "spec: 表达式后存在多余 token（疑似不支持的运算符或字面量）: {:?}",
            p.toks.get(p.pos)
        ));
    }
    Ok(e)
}

/// spec 表达式求值。env 提供变量值。
pub fn eval_spec(e: &SpecExpr, env: &HashMap<String, u128>) -> Result<u128, String> {
    match e {
        SpecExpr::Num(n) => Ok(*n),
        SpecExpr::Var(name) => env
            .get(name)
            .copied()
            .ok_or_else(|| format!("spec 变量未绑定: {name}")),
        SpecExpr::Add(a, b) => Ok(eval_spec(a, env)?.wrapping_add(eval_spec(b, env)?)),
        SpecExpr::Sub(a, b) => Ok(eval_spec(a, env)?.wrapping_sub(eval_spec(b, env)?)),
        SpecExpr::Mul(a, b) => Ok(eval_spec(a, env)?.wrapping_mul(eval_spec(b, env)?)),
        SpecExpr::Mod(a, b) => {
            let d = eval_spec(b, env)?;
            if d == 0 { Ok(0) } else { Ok(eval_spec(a, env)? % d) }
        }
        SpecExpr::Le(a, b) => Ok((eval_spec(a, env)? <= eval_spec(b, env)?) as u128),
        SpecExpr::Ge(a, b) => Ok((eval_spec(a, env)? >= eval_spec(b, env)?) as u128),
        SpecExpr::Lt(a, b) => Ok((eval_spec(a, env)? < eval_spec(b, env)?) as u128),
        SpecExpr::Gt(a, b) => Ok((eval_spec(a, env)? > eval_spec(b, env)?) as u128),
        SpecExpr::Eq(a, b) => Ok((eval_spec(a, env)? == eval_spec(b, env)?) as u128),
        SpecExpr::Ne(a, b) => Ok((eval_spec(a, env)? != eval_spec(b, env)?) as u128),
        SpecExpr::And(a, b) => Ok(((eval_spec(a, env)? != 0) && (eval_spec(b, env)? != 0)) as u128),
        SpecExpr::Or(a, b) => Ok(((eval_spec(a, env)? != 0) || (eval_spec(b, env)? != 0)) as u128),
        SpecExpr::Not(a) => Ok((eval_spec(a, env)? == 0) as u128),
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Spt {
    Num(u128),
    Ident(String),
    Op(String),
    LParen,
    RParen,
}

fn tokenize_spec(src: &str) -> Result<Vec<Spt>, String> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i] as char;
        match c {
            ' ' | '\t' | '\n' => i += 1,
            '0'..='9' => {
                let start = i;
                while i < b.len() && (b[i] as char).is_ascii_digit() {
                    i += 1;
                }
                let mut v: u128 = match src[start..i].parse() {
                    Ok(x) => x,
                    // 溢出 u128 时明确报错（此前 unwrap_or(0) 会静默变 0 → 假通过）
                    Err(_) => return Err(format!("spec: 数值字面量超出 u128: {}", &src[start..i])),
                };
                // 处理 a^b 幂（原型仅支持 2^N / MAX_UINT）
                if i + 1 < b.len() && b[i] == b'^' {
                    // 形如 2^256
                    i += 1;
                    let start2 = i;
                    while i < b.len() && (b[i] as char).is_ascii_digit() {
                        i += 1;
                    }
                    let exp: u32 = match src[start2..i].parse() {
                        Ok(x) => x,
                        // 指数溢出 u32 时明确报错（此前 unwrap_or(0) → 2^0=1 假通过）
                        Err(_) => return Err(format!("spec: 指数超出 u32: {}", &src[start2..i])),
                    };
                    v = v.checked_pow(exp).unwrap_or(u128::MAX);
                    out.push(Spt::Num(v));
                } else {
                    out.push(Spt::Num(v));
                }
            }
            'a'..='z' | 'A'..='Z' | '_' => {
                let start = i;
                while i < b.len() && (b[i] as char).is_ascii_alphanumeric() || (i < b.len() && b[i] == b'_') {
                    i += 1;
                }
                let s = &src[start..i];
                if s == "MAX_UINT" {
                    out.push(Spt::Num(u128::MAX));
                } else if s == "true" {
                    out.push(Spt::Num(1));
                } else if s == "false" {
                    out.push(Spt::Num(0));
                } else {
                    out.push(Spt::Ident(s.to_string()));
                }
            }
            '(' => {
                out.push(Spt::LParen);
                i += 1;
            }
            ')' => {
                out.push(Spt::RParen);
                i += 1;
            }
            '&' | '|' | '!' | '=' | '<' | '>' | '+' | '-' | '*' | '%' => {
                // 双字符操作符。用 src.get 避免在多字节 UTF-8 边界上切片 panic。
                if i + 1 < b.len() {
                    if let Some(two) = src.get(i..i + 2) {
                        match two {
                            "<=" | ">=" | "==" | "!=" | "&&" | "||" => {
                                out.push(Spt::Op(two.to_string()));
                                i += 2;
                                continue;
                            }
                            _ => {}
                        }
                    }
                }
                out.push(Spt::Op(c.to_string()));
                i += 1;
            }
            // 未知字符不得静默跳过：否则不支持的运算符（如 `~`/`^`）会被丢弃，
            // 使 postcondition 被悄悄削弱成恒真（假通过）。
            other => return Err(format!("spec: 非法字符 '{}'", other)),
        }
    }
    Ok(out)
}

struct SpecParser {
    toks: Vec<Spt>,
    pos: usize,
    depth: usize,
}

impl SpecParser {
    fn peek(&self) -> Option<&Spt> {
        self.toks.get(self.pos)
    }

    fn bump(&mut self) -> Option<Spt> {
        let t = self.toks.get(self.pos).cloned();
        self.pos += 1;
        t
    }

    fn parse_or(&mut self) -> Result<SpecExpr, String> {
        let mut l = self.parse_and()?;
        let mut n = 0u32;
        while let Some(Spt::Op(op)) = self.peek() {
            if op == "||" {
                n += 1;
                if n > 256 {
                    return Err("spec: 表达式过长（运算项 >256）".into());
                }
                self.bump();
                let r = self.parse_and()?;
                l = SpecExpr::Or(Box::new(l), Box::new(r));
            } else {
                break;
            }
        }
        Ok(l)
    }

    fn parse_and(&mut self) -> Result<SpecExpr, String> {
        let mut l = self.parse_cmp()?;
        let mut n = 0u32;
        while let Some(Spt::Op(op)) = self.peek() {
            if op == "&&" {
                n += 1;
                if n > 256 {
                    return Err("spec: 表达式过长（运算项 >256）".into());
                }
                self.bump();
                let r = self.parse_cmp()?;
                l = SpecExpr::And(Box::new(l), Box::new(r));
            } else {
                break;
            }
        }
        Ok(l)
    }

    fn parse_cmp(&mut self) -> Result<SpecExpr, String> {
        let mut l = self.parse_add()?;
        let mut n = 0u32;
        loop {
            let op = match self.peek() {
                Some(Spt::Op(o)) => o.clone(),
                _ => break,
            };
            match op.as_str() {
                "<=" | ">=" | "==" | "!=" | "<" | ">" => {
                    n += 1;
                    if n > 256 {
                        return Err("spec: 表达式过长（运算项 >256）".into());
                    }
                    self.bump();
                    let r = self.parse_add()?;
                    l = match op.as_str() {
                        "<=" => SpecExpr::Le(Box::new(l), Box::new(r)),
                        ">=" => SpecExpr::Ge(Box::new(l), Box::new(r)),
                        "==" => SpecExpr::Eq(Box::new(l), Box::new(r)),
                        "!=" => SpecExpr::Ne(Box::new(l), Box::new(r)),
                        "<" => SpecExpr::Lt(Box::new(l), Box::new(r)),
                        _ => SpecExpr::Gt(Box::new(l), Box::new(r)),
                    };
                }
                _ => break,
            }
        }
        Ok(l)
    }

    fn parse_add(&mut self) -> Result<SpecExpr, String> {
        let mut l = self.parse_mul()?;
        let mut n = 0u32;
        loop {
            let op = match self.peek() {
                Some(Spt::Op(o)) => o.clone(),
                _ => break,
            };
            match op.as_str() {
                "+" | "-" => {
                    n += 1;
                    if n > 256 {
                        return Err("spec: 表达式过长（运算项 >256）".into());
                    }
                    self.bump();
                    let r = self.parse_mul()?;
                    l = if op == "+" {
                        SpecExpr::Add(Box::new(l), Box::new(r))
                    } else {
                        SpecExpr::Sub(Box::new(l), Box::new(r))
                    };
                }
                _ => break,
            }
        }
        Ok(l)
    }

    fn parse_mul(&mut self) -> Result<SpecExpr, String> {
        let mut l = self.parse_unary()?;
        let mut n = 0u32;
        loop {
            let op = match self.peek() {
                Some(Spt::Op(o)) => o.clone(),
                _ => break,
            };
            if op == "*" {
                n += 1;
                if n > 256 {
                    return Err("spec: 表达式过长（运算项 >256）".into());
                }
                self.bump();
                let r = self.parse_unary()?;
                l = SpecExpr::Mul(Box::new(l), Box::new(r));
            } else if op == "%" {
                n += 1;
                if n > 256 {
                    return Err("spec: 表达式过长（运算项 >256）".into());
                }
                self.bump();
                let r = self.parse_unary()?;
                l = SpecExpr::Mod(Box::new(l), Box::new(r));
            } else {
                break;
            }
        }
        Ok(l)
    }

    fn parse_unary(&mut self) -> Result<SpecExpr, String> {
        let op = match self.peek() {
            Some(Spt::Op(o)) => o.clone(),
            _ => String::new(),
        };
        if op == "!" {
            // 一元 ! 亦为递归入口，须计入深度（否则数千层 ! 会栈溢出 abort）。
            self.depth += 1;
            if self.depth > 128 {
                self.depth -= 1;
                return Err("spec: 表达式嵌套过深（>128）".into());
            }
            self.bump();
            let e = self.parse_unary()?;
            self.depth -= 1;
            return Ok(SpecExpr::Not(Box::new(e)));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<SpecExpr, String> {
        self.depth += 1;
        if self.depth > 128 {
            self.depth -= 1;
            return Err("spec: 表达式嵌套过深（>128）".into());
        }
        let r = match self.bump() {
            Some(Spt::Num(n)) => Ok(SpecExpr::Num(n)),
            Some(Spt::Ident(s)) => Ok(SpecExpr::Var(s)),
            Some(Spt::LParen) => {
                let e = self.parse_or()?;
                if let Some(Spt::RParen) = self.bump() {
                    Ok(e)
                } else {
                    Err("spec: 缺 )".into())
                }
            }
            other => Err(format!("spec: 意外 token {:?}", other)),
        };
        self.depth -= 1;
        r
    }
}

/// 一键验证：对给定输入->输出对评估表达式是否为真。
pub fn assert_spec(
    text: &str,
    inputs: &HashMap<String, u128>,
    outputs: &HashMap<String, u128>,
) -> Result<bool, String> {
    let expr = parse_spec(text)?;
    let mut env = inputs.clone();
    for (k, v) in outputs {
        env.insert(k.clone(), *v);
    }
    let v = eval_spec(&expr, &env)?;
    Ok(v != 0)
}