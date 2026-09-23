use crate::model::*;
use crate::xml::Element;
use std::collections::HashMap;

pub fn length(value: &str) -> Option<f64> {
    let v = value.trim();
    let split = v.find(|c: char| c.is_ascii_alphabetic() || c == '%').unwrap_or(v.len());
    let number: f64 = v[..split].trim().parse().ok()?;
    Some(match &v[split..] {
        "cm" => number * 72.0 / 2.54,
        "mm" => number * 72.0 / 25.4,
        "in" | "inch" => number * 72.0,
        "pt" | "" => number,
        "pc" => number * 12.0,
        "px" => number * 0.75,
        _ => return None,
    })
}

fn angle_degrees(value: Option<&str>) -> f64 {
    let Some(value) = value else { return 0.0 };
    let number: f64 = value.trim_end_matches("deg").trim().parse().unwrap_or(0.0);
    if value.ends_with("deg") { number } else { number / 10.0 }
}

fn percent(value: &str) -> Option<f64> {
    value.trim().strip_suffix('%')?.trim().parse::<f64>().ok().map(|v| v / 100.0)
}

#[derive(Debug, Clone, Default)]
struct Style {
    family: String,
    parent: Option<String>,
    ppr: ParagraphProps,
    rpr: RunProps,
    size_percent: Option<f64>,
    list_style: Option<String>,
    master_page: Option<String>,
    outline_level: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GraphicFill {
    Gradient { start: Color, end: Color, angle: f64, radial: bool },
    Hatch { color: Color, distance: f64, angle: f64, background: Option<Color> },
    ImageRef { href: String, repeat: bool },
}

#[derive(Debug, Clone, Default)]
pub struct GraphicProps {
    pub fill: Option<Color>,
    pub fill_style: Option<GraphicFill>,
    pub opacity: f64,
    pub stroke: Option<(f64, Color)>,
    pub padding: (f64, f64, f64, f64),
    pub wrap: Wrap,
    pub behind: bool,
    pub h_pos: String,
    pub v_pos: String,
    pub h_rel: String,
    pub v_rel: String,
    pub margins: (f64, f64, f64, f64),
}

#[derive(Debug, Clone, Default)]
pub struct TableProps {
    pub indent: f64,
}

#[derive(Debug, Clone, Default)]
pub struct CellProps {
    pub borders: Borders,
    pub margins: CellMargins,
    pub fill: Option<Color>,
    pub valign: VAlign,
}

#[derive(Debug, Clone)]
pub enum LevelKind {
    Bullet(String),
    Number { format: String, prefix: String, suffix: String, start: i64, display_levels: usize },
    None,
}

#[derive(Debug, Clone)]
pub struct ListLevel {
    pub kind: LevelKind,
    pub left: Option<f64>,
    pub hanging: Option<f64>,
    pub suffix: ListSuffix,
    pub rpr: Option<RunProps>,
    pub tab_pos: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct PageLayout {
    pub page: PageSetup,
    pub columns: usize,
    pub column_gap: f64,
}

#[derive(Debug, Clone)]
pub struct MasterPage {
    pub layout: String,
    pub header: Option<Element>,
    pub footer: Option<Element>,
}

#[derive(Debug, Default)]
pub struct Styles {
    fonts: HashMap<String, String>,
    styles: HashMap<String, Style>,
    raw: HashMap<String, Element>,
    default_paragraph: Style,
    lists: HashMap<String, Vec<Option<ListLevel>>>,
    pub page_layouts: HashMap<String, PageLayout>,
    pub master_pages: HashMap<String, MasterPage>,
    pub default_tab: f64,
    pub outline_style: Option<String>,
    gradients: HashMap<String, (Color, Color, f64, bool)>,
    hatches: HashMap<String, (Color, f64, f64)>,
    fill_images: HashMap<String, String>,
    opacities: HashMap<String, f64>,
    defaults: HashMap<String, Element>,
    pub page_layout_elements: HashMap<String, Element>,
}

impl Styles {
    pub fn parse(styles_root: Option<&Element>, content: &Element) -> Styles {
        let mut out = Styles {
            default_tab: 36.0,
            default_paragraph: Style {
                family: "paragraph".into(),
                rpr: RunProps {
                    font: Some("Liberation Serif".into()),
                    size: Some(12.0),
                    ..RunProps::default()
                },
                ..Style::default()
            },
            ..Styles::default()
        };

        for root in [styles_root, Some(content)].into_iter().flatten() {
            if let Some(decls) = root.child("font-face-decls") {
                for face in decls.children("font-face") {
                    if let (Some(name), Some(family)) = (face.attr("name"), face.attr("font-family")) {
                        out.fonts.insert(name.to_string(), family.trim_matches('\'').trim_matches('"').to_string());
                    }
                }
            }
        }

        let fonts = out.fonts.clone();
        for root in [styles_root, Some(content)].into_iter().flatten() {
            for group in ["styles", "automatic-styles"] {
                let Some(container) = root.child(group) else { continue };
                for el in container.elements() {
                    match el.name.as_str() {
                        "default-style" => {
                            if let Some(family) = el.attr("family") {
                                out.defaults.insert(family.to_string(), el.clone());
                            }
                            if el.attr("family") == Some("paragraph") {
                                let style = parse_style(el, &fonts);
                                out.default_paragraph.ppr.merge(&style.ppr);
                                out.default_paragraph.rpr.merge(&style.rpr);
                                if let Some(tab) = el
                                    .child("paragraph-properties")
                                    .and_then(|p| p.attr("tab-stop-distance"))
                                    .and_then(length)
                                {
                                    out.default_tab = tab;
                                }
                            }
                        }
                        "style" => {
                            let style = parse_style(el, &fonts);
                            if let Some(name) = el.attr("name") {
                                out.raw.insert(format!("{}:{}", style.family, name), el.clone());
                                out.styles.insert(format!("{}:{}", style.family, name), style);
                            }
                        }
                        "list-style" | "outline-style" => {
                            let name = el.attr("name").unwrap_or("outline").to_string();
                            if el.name == "outline-style" {
                                out.outline_style = Some(name.clone());
                            }
                            out.lists.insert(name, parse_list(el, &fonts));
                        }
                        "page-layout" => {
                            if let Some(name) = el.attr("name") {
                                out.page_layout_elements.insert(name.to_string(), el.clone());
                                out.page_layouts.insert(name.to_string(), parse_page_layout(el));
                            }
                        }
                        "gradient" => {
                            if let (Some(name), Some(start), Some(end)) = (
                                el.attr("name"),
                                el.attr("start-color").and_then(Color::parse_hex),
                                el.attr("end-color").and_then(Color::parse_hex),
                            ) {
                                let angle = angle_degrees(el.attr("angle"));
                                let radial = matches!(el.attr("style"), Some("radial") | Some("ellipsoid") | Some("square") | Some("rectangular"));
                                out.gradients.insert(name.to_string(), (start, end, angle, radial));
                            }
                        }
                        "hatch" => {
                            if let Some(name) = el.attr("name") {
                                let color = el.attr("color").and_then(Color::parse_hex).unwrap_or(Color(0, 0, 0));
                                let distance = el.attr("distance").and_then(length).unwrap_or(2.0);
                                out.hatches.insert(name.to_string(), (color, distance, angle_degrees(el.attr("rotation"))));
                            }
                        }
                        "fill-image" => {
                            if let (Some(name), Some(href)) = (el.attr("name"), el.attr("href")) {
                                out.fill_images.insert(name.to_string(), href.trim_start_matches("./").to_string());
                            }
                        }
                        "opacity" => {
                            if let Some(name) = el.attr("name") {
                                let start = el.attr("start").and_then(percent).unwrap_or(0.0);
                                let end = el.attr("end").and_then(percent).unwrap_or(0.0);
                                out.opacities.insert(name.to_string(), (1.0 - (start + end) / 2.0).clamp(0.0, 1.0));
                            }
                        }
                        _ => {}
                    }
                }
            }
            if let Some(masters) = root.child("master-styles") {
                for master in masters.children("master-page") {
                    let Some(name) = master.attr("name") else { continue };
                    out.master_pages.insert(
                        name.to_string(),
                        MasterPage {
                            layout: master.attr("page-layout-name").unwrap_or("").to_string(),
                            header: master.child("header").filter(|h| h.attr("display") != Some("false")).cloned(),
                            footer: master.child("footer").filter(|f| f.attr("display") != Some("false")).cloned(),
                        },
                    );
                }
            }
        }
        out
    }

