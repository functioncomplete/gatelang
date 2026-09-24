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
    let toks = tokenize_spec(text);
    let mut p = SpecParser { toks, pos: 0 };
    let e = p.parse_or()?;
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

fn tokenize_spec(src: &str) -> Vec<Spt> {
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
                let mut v: u128 = src[start..i].parse().unwrap_or(0);
                // 处理 a^b 幂（原型仅支持 2^N / MAX_UINT）
                if i + 1 < b.len() && b[i] == b'^' {
                    // 形如 2^256
                    i += 1;
                    let start2 = i;
                    while i < b.len() && (b[i] as char).is_ascii_digit() {
                        i += 1;
                    }
                    let exp: u32 = src[start2..i].parse().unwrap_or(0);
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
                if s == "MAX_UINT" || s == "max" {
                    out.push(Spt::Num(u128::MAX));
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
                // 双字符操作符
                if i + 1 < b.len() {
                    let two = &src[i..i + 2];
                    match two {
                        "<=" | ">=" | "==" | "!=" | "&&" | "||" => {
                            out.push(Spt::Op(two.to_string()));
                            i += 2;
                            continue;
                        }
                        _ => {}
                    }
                }
                out.push(Spt::Op(c.to_string()));
                i += 1;
            }
            _ => i += 1,
        }
    }
    out
}

struct SpecParser {
    toks: Vec<Spt>,
    pos: usize,
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
        while let Some(Spt::Op(op)) = self.peek() {
            if op == "||" {
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
        while let Some(Spt::Op(op)) = self.peek() {
            if op == "&&" {
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
        loop {
            let op = match self.peek() {
                Some(Spt::Op(o)) => o.clone(),
                _ => break,
            };
            match op.as_str() {
                "<=" | ">=" | "==" | "!=" | "<" | ">" => {
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
        loop {
            let op = match self.peek() {
                Some(Spt::Op(o)) => o.clone(),
                _ => break,
            };
            match op.as_str() {
                "+" | "-" => {
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
        loop {
            let op = match self.peek() {
                Some(Spt::Op(o)) => o.clone(),
                _ => break,
            };
            if op == "*" {
                self.bump();
                let r = self.parse_unary()?;
                l = SpecExpr::Mul(Box::new(l), Box::new(r));
            } else if op == "%" {
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
            self.bump();
            let e = self.parse_unary()?;
            return Ok(SpecExpr::Not(Box::new(e)));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<SpecExpr, String> {
        match self.bump() {
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
        }
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