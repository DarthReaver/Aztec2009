//! Structured scripting on top of the command language: blocks, `if` / `else`, `for`, `while`,
//! `proc` (with `return`), `break` / `continue`, expressions in parentheses, `++` / `+=`, local and
//! `global` variables, backtick command results and a small function library.
//!
//! Numbers are always floating point (so `5 / 2` is 2.5 where MEL would print 2), and arrays and
//! vectors are not supported.

use super::commands::{run_statement, Outcome};
use super::lexer::{lex, Token};
use super::parser::{fmt_num, Value};
use crate::session::Session;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

const STEP_LIMIT: u64 = 3_000_000;
const MAX_CALL_DEPTH: usize = 60;

// ====================================================================== state kept in the Session

#[derive(Default)]
pub struct Frame {
    pub vars: HashMap<String, Value>,
    pub globals: HashSet<String>,
}

pub struct Proc {
    pub name: String,
    pub params: Vec<String>,
    pub body: Vec<Stmt>,
}

pub enum Stmt {
    Cmd { text: String, line: usize },
    Block(Vec<Stmt>),
    If { cond: Expr, then: Box<Stmt>, els: Option<Box<Stmt>> },
    While { cond: Expr, body: Box<Stmt> },
    For { init: String, cond: Option<Expr>, step: String, body: Box<Stmt>, line: usize },
    Proc(Arc<Proc>),
    Return(Option<Expr>),
    Break,
    Continue,
}

enum Flow {
    Next,
    Break,
    Continue,
    Return(Option<Value>),
}

// ====================================================================== statement parser

struct P {
    c: Vec<char>,
    i: usize,
    line: usize,
    /// A newline ends a command (scripts without semicolons). Scripts that use `;` follow MEL: only `;` ends a command,
    /// so flags and the object name can sit on following lines.
    nl_ends: bool,
}

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

impl P {
    fn peek(&self) -> Option<char> {
        self.c.get(self.i).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.c.get(self.i).copied();
        if c == Some('\n') {
            self.line += 1;
        }
        self.i += 1;
        c
    }

    /// Skip blanks, newlines, `;` and comments.
    fn skip(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() || c == ';' {
                self.bump();
            } else if c == '/' && self.c.get(self.i + 1) == Some(&'/') {
                while let Some(c) = self.peek() {
                    if c == '\n' {
                        break;
                    }
                    self.bump();
                }
            } else if c == '/' && self.c.get(self.i + 1) == Some(&'*') {
                self.bump();
                self.bump();
                while self.i < self.c.len() && !(self.c[self.i] == '*' && self.c.get(self.i + 1) == Some(&'/')) {
                    self.bump();
                }
                self.bump();
                self.bump();
            } else {
                break;
            }
        }
    }

    fn word(&self) -> String {
        let mut j = self.i;
        while j < self.c.len() && is_word(self.c[j]) {
            j += 1;
        }
        self.c[self.i..j].iter().collect()
    }

    fn eat_word(&mut self, w: &str) -> bool {
        if self.word() == w {
            self.i += w.chars().count();
            true
        } else {
            false
        }
    }

    /// Text inside balanced parentheses; the cursor must be on `(`.
    fn parens(&mut self) -> Result<String, String> {
        self.skip();
        if self.peek() != Some('(') {
            return Err(format!("Line {}: expected '('", self.line));
        }
        self.bump();
        let start = self.i;
        let mut depth = 1;
        while let Some(c) = self.peek() {
            match c {
                '"' => self.skip_string(),
                '(' => {
                    depth += 1;
                    self.bump();
                }
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        let t: String = self.c[start..self.i].iter().collect();
                        self.bump();
                        return Ok(t);
                    }
                    self.bump();
                }
                _ => {
                    self.bump();
                }
            }
        }
        Err(format!("Line {}: missing ')'", self.line))
    }

    fn skip_string(&mut self) {
        self.bump();
        while let Some(c) = self.peek() {
            if c == '\\' {
                self.bump();
                self.bump();
                continue;
            }
            self.bump();
            if c == '"' || c == '\n' {
                break;
            }
        }
    }

    /// A command: up to `;`, a newline or a closing brace, outside strings, parentheses and backticks.
    fn command_text(&mut self) -> String {
        let mut out = String::new();
        let mut depth = 0i32;
        let mut tick = false;
        while let Some(c) = self.peek() {
            if c == '"' {
                let st = self.i;
                self.skip_string();
                out.extend(&self.c[st..self.i]);
                continue;
            }
            if c == '/' && self.c.get(self.i + 1) == Some(&'/') {
                while let Some(c) = self.peek() {
                    if c == '\n' {
                        break;
                    }
                    self.bump();
                }
                continue;
            }
            if c == '`' {
                tick = !tick;
            }
            if !tick {
                match c {
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    ';' if depth <= 0 => {
                        self.bump();
                        break;
                    }
                    '\n' if depth <= 0 && self.nl_ends => {
                        self.bump();
                        break;
                    }
                    '}' if depth <= 0 => break,
                    _ => {}
                }
            }
            out.push(c);
            self.bump();
        }
        out.trim().to_string()
    }

    fn body(&mut self) -> Result<Stmt, String> {
        self.skip();
        match self.stmt()? {
            Some(s) => Ok(s),
            None => Err(format!("Line {}: missing statement", self.line)),
        }
    }

    fn stmts(&mut self, in_block: bool) -> Result<Vec<Stmt>, String> {
        let mut v = Vec::new();
        loop {
            self.skip();
            match self.peek() {
                None => {
                    if in_block {
                        return Err(format!("Line {}: missing '}}'", self.line));
                    }
                    break;
                }
                Some('}') => {
                    if in_block {
                        self.bump();
                        break;
                    }
                    return Err(format!("Line {}: unexpected '}}'", self.line));
                }
                _ => {}
            }
            if let Some(s) = self.stmt()? {
                v.push(s);
            }
        }
        Ok(v)
    }

    fn stmt(&mut self) -> Result<Option<Stmt>, String> {
        self.skip();
        let Some(c) = self.peek() else { return Ok(None) };
        let line = self.line;
        if c == '{' {
            self.bump();
            return Ok(Some(Stmt::Block(self.stmts(true)?)));
        }
        if c == '}' {
            return Ok(None);
        }
        let w = self.word();
        match w.as_str() {
            "if" => {
                self.i += 2;
                let cond = parse_expr(&self.parens()?).map_err(|e| format!("Line {}: {}", line, e))?;
                let then = Box::new(self.body()?);
                let save = (self.i, self.line);
                self.skip();
                let els = if self.eat_word("else") {
                    Some(Box::new(self.body()?))
                } else {
                    self.i = save.0;
                    self.line = save.1;
                    None
                };
                Ok(Some(Stmt::If { cond, then, els }))
            }
            "while" => {
                self.i += 5;
                let cond = parse_expr(&self.parens()?).map_err(|e| format!("Line {}: {}", line, e))?;
                Ok(Some(Stmt::While { cond, body: Box::new(self.body()?) }))
            }
            "for" => {
                self.i += 3;
                let inner = self.parens()?;
                let parts = split_top(&inner, ';');
                if parts.len() != 3 {
                    return Err(format!("Line {}: for needs (init; condition; step); array loops (for ($x in ...)) are not supported", line));
                }
                let cond = if parts[1].trim().is_empty() { None } else { Some(parse_expr(&parts[1]).map_err(|e| format!("Line {}: {}", line, e))?) };
                Ok(Some(Stmt::For { init: parts[0].trim().to_string(), cond, step: parts[2].trim().to_string(), body: Box::new(self.body()?), line }))
            }
            "proc" | "global" => {
                let save = (self.i, self.line);
                if w == "global" {
                    self.i += 6;
                    self.skip();
                    if self.word() != "proc" {
                        self.i = save.0;
                        self.line = save.1;
                        return Ok(Some(Stmt::Cmd { text: self.command_text(), line }));
                    }
                }
                self.i += 4;
                Ok(Some(Stmt::Proc(Arc::new(self.proc_def(line)?))))
            }
            "return" => {
                self.i += 6;
                let t = self.command_text();
                Ok(Some(Stmt::Return(if t.is_empty() { None } else { Some(parse_expr(&t).map_err(|e| format!("Line {}: {}", line, e))?) })))
            }
            "break" => {
                self.i += 5;
                self.command_text();
                Ok(Some(Stmt::Break))
            }
            "continue" => {
                self.i += 8;
                self.command_text();
                Ok(Some(Stmt::Continue))
            }
            _ => {
                let text = self.command_text();
                if text.is_empty() {
                    return Ok(None);
                }
                Ok(Some(Stmt::Cmd { text, line }))
            }
        }
    }

    fn proc_def(&mut self, line: usize) -> Result<Proc, String> {
        self.skip();
        let st = self.i;
        while let Some(c) = self.peek() {
            if c == '(' {
                break;
            }
            self.bump();
        }
        let head: String = self.c[st..self.i].iter().collect();
        let name = head.split_whitespace().last().ok_or_else(|| format!("Line {}: proc needs a name", line))?.to_string();
        let params_text = self.parens()?;
        let mut params = Vec::new();
        for p in split_top(&params_text, ',') {
            let p = p.trim();
            if p.is_empty() {
                continue;
            }
            if p.contains('[') {
                return Err(format!("Line {}: array parameters are not supported", line));
            }
            let n = p.split_whitespace().last().unwrap_or("");
            params.push(n.trim_start_matches('$').to_string());
        }
        self.skip();
        if self.peek() != Some('{') {
            return Err(format!("Line {}: proc {} needs a body in braces", self.line, name));
        }
        self.bump();
        let body = self.stmts(true)?;
        Ok(Proc { name, params, body })
    }
}

