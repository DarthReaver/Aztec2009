/// Tokens of the Aztec scripting language (a MEL style command language).
#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    Word(String),
    Num(f64),
    Str(String),
    Flag(String),
    /// `( expression )`, evaluated when the command runs.
    Group(String),
    /// `` `command` ``: the command's result.
    Tick(String),
    /// Statement terminator: `;` or a newline.
    End,
}

fn starts_number(w: &str) -> bool {
    let mut it = w.chars();
    let digit_or_dot = |c: Option<char>, rest: &mut std::str::Chars| -> bool {
        match c {
            Some(d) if d.is_ascii_digit() => true,
            Some('.') => rest.next().is_some_and(|d| d.is_ascii_digit()),
            _ => false,
        }
    };
    match it.next() {
        Some(c) if c.is_ascii_digit() => true,
        Some('.') => it.next().is_some_and(|d| d.is_ascii_digit()),
        Some('-') | Some('+') => {
            let n = it.next();
            digit_or_dot(n, &mut it)
        }
        _ => false,
    }
}

fn classify(w: &str) -> Token {
    if starts_number(w) {
        if let Ok(n) = w.parse::<f64>() {
            return Token::Num(n);
        }
    }
    if w.len() > 1 && w.starts_with('-') {
        let second = w.chars().nth(1).unwrap();
        if second.is_ascii_alphabetic() || second == '_' {
            return Token::Flag(w[1..].to_string());
        }
    }
    Token::Word(w.to_string())
}

/// Convert source text into tokens paired with their 1 based line numbers.
pub fn lex(src: &str) -> Result<Vec<(Token, usize)>, String> {
    let chars: Vec<char> = src.chars().collect();
    let mut out: Vec<(Token, usize)> = Vec::new();
    let mut i = 0usize;
    let mut line = 1usize;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\n' => {
                out.push((Token::End, line));
                line += 1;
                i += 1;
            }
            ' ' | '\t' | '\r' => i += 1,
            ';' => {
                out.push((Token::End, line));
                i += 1;
            }
            '/' if i + 1 < chars.len() && chars[i + 1] == '/' => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if i + 1 < chars.len() && chars[i + 1] == '*' => {
                let start_line = line;
                i += 2;
                let mut closed = false;
                while i < chars.len() {
                    if chars[i] == '*' && i + 1 < chars.len() && chars[i + 1] == '/' {
                        i += 2;
                        closed = true;
                        break;
                    }
                    if chars[i] == '\n' {
                        line += 1;
                    }
                    i += 1;
                }
                if !closed {
                    return Err(format!("Line {}: unterminated block comment", start_line));
                }
            }
            '"' => {
                i += 1;
                let mut s = String::new();
                let mut closed = false;
                while i < chars.len() {
                    let ch = chars[i];
                    if ch == '"' {
                        closed = true;
                        i += 1;
                        break;
                    }
                    if ch == '\n' {
                        break;
                    }
                    if ch == '\\' && i + 1 < chars.len() {
                        i += 1;
                        match chars[i] {
                            'n' => s.push('\n'),
                            't' => s.push('\t'),
                            '\\' => s.push('\\'),
                            '"' => s.push('"'),
                            other => {
                                s.push('\\');
                                s.push(other);
                            }
                        }
                        i += 1;
                        continue;
                    }
                    s.push(ch);
                    i += 1;
                }
                if !closed {
                    return Err(format!("Line {}: unterminated string", line));
                }
                out.push((Token::Str(s), line));
            }
            '(' => {
                let start_line = line;
                i += 1;
                let (mut depth, mut s) = (1, String::new());
                while i < chars.len() {
                    let ch = chars[i];
                    if ch == '"' {
                        s.push(ch);
                        i += 1;
                        while i < chars.len() {
                            let c2 = chars[i];
                            s.push(c2);
                            i += 1;
                            if c2 == '\\' && i < chars.len() {
                                s.push(chars[i]);
                                i += 1;
                            } else if c2 == '"' {
                                break;
                            }
                        }
                        continue;
                    }
                    if ch == '\n' {
                        line += 1;
                    }
                    if ch == '(' {
                        depth += 1;
                    } else if ch == ')' {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    s.push(ch);
                    i += 1;
                }
                if depth != 0 {
                    return Err(format!("Line {}: missing ')'", start_line));
                }
                i += 1;
                out.push((Token::Group(s), start_line));
            }
            '`' => {
                let start_line = line;
                i += 1;
                let mut s = String::new();
                while i < chars.len() && chars[i] != '`' {
                    if chars[i] == '\n' {
                        line += 1;
                    }
                    s.push(chars[i]);
                    i += 1;
                }
                if i >= chars.len() {
                    return Err(format!("Line {}: missing closing backtick", start_line));
                }
                i += 1;
                out.push((Token::Tick(s), start_line));
            }
            _ => {
                let mut w = String::new();
                while i < chars.len() {
                    let ch = chars[i];
                    if ch.is_whitespace() || ch == ';' || ch == '"' {
                        break;
                    }
                    w.push(ch);
                    i += 1;
                }
                out.push((classify(&w), line));
            }
        }
    }
    out.push((Token::End, line));
    Ok(out)
}