    fn chain(&self, family: &str, name: &str) -> Vec<&Style> {
        let mut chain = Vec::new();
        let mut current = Some(name.to_string());
        let mut guard = 0;
        while let Some(cur) = current {
            let Some(style) = self.styles.get(&format!("{family}:{cur}")) else { break };
            chain.push(style);
            current = style.parent.clone();
            guard += 1;
            if guard > 32 {
                break;
            }
        }
        chain.reverse();
        chain
    }

    pub fn paragraph(&self, name: Option<&str>) -> (ParagraphProps, RunProps) {
        let mut ppr = self.default_paragraph.ppr.clone();
        let mut rpr = self.default_paragraph.rpr.clone();
        if rpr.kerning.is_none() {
            rpr.kerning = Some(true);
        }
        if let Some(name) = name {
            for style in self.chain("paragraph", name) {
                ppr.merge(&style.ppr);
                let mut r = style.rpr.clone();
                if let Some(pct) = style.size_percent {
                    r.size = Some(rpr.size.unwrap_or(12.0) * pct);
                }
                rpr.merge(&r);
            }
        }
        (ppr, rpr)
    }

    pub fn text(&self, name: &str, base_size: f64) -> RunProps {
        let mut rpr = RunProps::default();
        let mut size = base_size;
        for style in self.chain("text", name) {
            let mut r = style.rpr.clone();
            if let Some(pct) = style.size_percent {
                r.size = Some(size * pct);
            }
            if let Some(s) = r.size {
                size = s;
            }
            rpr.merge(&r);
        }
        rpr
    }

