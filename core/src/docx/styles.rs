use super::{flag, half_points, parse_borders, parse_cell_margins, twips};
use crate::model::*;
use crate::xml::Element;
use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct ThemeFonts {
    major: Option<String>,
    minor: Option<String>,
}

impl ThemeFonts {
    pub fn parse(theme: &Element) -> ThemeFonts {
        let scheme = theme
            .child("themeElements")
            .and_then(|e| e.child("fontScheme"));
        let latin = |name: &str| {
            scheme
                .and_then(|s| s.child(name))
                .and_then(|f| f.child("latin"))
                .and_then(|l| l.attr("typeface"))
                .filter(|t| !t.is_empty())
                .map(str::to_owned)
        };
        ThemeFonts {
            major: latin("majorFont"),
            minor: latin("minorFont"),
        }
    }

    fn resolve(&self, theme_font: &str) -> Option<String> {
        if theme_font.starts_with("major") {
            self.major.clone()
        } else if theme_font.starts_with("minor") {
            self.minor.clone()
        } else {
            None
        }
    }
}

#[derive(Debug, Default)]
struct Style {
    kind: String,
    based_on: Option<String>,
    ppr: ParagraphProps,
    rpr: RunProps,
    table_borders: Borders,
    table_margins: CellMargins,
}

#[derive(Debug, Default)]
pub struct Styles {
    doc_ppr: ParagraphProps,
    doc_rpr: RunProps,
    styles: HashMap<String, Style>,
    default_paragraph: Option<String>,
}

impl Styles {
    pub fn parse(root: Option<&Element>, theme: &ThemeFonts) -> Styles {
        let mut out = Styles {
            doc_rpr: RunProps {
                font: Some("Calibri".into()),
                size: Some(11.0),
                ..RunProps::default()
            },
            ..Styles::default()
        };
        let Some(root) = root else { return out };

        if let Some(defaults) = root.child("docDefaults") {
            if let Some(rpr) = defaults.child("rPrDefault").and_then(|d| d.child("rPr")) {
                out.doc_rpr.merge(&parse_rpr(rpr, theme));
            }
            if let Some(ppr) = defaults.child("pPrDefault").and_then(|d| d.child("pPr")) {
                out.doc_ppr.merge(&parse_ppr(ppr));
            }
        }

        for style in root.children("style") {
            let Some(id) = style.attr("styleId") else { continue };
            let kind = style.attr("type").unwrap_or("paragraph").to_owned();
            let parsed = Style {
                based_on: style
                    .child("basedOn")
                    .and_then(|b| b.attr("val"))
                    .map(str::to_owned),
                ppr: style.child("pPr").map(parse_ppr).unwrap_or_default(),
                rpr: style
                    .child("rPr")
                    .map(|r| parse_rpr(r, theme))
                    .unwrap_or_default(),
                table_borders: style
                    .child("tblPr")
                    .and_then(|p| p.child("tblBorders"))
                    .map(parse_borders)
                    .unwrap_or_default(),
                table_margins: style
                    .child("tblPr")
                    .and_then(|p| p.child("tblCellMar"))
                    .map(parse_cell_margins)
                    .unwrap_or_default(),
                kind: kind.clone(),
            };
            if kind == "paragraph" && style.attr("default").map(|d| d == "1" || d == "true") == Some(true) {
                out.default_paragraph = Some(id.to_owned());
            }
            out.styles.insert(id.to_owned(), parsed);
        }
        out
    }

    fn chain(&self, id: Option<&str>) -> Vec<&Style> {
        let mut chain = Vec::new();
        let mut current = id.map(str::to_owned);
        let mut guard = 0;
        while let Some(cur) = current {
            let Some(style) = self.styles.get(&cur) else { break };
            chain.push(style);
            current = style.based_on.clone();
            guard += 1;
            if guard > 32 {
                break;
            }
        }
        chain.reverse();
        chain
    }

