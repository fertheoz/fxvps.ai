//! Algorithmic pricing (parça 14): a group's own bid / ask offset formula.
//!
//! A formula is a small arithmetic expression over the live variables of
//! the symbol, evaluated on every client quote; its value is extra points
//! on that side (negative narrows). The language is deliberately tiny and
//! total: numbers, variables, `+ - * /`, unary minus, comparisons
//! (`< <= > >= == !=` → 1 or 0), `and` / `or` / `not`, parentheses and the
//! functions `min(a, b)`, `max(a, b)`, `abs(x)`, `clamp(x, lo, hi)`,
//! `if(cond, a, b)`. No loops, no state, no side effects: a formula can
//! only ever shift a price, never break the engine. Division by zero is 0.
//!
//! Variables: `spread` (raw LP spread, points), `net` (B-book net lots,
//! clients long positive), `vol` (daily volatility, percent), `hour`
//! (UTC), `news` (1 inside a news window), `markup` (the group's base
//! markup, points), `lots` (|net|).
use serde::{Deserialize, Serialize};

/// A group's pricing formulas (console: Groups → Algorithmic pricing).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct PricingAlgo {
    /// Extra points on the bid side.
    pub bid: String,
    /// Extra points on the ask side.
    pub ask: String,
}

/// Live inputs of one evaluation.
#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Vars {
    pub spread: f64,
    pub net: f64,
    pub vol: f64,
    pub hour: f64,
    pub news: f64,
    pub markup: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Num(f64),
    Var(Var),
    Neg(Box<Expr>),
    Not(Box<Expr>),
    Bin(Op, Box<Expr>, Box<Expr>),
    Call(Func, Vec<Expr>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Var {
    Spread,
    Net,
    Lots,
    Vol,
    Hour,
    News,
    Markup,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    And,
    Or,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Func {
    Min,
    Max,
    Abs,
    Clamp,
    If,
}

pub const MAX_LEN: usize = 400;

/// Parses a formula; errors name the position.
pub fn parse(src: &str) -> Result<Expr, String> {
    if src.len() > MAX_LEN {
        return Err(format!("formula longer than {MAX_LEN} chars"));
    }
    let toks = lex(src)?;
    let mut p = Parser { toks, pos: 0 };
    let e = p.or()?;
    if p.pos != p.toks.len() {
        return Err(format!("unexpected {:?} at token {}", p.toks[p.pos], p.pos));
    }
    Ok(e)
}

/// Evaluates a parsed formula; never panics, never NaN / infinite.
pub fn eval(e: &Expr, v: &Vars) -> f64 {
    let x = eval_inner(e, v, 0);
    if x.is_finite() {
        x
    } else {
        0.0
    }
}

/// Parses and evaluates in one go (the engine caches parsed formulas).
pub fn run(src: &str, v: &Vars) -> f64 {
    parse(src).map(|e| eval(&e, v)).unwrap_or(0.0)
}

fn eval_inner(e: &Expr, v: &Vars, depth: u32) -> f64 {
    if depth > 64 {
        return 0.0;
    }
    let b = |x: bool| if x { 1.0 } else { 0.0 };
    match e {
        Expr::Num(n) => *n,
        Expr::Var(var) => match var {
            Var::Spread => v.spread,
            Var::Net => v.net,
            Var::Lots => v.net.abs(),
            Var::Vol => v.vol,
            Var::Hour => v.hour,
            Var::News => v.news,
            Var::Markup => v.markup,
        },
        Expr::Neg(x) => -eval_inner(x, v, depth + 1),
        Expr::Not(x) => b(eval_inner(x, v, depth + 1) == 0.0),
        Expr::Bin(op, l, r) => {
            let (l, r) = (eval_inner(l, v, depth + 1), eval_inner(r, v, depth + 1));
            match op {
                Op::Add => l + r,
                Op::Sub => l - r,
                Op::Mul => l * r,
                Op::Div => {
                    if r == 0.0 {
                        0.0
                    } else {
                        l / r
                    }
                }
                Op::Lt => b(l < r),
                Op::Le => b(l <= r),
                Op::Gt => b(l > r),
                Op::Ge => b(l >= r),
                Op::Eq => b(l == r),
                Op::Ne => b(l != r),
                Op::And => b(l != 0.0 && r != 0.0),
                Op::Or => b(l != 0.0 || r != 0.0),
            }
        }
        Expr::Call(f, args) => {
            let a = |i: usize| args.get(i).map_or(0.0, |x| eval_inner(x, v, depth + 1));
            match f {
                Func::Min => a(0).min(a(1)),
                Func::Max => a(0).max(a(1)),
                Func::Abs => a(0).abs(),
                Func::Clamp => {
                    let (lo, hi) = (a(1), a(2));
                    if lo <= hi {
                        a(0).clamp(lo, hi)
                    } else {
                        a(0)
                    }
                }
                Func::If => {
                    if a(0) != 0.0 {
                        a(1)
                    } else {
                        a(2)
                    }
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Op(String),
    LParen,
    RParen,
    Comma,
}

fn lex(src: &str) -> Result<Vec<Tok>, String> {
    let cs: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < cs.len() {
        let c = cs[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit()
            || (c == '.' && cs.get(i + 1).is_some_and(|d| d.is_ascii_digit()))
        {
            let start = i;
            while i < cs.len() && (cs[i].is_ascii_digit() || cs[i] == '.') {
                i += 1;
            }
            let s: String = cs[start..i].iter().collect();
            out.push(Tok::Num(s.parse().map_err(|_| format!("bad number {s}"))?));
        } else if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < cs.len() && (cs[i].is_ascii_alphanumeric() || cs[i] == '_') {
                i += 1;
            }
            out.push(Tok::Ident(
                cs[start..i].iter().collect::<String>().to_lowercase(),
            ));
        } else if c == '(' {
            out.push(Tok::LParen);
            i += 1;
        } else if c == ')' {
            out.push(Tok::RParen);
            i += 1;
        } else if c == ',' {
            out.push(Tok::Comma);
            i += 1;
        } else {
            let two: String = cs[i..(i + 2).min(cs.len())].iter().collect();
            if ["<=", ">=", "==", "!=", "&&", "||"].contains(&two.as_str()) {
                out.push(Tok::Op(two));
                i += 2;
            } else if "+-*/<>".contains(c) {
                out.push(Tok::Op(c.to_string()));
                i += 1;
            } else {
                return Err(format!("unexpected character {c:?} at {i}"));
            }
        }
    }
    Ok(out)
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn eat_op(&mut self, ops: &[&str]) -> Option<String> {
        if let Some(Tok::Op(o)) = self.peek() {
            if ops.contains(&o.as_str()) {
                let o = o.clone();
                self.pos += 1;
                return Some(o);
            }
        }
        if let Some(Tok::Ident(w)) = self.peek() {
            if ops.contains(&w.as_str()) {
                let w = w.clone();
                self.pos += 1;
                return Some(w);
            }
        }
        None
    }

    fn or(&mut self) -> Result<Expr, String> {
        let mut l = self.and()?;
        while self.eat_op(&["||", "or"]).is_some() {
            let r = self.and()?;
            l = Expr::Bin(Op::Or, Box::new(l), Box::new(r));
        }
        Ok(l)
    }

    fn and(&mut self) -> Result<Expr, String> {
        let mut l = self.cmp()?;
        while self.eat_op(&["&&", "and"]).is_some() {
            let r = self.cmp()?;
            l = Expr::Bin(Op::And, Box::new(l), Box::new(r));
        }
        Ok(l)
    }

    fn cmp(&mut self) -> Result<Expr, String> {
        let l = self.sum()?;
        if let Some(o) = self.eat_op(&["<", "<=", ">", ">=", "==", "!="]) {
            let r = self.sum()?;
            let op = match o.as_str() {
                "<" => Op::Lt,
                "<=" => Op::Le,
                ">" => Op::Gt,
                ">=" => Op::Ge,
                "==" => Op::Eq,
                _ => Op::Ne,
            };
            return Ok(Expr::Bin(op, Box::new(l), Box::new(r)));
        }
        Ok(l)
    }

    fn sum(&mut self) -> Result<Expr, String> {
        let mut l = self.term()?;
        while let Some(o) = self.eat_op(&["+", "-"]) {
            let r = self.term()?;
            let op = if o == "+" { Op::Add } else { Op::Sub };
            l = Expr::Bin(op, Box::new(l), Box::new(r));
        }
        Ok(l)
    }

    fn term(&mut self) -> Result<Expr, String> {
        let mut l = self.unary()?;
        while let Some(o) = self.eat_op(&["*", "/"]) {
            let r = self.unary()?;
            let op = if o == "*" { Op::Mul } else { Op::Div };
            l = Expr::Bin(op, Box::new(l), Box::new(r));
        }
        Ok(l)
    }

    fn unary(&mut self) -> Result<Expr, String> {
        if self.eat_op(&["-"]).is_some() {
            return Ok(Expr::Neg(Box::new(self.unary()?)));
        }
        if self.eat_op(&["not", "!"]).is_some() {
            return Ok(Expr::Not(Box::new(self.unary()?)));
        }
        self.atom()
    }

    fn atom(&mut self) -> Result<Expr, String> {
        match self.peek().cloned() {
            Some(Tok::Num(n)) => {
                self.pos += 1;
                Ok(Expr::Num(n))
            }
            Some(Tok::LParen) => {
                self.pos += 1;
                let e = self.or()?;
                match self.peek() {
                    Some(Tok::RParen) => {
                        self.pos += 1;
                        Ok(e)
                    }
                    _ => Err("missing )".into()),
                }
            }
            Some(Tok::Ident(w)) => {
                self.pos += 1;
                if let Some(Tok::LParen) = self.peek() {
                    self.pos += 1;
                    let (f, arity) = match w.as_str() {
                        "min" => (Func::Min, 2),
                        "max" => (Func::Max, 2),
                        "abs" => (Func::Abs, 1),
                        "clamp" => (Func::Clamp, 3),
                        "if" => (Func::If, 3),
                        _ => return Err(format!("unknown function {w}")),
                    };
                    let mut args = Vec::new();
                    loop {
                        args.push(self.or()?);
                        match self.peek() {
                            Some(Tok::Comma) => self.pos += 1,
                            Some(Tok::RParen) => {
                                self.pos += 1;
                                break;
                            }
                            _ => return Err(format!("bad argument list of {w}")),
                        }
                    }
                    if args.len() != arity {
                        return Err(format!("{w} takes {arity} argument(s)"));
                    }
                    return Ok(Expr::Call(f, args));
                }
                let v = match w.as_str() {
                    "spread" => Var::Spread,
                    "net" => Var::Net,
                    "lots" => Var::Lots,
                    "vol" => Var::Vol,
                    "hour" => Var::Hour,
                    "news" => Var::News,
                    "markup" => Var::Markup,
                    "true" => return Ok(Expr::Num(1.0)),
                    "false" => return Ok(Expr::Num(0.0)),
                    _ => return Err(format!("unknown variable {w}")),
                };
                Ok(Expr::Var(v))
            }
            other => Err(format!("unexpected {other:?}")),
        }
    }
}

impl PricingAlgo {
    pub fn validate(&self) -> Result<(), String> {
        parse(&self.bid).map_err(|e| format!("bid: {e}"))?;
        parse(&self.ask).map_err(|e| format!("ask: {e}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v() -> Vars {
        Vars {
            spread: 1.2,
            net: -3.0,
            vol: 0.8,
            hour: 14.0,
            news: 0.0,
            markup: 5.0,
        }
    }

    #[test]
    fn arithmetic_precedence_and_functions() {
        assert_eq!(run("1 + 2 * 3", &v()), 7.0);
        assert_eq!(run("(1 + 2) * 3", &v()), 9.0);
        assert_eq!(run("-net * 2", &v()), 6.0);
        assert_eq!(run("lots", &v()), 3.0);
        assert_eq!(run("min(spread, 1) + max(net, 0)", &v()), 1.0);
        assert_eq!(run("clamp(net * 10, -20, 20)", &v()), -20.0);
        assert_eq!(run("if(hour >= 13 and hour < 17, 2, 0)", &v()), 2.0);
        assert_eq!(run("if(news or spread > 5, markup * 2, 0)", &v()), 0.0);
        assert_eq!(run("abs(net) / 0", &v()), 0.0, "division by zero is 0");
        assert_eq!(run("not news", &v()), 1.0);
    }

    #[test]
    fn errors_are_reported_and_never_evaluate() {
        assert!(parse("spread +").is_err());
        assert!(parse("foo(1)").is_err());
        assert!(parse("unknown * 2").is_err());
        assert!(parse("min(1)").is_err());
        assert!(parse("(1 + 2").is_err());
        assert!(parse(&"1+".repeat(300)).is_err());
        assert_eq!(run("spread +", &v()), 0.0);
        assert!(PricingAlgo {
            bid: "0".into(),
            ask: "max(0, net) * 2".into()
        }
        .validate()
        .is_ok());
    }
}
