use super::color::{resolve_child, ColorContext, Theme};
use crate::model::*;
use crate::xml::Element;
use std::collections::HashMap;

pub const DEFAULT_SIZE: f64 = 18.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Spacing {
    Points(f64),
    Percent(f64),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Bullet {
    None,
    Char(String),
    AutoNum { scheme: String, start: i64 },
}

#[derive(Debug, Clone, Default)]
pub struct PPr {
    pub align: Option<Align>,
    pub mar_l: Option<f64>,
    pub indent: Option<f64>,
    pub line: Option<LineSpacing>,
    pub before: Option<Spacing>,
    pub after: Option<Spacing>,
    pub bullet: Option<Bullet>,
    pub bu_font: Option<String>,
    pub bu_size_pct: Option<f64>,
    pub bu_color: Option<Color>,
    pub def_rpr: RunProps,
}

impl PPr {
    pub fn merge(&mut self, other: &PPr) {
        if other.align.is_some() {
            self.align = other.align;
        }
        if other.mar_l.is_some() {
            self.mar_l = other.mar_l;
        }
        if other.indent.is_some() {
            self.indent = other.indent;
        }
        if other.line.is_some() {
            self.line = other.line;
        }
        if other.before.is_some() {
            self.before = other.before;
        }
        if other.after.is_some() {
            self.after = other.after;
        }
        if other.bullet.is_some() {
            self.bullet = other.bullet.clone();
        }
        if other.bu_font.is_some() {
            self.bu_font = other.bu_font.clone();
        }
        if other.bu_size_pct.is_some() {
            self.bu_size_pct = other.bu_size_pct;
        }
        if other.bu_color.is_some() {
            self.bu_color = other.bu_color;
        }
        self.def_rpr.merge(&other.def_rpr);
    }
}

#[derive(Debug, Clone, Default)]
pub struct LevelStyles {
    pub levels: Vec<Option<PPr>>,
}

impl LevelStyles {
    pub fn parse(el: &Element, theme: &Theme, colors: &ColorContext) -> LevelStyles {
        let mut levels = vec![None; 9];
        for child in el.elements() {
            let name = child.name.as_str();
            let Some(rest) = name.strip_prefix("lvl") else { continue };
            let Some(digit) = rest.strip_suffix("pPr") else { continue };
            let Ok(level) = digit.parse::<usize>() else { continue };
            if (1..=9).contains(&level) {
                levels[level - 1] = Some(parse_ppr(child, theme, colors));
            }
        }
        LevelStyles { levels }
    }

    pub fn level(&self, index: usize) -> Option<&PPr> {
        self.levels.get(index).and_then(|l| l.as_ref())
    }
}

pub fn parse_ppr(el: &Element, theme: &Theme, colors: &ColorContext) -> PPr {
    let mut ppr = PPr::default();
    ppr.align = match el.attr("algn") {
        Some("ctr") => Some(Align::Center),
        Some("r") => Some(Align::Right),
        Some("just") | Some("justLow") | Some("dist") => Some(Align::Justify),
        Some("l") => Some(Align::Left),
        _ => None,
    };
    ppr.mar_l = el.attr("marL").and_then(emu);
    ppr.indent = el.attr("indent").and_then(emu);
    for child in el.elements() {
        match child.name.as_str() {
            "lnSpc" => {
                ppr.line = child.elements().next().and_then(|v| match v.name.as_str() {
                    "spcPct" => v
                        .attr("val")
                        .and_then(|s| s.parse::<f64>().ok())
                        .map(|p| LineSpacing::Multiple(p / 100000.0)),
                    "spcPts" => v
                        .attr("val")
                        .and_then(|s| s.parse::<f64>().ok())
                        .map(|p| LineSpacing::Exact(p / 100.0)),
                    _ => None,
                });
            }
            "spcBef" => ppr.before = spacing(child),
            "spcAft" => ppr.after = spacing(child),
            "buNone" => ppr.bullet = Some(Bullet::None),
            "buChar" => {
                ppr.bullet = Some(Bullet::Char(child.attr("char").unwrap_or("•").to_string()));
            }
            "buAutoNum" => {
                ppr.bullet = Some(Bullet::AutoNum {
                    scheme: child.attr("type").unwrap_or("arabicPeriod").to_string(),
                    start: child
                        .attr("startAt")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(1),
                });
            }
            "buFont" => ppr.bu_font = child.attr("typeface").map(|t| theme.font(t)),
            "buSzPct" => {
                ppr.bu_size_pct = child
                    .attr("val")
                    .and_then(|v| v.parse::<f64>().ok())
                    .map(|v| v / 100000.0);
            }
            "buClr" => ppr.bu_color = resolve_child(child, colors),
            "defRPr" => ppr.def_rpr = parse_rpr(child, theme, colors),
            _ => {}
        }
    }
    ppr
}

fn spacing(el: &Element) -> Option<Spacing> {
    let v = el.elements().next()?;
    let value = v.attr("val")?.parse::<f64>().ok()?;
    match v.name.as_str() {
        "spcPts" => Some(Spacing::Points(value / 100.0)),
        "spcPct" => Some(Spacing::Percent(value / 100000.0)),
        _ => None,
    }
}

pub fn parse_rpr(el: &Element, theme: &Theme, colors: &ColorContext) -> RunProps {
    let mut props = RunProps::default();
    if let Some(size) = el.attr("sz").and_then(|v| v.parse::<f64>().ok()) {
        props.size = Some(size / 100.0);
    }
    if let Some(b) = el.attr("b") {
        props.bold = Some(b == "1" || b == "true");
    }
    if let Some(i) = el.attr("i") {
        props.italic = Some(i == "1" || i == "true");
    }
    if let Some(u) = el.attr("u") {
        props.underline = Some(u != "none");
    }
    if let Some(s) = el.attr("strike") {
        props.strike = Some(s != "noStrike");
    }
    if let Some(cap) = el.attr("cap") {
        props.caps = Some(cap == "all");
        props.small_caps = Some(cap == "small");
    }
    if let Some(base) = el.attr("baseline").and_then(|v| v.parse::<i64>().ok()) {
        props.vertical = Some(if base > 0 {
            VerticalAlign::Superscript
        } else if base < 0 {
            VerticalAlign::Subscript
        } else {
            VerticalAlign::Baseline
        });
    }
    for child in el.elements() {
        match child.name.as_str() {
            "solidFill" => {
                if let Some(c) = resolve_child(child, colors) {
                    props.color = Some(c);
                }
            }
            "latin" => {
                if let Some(face) = child.attr("typeface").filter(|f| !f.is_empty()) {
                    props.font = Some(theme.font(face));
                }
            }
            "hlinkClick" => {
                if let Some(c) = colors.theme.colors.get("hlink") {
                    props.color = Some(*c);
                }
                props.underline = Some(true);
            }
            _ => {}
        }
    }
    props
}

pub fn emu(value: &str) -> Option<f64> {
    value.trim().parse::<f64>().ok().map(|v| v / 12700.0)
}

pub struct TextContext<'a> {
    pub theme: &'a Theme,
    pub colors: &'a ColorContext<'a>,
    pub chain: Vec<&'a LevelStyles>,
    pub font_scale: f64,
    pub spacing_reduction: f64,
    pub slide_number: usize,
}