    fn paragraph_style<'a>(&'a self, id: Option<&'a str>) -> Option<&'a str> {
        id.filter(|i| self.styles.contains_key(*i))
            .or(self.default_paragraph.as_deref())
    }

    pub fn paragraph_props(&self, id: Option<&str>) -> ParagraphProps {
        let mut props = self.doc_ppr.clone();
        for style in self.chain(self.paragraph_style(id)) {
            props.merge(&style.ppr);
        }
        props
    }

    pub fn run_base(&self, paragraph_style: Option<&str>) -> RunProps {
        let mut props = self.doc_rpr.clone();
        for style in self.chain(self.paragraph_style(paragraph_style)) {
            props.merge(&style.rpr);
        }
        props
    }

    pub fn apply_character_style(&self, id: &str, props: &mut RunProps) {
        for style in self.chain(Some(id)) {
            if style.kind == "character" || style.kind == "paragraph" {
                props.merge(&style.rpr);
            }
        }
    }

    pub fn table_style(&self, id: Option<&str>) -> (Borders, CellMargins) {
        let mut borders = Borders::default();
        let mut margins = CellMargins::default();
        for style in self.chain(id) {
            borders.merge(&style.table_borders);
            margins.merge(&style.table_margins);
        }
        (borders, margins)
    }
}

pub fn parse_ppr(ppr: &Element) -> ParagraphProps {
    let mut props = ParagraphProps::default();
    for el in ppr.elements() {
        match el.name.as_str() {
            "jc" => {
                props.align = match el.attr("val") {
                    Some("center") => Some(Align::Center),
                    Some("right") | Some("end") => Some(Align::Right),
                    Some("both") | Some("distribute") => Some(Align::Justify),
                    Some(_) => Some(Align::Left),
                    None => None,
                }
            }
            "ind" => {
                props.indent_left = el.attr("left").or(el.attr("start")).and_then(twips);
                props.indent_right = el.attr("right").or(el.attr("end")).and_then(twips);
                if let Some(first) = el.attr("firstLine").and_then(twips) {
                    props.indent_first_line = Some(first);
                    props.indent_hanging = Some(0.0);
                }
                if let Some(hanging) = el.attr("hanging").and_then(twips) {
                    props.indent_hanging = Some(hanging);
                    props.indent_first_line = Some(0.0);
                }
            }
            "spacing" => {
                if el.attr("beforeAutospacing").map(|v| v == "1" || v == "true") == Some(true) {
                    props.space_before = Some(14.0);
                } else if let Some(before) = el.attr("before").and_then(twips) {
                    props.space_before = Some(before);
                }
                if el.attr("afterAutospacing").map(|v| v == "1" || v == "true") == Some(true) {
                    props.space_after = Some(14.0);
                } else if let Some(after) = el.attr("after").and_then(twips) {
                    props.space_after = Some(after);
                }
                let line = el.attr("line").and_then(|v| v.trim().parse::<f64>().ok());
                match (line, el.attr("lineRule")) {
                    (Some(line), Some("exact")) => props.line_spacing = Some(LineSpacing::Exact(line / 20.0)),
                    (Some(line), Some("atLeast")) => props.line_spacing = Some(LineSpacing::AtLeast(line / 20.0)),
                    (Some(line), _) => props.line_spacing = Some(LineSpacing::Multiple(line / 240.0)),
                    (None, Some("auto")) => props.line_spacing = Some(LineSpacing::Multiple(1.0)),
                    (None, _) => {}
                }
            }
            "numPr" => {
                let num_id = el.child("numId").and_then(|n| n.attr("val")).unwrap_or("0");
                let ilvl = el
                    .child("ilvl")
                    .and_then(|l| l.attr("val"))
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                props.numbering = Some((num_id.to_string(), ilvl));
            }
            "tabs" => {
                for tab in el.children("tab") {
                    if let Some(pos) = tab.attr("pos").and_then(twips) {
                        props.tabs.push(TabStop {
                            pos,
                            clear: tab.attr("val") == Some("clear"),
                        });
                    }
                }
            }
            "pBdr" => {
                props.borders = super::parse_borders(el);
                for side in el.elements() {
                    let space = side.attr("space").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                    match side.name.as_str() {
                        "top" => props.border_space.top = space,
                        "left" | "start" => props.border_space.left = space,
                        "bottom" => props.border_space.bottom = space,
                        "right" | "end" => props.border_space.right = space,
                        _ => {}
                    }
                }
            }
            "shd" => {
                props.shading = el
                    .attr("fill")
                    .filter(|f| *f != "auto")
                    .and_then(Color::parse_hex);
            }
            "pageBreakBefore" => props.page_break_before = Some(flag(el)),
            "keepNext" => props.keep_next = Some(flag(el)),
            "contextualSpacing" => props.contextual_spacing = Some(flag(el)),
            _ => {}
        }
    }
    props
}