/// Split at `sep` outside parentheses, strings and backticks.
fn split_top(s: &str, sep: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let (mut depth, mut instr, mut tick, mut esc) = (0i32, false, false, false);
    for c in s.chars() {
        if instr {
            cur.push(c);
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                instr = false;
            }
            continue;
        }
        match c {
            '"' => instr = true,
            '`' => tick = !tick,
            '(' if !tick => depth += 1,
            ')' if !tick => depth -= 1,
            c if c == sep && depth == 0 && !tick => {
                out.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    out.push(cur);
    out
}

/// Does the script end commands with `;` (outside strings, comments and parentheses)?
fn uses_semicolons(c: &[char]) -> bool {
    let (mut i, mut depth) = (0, 0i32);
    while i < c.len() {
        match c[i] {
            '"' => {
                i += 1;
                while i < c.len() && c[i] != '"' {
                    if c[i] == '\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            '/' if c.get(i + 1) == Some(&'/') => {
                while i < c.len() && c[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            '/' if c.get(i + 1) == Some(&'*') => {
                i += 2;
                while i + 1 < c.len() && !(c[i] == '*' && c[i + 1] == '/') {
                    i += 1;
                }
                i += 1;
            }
            '(' => depth += 1,
            ')' => depth -= 1,
            ';' if depth <= 0 => {
                // a ; inside a for (...) header has depth > 0 and does not count
                return true;
            }
            _ => {}
        }
        i += 1;
    }
    false
}

pub fn parse_program(src: &str) -> Result<Vec<Stmt>, String> {
    let chars: Vec<char> = src.chars().collect();
    let nl_ends = !uses_semicolons(&chars);
    let mut p = P { c: chars, i: 0, line: 1, nl_ends };
    p.stmts(false)
}

// ====================================================================== expressions

#[derive(Clone, Debug)]
pub enum Expr {
    Num(f64),
    Str(String),
    Var(String),
    Neg(Box<Expr>),
    Not(Box<Expr>),
    Bin(Box<Expr>, &'static str, Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
    Tick(String),
}

struct X {
    c: Vec<char>,
    i: usize,
}

impl X {
    fn ws(&mut self) {
        while self.i < self.c.len() && self.c[self.i].is_whitespace() {
            self.i += 1;
        }
    }
    fn peek(&mut self) -> Option<char> {
        self.ws();
        self.c.get(self.i).copied()
    }
    fn two(&mut self, a: char, b: char) -> bool {
        self.ws();
        self.c.get(self.i) == Some(&a) && self.c.get(self.i + 1) == Some(&b)
    }
    fn eat(&mut self, s: &str) -> bool {
        self.ws();
        let n = s.chars().count();
        if self.c.len() >= self.i + n && self.c[self.i..self.i + n].iter().copied().eq(s.chars()) {
            self.i += n;
            true
        } else {
            false
        }
    }

    fn ternary(&mut self) -> Result<Expr, String> {
        let c = self.or()?;
        if self.peek() == Some('?') {
            self.i += 1;
            let a = self.ternary()?;
            if !self.eat(":") {
                return Err("Expected ':' in ?: expression".to_string());
            }
            let b = self.ternary()?;
            return Ok(Expr::Cond(Box::new(c), Box::new(a), Box::new(b)));
        }
        Ok(c)
    }
    fn or(&mut self) -> Result<Expr, String> {
        let mut l = self.and()?;
        while self.eat("||") {
            l = Expr::Or(Box::new(l), Box::new(self.and()?));
        }
        Ok(l)
    }
    fn and(&mut self) -> Result<Expr, String> {
        let mut l = self.eq()?;
        while self.eat("&&") {
            l = Expr::And(Box::new(l), Box::new(self.eq()?));
        }
        Ok(l)
    }
    fn eq(&mut self) -> Result<Expr, String> {
        let mut l = self.rel()?;
        loop {
            let op = if self.eat("==") {
                "=="
            } else if self.eat("!=") {
                "!="
            } else {
                break;
            };
            l = Expr::Bin(Box::new(l), op, Box::new(self.rel()?));
        }
        Ok(l)
    }
    fn rel(&mut self) -> Result<Expr, String> {
        let mut l = self.add()?;
        loop {
            let op = if self.eat("<=") {
                "<="
            } else if self.eat(">=") {
                ">="
            } else if self.eat("<") {
                "<"
            } else if self.eat(">") {
                ">"
            } else {
                break;
            };
            l = Expr::Bin(Box::new(l), op, Box::new(self.add()?));
        }
        Ok(l)
    }
    fn add(&mut self) -> Result<Expr, String> {
        let mut l = self.mul()?;
        loop {
            // `+=` / `++` belong to statements, not expressions
            let op = if self.peek() == Some('+') && !self.two('+', '+') && !self.two('+', '=') {
                self.i += 1;
                "+"
            } else if self.peek() == Some('-') && !self.two('-', '-') && !self.two('-', '=') {
                self.i += 1;
                "-"
            } else {
                break;
            };
            l = Expr::Bin(Box::new(l), op, Box::new(self.mul()?));
        }
        Ok(l)
    }
    fn mul(&mut self) -> Result<Expr, String> {
        let mut l = self.unary()?;
        loop {
            let op = match self.peek() {
                Some('*') => "*",
                Some('/') => "/",
                Some('%') => "%",
                _ => break,
            };
            self.i += 1;
            l = Expr::Bin(Box::new(l), op, Box::new(self.unary()?));
        }
        Ok(l)
    }
    fn unary(&mut self) -> Result<Expr, String> {
        match self.peek() {
            Some('-') => {
                self.i += 1;
                Ok(Expr::Neg(Box::new(self.unary()?)))
            }
            Some('+') => {
                self.i += 1;
                self.unary()
            }
            Some('!') if !self.two('!', '=') => {
                self.i += 1;
                Ok(Expr::Not(Box::new(self.unary()?)))
            }
            _ => self.primary(),
        }
    }
    fn primary(&mut self) -> Result<Expr, String> {
        let c = self.peek().ok_or("Unexpected end of expression")?;
        if c == '(' {
            self.i += 1;
            let e = self.ternary()?;
            if !self.eat(")") {
                return Err("Missing ')' in expression".to_string());
            }
            return Ok(e);
        }
        if c == '"' {
            self.i += 1;
            let mut s = String::new();
            loop {
                let ch = *self.c.get(self.i).ok_or("Unterminated string in expression")?;
                self.i += 1;
                match ch {
                    '"' => break,
                    '\\' => {
                        let n = *self.c.get(self.i).ok_or("Unterminated string in expression")?;
                        self.i += 1;
                        s.push(match n {
                            'n' => '\n',
                            't' => '\t',
                            o => o,
                        });
                    }
                    o => s.push(o),
                }
            }
            return Ok(Expr::Str(s));
        }
        if c == '`' {
            self.i += 1;
            let st = self.i;
            while self.i < self.c.len() && self.c[self.i] != '`' {
                self.i += 1;
            }
            if self.i >= self.c.len() {
                return Err("Unterminated backtick".to_string());
            }
            let t: String = self.c[st..self.i].iter().collect();
            self.i += 1;
            return Ok(Expr::Tick(t));
        }
        if c == '$' {
            self.i += 1;
            let st = self.i;
            while self.i < self.c.len() && is_word(self.c[self.i]) {
                self.i += 1;
            }
            if st == self.i {
                return Err("Expected a variable name after '$'".to_string());
            }
            return Ok(Expr::Var(self.c[st..self.i].iter().collect()));
        }
        if c.is_ascii_digit() || c == '.' {
            let st = self.i;
            while self.i < self.c.len() && (self.c[self.i].is_ascii_digit() || self.c[self.i] == '.') {
                self.i += 1;
            }
            if matches!(self.c.get(self.i), Some('e') | Some('E')) && self.c.get(self.i + 1).map(|d| d.is_ascii_digit() || *d == '-' || *d == '+').unwrap_or(false) {
                self.i += 2;
                while self.i < self.c.len() && self.c[self.i].is_ascii_digit() {
                    self.i += 1;
                }
            }
            let t: String = self.c[st..self.i].iter().collect();
            return t.parse::<f64>().map(Expr::Num).map_err(|_| format!("Bad number '{}'", t));
        }
        if is_word(c) {
            let st = self.i;
            while self.i < self.c.len() && (is_word(self.c[self.i]) || self.c[self.i] == '.') {
                self.i += 1;
            }
            let name: String = self.c[st..self.i].iter().collect();
            if self.peek() == Some('(') {
                self.i += 1;
                let mut args = Vec::new();
                if self.peek() == Some(')') {
                    self.i += 1;
                } else {
                    loop {
                        args.push(self.ternary()?);
                        if self.eat(",") {
                            continue;
                        }
                        if self.eat(")") {
                            break;
                        }
                        return Err(format!("Missing ')' after the arguments of {}", name));
                    }
                }
                return Ok(Expr::Call(name, args));
            }
            return Ok(match name.as_str() {
                "true" | "on" | "yes" => Expr::Num(1.0),
                "false" | "off" | "no" => Expr::Num(0.0),
                _ => Expr::Str(name),
            });
        }
        Err(format!("Unexpected '{}' in expression", c))
    }
}

pub fn parse_expr(text: &str) -> Result<Expr, String> {
    let mut x = X { c: text.chars().collect(), i: 0 };
    if x.peek().is_none() {
        return Err("Empty expression".to_string());
    }
    let e = x.ternary()?;
    if x.peek().is_some() {
        return Err(format!("Unexpected '{}' in expression", x.c[x.i]));
    }
    Ok(e)
}

// ====================================================================== variables

pub fn get_var(s: &Session, name: &str) -> Option<Value> {
    if let Some(f) = s.frames.last() {
        if !f.globals.contains(name) {
            if let Some(v) = f.vars.get(name) {
                return Some(v.clone());
            }
        }
    }
    s.vars.get(name).cloned()
}

fn set_var(s: &mut Session, name: &str, v: Value, declare: bool) {
    match s.frames.last_mut() {
        Some(f) if declare && !f.globals.contains(name) => {
            f.vars.insert(name.to_string(), v);
        }
        Some(f) if !f.globals.contains(name) && (f.vars.contains_key(name) || !s.vars.contains_key(name)) => {
            f.vars.insert(name.to_string(), v);
        }
        _ => {
            s.vars.insert(name.to_string(), v);
        }
    }
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Num(n) => *n != 0.0,
        Value::Str(t) => match t.trim().parse::<f64>() {
            Ok(n) => n != 0.0,
            Err(_) => !t.is_empty(),
        },
    }
}

fn num_of(v: &Value) -> Result<f64, String> {
    v.as_f64()
}

fn lines_to_value(lines: Vec<String>) -> Value {
    match lines.len() {
        0 => Value::Num(0.0),
        1 => match lines[0].trim().parse::<f64>() {
            Ok(n) => Value::Num(n),
            Err(_) => Value::Str(lines[0].clone()),
        },
        _ => Value::Str(lines.join(" ")),
    }
}

fn to_token(v: &Value) -> Token {
    match v {
        Value::Num(n) => Token::Num(*n),
        Value::Str(t) => Token::Str(t.clone()),
    }
}

// ====================================================================== evaluation

pub fn eval(s: &mut Session, e: &Expr) -> Result<Value, String> {
    Ok(match e {
        Expr::Num(n) => Value::Num(*n),
        Expr::Str(t) => Value::Str(t.clone()),
        Expr::Var(n) => get_var(s, n).ok_or_else(|| format!("Undefined variable: ${}", n))?,
        Expr::Neg(a) => Value::Num(-num_of(&eval(s, a)?)?),
        Expr::Not(a) => Value::Num(if truthy(&eval(s, a)?) { 0.0 } else { 1.0 }),
        Expr::And(a, b) => Value::Num((truthy(&eval(s, a)?) && truthy(&eval(s, b)?)) as i32 as f64),
        Expr::Or(a, b) => Value::Num((truthy(&eval(s, a)?) || truthy(&eval(s, b)?)) as i32 as f64),
        Expr::Cond(c, a, b) => {
            if truthy(&eval(s, c)?) {
                eval(s, a)?
            } else {
                eval(s, b)?
            }
        }
        Expr::Bin(a, op, b) => {
            let (l, r) = (eval(s, a)?, eval(s, b)?);
            match *op {
                "+" => match (&l, &r) {
                    (Value::Str(_), _) | (_, Value::Str(_)) => Value::Str(format!("{}{}", l.as_string(), r.as_string())),
                    _ => Value::Num(num_of(&l)? + num_of(&r)?),
                },
                "-" => Value::Num(num_of(&l)? - num_of(&r)?),
                "*" => Value::Num(num_of(&l)? * num_of(&r)?),
                "/" => {
                    let d = num_of(&r)?;
                    if d == 0.0 {
                        return Err("Division by zero".to_string());
                    }
                    Value::Num(num_of(&l)? / d)
                }
                "%" => {
                    let d = num_of(&r)?;
                    if d == 0.0 {
                        return Err("Division by zero".to_string());
                    }
                    Value::Num(num_of(&l)? % d)
                }
                "==" | "!=" => {
                    let same = match (&l, &r) {
                        (Value::Num(x), Value::Num(y)) => x == y,
                        _ => l.as_string() == r.as_string(),
                    };
                    Value::Num((same == (*op == "==")) as i32 as f64)
                }
                _ => {
                    let (x, y) = match (&l, &r) {
                        (Value::Str(p), Value::Str(q)) if p.parse::<f64>().is_err() || q.parse::<f64>().is_err() => {
                            let o = p.cmp(q);
                            let v = match *op {
                                "<" => o.is_lt(),
                                ">" => o.is_gt(),
                                "<=" => o.is_le(),
                                _ => o.is_ge(),
                            };
                            return Ok(Value::Num(v as i32 as f64));
                        }
                        _ => (num_of(&l)?, num_of(&r)?),
                    };
                    Value::Num(match *op {
                        "<" => x < y,
                        ">" => x > y,
                        "<=" => x <= y,
                        _ => x >= y,
                    } as i32 as f64)
                }
            }
        }
        Expr::Tick(text) => {
            let lines = run_command_text(s, text)?;
            lines_to_value(lines)
        }
        // catch(expr): 1 when evaluating the expression (usually a command in backticks) failed
        Expr::Call(name, args) if name == "catch" && args.len() == 1 => match eval(s, &args[0]) {
            Ok(_) => Value::Num(0.0),
            Err(e) => {
                s.last_error = e;
                Value::Num(1.0)
            }
        },
        // eval("commands"): runs the text, giving the last line of output
        Expr::Call(name, args) if name == "eval" && args.len() == 1 => {
            let text = eval(s, &args[0])?.as_string();
            let o = execute(s, &text);
            if let Some(e) = o.error {
                return Err(e);
            }
            lines_to_value(o.lines)
        }
        Expr::Call(name, args) => {
            let mut vals = Vec::with_capacity(args.len());
            for a in args {
                vals.push(eval(s, a)?);
            }
            call(s, name, vals)?
        }
    })
}

fn next_rand(s: &mut Session) -> f64 {
    let mut x = if s.rng == 0 { 0x9E37_79B9_7F4A_7C15 } else { s.rng };
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    s.rng = x;
    (x >> 11) as f64 / (1u64 << 53) as f64
}

fn call(s: &mut Session, name: &str, a: Vec<Value>) -> Result<Value, String> {
    if let Some(p) = s.procs.get(name).cloned() {
        let mut out = Vec::new();
        let r = call_proc(s, &p, a, &mut out);
        s.pending.extend(out);
        return Ok(r?.unwrap_or(Value::Num(0.0)));
    }
    let n = |i: usize| -> Result<f64, String> { a.get(i).ok_or_else(|| format!("{} needs more arguments", name))?.as_f64() };
    let one = |f: fn(f64) -> f64| -> Result<Value, String> { Ok(Value::Num(f(n(0)?))) };
    Ok(match name {
        "abs" => return one(f64::abs),
        "sqrt" => return one(f64::sqrt),
        "sin" => return one(f64::sin),
        "cos" => return one(f64::cos),
        "tan" => return one(f64::tan),
        "asin" => return one(f64::asin),
        "acos" => return one(f64::acos),
        "atan" => return one(f64::atan),
        "exp" => return one(f64::exp),
        "log" => return one(f64::ln),
        "log10" => return one(f64::log10),
        "floor" => return one(f64::floor),
        "ceil" => return one(f64::ceil),
        "trunc" | "int" => return one(f64::trunc),
        "float" => Value::Num(n(0)?),
        "sign" => Value::Num(if n(0)? > 0.0 { 1.0 } else if n(0)? < 0.0 { -1.0 } else { 0.0 }),
        "deg_to_rad" => Value::Num(n(0)?.to_radians()),
        "rad_to_deg" => Value::Num(n(0)?.to_degrees()),
        "atan2" => Value::Num(n(0)?.atan2(n(1)?)),
        "pow" => Value::Num(n(0)?.powf(n(1)?)),
        "fmod" => Value::Num(n(0)? % n(1)?),
        "min" => Value::Num(n(0)?.min(n(1)?)),
        "max" => Value::Num(n(0)?.max(n(1)?)),
        "clamp" => Value::Num(n(2)?.clamp(n(0)?.min(n(1)?), n(0)?.max(n(1)?))),
        "rand" => {
            let (lo, hi) = if a.len() >= 2 { (n(0)?, n(1)?) } else { (0.0, n(0).unwrap_or(1.0)) };
            Value::Num(lo + (hi - lo) * next_rand(s))
        }
        "seed" => {
            s.rng = n(0)? as u64 ^ 0x9E37_79B9_7F4A_7C15;
            Value::Num(0.0)
        }
        "size" | "strlen" => Value::Num(a.first().map(|v| v.as_string().chars().count()).unwrap_or(0) as f64),
        "tolower" => Value::Str(a.first().map(|v| v.as_string().to_lowercase()).unwrap_or_default()),
        "toupper" => Value::Str(a.first().map(|v| v.as_string().to_uppercase()).unwrap_or_default()),
        "substring" => {
            let t: Vec<char> = a.first().map(|v| v.as_string()).unwrap_or_default().chars().collect();
            let (from, to) = (n(1)?.max(1.0) as usize, n(2)?.max(0.0) as usize);
            Value::Str(if from > to || from > t.len() { String::new() } else { t[from - 1..to.min(t.len())].iter().collect() })
        }
        "strcmp" => Value::Num(match a.first().map(|v| v.as_string()).cmp(&a.get(1).map(|v| v.as_string())) {
            std::cmp::Ordering::Less => -1.0,
            std::cmp::Ordering::Equal => 0.0,
            std::cmp::Ordering::Greater => 1.0,
        }),
        _ => {
            // any other name is a command: getAttr("pCube1.tx"), objExists("a"), ...
            let mut toks = vec![Token::Word(name.to_string())];
            toks.extend(a.iter().map(to_token));
            lines_to_value(run_statement(s, &toks)?)
        }
    })
}

/// Call a user procedure; its printed output goes to `out`, its return value is returned.
pub fn call_proc(s: &mut Session, p: &Arc<Proc>, args: Vec<Value>, out: &mut Vec<String>) -> Result<Option<Value>, String> {
    if s.frames.len() >= MAX_CALL_DEPTH {
        return Err(format!("Procedure {} is nested too deeply (runaway recursion?)", p.name));
    }
    let mut f = Frame::default();
    for (i, name) in p.params.iter().enumerate() {
        f.vars.insert(name.clone(), args.get(i).cloned().unwrap_or(Value::Num(0.0)));
    }
    s.frames.push(f);
    let r = exec_block(s, &p.body, out);
    s.frames.pop();
    match r? {
        Flow::Return(v) => Ok(v),
        _ => Ok(None),
    }
}

/// Run a command given as text (the inside of backticks) and return what it prints.
fn run_command_text(s: &mut Session, text: &str) -> Result<Vec<String>, String> {
    let mut toks: Vec<Token> = lex(text)?.into_iter().map(|(t, _)| t).filter(|t| *t != Token::End).collect();
    if toks.is_empty() {
        return Ok(vec![]);
    }
    // a bare function call such as `rand(3)` or `myProc(1, 2)` inside backticks
    if let Ok(e @ Expr::Call(..)) = parse_expr(text) {
        let v = eval(s, &e)?;
        return Ok(vec![v.as_string()]);
    }
    toks = resolve_tokens(s, &toks)?;
    run_statement(s, &toks)
}

/// Replace `$var`, `( expression )` and backtick tokens by their values.
pub fn resolve_tokens(s: &mut Session, toks: &[Token]) -> Result<Vec<Token>, String> {
    let mut out = Vec::with_capacity(toks.len());
    for t in toks {
        match t {
            Token::Word(w) if w.starts_with('$') && w.len() > 1 => {
                let v = get_var(s, &w[1..]).ok_or_else(|| format!("Undefined variable: {}", w))?;
                out.push(to_token(&v));
            }
            Token::Group(text) => {
                let v = eval(s, &parse_expr(text)?)?;
                out.push(to_token(&v));
            }
            Token::Tick(text) => {
                let v = lines_to_value(run_command_text(s, text)?);
                out.push(to_token(&v));
            }
            other => out.push(other.clone()),
        }
    }
    Ok(out)
}

// ====================================================================== statements

struct Assign<'a> {
    name: &'a str,
    op: &'a str,
    rhs: &'a str,
    decl: Option<&'a str>,
    global: bool,
}

/// Recognize `int $i = 0`, `$x += 2`, `$i++`, `float $f` ...
fn parse_assign(t: &str) -> Result<Option<Assign<'_>>, String> {
    let mut rest = t.trim();
    let mut global = false;
    if let Some(r) = rest.strip_prefix("global ") {
        global = true;
        rest = r.trim_start();
    }
    let mut decl = None;
    for ty in ["int", "float", "string", "vector", "matrix"] {
        if let Some(r) = rest.strip_prefix(ty) {
            if r.starts_with(char::is_whitespace) {
                decl = Some(ty);
                rest = r.trim_start();
                break;
            }
        }
    }
    let (pre, rest) = if let Some(r) = rest.strip_prefix("++") {
        (Some("++"), r)
    } else if let Some(r) = rest.strip_prefix("--") {
        (Some("--"), r)
    } else {
        (None, rest)
    };
    let Some(r) = rest.strip_prefix('$') else {
        return if decl.is_some() || global { Err("Expected a variable after the type".to_string()) } else { Ok(None) };
    };
    let end = r.find(|c: char| !is_word(c)).unwrap_or(r.len());
    let (name, after) = (&r[..end], r[end..].trim_start());
    if name.is_empty() {
        return Ok(None);
    }
    if after.starts_with('[') || after.starts_with("{") {
        return Err("Arrays are not supported".to_string());
    }
    if let Some(p) = pre {
        return Ok(Some(Assign { name, op: p, rhs: "", decl, global }));
    }
    for op in ["++", "--", "+=", "-=", "*=", "/="] {
        if let Some(r) = after.strip_prefix(op) {
            return Ok(Some(Assign { name, op, rhs: r, decl, global }));
        }
    }
    if let Some(r) = after.strip_prefix('=') {
        if !r.starts_with('=') {
            return Ok(Some(Assign { name, op: "=", rhs: r, decl, global }));
        }
    }
    if after.is_empty() && (decl.is_some() || global) {
        return Ok(Some(Assign { name, op: "", rhs: "", decl, global }));
    }
    Ok(None)
}

fn coerce(ty: Option<&str>, v: Value) -> Result<Value, String> {
    Ok(match ty {
        Some("int") => Value::Num(v.as_f64()?.trunc()),
        Some("float") => Value::Num(v.as_f64()?),
        Some("string") => Value::Str(v.as_string()),
        _ => v,
    })
}

fn step(s: &mut Session) -> Result<(), String> {
    s.steps += 1;
    if s.steps > STEP_LIMIT {
        return Err("The script ran for too long and was stopped (infinite loop?)".to_string());
    }
    Ok(())
}

fn exec_cmd(s: &mut Session, text: &str, line: usize, out: &mut Vec<String>) -> Result<(), String> {
    step(s)?;
    // Echo All Commands: the history shows every command a script runs
    if s.echo_all {
        out.push(format!("{};", text.trim()));
    }
    let wrap = |s: &Session, e: String| if s.err_lines && !e.starts_with("Line ") { format!("Line {}: {}", line, e) } else { e };
    let run = |s: &mut Session, out: &mut Vec<String>| -> Result<(), String> {
        if let Some(a) = parse_assign(text)? {
            if a.global {
                if let Some(f) = s.frames.last_mut() {
                    f.globals.insert(a.name.to_string());
                }
            }
            let declare = a.decl.is_some() && !a.global;
            let current = get_var(s, a.name);
            let val = match a.op {
                "" => match a.decl {
                    Some("string") => Value::Str(String::new()),
                    _ => Value::Num(0.0),
                },
                "++" | "--" => {
                    let c = current.clone().ok_or_else(|| format!("Undefined variable: ${}", a.name))?.as_f64()?;
                    Value::Num(if a.op == "++" { c + 1.0 } else { c - 1.0 })
                }
                "=" => eval(s, &parse_expr(a.rhs)?)?,
                op => {
                    let c = current.clone().ok_or_else(|| format!("Undefined variable: ${}", a.name))?;
                    let r = eval(s, &parse_expr(a.rhs)?)?;
                    let bin = Expr::Bin(Box::new(value_expr(c)), &["+", "-", "*", "/"][["+=", "-=", "*=", "/="].iter().position(|o| *o == op).unwrap()], Box::new(value_expr(r)));
                    eval(s, &bin)?
                }
            };
            // a declaration without a value keeps an existing global
            if a.op.is_empty() && !declare && current.is_some() {
                return Ok(());
            }
            let val = coerce(a.decl, val)?;
            set_var(s, a.name, val, declare);
            return Ok(());
        }
        // `myProc(1, 2);` calls a procedure like a command
        if let Ok(Expr::Call(name, args)) = parse_expr(text) {
            if let Some(p) = s.procs.get(&name).cloned() {
                let mut vals = Vec::new();
                for a in &args {
                    vals.push(eval(s, a)?);
                }
                if let Some(v) = call_proc(s, &p, vals, out)? {
                    out.push(v.as_string());
                }
                return Ok(());
            }
        }
        let toks: Vec<Token> = lex(text)?.into_iter().map(|(t, _)| t).filter(|t| *t != Token::End).collect();
        if toks.is_empty() {
            return Ok(());
        }
        let toks = resolve_tokens(s, &toks)?;
        out.extend(run_statement(s, &toks)?);
        Ok(())
    };
    let r = run(s, out).map_err(|e| wrap(s, e));
    out.extend(s.pending.drain(..));
    r
}

fn value_expr(v: Value) -> Expr {
    match v {
        Value::Num(n) => Expr::Num(n),
        Value::Str(t) => Expr::Str(t),
    }
}

fn exec_block(s: &mut Session, stmts: &[Stmt], out: &mut Vec<String>) -> Result<Flow, String> {
    for st in stmts {
        match exec(s, st, out)? {
            Flow::Next => {}
            other => return Ok(other),
        }
    }
    Ok(Flow::Next)
}

fn exec(s: &mut Session, st: &Stmt, out: &mut Vec<String>) -> Result<Flow, String> {
    step(s)?;
    match st {
        Stmt::Cmd { text, line } => {
            exec_cmd(s, text, *line, out)?;
            Ok(Flow::Next)
        }
        Stmt::Block(v) => exec_block(s, v, out),
        Stmt::If { cond, then, els } => {
            if truthy(&eval(s, cond)?) {
                exec(s, then, out)
            } else if let Some(e) = els {
                exec(s, e, out)
            } else {
                Ok(Flow::Next)
            }
        }
        Stmt::While { cond, body } => {
            while truthy(&eval(s, cond)?) {
                match exec(s, body, out)? {
                    Flow::Break => break,
                    Flow::Return(v) => return Ok(Flow::Return(v)),
                    _ => {}
                }
                step(s)?;
            }
            Ok(Flow::Next)
        }
        Stmt::For { init, cond, step: stp, body, line } => {
            exec_cmd(s, init, *line, out)?;
            loop {
                if let Some(c) = cond {
                    if !truthy(&eval(s, c)?) {
                        break;
                    }
                }
                match exec(s, body, out)? {
                    Flow::Break => break,
                    Flow::Return(v) => return Ok(Flow::Return(v)),
                    _ => {}
                }
                exec_cmd(s, stp, *line, out)?;
            }
            Ok(Flow::Next)
        }
        Stmt::Proc(p) => {
            s.procs.insert(p.name.clone(), p.clone());
            Ok(Flow::Next)
        }
        Stmt::Return(e) => Ok(Flow::Return(match e {
            Some(e) => Some(eval(s, e)?),
            None => None,
        })),
        Stmt::Break => Ok(Flow::Break),
        Stmt::Continue => Ok(Flow::Continue),
    }
}

/// Run a user procedure used as a command (`myProc 1 2;`).
pub fn run_proc_command(s: &mut Session, name: &str, rest: &[Token]) -> Option<Result<Vec<String>, String>> {
    let p = s.procs.get(name).cloned()?;
    let args: Vec<Value> = rest
        .iter()
        .filter_map(|t| match t {
            Token::Num(n) => Some(Value::Num(*n)),
            Token::Str(x) | Token::Word(x) => Some(Value::Str(x.clone())),
            _ => None,
        })
        .collect();
    let mut out = Vec::new();
    Some(call_proc(s, &p, args, &mut out).map(|r| {
        if let Some(v) = r {
            out.push(v.as_string());
        }
        out
    }))
}

/// Entry point used by `execute_script`.
pub fn execute(s: &mut Session, src: &str) -> Outcome {
    let mut lines = Vec::new();
    let prog = match parse_program(src) {
        Ok(p) => p,
        Err(e) => return Outcome { lines, error: Some(e) },
    };
    let top = s.exec_depth == 0;
    if top {
        s.steps = 0;
    }
    s.exec_depth += 1;
    let saved_err_lines = s.err_lines;
    s.err_lines = src.contains('\n');
    for st in &prog {
        if let Stmt::Proc(p) = st {
            s.procs.insert(p.name.clone(), p.clone());
        }
    }
    let r = exec_block(s, &prog, &mut lines);
    s.err_lines = saved_err_lines;
    s.exec_depth -= 1;
    s.frames.truncate(if top { 0 } else { s.frames.len() });
    match r {
        Ok(Flow::Return(Some(v))) => {
            lines.push(v.as_string());
            Outcome { lines, error: None }
        }
        Ok(_) => Outcome { lines, error: None },
        Err(e) => Outcome { lines, error: Some(e) },
    }
}

pub fn fmt_value(v: &Value) -> String {
    match v {
        Value::Num(n) => fmt_num(*n),
        Value::Str(t) => t.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> (Vec<String>, Option<String>) {
        let mut s = Session::new();
        let o = execute(&mut s, src);
        (o.lines, o.error)
    }

    fn ok(src: &str) -> Vec<String> {
        let (l, e) = run(src);
        assert_eq!(e, None, "script failed: {}", src);
        l
    }

    #[test]
    fn arithmetic_precedence_and_strings() {
        assert_eq!(ok("print (1 + 2 * 3 - 4 / 2);"), ["5"]);
        assert_eq!(ok("print (\"a\" + 1 + \"b\");"), ["a1b"]);
        assert_eq!(ok("print ((2 + 3) * 4 % 7);"), ["6"]);
        assert_eq!(ok("print (-3 + 5);"), ["2"]);
        assert_eq!(ok("print (!0 + !5);"), ["1"]);
    }

    #[test]
    fn comparisons_logic_and_ternary() {
        assert_eq!(ok("print (3 > 2 && 2 >= 2 || 0);"), ["1"]);
        assert_eq!(ok("print (\"abc\" == \"abc\");"), ["1"]);
        assert_eq!(ok("print (\"abc\" != \"abd\");"), ["1"]);
        assert_eq!(ok("print (1 < 2 ? \"yes\" : \"no\");"), ["yes"]);
        assert_eq!(ok("$x = 4; print ($x == 4 ? $x * 2 : 0);"), ["8"]);
    }

    #[test]
    fn loops_break_and_continue() {
        let src = "int $s = 0; for ($i = 1; $i <= 10; $i++) { if ($i % 2) continue; if ($i > 8) break; $s += $i; } print $s;";
        assert_eq!(ok(src), ["20"]);
        assert_eq!(ok("$i = 0; while ($i < 5) $i++; print $i;"), ["5"]);
        assert_eq!(ok("for ($i = 0; $i < 3; $i++) print $i;"), ["0", "1", "2"]);
    }

    #[test]
    fn if_else_chains() {
        let src = "$v = 5; if ($v < 3) print \"low\"; else if ($v < 7) print \"mid\"; else print \"high\";";
        assert_eq!(ok(src), ["mid"]);
        assert_eq!(ok("if (0) { print 1 } else { print 2 }"), ["2"]);
        assert_eq!(ok("if (1)\n{\n print 1\n}\nelse\n{\n print 2\n}\n"), ["1"]);
    }

    #[test]
    fn procs_return_values_and_recursion() {
        let src = "proc int fib(int $n) { if ($n < 2) return $n; return fib($n - 1) + fib($n - 2); }\nprint (fib(10));";
        assert_eq!(ok(src), ["55"]);
        assert_eq!(ok("global proc string greet(string $w) { return (\"hi \" + $w); } print (greet(\"there\"));"), ["hi there"]);
        // procedures may be used before their definition because definitions are hoisted
        assert_eq!(ok("print (twice(4)); proc int twice(int $x) { return $x * 2; }"), ["8"]);
    }

    #[test]
    fn proc_called_as_command_and_statement() {
        assert_eq!(ok("proc show(string $a, float $b) { print ($a + \":\" + $b); } show \"v\" 2.5; show(\"w\", 1);"), ["v:2.5", "w:1"]);
    }

    #[test]
    fn variable_scoping() {
        // a proc local does not leak, a global declared inside does
        let src = "proc f() { int $loc = 5; global int $g; $g = 7; } f(); print $g;";
        assert_eq!(ok(src), ["7"]);
        let (_, e) = run("proc f() { int $loc = 5; } f(); print $loc;");
        assert!(e.unwrap().contains("Undefined variable"));
    }

    #[test]
    fn typed_declarations_coerce() {
        assert_eq!(ok("int $i = 7.9; print $i;"), ["7"]);
        assert_eq!(ok("string $s = 12; print ($s + \"!\");"), ["12!"]);
        assert_eq!(ok("float $f; print $f;"), ["0"]);
        assert_eq!(ok("$a = 10; $a -= 3; $a *= 2; $a /= 7; print $a;"), ["2"]);
    }

    #[test]
    fn backticks_and_command_calls() {
        assert_eq!(ok("polyCube -name c; setAttr \"c.translateX\" 4; print (`getAttr \"c.translateX\"` + 1);"), ["c", "5"]);
        assert_eq!(ok("polyCube -name c; print (objExists(\"c\") + objExists(\"nope\"));"), ["c", "1"]);
        assert_eq!(ok("polyCube -name c; $n = `polyEvaluate -f c`; print $n;"), ["c", "6"]);
    }

    #[test]
    fn expressions_as_command_arguments() {
        assert_eq!(ok("polyCube -name c; for ($i = 1; $i <= 3; $i++) move ($i * 2) 0 0 c; print (`getAttr \"c.translateX\"`);"), ["c", "6"]);
    }

    #[test]
    fn math_and_string_functions() {
        assert_eq!(ok("print (abs(-2) + sqrt(9) + floor(2.7) + ceil(0.2) + pow(2, 3));"), ["16"]);
        assert_eq!(ok("print (substring(\"abcdef\", 2, 4) + toupper(\"x\") + size(\"four\"));"), ["bcdX4"]);
        assert_eq!(ok("print (clamp(0, 1, 5) + min(3, 2));"), ["3"]);
        assert_eq!(ok("print (deg_to_rad(180));"), ["3.141593"]);
    }

    #[test]
    fn runaway_scripts_are_stopped() {
        let (_, e) = run("$i = 0; while (1) { $i++; }");
        assert!(e.unwrap().contains("too long"));
        let (_, e) = run("proc r() { r(); } r();");
        assert!(e.unwrap().contains("nested too deeply"));
    }

    #[test]
    fn syntax_errors_name_the_line() {
        let (_, e) = run("print 1\nif (1) {\n print 2\n");
        assert!(e.unwrap().contains("missing '}'"));
        let (_, e) = run("print 1\nmove 1 2 nope\n");
        assert!(e.unwrap().starts_with("Line 2:"));
        let (_, e) = run("print (1 +);");
        assert!(e.is_some());
        let (_, e) = run("for ($i = 0; $i < 3) print 1;");
        assert!(e.unwrap().contains("for needs"));
    }

    #[test]
    fn comments_and_strings_with_special_characters() {
        assert_eq!(ok("print \"a;b\"; // trailing ; comment\nprint \"x // y\"; /* block ; */ print 3;"), ["a;b", "x // y", "3"]);
        assert_eq!(ok("print (\"(paren)\" + \"{brace}\");"), ["(paren){brace}"]);
    }

    #[test]
    fn single_line_errors_have_no_prefix() {
        let (_, e) = run("move 1 2 nope");
        assert!(!e.unwrap().starts_with("Line"));
    }
}
