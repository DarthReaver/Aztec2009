//! Expressions (Window > Animation Editors > Expression Editor): small MEL-style programs that set attributes from other
//! attributes, the time and math functions. They run after the animation of every frame and after every attribute change.

use super::graph::Scene;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Expression {
    pub name: String,
    pub text: String,
    /// The object the expression belongs to (informational, as in Maya's -object flag).
    pub object: String,
    pub enabled: bool,
    prog: Arc<Vec<Stmt>>,
    /// Set when the last run failed (missing attribute, ...).
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
enum Target {
    Var(String),
    Attr(String, String),
}

#[derive(Clone, Debug)]
enum Stmt {
    Assign { target: Target, op: char, value: Expr, declare: bool },
    If { cond: Expr, then: Box<Stmt>, els: Option<Box<Stmt>> },
    Block(Vec<Stmt>),
    Eval(Expr),
}

#[derive(Clone, Debug)]
enum Expr {
    Num(f64),
    Var(String),
    Attr(String, String),
    Call(String, Vec<Expr>),
    Neg(Box<Expr>),
    Not(Box<Expr>),
    Bin(Box<Expr>, String, Box<Expr>),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
}

// ---------------------------------------------------------------------------------------------- parser

struct P {
    c: Vec<char>,
    i: usize,
}

fn word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

impl P {
    fn ws(&mut self) {
        loop {
            while self.i < self.c.len() && self.c[self.i].is_whitespace() {
                self.i += 1;
            }
            if self.c.get(self.i) == Some(&'/') && self.c.get(self.i + 1) == Some(&'/') {
                while self.i < self.c.len() && self.c[self.i] != '\n' {
                    self.i += 1;
                }
            } else if self.c.get(self.i) == Some(&'/') && self.c.get(self.i + 1) == Some(&'*') {
                self.i += 2;
                while self.i + 1 < self.c.len() && !(self.c[self.i] == '*' && self.c[self.i + 1] == '/') {
                    self.i += 1;
                }
                self.i += 2;
            } else {
                break;
            }
        }
    }

    fn peek(&mut self) -> Option<char> {
        self.ws();
        self.c.get(self.i).copied()
    }

    fn eat(&mut self, s: &str) -> bool {
        self.ws();
        let n = s.chars().count();
        if self.c.len() >= self.i + n && self.c[self.i..self.i + n].iter().collect::<String>() == s {
            // an operator must not be the start of a longer one
            if let Some(next) = self.c.get(self.i + n) {
                let longer = matches!((s, *next), ("=", '=') | ("<", '=') | (">", '=') | ("!", '=') | ("|", '|') | ("&", '&'));
                if longer {
                    return false;
                }
            }
            self.i += n;
            true
        } else {
            false
        }
    }

    fn word(&mut self) -> String {
        self.ws();
        let mut j = self.i;
        while j < self.c.len() && word_char(self.c[j]) {
            j += 1;
        }
        self.c[self.i..j].iter().collect()
    }

    fn at_word(&mut self, w: &str) -> bool {
        self.word() == w
    }

    fn err<T>(&self, m: &str) -> Result<T, String> {
        let line = self.c[..self.i.min(self.c.len())].iter().filter(|c| **c == '\n').count() + 1;
        Err(format!("Expression line {}: {}", line, m))
    }

    fn stmts(&mut self, until_brace: bool) -> Result<Vec<Stmt>, String> {
        let mut v = Vec::new();
        loop {
            self.ws();
            while self.eat(";") {}
            match self.peek() {
                None => {
                    if until_brace {
                        return self.err("missing }");
                    }
                    return Ok(v);
                }
                Some('}') if until_brace => {
                    self.i += 1;
                    return Ok(v);
                }
                _ => v.push(self.stmt()?),
            }
        }
    }

    fn stmt(&mut self) -> Result<Stmt, String> {
        if self.peek() == Some('{') {
            self.i += 1;
            return Ok(Stmt::Block(self.stmts(true)?));
        }
        if self.at_word("if") {
            self.i += 2;
            if !self.eat("(") {
                return self.err("if needs (condition)");
            }
            let cond = self.ternary()?;
            if !self.eat(")") {
                return self.err("missing ) after the condition");
            }
            let then = Box::new(self.stmt()?);
            self.ws();
            let save = self.i;
            while self.eat(";") {}
            let els = if self.at_word("else") {
                self.i += 4;
                Some(Box::new(self.stmt()?))
            } else {
                self.i = save;
                None
            };
            return Ok(Stmt::If { cond, then, els });
        }
        let w = self.word();
        let declare = matches!(w.as_str(), "float" | "int" | "string" | "vector");
        if declare {
            self.i += w.chars().count();
        }
        // assignment or a plain expression
        let start = self.i;
        if let Some(t) = self.target()? {
            for op in ["+=", "-=", "*=", "/="] {
                if self.eat(op) {
                    let value = self.ternary()?;
                    self.end_stmt()?;
                    return Ok(Stmt::Assign { target: t, op: op.chars().next().unwrap(), value, declare });
                }
            }
            if self.eat("=") {
                let value = self.ternary()?;
                self.end_stmt()?;
                return Ok(Stmt::Assign { target: t, op: '=', value, declare });
            }
            if declare {
                self.end_stmt()?;
                return Ok(Stmt::Assign { target: t, op: '=', value: Expr::Num(0.0), declare });
            }
        }
        self.i = start;
        let e = self.ternary()?;
        self.end_stmt()?;
        Ok(Stmt::Eval(e))
    }

    fn end_stmt(&mut self) -> Result<(), String> {
        if self.eat(";") || self.peek().is_none() || self.peek() == Some('}') {
            Ok(())
        } else {
            self.err("expected ;")
        }
    }

    /// `$var` or `node.attr` at the cursor.
    fn target(&mut self) -> Result<Option<Target>, String> {
        self.ws();
        if self.c.get(self.i) == Some(&'$') {
            self.i += 1;
            let w = self.word();
            if w.is_empty() {
                return self.err("a variable needs a name after $");
            }
            self.i += w.chars().count();
            return Ok(Some(Target::Var(w)));
        }
        let save = self.i;
        let mut j = self.i;
        while j < self.c.len() && (word_char(self.c[j]) || self.c[j] == '|' || self.c[j] == ':') {
            j += 1;
        }
        if j > self.i && self.c.get(j) == Some(&'.') {
            let node: String = self.c[self.i..j].iter().collect();
            self.i = j + 1;
            let attr = self.word();
            if attr.is_empty() {
                self.i = save;
                return Ok(None);
            }
            self.i += attr.chars().count();
            // an array index such as .translate[0] is not supported; a trailing .x style is part of the name
            return Ok(Some(Target::Attr(node, attr)));
        }
        self.i = save;
        Ok(None)
    }

    fn ternary(&mut self) -> Result<Expr, String> {
        let c = self.or()?;
        if self.eat("?") {
            let a = self.ternary()?;
            if !self.eat(":") {
                return self.err("a ? b : c needs the colon");
            }
            let b = self.ternary()?;
            return Ok(Expr::Ternary(Box::new(c), Box::new(a), Box::new(b)));
        }
        Ok(c)
    }

    fn bin(&mut self, ops: &[&str], next: fn(&mut P) -> Result<Expr, String>) -> Result<Expr, String> {
        let mut l = next(self)?;
        'outer: loop {
            for op in ops {
                if self.eat(op) {
                    let r = next(self)?;
                    l = Expr::Bin(Box::new(l), op.to_string(), Box::new(r));
                    continue 'outer;
                }
            }
            return Ok(l);
        }
    }

    fn or(&mut self) -> Result<Expr, String> {
        self.bin(&["||"], P::and)
    }
    fn and(&mut self) -> Result<Expr, String> {
        self.bin(&["&&"], P::eq)
    }
    fn eq(&mut self) -> Result<Expr, String> {
        self.bin(&["==", "!="], P::rel)
    }
    fn rel(&mut self) -> Result<Expr, String> {
        self.bin(&["<=", ">=", "<", ">"], P::add)
    }
    fn add(&mut self) -> Result<Expr, String> {
        self.bin(&["+", "-"], P::mul)
    }
    fn mul(&mut self) -> Result<Expr, String> {
        self.bin(&["*", "/", "%"], P::unary)
    }

    fn unary(&mut self) -> Result<Expr, String> {
        if self.eat("-") {
            return Ok(Expr::Neg(Box::new(self.unary()?)));
        }
        if self.eat("!") {
            return Ok(Expr::Not(Box::new(self.unary()?)));
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Expr, String> {
        match self.peek() {
            None => self.err("the expression ends too early"),
            Some('(') => {
                self.i += 1;
                let e = self.ternary()?;
                if !self.eat(")") {
                    return self.err("missing )");
                }
                Ok(e)
            }
            Some(c) if c.is_ascii_digit() || c == '.' => {
                let mut j = self.i;
                while j < self.c.len() && (self.c[j].is_ascii_digit() || self.c[j] == '.') {
                    j += 1;
                }
                // 1e-3 style exponents
                if matches!(self.c.get(j), Some('e') | Some('E')) && self.c.get(j + 1).map(|c| c.is_ascii_digit() || *c == '-' || *c == '+').unwrap_or(false) {
                    j += 2;
                    while j < self.c.len() && self.c[j].is_ascii_digit() {
                        j += 1;
                    }
                }
                let s: String = self.c[self.i..j].iter().collect();
                self.i = j;
                s.parse::<f64>().map(Expr::Num).or_else(|_| self.err(&format!("'{}' is not a number", s)))
            }
            Some('$') => {
                self.i += 1;
                let w = self.word();
                self.i += w.chars().count();
                Ok(Expr::Var(w))
            }
            Some(_) => {
                if let Some(Target::Attr(n, a)) = self.target()? {
                    return Ok(Expr::Attr(n, a));
                }
                let w = self.word();
                if w.is_empty() {
                    return self.err("unexpected character");
                }
                self.i += w.chars().count();
                if self.eat("(") {
                    let mut args = Vec::new();
                    if !self.eat(")") {
                        loop {
                            args.push(self.ternary()?);
                            if self.eat(")") {
                                break;
                            }
                            if !self.eat(",") {
                                return self.err("missing , or ) in the argument list");
                            }
                        }
                    }
                    return Ok(Expr::Call(w, args));
                }
                match w.as_str() {
                    "true" | "on" | "yes" => Ok(Expr::Num(1.0)),
                    "false" | "off" | "no" => Ok(Expr::Num(0.0)),
                    "time" | "frame" | "pi" | "fps" => Ok(Expr::Call(w, Vec::new())),
                    _ => self.err(&format!("don't know '{}': attributes are written node.attribute, variables $name", w)),
                }
            }
        }
    }
}

fn parse(text: &str) -> Result<Vec<Stmt>, String> {
    let mut p = P { c: text.chars().collect(), i: 0 };
    p.stmts(false)
}

// ---------------------------------------------------------------------------------------------- evaluation

fn hash01(a: u64, b: u64) -> f64 {
    let mut h = a.wrapping_mul(0x9E3779B97F4A7C15) ^ b.wrapping_mul(0xC2B2AE3D27D4EB4F);
    h ^= h >> 29;
    h = h.wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 32;
    (h >> 11) as f64 / (1u64 << 53) as f64
}

fn noise1(x: f64) -> f64 {
    let (i, f) = (x.floor(), x - x.floor());
    let s = f * f * (3.0 - 2.0 * f);
    let (a, b) = (hash01(i as i64 as u64, 7) * 2.0 - 1.0, hash01((i + 1.0) as i64 as u64, 7) * 2.0 - 1.0);
    a + (b - a) * s
}

struct Ctx<'a> {
    scene: &'a mut Scene,
    vars: std::collections::HashMap<String, f64>,
    seed: u64,
    calls: u64,
}

impl Ctx<'_> {
    fn attr(&mut self, node: &str, attr: &str) -> Result<f64, String> {
        let id = self.scene.find(node).ok_or_else(|| format!("No object matches name: {}", node))?;
        let a = super::plugs::canonical_attr(attr);
        self.scene.get_attr(id, &a).map(|v| v as f64)
    }

    fn eval(&mut self, e: &Expr) -> Result<f64, String> {
        Ok(match e {
            Expr::Num(n) => *n,
            Expr::Var(v) => *self.vars.get(v).ok_or_else(|| format!("The variable ${} has no value", v))?,
            Expr::Attr(n, a) => self.attr(n, a)?,
            Expr::Neg(x) => -self.eval(x)?,
            Expr::Not(x) => (self.eval(x)? == 0.0) as u8 as f64,
            Expr::Ternary(c, a, b) => {
                if self.eval(c)? != 0.0 {
                    self.eval(a)?
                } else {
                    self.eval(b)?
                }
            }
            Expr::Bin(l, op, r) => {
                let a = self.eval(l)?;
                // short circuit
                if op == "&&" {
                    return Ok((a != 0.0 && self.eval(r)? != 0.0) as u8 as f64);
                }
                if op == "||" {
                    return Ok((a != 0.0 || self.eval(r)? != 0.0) as u8 as f64);
                }
                let b = self.eval(r)?;
                match op.as_str() {
                    "+" => a + b,
                    "-" => a - b,
                    "*" => a * b,
                    "/" => {
                        if b == 0.0 {
                            return Err("Division by zero".to_string());
                        }
                        a / b
                    }
                    "%" => {
                        if b == 0.0 {
                            return Err("Division by zero".to_string());
                        }
                        a % b
                    }
                    "==" => (a == b) as u8 as f64,
                    "!=" => (a != b) as u8 as f64,
                    "<" => (a < b) as u8 as f64,
                    ">" => (a > b) as u8 as f64,
                    "<=" => (a <= b) as u8 as f64,
                    _ => (a >= b) as u8 as f64,
                }
            }
            Expr::Call(f, args) => {
                let v: Vec<f64> = args.iter().map(|a| self.eval(a)).collect::<Result<_, _>>()?;
                let need = |n: usize| -> Result<(), String> { if v.len() == n { Ok(()) } else { Err(format!("{} takes {} argument(s)", f, n)) } };
                match f.as_str() {
                    "time" => self.scene.timeline.current as f64 / self.scene.timeline.fps.max(1.0) as f64,
                    "frame" => self.scene.timeline.current as f64,
                    "fps" => self.scene.timeline.fps as f64,
                    "pi" => std::f64::consts::PI,
                    "sin" => { need(1)?; v[0].sin() }
                    "cos" => { need(1)?; v[0].cos() }
                    "tan" => { need(1)?; v[0].tan() }
                    "asin" => { need(1)?; v[0].asin() }
                    "acos" => { need(1)?; v[0].acos() }
                    "atan" => { need(1)?; v[0].atan() }
                    "atan2" => { need(2)?; v[0].atan2(v[1]) }
                    "abs" => { need(1)?; v[0].abs() }
                    "sqrt" => { need(1)?; if v[0] < 0.0 { return Err("sqrt of a negative number".to_string()); } v[0].sqrt() }
                    "exp" => { need(1)?; v[0].exp() }
                    "log" => { need(1)?; if v[0] <= 0.0 { return Err("log of a number that is not positive".to_string()); } v[0].ln() }
                    "pow" => { need(2)?; v[0].powf(v[1]) }
                    "floor" => { need(1)?; v[0].floor() }
                    "ceil" => { need(1)?; v[0].ceil() }
                    "trunc" => { need(1)?; v[0].trunc() }
                    "sign" => { need(1)?; if v[0] > 0.0 { 1.0 } else if v[0] < 0.0 { -1.0 } else { 0.0 } }
                    "fmod" => { need(2)?; if v[1] == 0.0 { return Err("fmod by zero".to_string()); } v[0] % v[1] }
                    "min" => { need(2)?; v[0].min(v[1]) }
                    "max" => { need(2)?; v[0].max(v[1]) }
                    "clamp" => { need(3)?; v[2].clamp(v[0].min(v[1]), v[0].max(v[1])) }
                    "smoothstep" => { need(3)?; let t = ((v[2] - v[0]) / (v[1] - v[0]).max(1e-12)).clamp(0.0, 1.0); t * t * (3.0 - 2.0 * t) }
                    "deg_to_rad" => { need(1)?; v[0].to_radians() }
                    "rad_to_deg" => { need(1)?; v[0].to_degrees() }
                    "noise" => { if v.is_empty() { return Err("noise needs a value".to_string()); } noise1(v[0] + v.get(1).copied().unwrap_or(0.0) * 17.3 + v.get(2).copied().unwrap_or(0.0) * 31.7) }
                    "rand" | "random" => {
                        self.calls += 1;
                        let r = hash01(self.seed ^ (self.scene.timeline.current.to_bits() as u64) << 7, self.calls);
                        match v.len() {
                            1 => r * v[0],
                            2 => v[0] + r * (v[1] - v[0]),
                            _ => return Err("rand takes one or two arguments".to_string()),
                        }
                    }
                    other => return Err(format!("Unknown function '{}'", other)),
                }
            }
        })
    }

    fn run(&mut self, s: &Stmt) -> Result<(), String> {
        match s {
            Stmt::Block(b) => {
                for x in b {
                    self.run(x)?;
                }
            }
            Stmt::Eval(e) => {
                self.eval(e)?;
            }
            Stmt::If { cond, then, els } => {
                if self.eval(cond)? != 0.0 {
                    self.run(then)?;
                } else if let Some(e) = els {
                    self.run(e)?;
                }
            }
            Stmt::Assign { target, op, value, declare } => {
                let v = self.eval(value)?;
                match target {
                    Target::Var(n) => {
                        let cur = if *declare { 0.0 } else { self.vars.get(n).copied().unwrap_or(0.0) };
                        let nv = combine(cur, *op, v)?;
                        self.vars.insert(n.clone(), nv);
                    }
                    Target::Attr(node, attr) => {
                        let id = self.scene.find(node).ok_or_else(|| format!("No object matches name: {}", node))?;
                        let a = super::plugs::canonical_attr(attr);
                        let cur = if *op == '=' { 0.0 } else { self.scene.get_attr(id, &a)? as f64 };
                        let nv = combine(cur, *op, v)?;
                        let plug = self.scene.nodes.get(id).and_then(|n| n.plug(&a)).ok_or_else(|| format!("Attribute '{}' not found on '{}'", a, node))?;
                        if plug.source.is_some() {
                            return Err(format!("{}.{} is connected and cannot be driven by an expression", node, a));
                        }
                        self.scene.set_eval(id, &a, nv as f32);
                    }
                }
            }
        }
        Ok(())
    }
}

