use crate::model::Color;

pub fn builtin(id: u32) -> Option<&'static str> {
    Some(match id {
        0 => "General",
        1 => "0",
        2 => "0.00",
        3 => "#,##0",
        4 => "#,##0.00",
        9 => "0%",
        10 => "0.00%",
        11 => "0.00E+00",
        12 => "# ?/?",
        13 => "# ??/??",
        14 => "m/d/yyyy",
        15 => "d-mmm-yy",
        16 => "d-mmm",
        17 => "mmm-yy",
        18 => "h:mm AM/PM",
        19 => "h:mm:ss AM/PM",
        20 => "h:mm",
        21 => "h:mm:ss",
        22 => "m/d/yyyy h:mm",
        37 => "#,##0 ;(#,##0)",
        38 => "#,##0 ;[Red](#,##0)",
        39 => "#,##0.00;(#,##0.00)",
        40 => "#,##0.00;[Red](#,##0.00)",
        45 => "mm:ss",
        46 => "[h]:mm:ss",
        47 => "mmss.0",
        48 => "##0.0E+0",
        49 => "@",
        _ => return None,
    })
}

pub struct Formatted {
    pub text: String,
    pub color: Option<Color>,
}

pub fn is_date_format(code: &str) -> bool {
    split_sections(code).first().map(|s| is_date(&strip_brackets(s).0)).unwrap_or(false)
}

pub fn format_number(value: f64, code: &str) -> Formatted {
    let sections = split_sections(code);
    let (section, value) = match sections.len() {
        0 => return Formatted { text: general(value), color: None },
        1 => (sections[0].as_str(), value),
        _ => {
            if value < 0.0 {
                (sections[1].as_str(), value.abs())
            } else if value == 0.0 && sections.len() > 2 {
                (sections[2].as_str(), value)
            } else {
                (sections[0].as_str(), value)
            }
        }
    };
    let (section, color) = strip_brackets(section);
    let trimmed = section.trim();
    let text = if trimmed.eq_ignore_ascii_case("general") || trimmed.is_empty() {
        general(value)
    } else if is_date(&section) {
        format_date(value, &section)
    } else {
        format_numeric(value, &section)
    };
    Formatted { text, color }
}

pub fn format_text(text: &str, code: &str) -> String {
    let sections = split_sections(code);
    let section = match sections.len() {
        4 => sections[3].clone(),
        1 if sections[0].contains('@') => sections[0].clone(),
        _ => return text.to_string(),
    };
    let (section, _) = strip_brackets(&section);
    let mut out = String::new();
    let mut chars = section.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '@' => out.push_str(text),
            '"' => {
                for q in chars.by_ref() {
                    if q == '"' {
                        break;
                    }
                    out.push(q);
                }
            }
            '\\' => {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            }
            '_' => {
                chars.next();
                out.push(' ');
            }
            '*' => {
                chars.next();
            }
            other => out.push(other),
        }
    }
    out
}

fn split_sections(code: &str) -> Vec<String> {
    let mut sections = vec![String::new()];
    let mut quoted = false;
    let mut bracket = false;
    let mut chars = code.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                sections.last_mut().unwrap().push(c);
            }
            '[' if !quoted => {
                bracket = true;
                sections.last_mut().unwrap().push(c);
            }
            ']' if !quoted => {
                bracket = false;
                sections.last_mut().unwrap().push(c);
            }
            '\\' => {
                sections.last_mut().unwrap().push(c);
                if let Some(n) = chars.next() {
                    sections.last_mut().unwrap().push(n);
                }
            }
            ';' if !quoted && !bracket => sections.push(String::new()),
            _ => sections.last_mut().unwrap().push(c),
        }
    }
    sections
}