pub fn text_blocks(body: &Element, ctx: &TextContext) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut counters: HashMap<usize, i64> = HashMap::new();
    for p in body.children("p") {
        blocks.push(Block::Paragraph(paragraph(p, ctx, &mut counters)));
    }
    blocks
}

fn paragraph(p: &Element, ctx: &TextContext, counters: &mut HashMap<usize, i64>) -> Paragraph {
    let ppr_el = p.child("pPr");
    let level = ppr_el
        .and_then(|e| e.attr("lvl"))
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0)
        .min(8);

    let mut ppr = PPr::default();
    for styles in &ctx.chain {
        if let Some(l) = styles.level(level) {
            ppr.merge(l);
        }
    }
    if let Some(el) = ppr_el {
        ppr.merge(&parse_ppr(el, ctx.theme, ctx.colors));
    }

    let mut base = RunProps {
        font: Some(ctx.theme.minor.clone()),
        size: Some(DEFAULT_SIZE),
        color: ctx.colors.theme.colors.get("dk1").copied().or(Some(Color(0, 0, 0))),
        ..RunProps::default()
    };
    base.merge(&ppr.def_rpr);

    let scale = |props: &mut RunProps| {
        if let Some(size) = props.size {
            props.size = Some((size * ctx.font_scale).max(1.0));
        }
    };

    let mut inlines = Vec::new();
    let mut last_props = base.clone();
    for child in p.elements() {
        match child.name.as_str() {
            "r" | "fld" => {
                let mut props = base.clone();
                if let Some(rpr) = child.child("rPr") {
                    props.merge(&parse_rpr(rpr, ctx.theme, ctx.colors));
                }
                scale(&mut props);
                let text = if child.name == "fld" && child.attr("type") == Some("slidenum") {
                    ctx.slide_number.to_string()
                } else {
                    child.child("t").map(|t| t.text()).unwrap_or_default()
                };
                last_props = props.clone();
                if !text.is_empty() {
                    inlines.push(Inline::Text { text, props });
                }
            }
            "br" => inlines.push(Inline::LineBreak),
            _ => {}
        }
    }

    let mut mark = base.clone();
    if let Some(end) = p.child("endParaRPr") {
        mark.merge(&parse_rpr(end, ctx.theme, ctx.colors));
    }
    scale(&mut mark);
    let size = last_props.size.or(mark.size).unwrap_or(DEFAULT_SIZE);

    let to_points = |s: Option<Spacing>| match s {
        Some(Spacing::Points(v)) => Some(v),
        Some(Spacing::Percent(p)) => Some(p * size),
        None => None,
    };

    let mar_l = ppr.mar_l.unwrap_or(0.0);
    let indent = ppr.indent.unwrap_or(0.0);
    let line = match ppr.line {
        Some(LineSpacing::Multiple(m)) => Some(LineSpacing::Multiple((m - ctx.spacing_reduction).max(0.5))),
        other => other,
    };

    let mut props = ParagraphProps {
        align: ppr.align,
        indent_left: Some(mar_l),
        space_before: to_points(ppr.before),
        space_after: to_points(ppr.after),
        line_spacing: line,
        ..ParagraphProps::default()
    };
    if indent < 0.0 {
        props.indent_hanging = Some(-indent);
        props.indent_first_line = Some(0.0);
    } else {
        props.indent_first_line = Some(indent);
        props.indent_hanging = Some(0.0);
    }

    let has_text = inlines.iter().any(|i| matches!(i, Inline::Text { text, .. } if !text.trim().is_empty()));
    let list = if has_text {
        match &ppr.bullet {
            Some(Bullet::Char(ch)) => Some(bullet_label(ch, &ppr, &last_props, size)),
            Some(Bullet::AutoNum { scheme, start }) => {
                let count = counters.entry(level).or_insert(start - 1);
                *count += 1;
                let value = *count;
                counters.retain(|l, _| *l <= level);
                let text = auto_number(scheme, value);
                let mut label_props = last_props.clone();
                if let Some(c) = ppr.bu_color {
                    label_props.color = Some(c);
                }
                if let Some(pct) = ppr.bu_size_pct {
                    label_props.size = Some(size * pct);
                }
                Some(ListLabel {
                    text,
                    props: label_props,
                    suffix: ListSuffix::Tab,
                })
            }
            _ => None,
        }
    } else {
        None
    };

    Paragraph {
        props,
        mark,
        inlines,
        anchors: Vec::new(),
        list,
    }
}