    pub fn list_style_of(&self, paragraph_style: &str) -> Option<String> {
        self.chain("paragraph", paragraph_style)
            .iter()
            .rev()
            .find_map(|s| s.list_style.clone())
    }

    pub fn outline_level_of(&self, paragraph_style: Option<&str>) -> Option<usize> {
        self.chain("paragraph", paragraph_style?)
            .iter()
            .rev()
            .find_map(|s| s.outline_level)
    }

    pub fn master_page(&self, paragraph_style: &str) -> Option<String> {
        self.chain("paragraph", paragraph_style)
            .iter()
            .rev()
            .find_map(|s| s.master_page.clone())
    }

    pub fn list_level(&self, list_style: &str, level: usize) -> Option<ListLevel> {
        self.lists.get(list_style)?.get(level)?.clone()
    }

    pub fn graphic(&self, name: &str) -> GraphicProps {
        let mut props = GraphicProps {
            wrap: Wrap::TopAndBottom,
            h_pos: "from-left".into(),
            v_pos: "from-top".into(),
            h_rel: "paragraph".into(),
            v_rel: "paragraph".into(),
            opacity: 1.0,
            ..GraphicProps::default()
        };
        for style_name in self.chain_names("graphic", name) {
            let Some(el) = self.raw_graphic(&style_name) else { continue };
            let Some(g) = el.child("graphic-properties") else { continue };
            if let Some(opacity) = g.attr("opacity").and_then(percent) {
                props.opacity = opacity;
            } else if let Some(opacity) = g.attr("opacity-name").and_then(|n| self.opacities.get(n)) {
                props.opacity = *opacity;
            }
            match g.attr("fill") {
                Some("none") => {
                    props.fill = None;
                    props.fill_style = None;
                }
                Some("solid") => {
                    props.fill = g.attr("fill-color").and_then(Color::parse_hex).or(props.fill);
                    props.fill_style = None;
                }
                Some("gradient") => {
                    props.fill_style = g
                        .attr("fill-gradient-name")
                        .and_then(|n| self.gradients.get(n))
                        .map(|(start, end, angle, radial)| GraphicFill::Gradient { start: *start, end: *end, angle: *angle, radial: *radial });
                }
                Some("hatch") => {
                    let background = if g.attr("fill-hatch-solid") == Some("true") {
                        g.attr("fill-color").or(g.attr("background-color")).and_then(Color::parse_hex)
                    } else {
                        None
                    };
                    props.fill_style = g
                        .attr("fill-hatch-name")
                        .and_then(|n| self.hatches.get(n))
                        .map(|(color, distance, angle)| GraphicFill::Hatch { color: *color, distance: *distance, angle: *angle, background });
                }
                Some("bitmap") => {
                    let repeat = g
                        .attr("repeat")
                        .or_else(|| g.child("background-image").and_then(|b| b.attr("repeat")))
                        .map(|r| r == "repeat")
                        .unwrap_or(true);
                    let href = g
                        .attr("fill-image-name")
                        .and_then(|n| self.fill_images.get(n).cloned())
                        .or_else(|| g.child("background-image").and_then(|b| b.attr("href")).map(|h| h.trim_start_matches("./").to_string()));
                    props.fill_style = href.map(|href| GraphicFill::ImageRef { href, repeat });
                }
                _ => {
                    if let Some(c) = g.attr("fill-color").and_then(Color::parse_hex) {
                        props.fill = Some(c);
                    }
                    if let Some(c) = g.attr("background-color").and_then(Color::parse_hex) {
                        props.fill = Some(c);
                    }
                }
            }
            match g.attr("stroke") {
                Some("none") => props.stroke = None,
                Some(_) => {
                    let width = g.attr("stroke-width").and_then(length).unwrap_or(0.5);
                    let color = g.attr("stroke-color").and_then(Color::parse_hex).unwrap_or(Color(0, 0, 0));
                    props.stroke = Some((width, color));
                }
                None => {}
            }
            if let Some(border) = g.attr("border").filter(|b| *b != "none") {
                if let Some((w, c, _)) = parse_border(border) {
                    props.stroke = Some((w, c));
                }
            } else if g.attr("border") == Some("none") {
                props.stroke = None;
            }
            let pad = |n: &str| g.attr(n).and_then(length);
            if let Some(all) = pad("padding") {
                props.padding = (all, all, all, all);
            }
            props.padding = (
                pad("padding-top").unwrap_or(props.padding.0),
                pad("padding-left").unwrap_or(props.padding.1),
                pad("padding-bottom").unwrap_or(props.padding.2),
                pad("padding-right").unwrap_or(props.padding.3),
            );
            if let Some(wrap) = g.attr("wrap") {
                props.wrap = match wrap {
                    "none" => Wrap::TopAndBottom,
                    "run-through" => Wrap::None,
                    _ => Wrap::Square,
                };
            }
            if let Some(rt) = g.attr("run-through") {
                props.behind = rt == "background";
            }
            if let Some(v) = g.attr("horizontal-pos") {
                props.h_pos = v.to_string();
            }
            if let Some(v) = g.attr("vertical-pos") {
                props.v_pos = v.to_string();
            }
            if let Some(v) = g.attr("horizontal-rel") {
                props.h_rel = v.to_string();
            }
            if let Some(v) = g.attr("vertical-rel") {
                props.v_rel = v.to_string();
            }
            let m = |n: &str| g.attr(n).and_then(length).unwrap_or(0.0);
            props.margins = (m("margin-top"), m("margin-left"), m("margin-bottom"), m("margin-right"));
        }
        props
    }