fn strip_brackets(section: &str) -> (String, Option<Color>) {
    let mut out = String::new();
    let mut color = None;
    let mut chars = section.chars();
    let mut quoted = false;
    while let Some(c) = chars.next() {
        if c == '"' {
            quoted = !quoted;
            out.push(c);
            continue;
        }
        if c == '[' && !quoted {
            let mut inner = String::new();
            for n in chars.by_ref() {
                if n == ']' {
                    break;
                }
                inner.push(n);
            }
            let lower = inner.to_lowercase();
            if let Some(currency) = inner.strip_prefix('$') {
                out.push_str(currency.split('-').next().unwrap_or(""));
            } else if lower.starts_with('h') || lower.starts_with('m') || lower.starts_with('s') {
                out.push('[');
                out.push_str(&inner);
                out.push(']');
            } else {
                color = color.or_else(|| named_color(&lower));
            }
            continue;
        }
        out.push(c);
    }
    (out, color)
}

fn named_color(name: &str) -> Option<Color> {
    Some(match name {
        "red" => Color(255, 0, 0),
        "blue" => Color(0, 0, 255),
        "green" => Color(0, 128, 0),
        "black" => Color(0, 0, 0),
        "white" => Color(255, 255, 255),
        "yellow" => Color(255, 255, 0),
        "magenta" => Color(255, 0, 255),
        "cyan" => Color(0, 255, 255),
        _ => return None,
    })
}

fn is_date(section: &str) -> bool {
    let mut quoted = false;
    let mut has_date = false;
    for c in section.chars() {
        match c {
            '"' => quoted = !quoted,
            '0' | '#' | '?' if !quoted => return false,
            'y' | 'Y' | 'd' | 'D' | 'h' | 'H' | 's' | 'S' | 'm' | 'M' if !quoted => has_date = true,
            _ => {}
        }
    }
    has_date
}

