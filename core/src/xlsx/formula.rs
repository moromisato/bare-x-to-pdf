use std::str::Chars;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Empty,
    Num(f64),
    Str(String),
    Bool(bool),
    Range(Vec<Value>),
}

impl Value {
    pub fn truthy(&self) -> bool {
        match self {
            Value::Num(n) => *n != 0.0,
            Value::Bool(b) => *b,
            Value::Str(s) => !s.is_empty(),
            Value::Empty => false,
            Value::Range(items) => items.first().is_some_and(Value::truthy),
        }
    }

    fn number(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            Value::Empty => Some(0.0),
            Value::Str(s) => s.trim().parse().ok(),
            Value::Range(items) => items.first().and_then(Value::number),
        }
    }

    fn text(&self) -> String {
        match self {
            Value::Str(s) => s.clone(),
            Value::Num(n) => crate::xlsx::format::general(*n),
            Value::Bool(b) => if *b { "TRUE".into() } else { "FALSE".into() },
            Value::Empty => String::new(),
            Value::Range(items) => items.first().map(Value::text).unwrap_or_default(),
        }
    }

    fn scalars(&self) -> Vec<Value> {
        match self {
            Value::Range(items) => items.clone(),
            other => vec![other.clone()],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Num(f64),
    Str(String),
    Ident(String),
    Ref { col: u32, row: u32, abs_col: bool, abs_row: bool },
    Op(String),
    LParen,
    RParen,
    Comma,
    Colon,
}

fn tokenize(input: &str) -> Option<Vec<Token>> {
    let mut out = Vec::new();
    let mut chars = input.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c.is_ascii_digit() || (c == '.' && chars.clone().nth(1).is_some_and(|d| d.is_ascii_digit())) {
            let mut s = String::new();
            while let Some(&d) = chars.peek() {
                if d.is_ascii_digit() || d == '.' || ((d == 'e' || d == 'E') && !s.is_empty()) || ((d == '+' || d == '-') && s.ends_with(['e', 'E'])) {
                    s.push(d);
                    chars.next();
                } else {
                    break;
                }
            }
            out.push(Token::Num(s.parse().ok()?));
        } else if c == '"' {
            chars.next();
            let mut s = String::new();
            loop {
                match chars.next() {
                    Some('"') => {
                        if chars.peek() == Some(&'"') {
                            chars.next();
                            s.push('"');
                        } else {
                            break;
                        }
                    }
                    Some(d) => s.push(d),
                    None => return None,
                }
            }
            out.push(Token::Str(s));
        } else if c == '$' || c.is_ascii_alphabetic() || c == '_' {
            let mut s = String::new();
            while let Some(&d) = chars.peek() {
                if d.is_ascii_alphanumeric() || d == '$' || d == '_' || d == '.' {
                    s.push(d);
                    chars.next();
                } else {
                    break;
                }
            }
            if chars.peek() == Some(&'!') {
                chars.next();
                s.clear();
                while let Some(&d) = chars.peek() {
                    if d.is_ascii_alphanumeric() || d == '$' {
                        s.push(d);
                        chars.next();
                    } else {
                        break;
                    }
                }
            }
            match cell_ref(&s) {
                Some(token) => out.push(token),
                None => out.push(Token::Ident(s.to_ascii_uppercase())),
            }
        } else if c == '\'' {
            chars.next();
            for d in chars.by_ref() {
                if d == '\'' {
                    break;
                }
            }
            if chars.peek() == Some(&'!') {
                chars.next();
            }
        } else {
            chars.next();
            match c {
                '(' => out.push(Token::LParen),
                ')' => out.push(Token::RParen),
                ',' | ';' => out.push(Token::Comma),
                ':' => out.push(Token::Colon),
                '<' | '>' => {
                    let mut op = c.to_string();
                    if let Some(&n) = chars.peek() {
                        if n == '=' || (c == '<' && n == '>') {
                            op.push(n);
                            chars.next();
                        }
                    }
                    out.push(Token::Op(op));
                }
                '=' | '+' | '-' | '*' | '/' | '^' | '&' | '%' => out.push(Token::Op(c.to_string())),
                _ => return None,
            }
        }
    }
    Some(out)
}

