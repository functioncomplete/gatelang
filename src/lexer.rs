//! 词法分析器。将 GateLang 源码词素化为 Token 流。
//!
//! 支持 token：标识符、关键字（circuit/state/spec/fn/latch/return/if/else/for/match）、
//! 数字字面量（十进制 / 0b 二进制）、位宽泛型 `Bits<4>`（lexer 输出 < >）、
//! 标点（{ } ( ) [ ] , ; : - > < = <- + & | ^ ! == != . ..）与行注释 // 块注释 /* */。

use crate::ast::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tok {
    // 关键字
    Circuit,
    State,
    Spec,
    Fn,
    Latch,
    Return,
    If,
    Else,
    For,
    Match,
    Pre,
    Post,
    Invariant,
    EdgeCases,
    /// `cut:` —— 已验证割点（引理组合层）
    Cut,
    // 标识符
    Ident(String),
    // 字面量
    Uint(u128, String), // (值, 原始文本)
    // 标点与运算符
    LCurly,
    RCurly,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    Semi,
    Colon,
    Arrow,   // ->
    Lt,      // <
    Gt,      // >
    Assign,  // =
    Update,  // <-
    Plus,
    Minus,
    Star,
    Percent,
    Amp,
    Pipe,
    AmpAmp,
    PipePipe,
    Le,     // <=
    Ge,     // >=
    Caret,
    Bang,
    EqEq,
    Ne,
    Dot,
    DotDot,  // ..
    Underscore,
    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
}

pub struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
    line: usize,
    col: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str) -> Self {
        Lexer { src: src.as_bytes(), pos: 0, line: 1, col: 1 }
    }

    fn span(&self) -> Span {
        Span::new(self.line, self.col)
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn peek2(&self) -> Option<u8> {
        self.src.get(self.pos + 1).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let c = self.peek()?;
        self.pos += 1;
        if c == b'\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    fn skip_ws_and_comments(&mut self) {
        loop {
            match self.peek() {
                Some(b' ') | Some(b'\t') | Some(b'\r') | Some(b'\n') => {
                    self.bump();
                }
                Some(b'/') if self.peek2() == Some(b'/') => {
                    while let Some(c) = self.peek() {
                        if c == b'\n' {
                            break;
                        }
                        self.bump();
                    }
                }
                Some(b'/') if self.peek2() == Some(b'*') => {
                    self.bump();
                    self.bump();
                    while let Some(c) = self.peek() {
                        if c == b'*' && self.peek2() == Some(b'/') {
                            self.bump();
                            self.bump();
                            break;
                        }
                        self.bump();
                    }
                }
                _ => break,
            }
        }
    }

    fn lex_ident(&mut self, first: u8) -> (String, Span) {
        let mut s = String::new();
        let sp = self.span();
        s.push(first as char);
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || c == b'_' {
                s.push(self.bump().unwrap() as char);
            } else {
                break;
            }
        }
        (s, sp)
    }

    fn lex_number(&mut self, first: u8) -> (u128, String, Span) {
        let sp = self.span();
        let mut text = String::new();
        text.push(first as char);
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || c == b'b' || c == b'_' {
                if c != b'_' {
                    text.push(self.bump().unwrap() as char);
                } else {
                    self.bump();
                }
            } else {
                break;
            }
        }
        let value = if text.starts_with("0b") {
            u128::from_str_radix(&text[2..], 2).unwrap_or(0)
        } else {
            text.parse().unwrap_or(0)
        };
        (value, text, sp)
    }

    pub fn tokenize(mut self) -> Result<Vec<Token>, String> {
        let mut out = Vec::new();
        loop {
            self.skip_ws_and_comments();
            let sp = self.span();
            let Some(c) = self.peek() else {
                out.push(Token { tok: Tok::Eof, span: sp });
                break;
            };
            match c {
                b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                    let first = self.bump().unwrap();
                    let (s, _) = self.lex_ident(first);
                    let tok = match s.as_str() {
                        "circuit" => Tok::Circuit,
                        "state" => Tok::State,
                        "spec" => Tok::Spec,
                        "fn" => Tok::Fn,
                        "latch" => Tok::Latch,
                        "return" => Tok::Return,
                        "if" => Tok::If,
                        "else" => Tok::Else,
                        "for" => Tok::For,
                        "match" => Tok::Match,
                        "precondition" => Tok::Pre,
                        "postcondition" => Tok::Post,
                        "invariant" => Tok::Invariant,
                        "edge_cases" => Tok::EdgeCases,
                        "cut" => Tok::Cut,
                        _ => Tok::Ident(s),
                    };
                    out.push(Token { tok, span: sp });
                }
                b'0'..=b'9' => {
                    let first = self.bump().unwrap();
                    let (v, raw, _) = self.lex_number(first);
                    out.push(Token { tok: Tok::Uint(v, raw), span: sp });
                }
                b'{' => { self.bump(); out.push(Token { tok: Tok::LCurly, span: sp }); }
                b'}' => { self.bump(); out.push(Token { tok: Tok::RCurly, span: sp }); }
                b'(' => { self.bump(); out.push(Token { tok: Tok::LParen, span: sp }); }
                b')' => { self.bump(); out.push(Token { tok: Tok::RParen, span: sp }); }
                b'[' => { self.bump(); out.push(Token { tok: Tok::LBracket, span: sp }); }
                b']' => { self.bump(); out.push(Token { tok: Tok::RBracket, span: sp }); }
                b',' => { self.bump(); out.push(Token { tok: Tok::Comma, span: sp }); }
                b';' => { self.bump(); out.push(Token { tok: Tok::Semi, span: sp }); }
                b':' => { self.bump(); out.push(Token { tok: Tok::Colon, span: sp }); }
                b'-' if self.peek2() == Some(b'>') => {
                    self.bump(); self.bump();
                    out.push(Token { tok: Tok::Arrow, span: sp });
                }
                b'<' if self.peek2() == Some(b'-') => {
                    self.bump(); self.bump();
                    out.push(Token { tok: Tok::Update, span: sp });
                }
                b'<' if self.peek2() == Some(b'=') => {
                    self.bump(); self.bump();
                    out.push(Token { tok: Tok::Le, span: sp });
                }
                b'<' => { self.bump(); out.push(Token { tok: Tok::Lt, span: sp }); }
                b'>' if self.peek2() == Some(b'=') => {
                    self.bump(); self.bump();
                    out.push(Token { tok: Tok::Ge, span: sp });
                }
                b'>' => { self.bump(); out.push(Token { tok: Tok::Gt, span: sp }); }
                b'=' if self.peek2() == Some(b'=') => {
                    self.bump(); self.bump();
                    out.push(Token { tok: Tok::EqEq, span: sp });
                }
                b'=' => { self.bump(); out.push(Token { tok: Tok::Assign, span: sp }); }
                b'!' if self.peek2() == Some(b'=') => {
                    self.bump(); self.bump();
                    out.push(Token { tok: Tok::Ne, span: sp });
                }
                b'!' => { self.bump(); out.push(Token { tok: Tok::Bang, span: sp }); }
                b'+' => { self.bump(); out.push(Token { tok: Tok::Plus, span: sp }); }
                b'-' => { self.bump(); out.push(Token { tok: Tok::Minus, span: sp }); }
                b'*' => { self.bump(); out.push(Token { tok: Tok::Star, span: sp }); }
                b'%' => { self.bump(); out.push(Token { tok: Tok::Percent, span: sp }); }
                b'&' if self.peek2() == Some(b'&') => {
                    self.bump(); self.bump();
                    out.push(Token { tok: Tok::AmpAmp, span: sp });
                }
                b'&' => { self.bump(); out.push(Token { tok: Tok::Amp, span: sp }); }
                b'|' if self.peek2() == Some(b'|') => {
                    self.bump(); self.bump();
                    out.push(Token { tok: Tok::PipePipe, span: sp });
                }
                b'|' => { self.bump(); out.push(Token { tok: Tok::Pipe, span: sp }); }
                b'^' => { self.bump(); out.push(Token { tok: Tok::Caret, span: sp }); }
                b'.' if self.peek2() == Some(b'.') => {
                    self.bump(); self.bump();
                    out.push(Token { tok: Tok::DotDot, span: sp });
                }
                b'.' => { self.bump(); out.push(Token { tok: Tok::Dot, span: sp }); }
                other => {
                    // 未知字符必须报错：静默跳过会悄悄改变语义（如 `y == ~a` 被当作 `y == a`）
                    return Err(format!("词法错误: 未知字符 '{}' @ {:?}", other as char, sp));
                }
            }
        }
        Ok(out)
    }
}