    fn chain_names(&self, family: &str, name: &str) -> Vec<String> {
        let mut names = Vec::new();
        let mut current = Some(name.to_string());
        let mut guard = 0;
        while let Some(cur) = current {
            let Some(style) = self.styles.get(&format!("{family}:{cur}")) else { break };
            names.push(cur.clone());
            current = style.parent.clone();
            guard += 1;
            if guard > 32 {
                break;
            }
        }
        names.reverse();
        names
    }

    pub fn elements(&self, family: &str, name: &str) -> Vec<&Element> {
        self.chain_names(family, name).iter().filter_map(|n| self.raw.get(&format!("{family}:{n}"))).collect()
    }

    pub fn default_style(&self, family: &str) -> Option<&Element> {
        self.defaults.get(family)
    }

    pub fn run_props(&self, text_properties: &Element) -> RunProps {
        parse_text_properties(text_properties, &self.fonts).0
    }

    pub fn section_columns(&self, name: &str) -> Option<ColumnsBlock> {
        let el = self.raw.get(&format!("section:{name}"))?;
        let props = el.child("section-properties")?;
        let columns = props.child("columns")?;
        let count: usize = columns.attr("column-count").and_then(|v| v.parse().ok()).unwrap_or(1);
        let gap = columns.attr("column-gap").and_then(length).unwrap_or(0.0);
        let separator = columns.child("column-sep").map(|sep| ColumnSeparator {
            width: sep.attr("width").and_then(length).unwrap_or(0.25).max(0.25),
            color: sep.attr("color").and_then(Color::parse_hex).unwrap_or(Color(0, 0, 0)),
            height: sep
                .attr("height")
                .and_then(|v| v.trim_end_matches('%').parse::<f64>().ok())
                .map(|v| v / 100.0)
                .unwrap_or(1.0),
            valign: match sep.attr("vertical-align") {
                Some("bottom") => VAlign::Bottom,
                Some("middle") => VAlign::Center,
                _ => VAlign::Top,
            },
            dotted: matches!(sep.attr("style"), Some("dotted") | Some("dashed")),
        });
        Some(ColumnsBlock {
            count,
            gap,
            blocks: Vec::new(),
            separator,
            balanced: props.attr("dont-balance-text-columns") != Some("true"),
        })
    }