fn cell_ref(text: &str) -> Option<Token> {
    let mut chars: Chars = text.chars();
    let mut abs_col = false;
    let mut abs_row = false;
    let mut letters = String::new();
    let mut digits = String::new();
    let mut rest = chars.clone();
    if let Some('$') = chars.clone().next() {
        abs_col = true;
        chars.next();
        rest = chars.clone();
    }
    let _ = rest;
    let remaining: String = chars.collect();
    let mut seen_digit = false;
    for (i, ch) in remaining.chars().enumerate() {
        if ch.is_ascii_alphabetic() && !seen_digit {
            letters.push(ch.to_ascii_uppercase());
        } else if ch == '$' && !seen_digit && !letters.is_empty() && i > 0 {
            abs_row = true;
        } else if ch.is_ascii_digit() {
            seen_digit = true;
            digits.push(ch);
        } else {
            return None;
        }
    }
    if letters.is_empty() || digits.is_empty() || letters.len() > 3 {
        return None;
    }
    let col = letters.chars().fold(0u32, |acc, ch| acc * 26 + (ch as u32 - 'A' as u32 + 1));
    let row: u32 = digits.parse().ok()?;
    Some(Token::Ref { col, row, abs_col, abs_row })
}

pub struct Context<'a> {
    pub cell: &'a dyn Fn(u32, u32) -> Value,
    pub row_offset: i64,
    pub col_offset: i64,
}

struct Parser<'a, 'b> {
    tokens: Vec<Token>,
    pos: usize,
    ctx: &'a Context<'b>,
}

pub fn evaluate(formula: &str, ctx: &Context) -> Option<Value> {
    let tokens = tokenize(formula.trim_start_matches('='))?;
    let mut parser = Parser { tokens, pos: 0, ctx };
    let value = parser.comparison()?;
    (parser.pos == parser.tokens.len()).then_some(value)
}