fn combine(cur: f64, op: char, v: f64) -> Result<f64, String> {
    Ok(match op {
        '+' => cur + v,
        '-' => cur - v,
        '*' => cur * v,
        '/' => {
            if v == 0.0 {
                return Err("Division by zero".to_string());
            }
            cur / v
        }
        _ => v,
    })
}

impl Scene {
    /// Create an expression. It runs once at once, so a wrong attribute name is reported here.
    pub fn add_expression(&mut self, name: Option<&str>, text: &str, object: &str) -> Result<String, String> {
        let prog = parse(text)?;
        let n = match name {
            Some(n) => {
                if self.expressions.iter().any(|e| e.name == n) {
                    return Err(format!("An expression named {} exists", n));
                }
                n.to_string()
            }
            None => {
                let mut i = 1;
                while self.expressions.iter().any(|e| e.name == format!("expression{}", i)) {
                    i += 1;
                }
                format!("expression{}", i)
            }
        };
        let ex = Expression { name: n.clone(), text: text.to_string(), object: object.to_string(), enabled: true, prog: Arc::new(prog), error: None };
        self.expressions.push(ex);
        self.eval_expressions();
        if let Some(e) = self.expressions.iter().find(|e| e.name == n).and_then(|e| e.error.clone()) {
            self.expressions.retain(|x| x.name != n);
            return Err(e);
        }
        Ok(n)
    }