    fn raw_graphic(&self, name: &str) -> Option<&Element> {
        self.raw.get(&format!("graphic:{name}"))
            .or_else(|| self.raw.get(&format!("table:{name}")))
            .or_else(|| self.raw.get(&format!("table-column:{name}")))
            .or_else(|| self.raw.get(&format!("table-row:{name}")))
            .or_else(|| self.raw.get(&format!("table-cell:{name}")))
    }

    pub fn table(&self, name: &str) -> TableProps {
        let mut props = TableProps::default();
        if let Some(t) = self.raw.get(&format!("table:{name}")).and_then(|e| e.child("table-properties")) {
            props.indent = t.attr("margin-left").and_then(length).unwrap_or(0.0);
        }
        props
    }

    pub fn column_width(&self, name: &str) -> Option<f64> {
        self.raw
            .get(&format!("table-column:{name}"))
            .and_then(|e| e.child("table-column-properties"))
            .and_then(|p| p.attr("column-width"))
            .and_then(length)
    }

    pub fn row_height(&self, name: &str) -> (Option<f64>, bool) {
        let Some(p) = self.raw.get(&format!("table-row:{name}")).and_then(|e| e.child("table-row-properties")) else {
            return (None, false);
        };
        if let Some(h) = p.attr("row-height").and_then(length) {
            return (Some(h), true);
        }
        (p.attr("min-row-height").and_then(length), false)
    }

