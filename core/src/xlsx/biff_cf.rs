use super::{CondRule, Dxf};
use crate::model::{Color, RunProps};

pub struct Condition {
    pub rule: CondRule,
    pub dxf: Dxf,
}

pub fn conditions(condfmt: &[u8], rules: &[&[u8]], palette: &dyn Fn(usize) -> Option<Color>, priority: &mut i64) -> Vec<Condition> {
    let le16 = |d: &[u8], at: usize| d.get(at..at + 2).map_or(0, |b| u16::from_le_bytes([b[0], b[1]]));
    let count = le16(condfmt, 12) as usize;
    let ranges: Vec<(u32, u32, u32, u32)> = (0..count)
        .filter_map(|i| {
            let at = 14 + i * 8;
            (condfmt.len() >= at + 8).then(|| {
                let (r1, r2, c1, c2) = (le16(condfmt, at), le16(condfmt, at + 2), le16(condfmt, at + 4), le16(condfmt, at + 6));
                (r1 as u32 + 1, c1 as u32 + 1, r2 as u32 + 1, c2 as u32 + 1)
            })
        })
        .collect();
    let Some(first) = ranges.first() else { return Vec::new() };
    let origin = (first.0, first.1);
    let mut out = Vec::new();
    for data in rules {
        let Some(parsed) = rule(data, origin, palette) else { continue };
        *priority += 1;
        for range in &ranges {
            out.push(Condition {
                rule: CondRule { range: *range, origin, priority: *priority, dxf: 0, ..parsed.0.clone() },
                dxf: parsed.1.clone(),
            });
        }
    }
    out
}

fn rule(data: &[u8], origin: (u32, u32), palette: &dyn Fn(usize) -> Option<Color>) -> Option<(CondRule, Dxf)> {
    let le16 = |at: usize| data.get(at..at + 2).map_or(0, |b| u16::from_le_bytes([b[0], b[1]]));
    let le32 = |at: usize| data.get(at..at + 4).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let (kind, comparison) = (*data.first()?, *data.get(1)?);
    let (len1, len2) = (le16(2) as usize, le16(4) as usize);
    let flags = le32(6);
    let user_format = le16(10) & 1 != 0;
    let mut pos = 12;
    let mut dxf = Dxf::default();
    if flags & (1 << 25) != 0 {
        pos += if user_format { le16(pos) as usize } else { 2 };
    }
    if flags & (1 << 26) != 0 {
        let font = data.get(pos..pos + 118)?;
        let f32_at = |at: usize| u32::from_le_bytes(font[at..at + 4].try_into().unwrap());
        let style = f32_at(68);
        let unset = f32_at(88);
        let mut props = RunProps::default();
        if unset & 0x02 == 0 {
            props.italic = Some(style & 0x02 != 0);
        }
        if unset & 0x80 == 0 {
            props.strike = Some(style & 0x80 != 0);
        }
        if f32_at(100) == 0 {
            props.bold = Some(u16::from_le_bytes([font[72], font[73]]) >= 700);
        }
        if f32_at(96) == 0 {
            props.underline = Some(font[76] != 0);
        }
        let color = f32_at(80);
        if color != u32::MAX {
            props.color = palette(color as usize);
        }
        dxf.font = props;
        pos += 118;
    }
    if flags & (1 << 27) != 0 {
        pos += 8;
    }
    if flags & (1 << 28) != 0 {
        pos += 8;
    }
    if flags & (1 << 29) != 0 {
        let pattern = le16(pos) >> 10;
        let colors = le16(pos + 2);
        let (fore, back) = (palette((colors & 0x7F) as usize), palette(((colors >> 7) & 0x7F) as usize));
        dxf.fill = match pattern {
            0 => None,
            1 => back.or(fore),
            _ => fore.or(back),
        };
        pos += 4;
    }
    if flags & (1 << 30) != 0 {
        pos += 2;
    }
    let first = data.get(pos..pos + len1)?;
    let second = data.get(pos + len1..pos + len1 + len2)?;
    let mut formulas = vec![decompile(first, origin)?];
    if len2 > 0 {
        formulas.push(decompile(second, origin)?);
    }
    let (kind, operator) = match kind {
        1 => (
            "cellIs",
            match comparison {
                1 => "between",
                2 => "notBetween",
                3 => "equal",
                4 => "notEqual",
                5 => "greaterThan",
                6 => "lessThan",
                7 => "greaterThanOrEqual",
                8 => "lessThanOrEqual",
                _ => return None,
            },
        ),
        2 => ("expression", ""),
        _ => return None,
    };
    Some((
        CondRule {
            range: (0, 0, 0, 0),
            origin,
            priority: 0,
            dxf: 0,
            kind: kind.into(),
            operator: operator.into(),
            formulas,
            text: None,
        },
        dxf,
    ))
}

