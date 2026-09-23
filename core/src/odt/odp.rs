use super::styles::{length, GraphicFill, GraphicProps, Styles};
use super::{base64_decode, entry, Reader};
use crate::error::Error;
use crate::model::*;
use crate::xml::{self, Element};
use std::collections::HashMap;
use std::io::{Cursor, Read};

struct Package {
    content: Element,
    styles: Option<Element>,
    media: HashMap<String, ImageData>,
    objects: HashMap<String, Element>,
}

fn package(bytes: &[u8]) -> Result<Package, Error> {
    if !bytes.starts_with(b"PK") {
        let root = xml::parse(bytes)?;
        return Ok(Package { styles: Some(root.clone()), content: root, media: HashMap::new(), objects: HashMap::new() });
    }
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| Error::new(format!("not an odp container: {e}")))?;
    let content = entry(&mut archive, "content.xml")?.ok_or_else(|| Error::new("odp has no content.xml"))?;
    let styles = entry(&mut archive, "styles.xml")?;
    let mut media = HashMap::new();
    let mut objects = HashMap::new();
    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let name = file.name().to_string();
        if file.is_dir() {
            continue;
        }
        if name.starts_with("Pictures/") || name.starts_with("media/") {
            let mut data = Vec::new();
            file.read_to_end(&mut data)?;
            if let Some(format) = ImageFormat::sniff(&data) {
                media.insert(name, ImageData { data, format });
            }
        } else if name.starts_with("Object ") && name.ends_with("/content.xml") {
            let mut data = Vec::new();
            file.read_to_end(&mut data)?;
            if let Ok(root) = xml::parse(&data) {
                objects.insert(name.trim_end_matches("/content.xml").to_string(), root);
            }
        }
    }
    Ok(Package { content: xml::parse(&content)?, styles: styles.as_deref().map(xml::parse).transpose()?, media, objects })
}

pub fn read(bytes: &[u8]) -> Result<Document, Error> {
    let pkg = package(bytes)?;
    let styles = Styles::parse(pkg.styles.as_ref(), &pkg.content);
    let presentation = pkg
        .content
        .child("body")
        .and_then(|b| b.child("presentation"))
        .ok_or_else(|| Error::new("odp has no office:presentation body"))?;
    let reader = Reader {
        frame_base: std::cell::RefCell::new(None),
        styles: &styles,
        media: &pkg.media,
        counters: std::cell::RefCell::new(HashMap::new()),
        list_keys: std::cell::RefCell::new(HashMap::new()),
        style_keys: std::cell::RefCell::new(HashMap::new()),
        next_key: std::cell::Cell::new(0),
    };
    let slides = Slides { reader: &reader, styles: &styles, pkg: &pkg };
    let mut doc = Document { default_tab: styles.default_tab, fixed_line_metrics: true, ..Document::default() };
    for page in presentation.children("page") {
        doc.sections.push(slides.page(page));
    }
    if doc.sections.is_empty() {
        doc.sections.push(Section { blocks: vec![Block::Paragraph(Paragraph::default())], columns: 1, ..Section::default() });
    }
    Ok(doc)
}

struct Slides<'a> {
    reader: &'a Reader<'a>,
    styles: &'a Styles,
    pkg: &'a Package,
}

fn attr_len(el: &Element, name: &str) -> Option<f64> {
    el.attr(name).and_then(length)
}

fn drawing_page_fill(styles: &Styles, name: Option<&str>) -> Option<Option<Color>> {
    let mut result = None;
    for el in styles.elements("drawing-page", name?) {
        let Some(p) = el.child("drawing-page-properties") else { continue };
        match p.attr("fill") {
            Some("none") => result = Some(None),
            Some("solid") => result = Some(p.attr("fill-color").and_then(Color::parse_hex)),
            _ => {
                if let Some(c) = p.attr("fill-color").and_then(Color::parse_hex) {
                    if p.attr("fill").is_none() {
                        result = Some(Some(c));
                    }
                }
            }
        }
    }
    result
}

fn drawing_page_flag(styles: &Styles, name: Option<&str>, attr: &str) -> Option<bool> {
    let name = name?;
    styles.elements("drawing-page", name).into_iter().rev().find_map(|el| el.child("drawing-page-properties")?.attr(attr)).map(|v| v == "true")
}