fn bullet_label(ch: &str, ppr: &PPr, run: &RunProps, size: f64) -> ListLabel {
    let mut props = run.clone();
    props.bold = Some(false);
    props.italic = Some(false);
    props.underline = Some(false);
    if let Some(c) = ppr.bu_color {
        props.color = Some(c);
    }
    if let Some(pct) = ppr.bu_size_pct {
        props.size = Some(size * pct);
    }
    let symbol_font = ppr
        .bu_font
        .as_deref()
        .map(|f| {
            let l = f.to_lowercase();
            l.contains("symbol") || l.contains("wingdings") || l.contains("webdings")
        })
        .unwrap_or(false);
    if let Some(font) = &ppr.bu_font {
        if !symbol_font {
            props.font = Some(font.clone());
        }
    }
    ListLabel {
        text: map_bullet(ch),
        props,
        suffix: ListSuffix::Tab,
    }
}

pub fn map_bullet(text: &str) -> String {
    text.chars()
        .map(|c| match c as u32 {
            0xF0B7 | 0xF0FC | 0xF0A8 | 0xF06C => '•',
            0xF0A7 | 0xF0A0 | 0xF06E => '▪',
            0xF0D8 | 0xF0E0 => '➢',
            0xF076 => '❖',
            0xF0B2 | 0xF071 => '□',
            0xF0FE | 0xF0FD => '☒',
            0xF000..=0xF0FF => '•',
            _ => c,
        })
        .collect()
}

fn auto_number(scheme: &str, value: i64) -> String {
    use crate::docx::format_number;
    let (format, prefix, suffix) = match scheme {
        s if s.starts_with("alphaLc") => ("lowerLetter", "", ""),
        s if s.starts_with("alphaUc") => ("upperLetter", "", ""),
        s if s.starts_with("romanLc") => ("lowerRoman", "", ""),
        s if s.starts_with("romanUc") => ("upperRoman", "", ""),
        _ => ("decimal", "", ""),
    };
    let number = format_number(value, format);
    let decorated = if scheme.ends_with("ParenBoth") {
        format!("({number})")
    } else if scheme.ends_with("ParenR") {
        format!("{number})")
    } else if scheme.ends_with("Period") {
        format!("{number}.")
    } else {
        number
    };
    format!("{prefix}{decorated}{suffix}")
}
