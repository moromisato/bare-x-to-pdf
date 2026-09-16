use super::styles::{parse_ppr, parse_rpr, ThemeFonts};
use crate::model::*;
use crate::xml::Element;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Default)]
pub struct Level {
    pub start: i64,
    pub format: String,
    pub text: String,
    pub suffix: ListSuffix,
    pub ppr: ParagraphProps,
    pub rpr: RunProps,
    pub legal: bool,
}

#[derive(Debug, Clone, Default)]
struct Override {
    start: Option<i64>,
    level: Option<Level>,
}

#[derive(Debug, Clone, Default)]
struct Instance {
    abstract_id: String,
    overrides: HashMap<usize, Override>,
}

#[derive(Debug, Default)]
pub struct Numbering {
    abstracts: HashMap<String, HashMap<usize, Level>>,
    instances: HashMap<String, Instance>,
}

impl Numbering {
    pub fn parse(root: Option<&Element>, theme: &ThemeFonts) -> Numbering {
        let mut out = Numbering::default();
        let Some(root) = root else { return out };

        for abs in root.children("abstractNum") {
            let Some(id) = abs.attr("abstractNumId") else { continue };
            let levels = abs
                .children("lvl")
                .filter_map(|lvl| Some((lvl.attr("ilvl")?.parse::<usize>().ok()?, parse_level(lvl, theme))))
                .collect();
            out.abstracts.insert(id.to_string(), levels);
        }

        for num in root.children("num") {
            let Some(id) = num.attr("numId") else { continue };
            let Some(abstract_id) = num.child("abstractNumId").and_then(|a| a.attr("val")) else { continue };
            let mut instance = Instance {
                abstract_id: abstract_id.to_string(),
                overrides: HashMap::new(),
            };
            for over in num.children("lvlOverride") {
                let Some(ilvl) = over.attr("ilvl").and_then(|v| v.parse::<usize>().ok()) else { continue };
                instance.overrides.insert(
                    ilvl,
                    Override {
                        start: over
                            .child("startOverride")
                            .and_then(|s| s.attr("val"))
                            .and_then(|v| v.parse().ok()),
                        level: over.child("lvl").map(|l| parse_level(l, theme)),
                    },
                );
            }
            out.instances.insert(id.to_string(), instance);
        }

        for (_, levels) in out.abstracts.iter_mut() {
            for ilvl in 0..9 {
                levels.entry(ilvl).or_insert_with(|| Level {
                    start: 1,
                    format: "decimal".into(),
                    text: format!("%{}.", ilvl + 1),
                    ..Level::default()
                });
            }
        }
        out
    }

    pub fn level(&self, num_id: &str, ilvl: usize) -> Option<Level> {
        let instance = self.instances.get(num_id)?;
        let levels = self.abstracts.get(&instance.abstract_id)?;
        let over = instance.overrides.get(&ilvl);
        let mut level = over
            .and_then(|o| o.level.clone())
            .or_else(|| levels.get(&ilvl).cloned())?;
        if let Some(start) = over.and_then(|o| o.start) {
            level.start = start;
        }
        Some(level)
    }

    fn all_levels(&self, num_id: &str) -> Vec<Level> {
        (0..9).filter_map(|i| self.level(num_id, i)).collect()
    }
}

fn parse_level(lvl: &Element, theme: &ThemeFonts) -> Level {
    Level {
        start: lvl
            .child("start")
            .and_then(|s| s.attr("val"))
            .and_then(|v| v.parse().ok())
            .unwrap_or(1),
        format: lvl
            .child("numFmt")
            .and_then(|f| f.attr("val"))
            .unwrap_or("decimal")
            .to_string(),
        text: lvl
            .child("lvlText")
            .and_then(|t| t.attr("val"))
            .unwrap_or("")
            .to_string(),
        suffix: match lvl.child("suff").and_then(|s| s.attr("val")) {
            Some("space") => ListSuffix::Space,
            Some("nothing") => ListSuffix::Nothing,
            _ => ListSuffix::Tab,
        },
        ppr: lvl.child("pPr").map(parse_ppr).unwrap_or_default(),
        rpr: lvl.child("rPr").map(|r| parse_rpr(r, theme)).unwrap_or_default(),
        legal: lvl.child("isLgl").is_some(),
    }
}