impl Slides<'_> {
    fn page(&self, page: &Element) -> Section {
        let master_name = page.attr("master-page-name").unwrap_or("");
        let master = self.styles.master_page_elements.get(master_name);
        let layout = master
            .and_then(|m| m.attr("page-layout-name"))
            .and_then(|l| self.styles.page_layout_elements.get(l))
            .and_then(|l| l.child("page-layout-properties"));
        let width = layout.and_then(|l| attr_len(l, "page-width")).unwrap_or(28.0 * 72.0 / 2.54);
        let height = layout.and_then(|l| attr_len(l, "page-height")).unwrap_or(15.75 * 72.0 / 2.54);

        let mut anchors = Vec::new();
        let page_style = page.attr("style-name");
        let master_style = master.and_then(|m| m.attr("style-name"));
        let background = drawing_page_fill(self.styles, page_style).or_else(|| drawing_page_fill(self.styles, master_style)).flatten();
        if let Some(color) = background.filter(|c| *c != Color(255, 255, 255)) {
            let rect = TextBox { fill: Some(color), shape: ShapeKind::Rect, ..TextBox::default() };
            anchors.push(crate::pptx::page_anchor(0.0, 0.0, width, height, DrawingContent::TextBox(rect), 0.0, false, false, true));
        }
        let show_master = drawing_page_flag(self.styles, page_style, "background-objects-visible").unwrap_or(true);
        if let (Some(master), true) = (master, show_master) {
            for el in master.elements() {
                self.shape(el, true, &mut anchors);
            }
        }
        for el in page.elements() {
            self.shape(el, false, &mut anchors);
        }
        Section {
            page: PageSetup { width, height, margin: Margins { top: 0.0, right: 0.0, bottom: 0.0, left: 0.0, header: 0.0, footer: 0.0 } },
            blocks: vec![Block::Paragraph(Paragraph { mark: RunProps { size: Some(1.0), ..RunProps::default() }, anchors, ..Paragraph::default() })],
            columns: 1,
            column_gap: 0.0,
            ..Section::default()
        }
    }

    fn style_of<'e>(&self, el: &'e Element) -> Option<(&'static str, &'e str)> {
        el.attr("presentation:style-name")
            .map(|s| ("presentation", s))
            .or_else(|| el.attr("draw:style-name").map(|s| ("graphic", s)))
            .or_else(|| el.attr("style-name").map(|s| ("graphic", s)))
    }

    fn graphic(&self, el: &Element) -> GraphicProps {
        let mut props = GraphicProps { opacity: 1.0, ..GraphicProps::default() };
        if let Some((family, name)) = self.style_of(el) {
            props = self.styles.graphic_with_default(family, name);
        }
        props
    }

    fn graphic_attr(&self, el: &Element, attr: &str) -> Option<String> {
        let (family, name) = self.style_of(el)?;
        let mut value = self.styles.default_style("graphic").and_then(|d| d.child("graphic-properties")).and_then(|g| g.attr(attr)).map(str::to_owned);
        for style in self.styles.elements(family, name) {
            if let Some(v) = style.child("graphic-properties").and_then(|g| g.attr(attr)) {
                value = Some(v.to_string());
            }
        }
        value
    }

    fn text_base(&self, el: &Element) -> (ParagraphProps, RunProps) {
        let (mut ppr, mut rpr) = self.styles.default_text("graphic");
        if rpr.size.is_none() {
            rpr.size = Some(18.0);
        }
        if rpr.font.is_none() {
            rpr.font = Some("Liberation Sans".into());
        }
        if let Some((family, name)) = self.style_of(el) {
            let (p, r) = self.styles.family_text(family, name, rpr.size.unwrap_or(18.0));
            ppr.merge(&p);
            rpr.merge(&r);
        }
        rpr.kerning = Some(true);
        (ppr, rpr)
    }

    fn text_blocks(&self, el: &Element, container: &Element) -> Vec<Block> {
        let base = self.text_base(el);
        *self.reader.frame_base.borrow_mut() = Some(base);
        let blocks = self.reader.blocks(container);
        *self.reader.frame_base.borrow_mut() = None;
        blocks
    }

    fn fill_style(&self, graphic: &GraphicProps) -> Option<FillStyle> {
        match &graphic.fill_style {
            Some(GraphicFill::Gradient { start, end, angle, radial }) => Some(FillStyle::Gradient { start: *start, end: *end, angle: *angle, radial: *radial }),
            Some(GraphicFill::Hatch { color, distance, angle, background }) => Some(FillStyle::Hatch { color: *color, distance: *distance, angle: *angle, background: *background }),
            Some(GraphicFill::ImageRef { href, repeat }) => self.pkg.media.get(href.as_str()).map(|data| FillStyle::Image { data: data.clone(), repeat: *repeat }),
            None => None,
        }
    }

    fn valign(&self, el: &Element) -> VAlign {
        match self.graphic_attr(el, "textarea-vertical-align").as_deref() {
            Some("middle") => VAlign::Center,
            Some("bottom") => VAlign::Bottom,
            _ => VAlign::Top,
        }
    }

    fn geometry(&self, el: &Element) -> Option<(f64, f64, f64, f64, f64)> {
        let w = attr_len(el, "width").unwrap_or(0.0);
        let h = attr_len(el, "height").unwrap_or(0.0);
        if let Some(transform) = el.attr("transform") {
            let (angle, tx, ty) = parse_transform(transform);
            let (cx, cy) = (w / 2.0, h / 2.0);
            let (sin, cos) = (-angle).sin_cos();
            let center = (tx + cx * cos - cy * sin, ty + cx * sin + cy * cos);
            return Some((center.0 - cx, center.1 - cy, w, h, angle.to_degrees()));
        }
        Some((attr_len(el, "x").unwrap_or(0.0), attr_len(el, "y").unwrap_or(0.0), w, h, 0.0))
    }

    fn shape(&self, el: &Element, master: bool, out: &mut Vec<Anchor>) {
        match el.name.as_str() {
            "g" => {
                for child in el.elements() {
                    self.shape(child, master, out);
                }
                return;
            }
            "notes" | "forms" | "page-thumbnail" | "animations" | "anim" | "par" | "seq" => return,
            _ => {}
        }
        if master && el.attr("class").is_some() {
            return;
        }
        if el.attr("placeholder") == Some("true") {
            return;
        }
        let Some((x, y, w, h, rot)) = (match el.name.as_str() {
            "line" | "connector" => None,
            _ => self.geometry(el),
        })
        .or_else(|| self.line_geometry(el)) else {
            return;
        };
        let Some(content) = self.content(el, w, h) else { return };
        let geometry = el.child("enhanced-geometry");
        let flip_h = geometry.and_then(|g| g.attr("mirror-horizontal")) == Some("true");
        let flip_v = geometry.and_then(|g| g.attr("mirror-vertical")) == Some("true");
        out.push(crate::pptx::page_anchor(x, y, w, h, content, -rot, flip_h, flip_v, master));
    }

    fn line_geometry(&self, el: &Element) -> Option<(f64, f64, f64, f64, f64)> {
        let (x1, y1) = (attr_len(el, "x1")?, attr_len(el, "y1")?);
        let (x2, y2) = (attr_len(el, "x2")?, attr_len(el, "y2")?);
        Some((x1.min(x2), y1.min(y2), (x2 - x1).abs(), (y2 - y1).abs(), 0.0))
    }

    fn content(&self, el: &Element, w: f64, h: f64) -> Option<DrawingContent> {
        let graphic = self.graphic(el);
        match el.name.as_str() {
            "frame" => {
                if let Some(table) = el.child("table") {
                    *self.reader.frame_base.borrow_mut() = Some(self.text_base(el));
                    let table = self.reader.table(table);
                    *self.reader.frame_base.borrow_mut() = None;
                    return Some(DrawingContent::Table(table));
                }
                if let Some(object) = el.child("object") {
                    let href = object.attr("href").unwrap_or("").trim_start_matches("./").trim_end_matches('/');
                    if let Some(root) = self.pkg.objects.get(href) {
                        return Some(crate::odf_chart::chart_content(root));
                    }
                }
                if let Some(image) = el.child("image") {
                    let data = image
                        .attr("href")
                        .and_then(|href| self.pkg.media.get(href.trim_start_matches("./")))
                        .cloned()
                        .or_else(|| {
                            image
                                .child("binary-data")
                                .and_then(|b| base64_decode(&b.text()))
                                .and_then(|data| ImageFormat::sniff(&data).map(|format| ImageData { data, format }))
                        });
                    if let Some(data) = data {
                        return Some(DrawingContent::Image(data));
                    }
                }
                if el.child("object").is_some() {
                    return Some(DrawingContent::Placeholder(None));
                }
                let text_box = el.child("text-box")?;
                let blocks = self.text_blocks(el, text_box);
                Some(DrawingContent::TextBox(TextBox {
                    blocks,
                    fill: graphic.fill,
                    fill_style: self.fill_style(&graphic),
                    opacity: graphic.opacity,
                    stroke: graphic.stroke,
                    inset: graphic.padding,
                    valign: self.valign(el),
                    shape: ShapeKind::Rect,
                    ..TextBox::default()
                }))
            }
            "polygon" | "polyline" | "path" => {
                let view: Vec<f64> = el.attr("viewBox").unwrap_or("0 0 1 1").split_whitespace().filter_map(|v| v.parse().ok()).collect();
                let (vx, vy) = (view.first().copied().unwrap_or(0.0), view.get(1).copied().unwrap_or(0.0));
                let (vw, vh) = (view.get(2).copied().unwrap_or(1.0).max(1e-9), view.get(3).copied().unwrap_or(1.0).max(1e-9));
                let norm = |x: f64, y: f64| ((x - vx) / vw, (y - vy) / vh);
                let commands = if el.name == "path" {
                    super::geometry::svg_path(el.attr("d")?, &norm)
                } else {
                    let points: Vec<(f64, f64)> = el
                        .attr("points")?
                        .split_whitespace()
                        .filter_map(|pair| {
                            let (x, y) = pair.split_once(',')?;
                            Some(norm(x.parse().ok()?, y.parse().ok()?))
                        })
                        .collect();
                    let mut commands: Vec<PathCommand> = points
                        .iter()
                        .enumerate()
                        .map(|(i, &(x, y))| if i == 0 { PathCommand::Move(x, y) } else { PathCommand::Line(x, y) })
                        .collect();
                    if el.name == "polygon" {
                        commands.push(PathCommand::Close);
                    }
                    commands
                };
                if commands.is_empty() {
                    return None;
                }
                let open = el.name == "polyline" || !commands.iter().any(|c| matches!(c, PathCommand::Close));
                let blocks = if el.children("p").next().is_some() { self.text_blocks(el, el) } else { Vec::new() };
                Some(DrawingContent::TextBox(TextBox {
                    blocks,
                    fill: if open { None } else { graphic.fill },
                    fill_style: if open { None } else { self.fill_style(&graphic) },
                    opacity: graphic.opacity,
                    stroke: graphic.stroke,
                    inset: graphic.padding,
                    valign: self.valign(el),
                    shape: ShapeKind::Path(commands),
                    ..TextBox::default()
                }))
            }
            "custom-shape" | "rect" | "ellipse" | "circle" | "line" | "connector" => {
                let fontwork = el.child("enhanced-geometry").and_then(|g| g.attr("text-path")) == Some("true");
                let kind = match el.name.as_str() {
                    _ if fontwork => ShapeKind::Rect,
                    "ellipse" | "circle" => ShapeKind::Ellipse,
                    "line" | "connector" => ShapeKind::Line,
                    "rect" => match attr_len(el, "corner-radius") {
                        Some(r) if r > 0.0 => ShapeKind::RoundRect((r / w.min(h).max(1.0)).min(0.5)),
                        _ => ShapeKind::Rect,
                    },
                    _ => custom_kind(el, w, h),
                };
                let blocks = if el.children("p").next().is_some() || el.children("list").next().is_some() { self.text_blocks(el, el) } else { Vec::new() };
                Some(DrawingContent::TextBox(TextBox {
                    blocks,
                    fill: if kind == ShapeKind::Line || fontwork { None } else { graphic.fill },
                    fill_style: if fontwork { None } else { self.fill_style(&graphic) },
                    opacity: graphic.opacity,
                    stroke: if fontwork { None } else { graphic.stroke },
                    inset: graphic.padding,
                    valign: self.valign(el),
                    shape: kind,
                    ..TextBox::default()
                }))
            }
            _ => None,
        }
    }
}