pub fn parse_rpr(rpr: &Element, theme: &ThemeFonts) -> RunProps {
    let mut props = RunProps::default();
    for el in rpr.elements() {
        match el.name.as_str() {
            "rFonts" => {
                let themed = el
                    .attr("asciiTheme")
                    .or(el.attr("hAnsiTheme"))
                    .and_then(|t| theme.resolve(t));
                let direct = el.attr("ascii").or(el.attr("hAnsi")).map(str::to_owned);
                if let Some(font) = themed.or(direct) {
                    props.font = Some(font);
                }
            }
            "sz" => props.size = el.attr("val").and_then(half_points),
            "spacing" => props.letter_spacing = el.attr("val").and_then(|v| v.parse::<f64>().ok()).map(|v| v / 20.0),
            "b" => props.bold = Some(flag(el)),
            "i" => props.italic = Some(flag(el)),
            "u" => props.underline = Some(!matches!(el.attr("val"), Some("none"))),
            "strike" | "dstrike" => props.strike = Some(flag(el)),
            "vanish" => props.hidden = Some(flag(el)),
            "caps" => props.caps = Some(flag(el)),
            "smallCaps" => props.small_caps = Some(flag(el)),
            "color" => {
                if let Some(color) = el.attr("val").filter(|v| *v != "auto").and_then(Color::parse_hex) {
                    props.color = Some(color);
                }
            }
            "highlight" => props.highlight = el.attr("val").and_then(highlight_color),
            "shd" => {
                if let Some(fill) = el.attr("fill").filter(|v| *v != "auto").and_then(Color::parse_hex) {
                    props.highlight = Some(fill);
                }
            }
            "vertAlign" => {
                props.vertical = Some(match el.attr("val") {
                    Some("superscript") => VerticalAlign::Superscript,
                    Some("subscript") => VerticalAlign::Subscript,
                    _ => VerticalAlign::Baseline,
                })
            }
            _ => {}
        }
    }
    props
}

fn highlight_color(name: &str) -> Option<Color> {
    Some(match name {
        "yellow" => Color(255, 255, 0),
        "green" => Color(0, 255, 0),
        "cyan" => Color(0, 255, 255),
        "magenta" => Color(255, 0, 255),
        "blue" => Color(0, 0, 255),
        "red" => Color(255, 0, 0),
        "darkBlue" => Color(0, 0, 128),
        "darkCyan" => Color(0, 128, 128),
        "darkGreen" => Color(0, 128, 0),
        "darkMagenta" => Color(128, 0, 128),
        "darkRed" => Color(128, 0, 0),
        "darkYellow" => Color(128, 128, 0),
        "darkGray" => Color(128, 128, 128),
        "lightGray" => Color(192, 192, 192),
        "black" => Color(0, 0, 0),
        "white" => Color(255, 255, 255),
        _ => return None,
    })
}