fn column_name(col: u32) -> String {
    let mut letters = String::new();
    let mut n = col;
    while n > 0 {
        letters.insert(0, (b'A' + ((n - 1) % 26) as u8) as char);
        n = (n - 1) / 26;
    }
    letters
}

fn reference(row: u32, col: u32, row_relative: bool, col_relative: bool) -> String {
    format!(
        "{}{}{}{}",
        if col_relative { "" } else { "$" },
        column_name(col + 1),
        if row_relative { "" } else { "$" },
        row + 1
    )
}

fn plain_ref(bytes: &[u8]) -> (u32, u32, bool, bool) {
    let row = u16::from_le_bytes([bytes[0], bytes[1]]) as u32;
    let col = u16::from_le_bytes([bytes[2], bytes[3]]);
    (row, (col & 0x3FFF) as u32, col & 0x8000 != 0, col & 0x4000 != 0)
}

fn relative_ref(bytes: &[u8], origin: (u32, u32)) -> (u32, u32, bool, bool) {
    let raw_row = u16::from_le_bytes([bytes[0], bytes[1]]);
    let raw_col = u16::from_le_bytes([bytes[2], bytes[3]]);
    let row_relative = raw_col & 0x8000 != 0;
    let col_relative = raw_col & 0x4000 != 0;
    let row = if row_relative { (origin.0 as i64 - 1 + raw_row as i16 as i64).max(0) as u32 } else { raw_row as u32 };
    let col = if col_relative { (origin.1 as i64 - 1 + (raw_col & 0xFF) as i8 as i64).max(0) as u32 } else { (raw_col & 0x3FFF) as u32 };
    (row, col, row_relative, col_relative)
}