fn custom_kind(el: &Element, w: f64, h: f64) -> ShapeKind {
    let geometry = el.child("enhanced-geometry");
    match geometry.and_then(|g| g.attr("type")).unwrap_or("rectangle") {
        "rectangle" | "ooxml-rect" | "mso-spt1" | "frame" => ShapeKind::Rect,
        "ellipse" | "circle" | "ring" => ShapeKind::Ellipse,
        "round-rectangle" | "round-square" => ShapeKind::RoundRect(0.1667),
        "diamond" => ShapeKind::Polygon(vec![(0.5, 0.0), (1.0, 0.5), (0.5, 1.0), (0.0, 0.5)]),
        "isosceles-triangle" => ShapeKind::Polygon(vec![(0.5, 0.0), (1.0, 1.0), (0.0, 1.0)]),
        "right-triangle" => ShapeKind::Polygon(vec![(0.0, 0.0), (1.0, 1.0), (0.0, 1.0)]),
        "parallelogram" => ShapeKind::Polygon(vec![(0.25, 0.0), (1.0, 0.0), (0.75, 1.0), (0.0, 1.0)]),
        "trapezoid" => ShapeKind::Polygon(vec![(0.0, 0.0), (1.0, 0.0), (0.75, 1.0), (0.25, 1.0)]),
        "pentagon" => ShapeKind::Polygon(vec![(0.5, 0.0), (1.0, 0.38), (0.81, 1.0), (0.19, 1.0), (0.0, 0.38)]),
        "hexagon" => ShapeKind::Polygon(vec![(0.25, 0.0), (0.75, 0.0), (1.0, 0.5), (0.75, 1.0), (0.25, 1.0), (0.0, 0.5)]),
        "octagon" => ShapeKind::Polygon(vec![(0.29, 0.0), (0.71, 0.0), (1.0, 0.29), (1.0, 0.71), (0.71, 1.0), (0.29, 1.0), (0.0, 0.71), (0.0, 0.29)]),
        "right-arrow" => ShapeKind::Polygon(vec![(0.0, 0.25), (0.75, 0.25), (0.75, 0.0), (1.0, 0.5), (0.75, 1.0), (0.75, 0.75), (0.0, 0.75)]),
        "left-arrow" => ShapeKind::Polygon(vec![(1.0, 0.25), (0.25, 0.25), (0.25, 0.0), (0.0, 0.5), (0.25, 1.0), (0.25, 0.75), (1.0, 0.75)]),
        "up-arrow" => ShapeKind::Polygon(vec![(0.25, 1.0), (0.25, 0.25), (0.0, 0.25), (0.5, 0.0), (1.0, 0.25), (0.75, 0.25), (0.75, 1.0)]),
        "down-arrow" => ShapeKind::Polygon(vec![(0.25, 0.0), (0.75, 0.0), (0.75, 0.75), (1.0, 0.75), (0.5, 1.0), (0.0, 0.75), (0.25, 0.75)]),
        "star5" => ShapeKind::Polygon(vec![
            (0.5, 0.0),
            (0.62, 0.38),
            (1.0, 0.38),
            (0.69, 0.62),
            (0.81, 1.0),
            (0.5, 0.76),
            (0.19, 1.0),
            (0.31, 0.62),
            (0.0, 0.38),
            (0.38, 0.38),
        ]),
        _ => geometry.and_then(|g| super::geometry::enhanced_path(g, w, h)).map(ShapeKind::Path).unwrap_or(ShapeKind::Rect),
    }
}

fn parse_transform(text: &str) -> (f64, f64, f64) {
    let mut angle = 0.0;
    let (mut tx, mut ty) = (0.0, 0.0);
    let mut rest = text;
    while let Some(open) = rest.find('(') {
        let name = rest[..open].trim();
        let Some(close) = rest[open..].find(')') else { break };
        let args: Vec<&str> = rest[open + 1..open + close].split(|c: char| c.is_whitespace() || c == ',').filter(|a| !a.is_empty()).collect();
        match name {
            "rotate" => angle = args.first().and_then(|a| a.parse::<f64>().ok()).unwrap_or(0.0),
            "translate" => {
                tx = args.first().and_then(|a| length(a)).unwrap_or(0.0);
                ty = args.get(1).and_then(|a| length(a)).unwrap_or(0.0);
            }
            _ => {}
        }
        rest = &rest[open + close + 1..];
    }
    (angle, tx, ty)
}