pub fn general(value: f64) -> String {
    if value == value.trunc() && value.abs() < 1e15 {
        return format!("{}", value as i64);
    }
    let magnitude = value.abs();
    if magnitude >= 1e11 || (magnitude < 1e-4 && magnitude > 0.0) {
        return format!("{:.5E}", value).replace("E", "E+").replace("E+-", "E-");
    }
    let int_digits = if magnitude >= 1.0 { magnitude.log10().floor() as usize + 1 } else { 1 };
    let decimals = 10usize.saturating_sub(int_digits);
    let s = format!("{:.*}", decimals, value);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn format_fraction(value: f64, section: &str) -> Option<String> {
    let slash = section.find('/')?;
    let before = &section[..slash];
    let after = &section[slash + 1..];
    let numerator_digits = before.chars().rev().take_while(|c| matches!(c, '?' | '#' | '0')).count();
    if numerator_digits == 0 {
        return None;
    }
    let denominator_spec: String = after.chars().take_while(|c| c.is_ascii_digit() || matches!(c, '?' | '#')).collect();
    if denominator_spec.is_empty() {
        return None;
    }
    let integer_part = before[..before.len() - numerator_digits].trim_end();
    let has_integer = integer_part.chars().any(|c| matches!(c, '?' | '#' | '0'));
    let sign = if value < 0.0 { "-" } else { "" };
    let magnitude = value.abs();
    let (whole, fractional) = if has_integer { (magnitude.trunc(), magnitude.fract()) } else { (0.0, magnitude) };
    let (num, den) = if let Ok(fixed) = denominator_spec.parse::<u64>() {
        ((fractional * fixed as f64).round() as u64, fixed)
    } else {
        let max_den = 10u64.pow(denominator_spec.len() as u32) - 1;
        best_fraction(fractional, max_den)
    };
    let (mut whole, mut num) = (whole as u64, num);
    if den > 0 && num == den {
        whole += 1;
        num = 0;
    }
    let mut out = String::from(sign);
    if has_integer {
        if whole > 0 || num == 0 {
            out.push_str(&whole.to_string());
        }
        if num > 0 {
            if whole > 0 {
                out.push(' ');
            }
            out.push_str(&format!("{num}/{den}"));
        }
    } else {
        out.push_str(&format!("{num}/{den}"));
    }
    Some(out)
}

fn best_fraction(value: f64, max_den: u64) -> (u64, u64) {
    let mut best = (0u64, 1u64);
    let mut best_err = f64::INFINITY;
    for den in 1..=max_den {
        let num = (value * den as f64).round();
        let err = (value - num / den as f64).abs();
        if err < best_err - 1e-12 {
            best = (num as u64, den);
            best_err = err;
        }
    }
    best
}

fn format_numeric(value: f64, section: &str) -> String {
    if section.contains('/') && !section.contains('"') {
        if let Some(fraction) = format_fraction(value, section) {
            return fraction;
        }
    }
    let mut prefix = String::new();
    let mut suffix = String::new();
    let mut int_digits = 0usize;
    let mut decimals = 0usize;
    let mut thousands = false;
    let mut percent = false;
    let mut scientific = false;
    let mut exponent_digits = 0usize;
    let mut seen_number = false;
    let mut in_decimals = false;
    let mut chars = section.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '0' | '#' | '?' => {
                seen_number = true;
                if in_decimals {
                    decimals += 1;
                } else if c == '0' {
                    int_digits += 1;
                }
            }
            '.' if !in_decimals => {
                seen_number = true;
                in_decimals = true;
            }
            ',' if seen_number && !in_decimals => thousands = true,
            '%' => {
                percent = true;
                if seen_number { suffix.push('%') } else { prefix.push('%') }
            }
            'E' | 'e' if seen_number => {
                scientific = true;
                if let Some('+') | Some('-') = chars.peek() {
                    chars.next();
                }
                while let Some('0') = chars.peek() {
                    chars.next();
                    exponent_digits += 1;
                }
            }
            '"' => {
                let target = if seen_number { &mut suffix } else { &mut prefix };
                for q in chars.by_ref() {
                    if q == '"' {
                        break;
                    }
                    target.push(q);
                }
            }
            '\\' => {
                if let Some(n) = chars.next() {
                    if seen_number { suffix.push(n) } else { prefix.push(n) }
                }
            }
            '_' => {
                chars.next();
                if seen_number { suffix.push(' ') } else { prefix.push(' ') }
            }
            '*' => {
                chars.next();
            }
            '@' | ',' => {}
            other => {
                if seen_number { suffix.push(other) } else { prefix.push(other) }
            }
        }
    }

    let mut v = value;
    if percent {
        v *= 100.0;
    }
    let body = if scientific {
        let raw = format!("{:.*E}", decimals, v);
        match raw.split_once('E') {
            Some((mantissa, exponent)) => {
                let (sign, digits) = match exponent.strip_prefix('-') {
                    Some(rest) => ("-", rest),
                    None => ("+", exponent),
                };
                format!("{mantissa}E{sign}{:0>width$}", digits, width = exponent_digits.max(2))
            }
            None => raw,
        }
    } else {
        let rounded = format!("{:.*}", decimals, v);
        let (int_part, frac_part) = match rounded.split_once('.') {
            Some((a, b)) => (a.to_string(), Some(b.to_string())),
            None => (rounded.clone(), None),
        };
        let negative = int_part.starts_with('-');
        let mut digits = int_part.trim_start_matches('-').to_string();
        if digits == "0" && int_digits == 0 && decimals > 0 {
            digits.clear();
        }
        while digits.len() < int_digits {
            digits.insert(0, '0');
        }
        if thousands {
            digits = group_thousands(&digits);
        }
        let mut out = String::new();
        if negative {
            out.push('-');
        }
        out.push_str(&digits);
        if let Some(frac) = frac_part {
            out.push('.');
            out.push_str(&frac);
        }
        out
    };
    format!("{prefix}{body}{suffix}")
}