    pub fn cell(&self, name: &str) -> CellProps {
        let mut props = CellProps::default();
        let Some(p) = self.raw.get(&format!("table-cell:{name}")).and_then(|e| e.child("table-cell-properties")) else {
            return props;
        };
        let side = |value: Option<&str>| -> BorderSide {
            match value {
                None => BorderSide::Unset,
                Some("none") => BorderSide::None,
                Some(v) => parse_border(v).map(|(w, c, s)| BorderSide::Line { width: w, color: c, style: s }).unwrap_or(BorderSide::None),
            }
        };
        let all = side(p.attr("border"));
        props.borders = Borders { top: all, left: all, bottom: all, right: all, ..Borders::default() };
        for (name, slot) in [("border-top", &mut props.borders.top), ("border-left", &mut props.borders.left), ("border-bottom", &mut props.borders.bottom), ("border-right", &mut props.borders.right)] {
            let s = side(p.attr(name));
            if s != BorderSide::Unset {
                *slot = s;
            }
        }
        let pad = |n: &str| p.attr(n).and_then(length);
        let all_pad = pad("padding");
        props.margins = CellMargins {
            top: pad("padding-top").or(all_pad),
            left: pad("padding-left").or(all_pad),
            bottom: pad("padding-bottom").or(all_pad),
            right: pad("padding-right").or(all_pad),
        };
        props.fill = p.attr("background-color").filter(|c| *c != "transparent").and_then(Color::parse_hex);
        props.valign = match p.attr("vertical-align") {
            Some("middle") => VAlign::Center,
            Some("bottom") => VAlign::Bottom,
            _ => VAlign::Top,
        };
        props
    }
}

fn parse_border(value: &str) -> Option<(f64, Color, LineStyle)> {
    let mut width = 0.5;
    let mut color = Color(0, 0, 0);
    let mut style = LineStyle::Solid;
    for part in value.split_whitespace() {
        if part == "none" || part == "hidden" {
            return None;
        }
        if let Some(c) = Color::parse_hex(part) {
            color = c;
        } else if let Some(w) = length(part) {
            width = w.max(0.25);
        } else {
            style = LineStyle::from_name(part);
        }
    }
    Some((width, color, style))
}

fn parse_style(el: &Element, fonts: &HashMap<String, String>) -> Style {
    let mut style = Style {
        family: el.attr("family").unwrap_or("paragraph").to_string(),
        parent: el.attr("parent-style-name").map(str::to_owned),
        list_style: el.attr("list-style-name").filter(|l| !l.is_empty()).map(str::to_owned),
        master_page: el.attr("master-page-name").filter(|m| !m.is_empty()).map(str::to_owned),
        outline_level: el.attr("default-outline-level").and_then(|v| v.parse().ok()).filter(|v| *v > 0),
        ..Style::default()
    };
    if let Some(p) = el.child("paragraph-properties") {
        style.ppr = parse_paragraph_properties(p);
    }
    if let Some(t) = el.child("text-properties") {
        let (rpr, pct) = parse_text_properties(t, fonts);
        style.rpr = rpr;
        style.size_percent = pct;
    }
    style
}

fn parse_paragraph_properties(p: &Element) -> ParagraphProps {
    let mut props = ParagraphProps::default();
    props.align = match p.attr("text-align") {
        Some("center") => Some(Align::Center),
        Some("end") | Some("right") => Some(Align::Right),
        Some("justify") => Some(Align::Justify),
        Some("start") | Some("left") => Some(Align::Left),
        _ => None,
    };
    props.indent_left = p.attr("margin-left").and_then(length);
    props.indent_right = p.attr("margin-right").and_then(length);
    if let Some(indent) = p.attr("text-indent").and_then(length) {
        if indent < 0.0 {
            props.indent_hanging = Some(-indent);
            props.indent_first_line = Some(0.0);
        } else {
            props.indent_first_line = Some(indent);
            props.indent_hanging = Some(0.0);
        }
    }
    props.space_before = p.attr("margin-top").and_then(length);
    props.space_after = p.attr("margin-bottom").and_then(length);
    if let Some(lh) = p.attr("line-height") {
        if let Some(pct) = percent(lh) {
            props.line_spacing = Some(LineSpacing::Multiple(pct));
        } else if let Some(v) = length(lh) {
            props.line_spacing = Some(LineSpacing::Exact(v));
        } else if lh == "normal" {
            props.line_spacing = Some(LineSpacing::Multiple(1.0));
        }
    }
    if let Some(v) = p.attr("line-height-at-least").and_then(length) {
        props.line_spacing = Some(LineSpacing::AtLeast(v));
    }
    if p.attr("break-before") == Some("page") {
        props.page_break_before = Some(true);
    }
    if let Some(k) = p.attr("keep-with-next") {
        props.keep_next = Some(k == "always");
    }
    if let Some(c) = p.attr("contextual-spacing") {
        props.contextual_spacing = Some(c == "true");
    }
    let side = |value: Option<&str>| -> BorderSide {
        match value {
            None => BorderSide::Unset,
            Some("none") => BorderSide::None,
            Some(v) => parse_border(v).map(|(w, c, s)| BorderSide::Line { width: w, color: c, style: s }).unwrap_or(BorderSide::None),
        }
    };
    let all = side(p.attr("border"));
    props.borders = Borders { top: all, left: all, bottom: all, right: all, ..Borders::default() };
    for (name, slot) in [("border-top", &mut props.borders.top), ("border-left", &mut props.borders.left), ("border-bottom", &mut props.borders.bottom), ("border-right", &mut props.borders.right)] {
        let s = side(p.attr(name));
        if s != BorderSide::Unset {
            *slot = s;
        }
    }
    let pad = |n: &str| p.attr(n).and_then(length);
    let all_pad = pad("padding").unwrap_or(0.0);
    props.border_space = BorderSpace {
        top: pad("padding-top").unwrap_or(all_pad),
        left: pad("padding-left").unwrap_or(all_pad),
        bottom: pad("padding-bottom").unwrap_or(all_pad),
        right: pad("padding-right").unwrap_or(all_pad),
    };
    if let Some(c) = p.attr("background-color").filter(|c| *c != "transparent").and_then(Color::parse_hex) {
        props.shading = Some(c);
    }
    if let Some(tabs) = p.child("tab-stops") {
        for tab in tabs.children("tab-stop") {
            if let Some(pos) = tab.attr("position").and_then(length) {
                props.tabs.push(TabStop { pos, clear: false });
            }
        }
    }
    props
}

fn parse_text_properties(t: &Element, fonts: &HashMap<String, String>) -> (RunProps, Option<f64>) {
    let mut props = RunProps::default();
    let mut pct = None;
    if let Some(name) = t.attr("font-name") {
        props.font = Some(fonts.get(name).cloned().unwrap_or_else(|| name.to_string()));
    } else if let Some(family) = t.attr("font-family") {
        props.font = Some(family.trim_matches('\'').trim_matches('"').to_string());
    }
    if let Some(size) = t.attr("font-size") {
        if let Some(p) = percent(size) {
            pct = Some(p);
        } else if let Some(v) = length(size) {
            props.size = Some(v);
        }
    }
    if let Some(spacing) = t.attr("letter-spacing") {
        props.letter_spacing = if spacing == "normal" { Some(0.0) } else { length(spacing) };
    }
    if let Some(kerning) = t.attr("letter-kerning") {
        props.kerning = Some(kerning == "true");
    }
    if let Some(w) = t.attr("font-weight") {
        props.bold = Some(w == "bold" || w.parse::<u32>().map(|n| n >= 600).unwrap_or(false));
    }
    if let Some(s) = t.attr("font-style") {
        props.italic = Some(s == "italic" || s == "oblique");
    }
    if let Some(u) = t.attr("text-underline-style") {
        props.underline = Some(u != "none");
    }
    if let Some(s) = t.attr("text-line-through-style") {
        props.strike = Some(s != "none");
    }
    if let Some(c) = t.attr("color").and_then(Color::parse_hex) {
        props.color = Some(c);
    }
    if let Some(c) = t.attr("background-color").filter(|c| *c != "transparent").and_then(Color::parse_hex) {
        props.highlight = Some(c);
    }
    if let Some(pos) = t.attr("text-position") {
        let first = pos.split_whitespace().next().unwrap_or("");
        props.vertical = Some(if first == "super" || first.trim_end_matches('%').parse::<f64>().map(|v| v > 0.0).unwrap_or(false) {
            VerticalAlign::Superscript
        } else if first == "sub" || first.starts_with('-') {
            VerticalAlign::Subscript
        } else {
            VerticalAlign::Baseline
        });
    }
    if t.attr("font-variant") == Some("small-caps") {
        props.small_caps = Some(true);
    }
    if t.attr("text-transform") == Some("uppercase") {
        props.caps = Some(true);
    }
    if t.attr("display") == Some("none") {
        props.hidden = Some(true);
    }
    (props, pct)
}

fn parse_list(el: &Element, fonts: &HashMap<String, String>) -> Vec<Option<ListLevel>> {
    let mut levels: Vec<Option<ListLevel>> = vec![None; 10];
    for lvl in el.elements() {
        let Some(level) = lvl.attr("level").and_then(|v| v.parse::<usize>().ok()) else { continue };
        if level == 0 || level > 10 {
            continue;
        }
        let kind = match lvl.name.as_str() {
            "list-level-style-bullet" => LevelKind::Bullet(
                lvl.attr("bullet-char").map(|c| crate::pptx::map_bullet_pub(c)).unwrap_or_else(|| "•".into()),
            ),
            "list-level-style-number" | "outline-level-style" => {
                let format = match lvl.attr("num-format") {
                    Some("1") => "decimal",
                    Some("a") => "lowerLetter",
                    Some("A") => "upperLetter",
                    Some("i") => "lowerRoman",
                    Some("I") => "upperRoman",
                    Some("") | None => "none",
                    _ => "decimal",
                };
                if format == "none" {
                    LevelKind::None
                } else {
                    LevelKind::Number {
                        format: format.to_string(),
                        prefix: lvl.attr("num-prefix").unwrap_or("").to_string(),
                        suffix: lvl.attr("num-suffix").unwrap_or("").to_string(),
                        start: lvl.attr("start-value").and_then(|v| v.parse().ok()).unwrap_or(1),
                        display_levels: lvl.attr("display-levels").and_then(|v| v.parse().ok()).unwrap_or(1),
                    }
                }
            }
            "list-level-style-image" => LevelKind::Bullet("•".into()),
            _ => continue,
        };
        let mut left = None;
        let mut hanging = None;
        let mut suffix = ListSuffix::Tab;
        let mut tab_pos = None;
        if let Some(p) = lvl.child("list-level-properties") {
            if let Some(align) = p.child("list-level-label-alignment") {
                left = align.attr("margin-left").and_then(length);
                tab_pos = align.attr("list-tab-stop-position").and_then(length);
                let indent = align.attr("text-indent").and_then(length).unwrap_or(0.0);
                hanging = Some((-indent).max(0.0));
                suffix = match align.attr("label-followed-by") {
                    Some("space") => ListSuffix::Space,
                    Some("nothing") => ListSuffix::Nothing,
                    _ => ListSuffix::Tab,
                };
            } else {
                let space = p.attr("space-before").and_then(length).unwrap_or(0.0);
                let min_label = p.attr("min-label-width").and_then(length).unwrap_or(0.0);
                left = Some(space + min_label);
                hanging = Some(min_label);
            }
        }
        let rpr = lvl.child("text-properties").map(|t| parse_text_properties(t, fonts).0);
        levels[level - 1] = Some(ListLevel { kind, left, hanging, suffix, rpr, tab_pos });
    }
    levels
}

fn parse_page_layout(el: &Element) -> PageLayout {
    let mut page = PageSetup::default();
    let mut columns = 1;
    let mut gap = 0.0;
    if let Some(p) = el.child("page-layout-properties") {
        if let Some(w) = p.attr("page-width").and_then(length) {
            page.width = w;
        }
        if let Some(h) = p.attr("page-height").and_then(length) {
            page.height = h;
        }
        let m = |n: &str| p.attr(n).and_then(length);
        if let Some(v) = m("margin-top") {
            page.margin.top = v;
            page.margin.header = v;
        }
        if let Some(v) = m("margin-bottom") {
            page.margin.bottom = v;
            page.margin.footer = v;
        }
        if let Some(v) = m("margin-left") {
            page.margin.left = v;
        }
        if let Some(v) = m("margin-right") {
            page.margin.right = v;
        }
        if let Some(cols) = p.child("columns") {
            columns = cols.attr("column-count").and_then(|c| c.parse().ok()).unwrap_or(1);
            gap = cols.attr("column-gap").and_then(length).unwrap_or(0.0);
        }
    }
    for (name, header) in [("header-style", true), ("footer-style", false)] {
        let Some(props) = el.child(name).and_then(|h| h.child("header-footer-properties")) else { continue };
        let explicit = props.attr("min-height").and_then(length).or_else(|| props.attr("height").and_then(length));
        if header {
            let spacing = props.attr("margin-bottom").and_then(length).unwrap_or(0.0);
            page.margin.top = page.margin.header + explicit.unwrap_or(14.0 + spacing);
        } else {
            let spacing = props.attr("margin-top").and_then(length).unwrap_or(0.0);
            page.margin.bottom = page.margin.footer + explicit.unwrap_or(14.0 + spacing);
        }
    }
    PageLayout { page, columns, column_gap: gap }
}
