use super::lexer::{lex, Token};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Num(f64),
    Str(String),
}

impl Value {
    pub fn as_f64(&self) -> Result<f64, String> {
        match self {
            Value::Num(n) => Ok(*n),
            Value::Str(s) => s.trim().parse::<f64>().map_err(|_| format!("Expected a number but got '{}'", s)),
        }
    }
    pub fn as_f32(&self) -> Result<f32, String> {
        self.as_f64().map(|v| v as f32)
    }
    pub fn as_string(&self) -> String {
        match self {
            Value::Num(n) => fmt_num(*n),
            Value::Str(s) => s.clone(),
        }
    }
}

/// Compact number formatting: integers print without a fraction.
pub fn fmt_num(n: f64) -> String {
    if n.is_nan() {
        return "nan".to_string();
    }
    if n.fract() == 0.0 && n.abs() < 1e12 {
        return format!("{}", n as i64);
    }
    let s = format!("{:.6}", n);
    let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    if s.is_empty() || s == "-" {
        "0".to_string()
    } else {
        s
    }
}

pub struct Statement {
    pub line: usize,
    pub tokens: Vec<Token>,
}

pub fn parse_script(src: &str) -> Result<Vec<Statement>, String> {
    let toks = lex(src)?;
    let mut out = Vec::new();
    let mut cur: Vec<Token> = Vec::new();
    let mut line = 1usize;
    for (t, l) in toks {
        match t {
            Token::End => {
                if !cur.is_empty() {
                    out.push(Statement { line, tokens: std::mem::take(&mut cur) });
                }
            }
            other => {
                if cur.is_empty() {
                    line = l;
                }
                cur.push(other);
            }
        }
    }
    Ok(out)
}

pub struct FlagSpec {
    /// First name is canonical; the rest are aliases (short and long forms).
    pub names: &'static [&'static str],
    pub arity: usize,
}

#[derive(Debug, Default)]
pub struct Parsed {
    pub flags: Vec<(String, Vec<Value>)>,
    pub args: Vec<Value>,
}

impl Parsed {
    pub fn has(&self, name: &str) -> bool {
        self.flags.iter().any(|(n, _)| n == name)
    }
    pub fn get(&self, name: &str) -> Option<&Vec<Value>> {
        self.flags.iter().rev().find(|(n, _)| n == name).map(|(_, v)| v)
    }
    pub fn num(&self, name: &str) -> Result<Option<f32>, String> {
        match self.get(name) {
            Some(v) if !v.is_empty() => Ok(Some(v[0].as_f32()?)),
            _ => Ok(None),
        }
    }
    pub fn string(&self, name: &str) -> Option<String> {
        self.get(name).and_then(|v| v.first()).map(|v| v.as_string())
    }
    pub fn arg_strings(&self) -> Vec<String> {
        self.args.iter().map(|a| a.as_string()).collect()
    }
}

pub fn parse_args(tokens: &[Token], spec: &[FlagSpec]) -> Result<Parsed, String> {
    let mut p = Parsed::default();
    let mut i = 0;
    let to_value = |t: &Token| -> Option<Value> {
        match t {
            Token::Num(n) => Some(Value::Num(*n)),
            Token::Str(s) => Some(Value::Str(s.clone())),
            Token::Word(w) => Some(Value::Str(w.clone())),
            _ => None,
        }
    };
    while i < tokens.len() {
        match &tokens[i] {
            Token::Flag(f) => {
                let s = spec
                    .iter()
                    .find(|s| s.names.contains(&f.as_str()))
                    .ok_or_else(|| format!("Unknown flag: -{}", f))?;
                let mut vals = Vec::new();
                for k in 0..s.arity {
                    match tokens.get(i + 1 + k).and_then(to_value) {
                        Some(v) => vals.push(v),
                        None => return Err(format!("Flag -{} expects {} value(s)", f, s.arity)),
                    }
                }
                i += 1 + s.arity;
                // Register under every alias so callers can look a flag up by any of its names.
                for nm in s.names {
                    p.flags.push((nm.to_string(), vals.clone()));
                }
            }
            t => {
                if let Some(v) = to_value(t) {
                    p.args.push(v);
                }
                i += 1;
            }
        }
    }
    Ok(p)
}