/// 便捷：将源码直接转为 Token 向量。
pub fn tokenize(src: &str) -> Result<Vec<Token>, String> {
    Lexer::new(src).tokenize()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexes_basic_structure() {
        let src = r#"circuit XOR(a: Bit, b: Bit) -> Bit {
            // 组合
            y = NAND(a, b)
            return y
        }"#;
        let toks = tokenize(src).unwrap();
        assert!(toks.iter().any(|t| t.tok == Tok::Circuit));
        assert!(toks.iter().any(|t| t.tok == Tok::Ident("XOR".into())));
        assert!(toks.iter().any(|t| t.tok == Tok::Arrow));
        assert!(toks.iter().any(|t| t.tok == Tok::Ident("NAND".into())));
    }

    #[test]
    fn lexes_latch_update_and_bits() {
        let src = "state C { latch v: Bits<8> = 0\n fn f(x: Bit) -> Bit { v <- v } }";
        let toks = tokenize(src).unwrap();
        assert!(toks.iter().any(|t| t.tok == Tok::State));
        assert!(toks.iter().any(|t| t.tok == Tok::Latch));
        assert!(toks.iter().any(|t| t.tok == Tok::Update));
        assert!(toks.iter().any(|t| t.tok == Tok::Uint(8, "8".into())));
    }

    #[test]
    fn lexes_ops() {
        let src = "a -> b <- c == d != e && f";
        let toks = tokenize(src).unwrap();
        assert!(toks.iter().any(|t| t.tok == Tok::Arrow));
        assert!(toks.iter().any(|t| t.tok == Tok::Update));
        assert!(toks.iter().any(|t| t.tok == Tok::EqEq));
        assert!(toks.iter().any(|t| t.tok == Tok::Ne));
    }

    #[test]
    fn rejects_unknown_characters() {
        // 未知字符应报错，而非静默跳过（否则会悄悄改变语义）
        assert!(tokenize("y == ~a").is_err());
        assert!(tokenize("y = a @ b").is_err());
    }
}