impl Parser<'_, '_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn eat_op(&mut self, ops: &[&str]) -> Option<String> {
        if let Some(Token::Op(op)) = self.peek() {
            if ops.contains(&op.as_str()) {
                let op = op.clone();
                self.pos += 1;
                return Some(op);
            }
        }
        None
    }

    fn comparison(&mut self) -> Option<Value> {
        let mut left = self.concat()?;
        while let Some(op) = self.eat_op(&["=", "<>", "<", ">", "<=", ">="]) {
            let right = self.concat()?;
            left = Value::Bool(compare(&left, &right, &op));
        }
        Some(left)
    }

    fn concat(&mut self) -> Option<Value> {
        let mut left = self.additive()?;
        while self.eat_op(&["&"]).is_some() {
            let right = self.additive()?;
            left = Value::Str(format!("{}{}", left.text(), right.text()));
        }
        Some(left)
    }

    fn additive(&mut self) -> Option<Value> {
        let mut left = self.multiplicative()?;
        while let Some(op) = self.eat_op(&["+", "-"]) {
            let right = self.multiplicative()?;
            let (a, b) = (left.number()?, right.number()?);
            left = Value::Num(if op == "+" { a + b } else { a - b });
        }
        Some(left)
    }

    fn multiplicative(&mut self) -> Option<Value> {
        let mut left = self.power()?;
        while let Some(op) = self.eat_op(&["*", "/"]) {
            let right = self.power()?;
            let (a, b) = (left.number()?, right.number()?);
            left = Value::Num(if op == "*" { a * b } else if b != 0.0 { a / b } else { return None });
        }
        Some(left)
    }

    fn power(&mut self) -> Option<Value> {
        let mut left = self.unary()?;
        while self.eat_op(&["^"]).is_some() {
            let right = self.unary()?;
            left = Value::Num(left.number()?.powf(right.number()?));
        }
        Some(left)
    }

    fn unary(&mut self) -> Option<Value> {
        if let Some(op) = self.eat_op(&["-", "+"]) {
            let value = self.unary()?;
            return Some(Value::Num(if op == "-" { -value.number()? } else { value.number()? }));
        }
        let mut value = self.primary()?;
        while self.eat_op(&["%"]).is_some() {
            value = Value::Num(value.number()? / 100.0);
        }
        Some(value)
    }

    fn primary(&mut self) -> Option<Value> {
        let token = self.peek()?.clone();
        self.pos += 1;
        match token {
            Token::Num(n) => Some(Value::Num(n)),
            Token::Str(s) => Some(Value::Str(s)),
            Token::LParen => {
                let value = self.comparison()?;
                matches!(self.peek(), Some(Token::RParen)).then(|| self.pos += 1)?;
                Some(value)
            }
            Token::Ref { col, row, abs_col, abs_row } => {
                let (c1, r1) = self.resolve(col, row, abs_col, abs_row)?;
                if matches!(self.peek(), Some(Token::Colon)) {
                    self.pos += 1;
                    if let Some(Token::Ref { col, row, abs_col, abs_row }) = self.peek().cloned() {
                        self.pos += 1;
                        let (c2, r2) = self.resolve(col, row, abs_col, abs_row)?;
                        let mut items = Vec::new();
                        for r in r1.min(r2)..=r1.max(r2) {
                            for c in c1.min(c2)..=c1.max(c2) {
                                items.push((self.ctx.cell)(r, c));
                            }
                        }
                        return Some(Value::Range(items));
                    }
                    return None;
                }
                Some((self.ctx.cell)(r1, c1))
            }
            Token::Ident(name) => {
                if matches!(self.peek(), Some(Token::LParen)) {
                    self.pos += 1;
                    let mut args = Vec::new();
                    if !matches!(self.peek(), Some(Token::RParen)) {
                        loop {
                            args.push(self.comparison()?);
                            match self.peek() {
                                Some(Token::Comma) => self.pos += 1,
                                Some(Token::RParen) => break,
                                _ => return None,
                            }
                        }
                    }
                    self.pos += 1;
                    call(&name, args)
                } else {
                    match name.as_str() {
                        "TRUE" => Some(Value::Bool(true)),
                        "FALSE" => Some(Value::Bool(false)),
                        _ => None,
                    }
                }
            }
            _ => None,
        }
    }

    fn resolve(&self, col: u32, row: u32, abs_col: bool, abs_row: bool) -> Option<(u32, u32)> {
        let c = if abs_col { col as i64 } else { col as i64 + self.ctx.col_offset };
        let r = if abs_row { row as i64 } else { row as i64 + self.ctx.row_offset };
        (c >= 1 && r >= 1).then_some((c as u32, r as u32))
    }
}

fn compare(a: &Value, b: &Value, op: &str) -> bool {
    let ordering = match (a.number(), b.number(), a, b) {
        (_, _, Value::Str(x), _) | (_, _, _, Value::Str(x)) if !matches!((a, b), (Value::Num(_), Value::Num(_))) => {
            let _ = x;
            a.text().to_lowercase().cmp(&b.text().to_lowercase())
        }
        (Some(x), Some(y), _, _) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
        _ => a.text().to_lowercase().cmp(&b.text().to_lowercase()),
    };
    use std::cmp::Ordering::*;
    match op {
        "=" => ordering == Equal,
        "<>" => ordering != Equal,
        "<" => ordering == Less,
        ">" => ordering == Greater,
        "<=" => ordering != Greater,
        ">=" => ordering != Less,
        _ => false,
    }
}

