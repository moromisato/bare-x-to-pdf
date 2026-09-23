use crate::model::PathCommand;
use crate::xml::Element;
use std::collections::HashMap;

struct Context<'a> {
    equations: HashMap<String, &'a str>,
    modifiers: Vec<f64>,
    vars: HashMap<&'static str, f64>,
    cache: std::cell::RefCell<HashMap<String, f64>>,
    depth: std::cell::Cell<usize>,
}

impl Context<'_> {
    fn equation(&self, name: &str) -> f64 {
        if let Some(v) = self.cache.borrow().get(name) {
            return *v;
        }
        if self.depth.get() > 64 {
            return 0.0;
        }
        self.depth.set(self.depth.get() + 1);
        let value = self.equations.get(name).map(|f| self.formula(f)).unwrap_or(0.0);
        self.depth.set(self.depth.get() - 1);
        self.cache.borrow_mut().insert(name.to_string(), value);
        value
    }

    fn value(&self, token: &str) -> f64 {
        if let Some(name) = token.strip_prefix('?') {
            return self.equation(name);
        }
        if let Some(index) = token.strip_prefix('$') {
            return index.parse::<usize>().ok().and_then(|i| self.modifiers.get(i).copied()).unwrap_or(0.0);
        }
        token.parse::<f64>().unwrap_or_else(|_| self.formula(token))
    }

    fn formula(&self, text: &str) -> f64 {
        let tokens = tokenize(text);
        let mut parser = Parser { tokens: &tokens, pos: 0, ctx: self };
        parser.expr()
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Num(f64),
    Ident(String),
    Op(char),
}

fn tokenize(text: &str) -> Vec<Token> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() || c == '.' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.' || chars[i] == 'e' || chars[i] == 'E') {
                i += 1;
            }
            out.push(Token::Num(chars[start..i].iter().collect::<String>().parse().unwrap_or(0.0)));
        } else if c.is_alphabetic() || c == '?' || c == '$' || c == '_' {
            let start = i;
            i += 1;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            out.push(Token::Ident(chars[start..i].iter().collect()));
        } else {
            out.push(Token::Op(c));
            i += 1;
        }
    }
    out
}

struct Parser<'a, 'b> {
    tokens: &'a [Token],
    pos: usize,
    ctx: &'a Context<'b>,
}

impl Parser<'_, '_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn expr(&mut self) -> f64 {
        let mut value = self.term();
        while let Some(Token::Op(op @ ('+' | '-'))) = self.peek().cloned() {
            self.pos += 1;
            let rhs = self.term();
            value = if op == '+' { value + rhs } else { value - rhs };
        }
        value
    }

    fn term(&mut self) -> f64 {
        let mut value = self.unary();
        while let Some(Token::Op(op @ ('*' | '/'))) = self.peek().cloned() {
            self.pos += 1;
            let rhs = self.unary();
            value = if op == '*' { value * rhs } else if rhs != 0.0 { value / rhs } else { 0.0 };
        }
        value
    }

    fn unary(&mut self) -> f64 {
        if let Some(Token::Op('-')) = self.peek() {
            self.pos += 1;
            return -self.unary();
        }
        if let Some(Token::Op('+')) = self.peek() {
            self.pos += 1;
            return self.unary();
        }
        self.primary()
    }

    fn args(&mut self) -> Vec<f64> {
        let mut out = Vec::new();
        if self.peek() != Some(&Token::Op('(')) {
            return out;
        }
        self.pos += 1;
        loop {
            if self.peek() == Some(&Token::Op(')')) {
                self.pos += 1;
                break;
            }
            out.push(self.expr());
            match self.peek() {
                Some(Token::Op(',')) => self.pos += 1,
                Some(Token::Op(')')) => {
                    self.pos += 1;
                    break;
                }
                _ => break,
            }
        }
        out
    }

    fn primary(&mut self) -> f64 {
        match self.peek().cloned() {
            Some(Token::Num(n)) => {
                self.pos += 1;
                n
            }
            Some(Token::Op('(')) => {
                self.pos += 1;
                let v = self.expr();
                if self.peek() == Some(&Token::Op(')')) {
                    self.pos += 1;
                }
                v
            }
            Some(Token::Ident(name)) => {
                self.pos += 1;
                if name.starts_with('?') || name.starts_with('$') {
                    return self.ctx.value(&name);
                }
                let a = self.args();
                let arg = |i: usize| a.get(i).copied().unwrap_or(0.0);
                match name.as_str() {
                    "abs" => arg(0).abs(),
                    "sqrt" => arg(0).max(0.0).sqrt(),
                    "sin" => arg(0).sin(),
                    "cos" => arg(0).cos(),
                    "tan" => arg(0).tan(),
                    "atan" => arg(0).atan(),
                    "atan2" => arg(0).atan2(arg(1)),
                    "min" => arg(0).min(arg(1)),
                    "max" => arg(0).max(arg(1)),
                    "if" => {
                        if arg(0) > 0.0 {
                            arg(1)
                        } else {
                            arg(2)
                        }
                    }
                    "pi" => std::f64::consts::PI,
                    other => self.ctx.vars.get(other).copied().unwrap_or(0.0),
                }
            }
            _ => {
                self.pos += 1;
                0.0
            }
        }
    }
}