    /// Used when a scene loads: keep the expression even if its nodes are missing for now.
    pub fn add_expression_loaded(&mut self, name: &str, text: &str, object: &str, enabled: bool) -> Result<(), String> {
        let prog = parse(text)?;
        self.expressions.push(Expression { name: name.to_string(), text: text.to_string(), object: object.to_string(), enabled, prog: Arc::new(prog), error: None });
        Ok(())
    }

    pub fn edit_expression(&mut self, name: &str, text: &str) -> Result<(), String> {
        let prog = parse(text)?;
        let ex = self.expressions.iter_mut().find(|e| e.name == name).ok_or_else(|| format!("No expression named {}", name))?;
        let old = (ex.text.clone(), ex.prog.clone());
        ex.text = text.to_string();
        ex.prog = Arc::new(prog);
        self.eval_expressions();
        if let Some(e) = self.expressions.iter().find(|e| e.name == name).and_then(|e| e.error.clone()) {
            if let Some(ex) = self.expressions.iter_mut().find(|e| e.name == name) {
                ex.text = old.0;
                ex.prog = old.1;
                ex.error = None;
            }
            return Err(e);
        }
        Ok(())
    }

    pub fn delete_expression(&mut self, name: &str) -> Result<(), String> {
        let n = self.expressions.len();
        self.expressions.retain(|e| e.name != name);
        if self.expressions.len() == n {
            return Err(format!("No expression named {}", name));
        }
        Ok(())
    }

    /// Run every enabled expression.
    pub fn eval_expressions(&mut self) {
        if self.expressions.is_empty() || self.in_expression {
            return;
        }
        self.in_expression = true;
        let list: Vec<(usize, Arc<Vec<Stmt>>, String, bool)> = self.expressions.iter().enumerate().map(|(i, e)| (i, e.prog.clone(), e.name.clone(), e.enabled)).collect();
        for (i, prog, name, enabled) in list {
            if !enabled {
                continue;
            }
            let seed = name.bytes().fold(1469598103934665603u64, |h, b| (h ^ b as u64).wrapping_mul(1099511628211));
            let mut ctx = Ctx { scene: self, vars: Default::default(), seed, calls: 0 };
            let mut result = Ok(());
            for s in prog.iter() {
                result = ctx.run(s);
                if result.is_err() {
                    break;
                }
            }
            if let Some(e) = self.expressions.get_mut(i) {
                e.error = result.err();
            }
        }
        self.in_expression = false;
    }
}