fn token_to_value(t: &Token, vars: &HashMap<String, Value>) -> Result<Value, String> {
    match t {
        Token::Num(n) => Ok(Value::Num(*n)),
        Token::Str(s) => Ok(Value::Str(s.clone())),
        Token::Word(w) if w.starts_with('$') => vars
            .get(&w[1..])
            .cloned()
            .ok_or_else(|| format!("Undefined variable: {}", w)),
        Token::Word(w) => Ok(Value::Str(w.clone())),
        Token::Flag(f) => Ok(Value::Str(format!("-{}", f))),
        Token::End => Err("Unexpected end of statement".to_string()),
        Token::Group(_) | Token::Tick(_) => Err("Unresolved expression".to_string()),
    }
}

/// Replace `$name` tokens with their values.
pub fn substitute(tokens: &[Token], vars: &HashMap<String, Value>) -> Result<Vec<Token>, String> {
    let mut out = Vec::with_capacity(tokens.len());
    for t in tokens {
        match t {
            Token::Word(w) if w.starts_with('$') && w.len() > 1 => match token_to_value(t, vars)? {
                Value::Num(n) => out.push(Token::Num(n)),
                Value::Str(s) => out.push(Token::Str(s)),
            },
            other => out.push(other.clone()),
        }
    }
    Ok(out)
}

/// Evaluate `a + b * c` style expressions (tokens must be space separated). Supports string concatenation with +.
pub fn eval_expr(tokens: &[Token], vars: &HashMap<String, Value>) -> Result<Value, String> {
    if tokens.is_empty() {
        return Err("Empty expression".to_string());
    }
    let mut operands: Vec<Value> = Vec::new();
    let mut ops: Vec<char> = Vec::new();
    for (i, t) in tokens.iter().enumerate() {
        if i % 2 == 0 {
            operands.push(token_to_value(t, vars)?);
        } else {
            match t {
                Token::Word(w) if w == "+" || w == "-" || w == "*" || w == "/" => ops.push(w.chars().next().unwrap()),
                _ => return Err("Expected an operator (+ - * /) between values".to_string()),
            }
        }
    }
    if operands.len() != ops.len() + 1 {
        return Err("Malformed expression".to_string());
    }
    // First pass: * and /
    let mut vals: Vec<Value> = vec![operands[0].clone()];
    let mut rest_ops: Vec<char> = Vec::new();
    for (k, op) in ops.iter().enumerate() {
        let rhs = operands[k + 1].clone();
        if *op == '*' || *op == '/' {
            let l = vals.pop().unwrap().as_f64()?;
            let r = rhs.as_f64()?;
            if *op == '/' && r == 0.0 {
                return Err("Division by zero".to_string());
            }
            vals.push(Value::Num(if *op == '*' { l * r } else { l / r }));
        } else {
            rest_ops.push(*op);
            vals.push(rhs);
        }
    }
    let mut acc = vals[0].clone();
    for (k, op) in rest_ops.iter().enumerate() {
        let rhs = vals[k + 1].clone();
        acc = match (&acc, &rhs, op) {
            (Value::Str(a), b, '+') => Value::Str(format!("{}{}", a, b.as_string())),
            (a, Value::Str(b), '+') => Value::Str(format!("{}{}", a.as_string(), b)),
            (a, b, '+') => Value::Num(a.as_f64()? + b.as_f64()?),
            (a, b, '-') => Value::Num(a.as_f64()? - b.as_f64()?),
            _ => return Err("Bad operator".to_string()),
        };
    }
    Ok(acc)
}