pub fn enhanced_path(geometry: &Element, width_pt: f64, height_pt: f64) -> Option<Vec<PathCommand>> {
    let path = geometry.attr("enhanced-path")?;
    let log_w = width_pt * 2540.0 / 72.0;
    let log_h = height_pt * 2540.0 / 72.0;
    let view: Vec<f64> = geometry.attr("viewBox").unwrap_or("0 0 0 0").split_whitespace().filter_map(|v| v.parse().ok()).collect();
    let (vx, vy, vw, vh) = (view.first().copied().unwrap_or(0.0), view.get(1).copied().unwrap_or(0.0), view.get(2).copied().unwrap_or(0.0), view.get(3).copied().unwrap_or(0.0));
    let sub: Vec<f64> = geometry.attr("sub-view-size").unwrap_or("").split_whitespace().filter_map(|v| v.parse().ok()).collect();
    let (space_w, space_h, origin) = if sub.len() >= 2 && sub[0] > 0.0 && sub[1] > 0.0 {
        (sub[0], sub[1], (0.0, 0.0))
    } else if vw > 0.0 && vh > 0.0 {
        (vw, vh, (vx, vy))
    } else {
        (log_w.max(1.0), log_h.max(1.0), (0.0, 0.0))
    };
    let mut vars: HashMap<&'static str, f64> = HashMap::new();
    vars.insert("logwidth", log_w);
    vars.insert("logheight", log_h);
    vars.insert("width", if vw > 0.0 { vw } else { log_w });
    vars.insert("height", if vh > 0.0 { vh } else { log_h });
    vars.insert("left", vx);
    vars.insert("top", vy);
    vars.insert("right", vx + if vw > 0.0 { vw } else { log_w });
    vars.insert("bottom", vy + if vh > 0.0 { vh } else { log_h });
    vars.insert("xstretch", 0.0);
    vars.insert("ystretch", 0.0);
    vars.insert("hasstroke", 1.0);
    vars.insert("hasfill", 1.0);
    let ctx = Context {
        equations: geometry.children("equation").filter_map(|e| Some((e.attr("name")?.to_string(), e.attr("formula")?))).collect(),
        modifiers: geometry.attr("modifiers").unwrap_or("").split_whitespace().filter_map(|v| v.parse().ok()).collect(),
        vars,
        cache: std::cell::RefCell::new(HashMap::new()),
        depth: std::cell::Cell::new(0),
    };
    let norm = |x: f64, y: f64| ((x - origin.0) / space_w, (y - origin.1) / space_h);
    let tokens: Vec<&str> = path.split_whitespace().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let mut command = 'M';
    let mut current = (0.0, 0.0);
    let mut start = (0.0, 0.0);
    let is_command = |t: &str| t.len() == 1 && t.chars().next().is_some_and(|c| c.is_ascii_alphabetic());
    let mut take = |i: &mut usize, n: usize| -> Option<Vec<f64>> {
        if *i + n > tokens.len() || tokens[*i..*i + n].iter().any(|t| is_command(t)) {
            return None;
        }
        let values = tokens[*i..*i + n].iter().map(|t| ctx.value(t)).collect();
        *i += n;
        Some(values)
    };
    while i < tokens.len() {
        if is_command(tokens[i]) {
            command = tokens[i].chars().next().unwrap();
            i += 1;
            match command {
                'Z' => {
                    out.push(PathCommand::Close);
                    current = start;
                    continue;
                }
                'N' | 'F' | 'S' => continue,
                _ => {}
            }
        }
        match command {
            'M' => {
                let Some(v) = take(&mut i, 2) else { i += 1; continue };
                current = (v[0], v[1]);
                start = current;
                let p = norm(v[0], v[1]);
                out.push(PathCommand::Move(p.0, p.1));
                command = 'L';
            }
            'L' => {
                let Some(v) = take(&mut i, 2) else { i += 1; continue };
                current = (v[0], v[1]);
                let p = norm(v[0], v[1]);
                out.push(PathCommand::Line(p.0, p.1));
            }
            'C' => {
                let Some(v) = take(&mut i, 6) else { i += 1; continue };
                let (a, b, c) = (norm(v[0], v[1]), norm(v[2], v[3]), norm(v[4], v[5]));
                current = (v[4], v[5]);
                out.push(PathCommand::Cubic(a.0, a.1, b.0, b.1, c.0, c.1));
            }
            'Q' => {
                let Some(v) = take(&mut i, 4) else { i += 1; continue };
                let (a, b) = (norm(v[0], v[1]), norm(v[2], v[3]));
                current = (v[2], v[3]);
                out.push(PathCommand::Quad(a.0, a.1, b.0, b.1));
            }
            'G' => {
                let Some(v) = take(&mut i, 4) else { i += 1; continue };
                let (rx, ry, st, sw) = (v[0], v[1], v[2].to_radians(), v[3].to_radians());
                let center = (current.0 - rx * st.cos(), current.1 - ry * st.sin());
                let steps = ((sw.abs() / (std::f64::consts::PI / 8.0)).ceil() as usize).max(1);
                for k in 1..=steps {
                    let a = st + sw * k as f64 / steps as f64;
                    let point = (center.0 + rx * a.cos(), center.1 + ry * a.sin());
                    let p = norm(point.0, point.1);
                    out.push(PathCommand::Line(p.0, p.1));
                    current = point;
                }
            }
            'U' | 'T' => {
                let Some(v) = take(&mut i, 6) else { i += 1; continue };
                let (cx, cy, rx, ry) = (v[0], v[1], v[2], v[3]);
                let (t0, t1) = (v[4].to_radians(), v[5].to_radians());
                let sweep = if t1 <= t0 { t1 + std::f64::consts::TAU - t0 } else { t1 - t0 };
                let steps = ((sweep / (std::f64::consts::PI / 16.0)).ceil() as usize).max(2);
                for k in 0..=steps {
                    let a = t0 + sweep * k as f64 / steps as f64;
                    let point = (cx + rx * a.cos(), cy - ry * a.sin());
                    let p = norm(point.0, point.1);
                    if k == 0 && command == 'U' {
                        out.push(PathCommand::Move(p.0, p.1));
                        start = point;
                    } else {
                        out.push(PathCommand::Line(p.0, p.1));
                    }
                    current = point;
                }
            }
            _ => i += 1,
        }
    }
    (!out.is_empty()).then_some(out)
}