#[derive(Debug, Default)]
pub struct Counters {
    values: HashMap<String, Vec<i64>>,
    started: HashSet<(String, usize)>,
}

impl Counters {
    pub fn next(&mut self, numbering: &Numbering, num_id: &str, ilvl: usize) -> Option<(String, Level)> {
        let instance = numbering.instances.get(num_id)?;
        let level = numbering.level(num_id, ilvl)?;
        let all = numbering.all_levels(num_id);
        if all.len() <= ilvl {
            return None;
        }

        let key = instance.abstract_id.clone();
        let values = self
            .values
            .entry(key)
            .or_insert_with(|| all.iter().map(|l| l.start - 1).collect());

        let first_use = self.started.insert((num_id.to_string(), ilvl));
        if first_use {
            if let Some(start) = instance.overrides.get(&ilvl).and_then(|o| o.start) {
                values[ilvl] = start - 1;
            }
        }

        values[ilvl] += 1;
        for deeper in ilvl + 1..values.len() {
            values[deeper] = all[deeper].start - 1;
        }

        let text = if level.format == "bullet" {
            bullet_text(&level.text)
        } else {
            let mut out = String::new();
            let mut chars = level.text.chars().peekable();
            while let Some(c) = chars.next() {
                if c == '%' {
                    if let Some(d) = chars.peek().and_then(|d| d.to_digit(10)) {
                        chars.next();
                        let idx = d as usize;
                        if idx >= 1 && idx <= values.len() {
                            let format = if level.legal { "decimal" } else { all[idx - 1].format.as_str() };
                            out.push_str(&format_number(values[idx - 1].max(0), format));
                        }
                        continue;
                    }
                }
                out.push(c);
            }
            out
        };
        Some((text, level))
    }
}

fn bullet_text(text: &str) -> String {
    text.chars()
        .map(|c| match c as u32 {
            0xF0B7 | 0xF0FC | 0xF0A8 => '•',
            0xF0A7 | 0xF0A0 => '▪',
            0xF0D8 => '➢',
            0xF076 => '❖',
            0xF0B2 | 0xF071 => '□',
            0xF06E => '■',
            0xF0DE => '⇨',
            0xF0E0..=0xF0FF | 0xF000..=0xF0DF => '•',
            _ => c,
        })
        .collect()
}

pub fn format_number(value: i64, format: &str) -> String {
    match format {
        "none" => String::new(),
        "lowerLetter" => letters(value).to_lowercase(),
        "upperLetter" => letters(value),
        "lowerRoman" => roman(value).to_lowercase(),
        "upperRoman" => roman(value),
        "decimalZero" => format!("{value:02}"),
        "ordinal" => format!("{value}{}", ordinal_suffix(value)),
        _ => value.to_string(),
    }
}

fn letters(value: i64) -> String {
    if value <= 0 {
        return String::new();
    }
    let index = (value - 1) % 26;
    let repeat = ((value - 1) / 26) as usize + 1;
    let letter = (b'A' + index as u8) as char;
    std::iter::repeat(letter).take(repeat).collect()
}

fn roman(mut value: i64) -> String {
    if value <= 0 {
        return value.to_string();
    }
    let table = [
        (1000, "M"), (900, "CM"), (500, "D"), (400, "CD"), (100, "C"), (90, "XC"),
        (50, "L"), (40, "XL"), (10, "X"), (9, "IX"), (5, "V"), (4, "IV"), (1, "I"),
    ];
    let mut out = String::new();
    for (n, s) in table {
        while value >= n {
            out.push_str(s);
            value -= n;
        }
    }
    out
}

fn ordinal_suffix(value: i64) -> &'static str {
    match (value % 10, value % 100) {
        (1, n) if n != 11 => "st",
        (2, n) if n != 12 => "nd",
        (3, n) if n != 13 => "rd",
        _ => "th",
    }
}