fn group_thousands(digits: &str) -> String {
    let mut out = String::new();
    let len = digits.len();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub fn serial_to_datetime(serial: f64) -> (i64, u32, u32, u32, u32, u32) {
    let days = serial.floor() as i64;
    let frac = serial - serial.floor();
    let epoch_days = if days < 61 { days - 1 } else { days - 2 };
    let (y, m, d) = civil_from_days(epoch_days + days_from_civil(1900, 1, 1));
    let total = (frac * 86400.0).round() as u32;
    (y, m, d, total / 3600, (total / 60) % 60, total % 60)
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn format_date(value: f64, section: &str) -> String {
    let (y, mo, d, h, mi, s) = serial_to_datetime(value);
    let months = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    let weekdays = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
    let dow = ((value.floor() as i64 - 1).rem_euclid(7)) as usize;
    let lower = section.to_lowercase();
    let twelve = lower.contains("am/pm") || lower.contains("a/p");
    let chars: Vec<char> = section.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let mut last_was_hour = false;
    while i < chars.len() {
        let c = chars[i];
        let run = |ch: char| {
            let mut n = 0;
            while i + n < chars.len() && chars[i + n].to_ascii_lowercase() == ch {
                n += 1;
            }
            n
        };
        match c.to_ascii_lowercase() {
            '"' => {
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    out.push(chars[i]);
                    i += 1;
                }
                i += 1;
            }
            '\\' => {
                if i + 1 < chars.len() {
                    out.push(chars[i + 1]);
                }
                i += 2;
            }
            '[' => {
                let mut j = i + 1;
                let mut inner = String::new();
                while j < chars.len() && chars[j] != ']' {
                    inner.push(chars[j]);
                    j += 1;
                }
                match inner.to_lowercase().chars().next() {
                    Some('h') => out.push_str(&((value * 24.0).floor() as i64).to_string()),
                    Some('m') => out.push_str(&((value * 1440.0).floor() as i64).to_string()),
                    Some('s') => out.push_str(&((value * 86400.0).floor() as i64).to_string()),
                    _ => {}
                }
                i = j + 1;
            }
            'y' => {
                let n = run('y');
                out.push_str(&if n <= 2 { format!("{:02}", y.rem_euclid(100)) } else { y.to_string() });
                i += n;
                last_was_hour = false;
            }
            'm' => {
                let n = run('m');
                let next_alpha = chars[i + n..].iter().find(|c| c.is_ascii_alphabetic()).map(|c| c.to_ascii_lowercase());
                if last_was_hour || (n <= 2 && next_alpha == Some('s')) {
                    out.push_str(&if n >= 2 { format!("{mi:02}") } else { mi.to_string() });
                } else {
                    out.push_str(&match n {
                        1 => mo.to_string(),
                        2 => format!("{mo:02}"),
                        3 => months[(mo - 1) as usize][..3].to_string(),
                        _ => months[(mo - 1) as usize].to_string(),
                    });
                }
                i += n;
                last_was_hour = false;
            }
            'd' => {
                let n = run('d');
                out.push_str(&match n {
                    1 => d.to_string(),
                    2 => format!("{d:02}"),
                    3 => weekdays[dow][..3].to_string(),
                    _ => weekdays[dow].to_string(),
                });
                i += n;
                last_was_hour = false;
            }
            'h' => {
                let n = run('h');
                let hour = if twelve { let h12 = h % 12; if h12 == 0 { 12 } else { h12 } } else { h };
                out.push_str(&if n >= 2 { format!("{hour:02}") } else { hour.to_string() });
                i += n;
                last_was_hour = true;
            }
            's' => {
                let n = run('s');
                out.push_str(&if n >= 2 { format!("{s:02}") } else { s.to_string() });
                i += n;
                last_was_hour = false;
            }
            'a' if lower[i..].starts_with("am/pm") => {
                out.push_str(if h >= 12 { "PM" } else { "AM" });
                i += 5;
            }
            'a' if lower[i..].starts_with("a/p") => {
                out.push_str(if h >= 12 { "P" } else { "A" });
                i += 3;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}