fn numbers(args: &[Value]) -> Vec<f64> {
    args.iter()
        .flat_map(Value::scalars)
        .filter_map(|v| match v {
            Value::Num(n) => Some(n),
            Value::Bool(b) => Some(if b { 1.0 } else { 0.0 }),
            _ => None,
        })
        .collect()
}

fn call(name: &str, args: Vec<Value>) -> Option<Value> {
    let nums = numbers(&args);
    let first = args.first().cloned().unwrap_or(Value::Empty);
    Some(match name {
        "SUM" => Value::Num(nums.iter().sum()),
        "AVERAGE" => Value::Num(if nums.is_empty() { return None } else { nums.iter().sum::<f64>() / nums.len() as f64 }),
        "MIN" => Value::Num(nums.iter().copied().fold(f64::INFINITY, f64::min)),
        "MAX" => Value::Num(nums.iter().copied().fold(f64::NEG_INFINITY, f64::max)),
        "COUNT" => Value::Num(nums.len() as f64),
        "COUNTA" => Value::Num(args.iter().flat_map(Value::scalars).filter(|v| !matches!(v, Value::Empty)).count() as f64),
        "COUNTBLANK" => Value::Num(args.iter().flat_map(Value::scalars).filter(|v| matches!(v, Value::Empty)).count() as f64),
        "ABS" => Value::Num(first.number()?.abs()),
        "INT" => Value::Num(first.number()?.floor()),
        "ROUND" => {
            let digits = args.get(1).and_then(Value::number).unwrap_or(0.0);
            let factor = 10f64.powf(digits);
            Value::Num((first.number()? * factor).round() / factor)
        }
        "MOD" => {
            let (a, b) = (first.number()?, args.get(1)?.number()?);
            if b == 0.0 { return None }
            Value::Num(a - b * (a / b).floor())
        }
        "AND" => Value::Bool(args.iter().flat_map(Value::scalars).all(|v| v.truthy())),
        "OR" => Value::Bool(args.iter().flat_map(Value::scalars).any(|v| v.truthy())),
        "NOT" => Value::Bool(!first.truthy()),
        "IF" => {
            if first.truthy() { args.get(1).cloned().unwrap_or(Value::Bool(true)) } else { args.get(2).cloned().unwrap_or(Value::Bool(false)) }
        }
        "IFERROR" => first,
        "ISBLANK" => Value::Bool(matches!(first, Value::Empty)),
        "ISNUMBER" => Value::Bool(matches!(first, Value::Num(_))),
        "ISTEXT" => Value::Bool(matches!(first, Value::Str(_))),
        "LEN" => Value::Num(first.text().chars().count() as f64),
        "UPPER" => Value::Str(first.text().to_uppercase()),
        "LOWER" => Value::Str(first.text().to_lowercase()),
        "TRIM" => Value::Str(first.text().split_whitespace().collect::<Vec<_>>().join(" ")),
        "LEFT" => {
            let n = args.get(1).and_then(Value::number).unwrap_or(1.0) as usize;
            Value::Str(first.text().chars().take(n).collect())
        }
        "RIGHT" => {
            let text = first.text();
            let n = args.get(1).and_then(Value::number).unwrap_or(1.0) as usize;
            let skip = text.chars().count().saturating_sub(n);
            Value::Str(text.chars().skip(skip).collect())
        }
        "MID" => {
            let start = args.get(1).and_then(Value::number).unwrap_or(1.0).max(1.0) as usize;
            let n = args.get(2).and_then(Value::number).unwrap_or(0.0) as usize;
            Value::Str(first.text().chars().skip(start - 1).take(n).collect())
        }
        "VALUE" | "N" => Value::Num(first.number()?),
        "EXACT" => Value::Bool(first.text() == args.get(1)?.text()),
        "ISEVEN" => Value::Bool(first.number()? as i64 % 2 == 0),
        "ISODD" => Value::Bool(first.number()? as i64 % 2 != 0),
        _ => return None,
    })
}