pub fn decompile(rgce: &[u8], origin: (u32, u32)) -> Option<String> {
    let mut stack: Vec<String> = Vec::new();
    let mut pos = 0;
    let binary = |stack: &mut Vec<String>, op: &str| -> Option<()> {
        let b = stack.pop()?;
        let a = stack.pop()?;
        stack.push(format!("{a}{op}{b}"));
        Some(())
    };
    while pos < rgce.len() {
        let token = rgce[pos];
        pos += 1;
        let base = if token >= 0x20 { (token & 0x1F) | 0x20 } else { token };
        match base {
            0x03 => binary(&mut stack, "+")?,
            0x04 => binary(&mut stack, "-")?,
            0x05 => binary(&mut stack, "*")?,
            0x06 => binary(&mut stack, "/")?,
            0x07 => binary(&mut stack, "^")?,
            0x08 => binary(&mut stack, "&")?,
            0x09 => binary(&mut stack, "<")?,
            0x0A => binary(&mut stack, "<=")?,
            0x0B => binary(&mut stack, "=")?,
            0x0C => binary(&mut stack, ">=")?,
            0x0D => binary(&mut stack, ">")?,
            0x0E => binary(&mut stack, "<>")?,
            0x12 => {}
            0x13 => {
                let a = stack.pop()?;
                stack.push(format!("-{a}"));
            }
            0x14 => {
                let a = stack.pop()?;
                stack.push(format!("{a}/100"));
            }
            0x15 => {
                let a = stack.pop()?;
                stack.push(format!("({a})"));
            }
            0x16 => stack.push(String::new()),
            0x17 => {
                let count = *rgce.get(pos)? as usize;
                let wide = rgce.get(pos + 1)? & 1 == 1;
                pos += 2;
                let text = if wide {
                    let units: Vec<u16> = rgce.get(pos..pos + count * 2)?.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                    pos += count * 2;
                    String::from_utf16_lossy(&units)
                } else {
                    let text: String = rgce.get(pos..pos + count)?.iter().map(|&b| b as char).collect();
                    pos += count;
                    text
                };
                stack.push(format!("\"{}\"", text.replace('"', "\"\"")));
            }
            0x19 => {
                let flags = *rgce.get(pos)?;
                let value = u16::from_le_bytes(rgce.get(pos + 1..pos + 3)?.try_into().ok()?);
                pos += 3;
                if flags & 0x04 != 0 {
                    pos += (value as usize + 1) * 2;
                }
                if flags & 0x10 != 0 {
                    let a = stack.pop()?;
                    stack.push(format!("SUM({a})"));
                }
            }
            0x1C => {
                pos += 1;
                stack.push("NA()".into());
            }
            0x1D => {
                stack.push(if *rgce.get(pos)? != 0 { "TRUE".into() } else { "FALSE".into() });
                pos += 1;
            }
            0x1E => {
                stack.push(u16::from_le_bytes(rgce.get(pos..pos + 2)?.try_into().ok()?).to_string());
                pos += 2;
            }
            0x1F => {
                let value = f64::from_le_bytes(rgce.get(pos..pos + 8)?.try_into().ok()?);
                stack.push(crate::xlsx::format::general(value));
                pos += 8;
            }
            0x21 => {
                let id = u16::from_le_bytes(rgce.get(pos..pos + 2)?.try_into().ok()?);
                pos += 2;
                let (name, args) = function(id)?;
                apply(&mut stack, name, args?)?;
            }
            0x22 => {
                let args = (*rgce.get(pos)? & 0x7F) as usize;
                let id = u16::from_le_bytes(rgce.get(pos + 1..pos + 3)?.try_into().ok()?) & 0x7FFF;
                pos += 3;
                let (name, _) = function(id)?;
                apply(&mut stack, name, args)?;
            }
            0x24 => {
                let (r, c, rr, cr) = plain_ref(rgce.get(pos..pos + 4)?);
                stack.push(reference(r, c, rr, cr));
                pos += 4;
            }
            0x25 => {
                let b = rgce.get(pos..pos + 8)?;
                let first = plain_ref(&[b[0], b[1], b[4], b[5]]);
                let last = plain_ref(&[b[2], b[3], b[6], b[7]]);
                stack.push(format!("{}:{}", reference(first.0, first.1, first.2, first.3), reference(last.0, last.1, last.2, last.3)));
                pos += 8;
            }
            0x2C => {
                let (r, c, rr, cr) = relative_ref(rgce.get(pos..pos + 4)?, origin);
                stack.push(reference(r, c, rr, cr));
                pos += 4;
            }
            0x2D => {
                let b = rgce.get(pos..pos + 8)?;
                let first = relative_ref(&[b[0], b[1], b[4], b[5]], origin);
                let last = relative_ref(&[b[2], b[3], b[6], b[7]], origin);
                stack.push(format!("{}:{}", reference(first.0, first.1, first.2, first.3), reference(last.0, last.1, last.2, last.3)));
                pos += 8;
            }
            _ => return None,
        }
    }
    (stack.len() == 1).then(|| stack.pop()).flatten()
}

fn apply(stack: &mut Vec<String>, name: &str, args: usize) -> Option<()> {
    if stack.len() < args {
        return None;
    }
    let values = stack.split_off(stack.len() - args);
    stack.push(format!("{name}({})", values.join(",")));
    Some(())
}

fn function(id: u16) -> Option<(&'static str, Option<usize>)> {
    Some(match id {
        0 => ("COUNT", None),
        1 => ("IF", None),
        4 => ("SUM", None),
        5 => ("AVERAGE", None),
        6 => ("MIN", None),
        7 => ("MAX", None),
        24 => ("ABS", Some(1)),
        25 => ("INT", Some(1)),
        27 => ("ROUND", Some(2)),
        31 => ("MID", Some(3)),
        32 => ("LEN", Some(1)),
        33 => ("VALUE", Some(1)),
        34 => ("TRUE", Some(0)),
        35 => ("FALSE", Some(0)),
        36 => ("AND", None),
        37 => ("OR", None),
        38 => ("NOT", Some(1)),
        39 => ("MOD", Some(2)),
        112 => ("LOWER", Some(1)),
        113 => ("UPPER", Some(1)),
        115 => ("LEFT", None),
        116 => ("RIGHT", None),
        117 => ("EXACT", Some(2)),
        118 => ("TRIM", Some(1)),
        127 => ("ISTEXT", Some(1)),
        128 => ("ISNUMBER", Some(1)),
        129 => ("ISBLANK", Some(1)),
        131 => ("N", Some(1)),
        169 => ("COUNTA", None),
        347 => ("COUNTBLANK", Some(1)),
        _ => return None,
    })
}
