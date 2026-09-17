pub(crate) mod color;
pub(crate) mod text;

pub(crate) fn map_bullet_pub(text: &str) -> String {
    text::map_bullet(text)
}

use crate::error::Error;
use crate::model::*;
use crate::xml::{self, Element};
use color::{parse_clr_map, resolve_child, ColorContext, Theme};
use std::collections::HashMap;
use std::io::{Cursor, Read};
use text::{emu, text_blocks, LevelStyles, TextContext};

const NO_STYLE_TABLE_GRID: &str = "{5940675A-B579-460E-94D1-54222C63F5DA}";
const NO_STYLE_NO_GRID: &str = "{2D5ABB26-0587-4C30-8999-92F81FD0307C}";

pub fn read(bytes: &[u8]) -> Result<Document, Error> {
    let mut package = Package::open(bytes)?;
    let presentation = package
        .xml("ppt/presentation.xml")?
        .ok_or_else(|| Error::new("pptx has no ppt/presentation.xml"))?;

    let size = presentation.child("sldSz");
    let width = size.and_then(|s| s.attr("cx")).and_then(emu).unwrap_or(720.0);
    let height = size.and_then(|s| s.attr("cy")).and_then(emu).unwrap_or(540.0);

    let pres_rels = package.rels("ppt/presentation.xml")?;
    let slide_ids: Vec<String> = presentation
        .child("sldIdLst")
        .map(|l| l.children("sldId").filter_map(|s| s.attr("r:id").map(str::to_owned)).collect())
        .unwrap_or_default();

    let default_theme = Theme::default();
    let default_colors = ColorContext {
        theme: &default_theme,
        clr_map: &color::default_clr_map(),
        placeholder: None,
    };
    let default_text = presentation
        .child("defaultTextStyle")
        .map(|d| LevelStyles::parse(d, &default_theme, &default_colors))
        .unwrap_or_default();

    let mut doc = Document {
        default_tab: 72.0,
        fixed_line_metrics: true,
        ..Document::default()
    };

    let mut number = 0;
    for id in slide_ids {
        let Some((_, target)) = pres_rels.get(&id) else { continue };
        let Some(slide) = package.xml(target)? else { continue };
        if slide.attr("show") == Some("0") {
            continue;
        }
        number += 1;
        let section = render_slide(&mut package, target, &slide, number, width, height, &default_text)?;
        doc.sections.push(section);
    }
    if doc.sections.is_empty() {
        return Err(Error::new("pptx has no visible slides"));
    }
    Ok(doc)
}

struct Package<'a> {
    archive: zip::ZipArchive<Cursor<&'a [u8]>>,
}

impl<'a> Package<'a> {
    fn open(bytes: &'a [u8]) -> Result<Self, Error> {
        Ok(Package {
            archive: zip::ZipArchive::new(Cursor::new(bytes))
                .map_err(|e| Error::new(format!("not a pptx container: {e}")))?,
        })
    }

    fn read(&mut self, path: &str) -> Result<Option<Vec<u8>>, Error> {
        match self.archive.by_name(path) {
            Ok(mut file) => {
                let mut data = Vec::new();
                file.read_to_end(&mut data)?;
                Ok(Some(data))
            }
            Err(zip::result::ZipError::FileNotFound) => Ok(None),
            Err(e) => Err(Error::new(format!("reading {path}: {e}"))),
        }
    }

    fn xml(&mut self, path: &str) -> Result<Option<Element>, Error> {
        self.read(path)?.as_deref().map(xml::parse).transpose()
    }

    fn rels(&mut self, part: &str) -> Result<HashMap<String, (String, String)>, Error> {
        let (dir, file) = part.rsplit_once('/').unwrap_or(("", part));
        let rels_path = if dir.is_empty() {
            format!("_rels/{file}.rels")
        } else {
            format!("{dir}/_rels/{file}.rels")
        };
        let mut map = HashMap::new();
        if let Some(rels) = self.xml(&rels_path)? {
            for rel in rels.children("Relationship") {
                let (Some(id), Some(kind), Some(target)) = (rel.attr("Id"), rel.attr("Type"), rel.attr("Target")) else {
                    continue;
                };
                if rel.attr("TargetMode") == Some("External") {
                    continue;
                }
                let kind = kind.rsplit('/').next().unwrap_or(kind).to_string();
                map.insert(id.to_string(), (kind, resolve_path(dir, target)));
            }
        }
        Ok(map)
    }

    fn media(&mut self, rels: &HashMap<String, (String, String)>) -> Result<HashMap<String, ImageData>, Error> {
        let mut media = HashMap::new();
        for (id, (kind, target)) in rels {
            if kind != "image" {
                continue;
            }
            if let Some(data) = self.read(target)? {
                if let Some(format) = ImageFormat::sniff(&data) {
                    media.insert(id.clone(), ImageData { data, format });
                }
            }
        }
        Ok(media)
    }
}

pub(crate) fn resolve_path(dir: &str, target: &str) -> String {
    if let Some(abs) = target.strip_prefix('/') {
        return abs.to_string();
    }
    let mut parts: Vec<&str> = if dir.is_empty() { Vec::new() } else { dir.split('/').collect() };
    for segment in target.split('/') {
        match segment {
            "." | "" => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

#[derive(Debug, Clone, Copy, Default)]
struct Xfrm {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    rot: f64,
    flip_h: bool,
    flip_v: bool,
}

fn parse_xfrm(props: Option<&Element>) -> Option<Xfrm> {
    let xfrm = props?.child("xfrm")?;
    let off = xfrm.child("off")?;
    let ext = xfrm.child("ext")?;
    Some(Xfrm {
        x: off.attr("x").and_then(emu)?,
        y: off.attr("y").and_then(emu)?,
        w: ext.attr("cx").and_then(emu)?,
        h: ext.attr("cy").and_then(emu)?,
        rot: xfrm
            .attr("rot")
            .and_then(|r| r.parse::<f64>().ok())
            .map(|r| r / 60000.0)
            .unwrap_or(0.0),
        flip_h: xfrm.attr("flipH") == Some("1"),
        flip_v: xfrm.attr("flipV") == Some("1"),
    })
}

#[derive(Debug, Clone, Copy)]
struct Transform {
    ox: f64,
    oy: f64,
    cx: f64,
    cy: f64,
    sx: f64,
    sy: f64,
}

impl Transform {
    fn identity() -> Self {
        Transform { ox: 0.0, oy: 0.0, cx: 0.0, cy: 0.0, sx: 1.0, sy: 1.0 }
    }

    fn apply(&self, x: Xfrm) -> Xfrm {
        Xfrm {
            x: self.ox + (x.x - self.cx) * self.sx,
            y: self.oy + (x.y - self.cy) * self.sy,
            w: x.w * self.sx,
            h: x.h * self.sy,
            ..x
        }
    }

    fn child(&self, group: &Element) -> Transform {
        let Some(xfrm) = group.child("grpSpPr").and_then(|g| g.child("xfrm")) else { return *self };
        let get = |name: &str, attr: &str| xfrm.child(name).and_then(|e| e.attr(attr)).and_then(emu);
        let (Some(ox), Some(oy), Some(ex), Some(ey)) = (get("off", "x"), get("off", "y"), get("ext", "cx"), get("ext", "cy")) else {
            return *self;
        };
        let chx = get("chOff", "x").unwrap_or(0.0);
        let chy = get("chOff", "y").unwrap_or(0.0);
        let chw = get("chExt", "cx").unwrap_or(0.0);
        let chh = get("chExt", "cy").unwrap_or(0.0);
        let placed = self.apply(Xfrm { x: ox, y: oy, w: ex, h: ey, ..Xfrm::default() });
        Transform {
            ox: placed.x,
            oy: placed.y,
            cx: chx,
            cy: chy,
            sx: if chw > 0.0 && ex > 0.0 { placed.w / chw } else { self.sx },
            sy: if chh > 0.0 && ey > 0.0 { placed.h / chh } else { self.sy },
        }
    }
}

#[derive(Debug, Clone)]
struct Placeholder {
    kind: String,
    idx: Option<String>,
    xfrm: Option<Xfrm>,
    styles: LevelStyles,
    body: Option<Element>,
}

struct MasterPart {
    theme: Theme,
    clr_map: HashMap<String, String>,
    title_style: LevelStyles,
    body_style: LevelStyles,
    other_style: LevelStyles,
    placeholders: Vec<Placeholder>,
    shapes: Vec<Anchor>,
    background: Option<Anchor>,
}

struct LayoutPart {
    placeholders: Vec<Placeholder>,
    shapes: Vec<Anchor>,
    background: Option<Anchor>,
    show_master_shapes: bool,
}

struct SlideCtx<'a> {
    theme: &'a Theme,
    colors: ColorContext<'a>,
    media: &'a HashMap<String, ImageData>,
    default_text: &'a LevelStyles,
    title_style: &'a LevelStyles,
    body_style: &'a LevelStyles,
    other_style: &'a LevelStyles,
    layout_placeholders: &'a [Placeholder],
    master_placeholders: &'a [Placeholder],
    slide_number: usize,
    page_width: f64,
    page_height: f64,
}

fn render_slide(
    package: &mut Package,
    slide_path: &str,
    slide: &Element,
    number: usize,
    width: f64,
    height: f64,
    default_text: &LevelStyles,
) -> Result<Section, Error> {
    let slide_rels = package.rels(slide_path)?;
    let layout_path = slide_rels
        .values()
        .find(|(kind, _)| kind == "slideLayout")
        .map(|(_, t)| t.clone());
    let layout_xml = match &layout_path {
        Some(p) => package.xml(p)?,
        None => None,
    };
    let layout_rels = match &layout_path {
        Some(p) => package.rels(p)?,
        None => HashMap::new(),
    };
    let master_path = layout_rels
        .values()
        .find(|(kind, _)| kind == "slideMaster")
        .map(|(_, t)| t.clone());
    let master_xml = match &master_path {
        Some(p) => package.xml(p)?,
        None => None,
    };
    let master_rels = match &master_path {
        Some(p) => package.rels(p)?,
        None => HashMap::new(),
    };
    let theme_path = master_rels
        .values()
        .find(|(kind, _)| kind == "theme")
        .map(|(_, t)| t.clone());
    let theme = match &theme_path {
        Some(p) => package.xml(p)?.map(|t| Theme::parse(&t)).unwrap_or_default(),
        None => Theme::default(),
    };

    let master_media = package.media(&master_rels)?;
    let layout_media = package.media(&layout_rels)?;
    let slide_media = package.media(&slide_rels)?;

    let master = master_xml
        .as_ref()
        .map(|m| parse_master(m, &theme, &master_media, default_text, number, width, height));
    let empty_levels = LevelStyles::default();
    let (clr_map, title_style, body_style, other_style, master_placeholders, master_shapes, master_bg) = match &master {
        Some(m) => (
            m.clr_map.clone(),
            &m.title_style,
            &m.body_style,
            &m.other_style,
            m.placeholders.as_slice(),
            m.shapes.as_slice(),
            m.background.clone(),
        ),
        None => (
            color::default_clr_map(),
            &empty_levels,
            &empty_levels,
            &empty_levels,
            &[][..],
            &[][..],
            None,
        ),
    };

    let slide_clr_map = slide
        .child("clrMapOvr")
        .and_then(|o| o.child("overrideClrMapping"))
        .map(parse_clr_map)
        .unwrap_or(clr_map);
    let colors = ColorContext {
        theme: &theme,
        clr_map: &slide_clr_map,
        placeholder: None,
    };

    let layout = layout_xml.as_ref().map(|l| {
        let ctx = SlideCtx {
            theme: &theme,
            colors: ColorContext { theme: &theme, clr_map: &slide_clr_map, placeholder: None },
            media: &layout_media,
            default_text,
            title_style,
            body_style,
            other_style,
            layout_placeholders: &[],
            master_placeholders,
            slide_number: number,
            page_width: width,
            page_height: height,
        };
        parse_layout(l, &ctx)
    });

    let ctx = SlideCtx {
        theme: &theme,
        colors,
        media: &slide_media,
        default_text,
        title_style,
        body_style,
        other_style,
        layout_placeholders: layout.as_ref().map(|l| l.placeholders.as_slice()).unwrap_or(&[]),
        master_placeholders,
        slide_number: number,
        page_width: width,
        page_height: height,
    };

    let mut anchors: Vec<Anchor> = Vec::new();
    let background = slide
        .child("cSld")
        .and_then(|c| c.child("bg"))
        .and_then(|bg| background(bg, &ctx))
        .or_else(|| layout.as_ref().and_then(|l| l.background.clone()))
        .or(master_bg);
    if let Some(bg) = background {
        anchors.push(bg);
    }

    let show_master = slide.attr("showMasterSp") != Some("0");
    if show_master && layout.as_ref().map(|l| l.show_master_shapes).unwrap_or(true) {
        anchors.extend(master_shapes.iter().cloned());
    }
    if let Some(layout) = &layout {
        anchors.extend(layout.shapes.iter().cloned());
    }
    if let Some(tree) = slide.child("cSld").and_then(|c| c.child("spTree")) {
        collect_shapes(tree, &Transform::identity(), &ctx, true, &mut anchors, &mut Vec::new());
    }

    Ok(Section {
        page: PageSetup {
            width,
            height,
            margin: Margins { top: 0.0, right: 0.0, bottom: 0.0, left: 0.0, header: 0.0, footer: 0.0 },
        },
        blocks: vec![Block::Paragraph(Paragraph {
            mark: RunProps { size: Some(1.0), ..RunProps::default() },
            anchors,
            ..Paragraph::default()
        })],
        columns: 1,
        column_gap: 0.0,
        ..Section::default()
    })
}

fn parse_master(
    master: &Element,
    theme: &Theme,
    media: &HashMap<String, ImageData>,
    default_text: &LevelStyles,
    number: usize,
    width: f64,
    height: f64,
) -> MasterPart {
    let clr_map = master.child("clrMap").map(parse_clr_map).unwrap_or_else(color::default_clr_map);
    let colors = ColorContext { theme, clr_map: &clr_map, placeholder: None };
    let styles = master.child("txStyles");
    let level = |name: &str| {
        styles
            .and_then(|s| s.child(name))
            .map(|s| LevelStyles::parse(s, theme, &colors))
            .unwrap_or_default()
    };
    let title_style = level("titleStyle");
    let body_style = level("bodyStyle");
    let other_style = level("otherStyle");

    let mut part = MasterPart {
        theme: theme.clone(),
        clr_map: clr_map.clone(),
        title_style,
        body_style,
        other_style,
        placeholders: Vec::new(),
        shapes: Vec::new(),
        background: None,
    };
    let ctx = SlideCtx {
        theme,
        colors: ColorContext { theme, clr_map: &clr_map, placeholder: None },
        media,
        default_text,
        title_style: &part.title_style,
        body_style: &part.body_style,
        other_style: &part.other_style,
        layout_placeholders: &[],
        master_placeholders: &[],
        slide_number: number,
        page_width: width,
        page_height: height,
    };
    part.background = master.child("cSld").and_then(|c| c.child("bg")).and_then(|bg| background(bg, &ctx));
    if let Some(tree) = master.child("cSld").and_then(|c| c.child("spTree")) {
        let mut shapes = Vec::new();
        let mut placeholders = Vec::new();
        collect_shapes(tree, &Transform::identity(), &ctx, false, &mut shapes, &mut placeholders);
        part.shapes = shapes;
        part.placeholders = placeholders;
    }
    let _ = &part.theme;
    part
}

fn parse_layout(layout: &Element, ctx: &SlideCtx) -> LayoutPart {
    let mut part = LayoutPart {
        placeholders: Vec::new(),
        shapes: Vec::new(),
        background: None,
        show_master_shapes: layout.attr("showMasterSp") != Some("0"),
    };
    part.background = layout.child("cSld").and_then(|c| c.child("bg")).and_then(|bg| background(bg, ctx));
    if let Some(tree) = layout.child("cSld").and_then(|c| c.child("spTree")) {
        let mut shapes = Vec::new();
        let mut placeholders = Vec::new();
        collect_shapes(tree, &Transform::identity(), ctx, false, &mut shapes, &mut placeholders);
        part.shapes = shapes;
        part.placeholders = placeholders;
    }
    part
}

fn background(bg: &Element, ctx: &SlideCtx) -> Option<Anchor> {
    let content = if let Some(props) = bg.child("bgPr") {
        match fill(props, &ctx.colors, ctx.media) {
            Fill::Solid(c) => DrawingContent::TextBox(TextBox { fill: Some(c), ..TextBox::default() }),
            Fill::Image(img) => DrawingContent::Image(img),
            Fill::None => return None,
        }
    } else if let Some(reference) = bg.child("bgRef") {
        let color = resolve_child(reference, &ctx.colors)?;
        DrawingContent::TextBox(TextBox { fill: Some(color), ..TextBox::default() })
    } else {
        return None;
    };
    Some(page_anchor(0.0, 0.0, ctx.page_width, ctx.page_height, content, 0.0, false, false, true))
}

pub(crate) fn page_anchor(x: f64, y: f64, w: f64, h: f64, content: DrawingContent, rot: f64, flip_h: bool, flip_v: bool, behind: bool) -> Anchor {
    Anchor {
        drawing: Drawing {
            width: w,
            height: h,
            content,
            rotation: rot,
            flip_h,
            flip_v,
        },
        horizontal: HPosition::Offset(HRef::Page, x),
        vertical: VPosition::Offset(VRef::Page, y),
        wrap: Wrap::None,
        behind,
        dist_top: 0.0,
        dist_bottom: 0.0,
        dist_left: 0.0,
        dist_right: 0.0,
    }
}

enum Fill {
    None,
    Solid(Color),
    Image(ImageData),
}

pub(crate) fn simple_shape(el: &Element, w: f64, h: f64, theme: &Theme, media: &HashMap<String, ImageData>) -> Option<DrawingContent> {
    let clr_map = color::default_clr_map();
    let colors = ColorContext { theme, clr_map: &clr_map, placeholder: None };
    if el.name == "pic" {
        let image = el
            .child("blipFill")?
            .child("blip")?
            .attr("embed")
            .and_then(|id| media.get(id))
            .cloned()?;
        return Some(DrawingContent::Image(image));
    }
    let sp_pr = el.child("spPr");
    let style = el.child("style");
    let shape_fill = match sp_pr.map(|p| fill(p, &colors, media)) {
        Some(Fill::Solid(c)) => Some(c),
        Some(Fill::Image(image)) => return Some(DrawingContent::Image(image)),
        _ => None,
    };
    let stroke = line(sp_pr, style, &colors);
    let custom = sp_pr.and_then(|p| p.child("custGeom")).and_then(custom_path);
    let geometry = sp_pr
        .and_then(|p| p.child("prstGeom"))
        .and_then(|g| g.attr("prst"))
        .unwrap_or("rect");
    let kind = match geometry {
        _ if custom.is_some() => ShapeKind::Path(custom.unwrap()),
        "ellipse" => ShapeKind::Ellipse,
        "roundRect" => ShapeKind::RoundRect(w.min(h) * 0.16667),
        "line" | "straightConnector1" | "bentConnector3" => ShapeKind::Line,
        other => preset_polygon(other).map(ShapeKind::Polygon).unwrap_or(ShapeKind::Rect),
    };
    let body = el.child("txBody");
    let body_chain: Vec<&Element> = body.and_then(|b| b.child("bodyPr")).into_iter().collect();
    let (inset, valign, scale, reduction) = body_settings(&body_chain);
    let default = LevelStyles::default();
    let text_ctx = TextContext {
        theme,
        colors: &colors,
        chain: vec![&default],
        font_scale: scale,
        spacing_reduction: reduction,
        slide_number: 0,
    };
    let blocks = body.map(|b| text_blocks(b, &text_ctx)).unwrap_or_default();
    let has_text = blocks.iter().any(|b| matches!(b, Block::Paragraph(p) if !p.inlines.is_empty()));
    if !has_text && shape_fill.is_none() && stroke.is_none() {
        return None;
    }
    Some(DrawingContent::TextBox(TextBox {
        blocks,
        fill: shape_fill,
        stroke,
        inset,
        auto_height: false,
        min_height: None,
        valign,
        shape: kind,
        ..TextBox::default()
    }))
}

fn fill(props: &Element, colors: &ColorContext, media: &HashMap<String, ImageData>) -> Fill {
    for child in props.elements() {
        match child.name.as_str() {
            "noFill" => return Fill::None,
            "solidFill" => return resolve_child(child, colors).map(Fill::Solid).unwrap_or(Fill::None),
            "gradFill" => {
                let color = child
                    .child("gsLst")
                    .and_then(|l| l.children("gs").next())
                    .and_then(|gs| resolve_child(gs, colors));
                return color.map(Fill::Solid).unwrap_or(Fill::None);
            }
            "pattFill" => {
                let color = child.child("fgClr").and_then(|c| resolve_child(c, colors));
                return color.map(Fill::Solid).unwrap_or(Fill::None);
            }
            "blipFill" => {
                let image = child
                    .child("blip")
                    .and_then(|b| b.attr("embed"))
                    .and_then(|id| media.get(id))
                    .cloned();
                return image.map(Fill::Image).unwrap_or(Fill::None);
            }
            _ => {}
        }
    }
    Fill::None
}

fn line(props: Option<&Element>, style: Option<&Element>, colors: &ColorContext) -> Option<(f64, Color)> {
    if let Some(ln) = props.and_then(|p| p.child("ln")) {
        if ln.child("noFill").is_some() {
            return None;
        }
        let width = ln.attr("w").and_then(emu).unwrap_or(0.75);
        if let Some(color) = ln.child("solidFill").and_then(|f| resolve_child(f, colors)) {
            return Some((width, color));
        }
        if ln.child("gradFill").is_some() {
            return Some((width, Color(0, 0, 0)));
        }
        if let Some(color) = style_color(style, "lnRef", colors) {
            return Some((width, color));
        }
        return None;
    }
    style_color(style, "lnRef", colors).map(|c| (0.75, c))
}

fn style_color(style: Option<&Element>, name: &str, colors: &ColorContext) -> Option<Color> {
    let reference = style?.child(name)?;
    if reference.attr("idx").map(|i| i == "0").unwrap_or(true) {
        return None;
    }
    resolve_child(reference, colors)
}

fn collect_shapes(
    tree: &Element,
    transform: &Transform,
    ctx: &SlideCtx,
    render_placeholders: bool,
    out: &mut Vec<Anchor>,
    placeholders: &mut Vec<Placeholder>,
) {
    for el in tree.elements() {
        match el.name.as_str() {
            "sp" => shape(el, transform, ctx, render_placeholders, out, placeholders),
            "pic" => picture(el, transform, ctx, out),
            "cxnSp" => connector(el, transform, ctx, out),
            "grpSp" => {
                let child = transform.child(el);
                collect_shapes(el, &child, ctx, render_placeholders, out, placeholders);
            }
            "graphicFrame" => graphic_frame(el, transform, ctx, out),
            "AlternateContent" => {
                if let Some(choice) = el.child("Choice") {
                    collect_shapes(choice, transform, ctx, render_placeholders, out, placeholders);
                } else if let Some(fallback) = el.child("Fallback") {
                    collect_shapes(fallback, transform, ctx, render_placeholders, out, placeholders);
                }
            }
            _ => {}
        }
    }
}

fn is_hidden(el: &Element, nv: &str) -> bool {
    el.child(nv)
        .and_then(|n| n.child("cNvPr"))
        .and_then(|c| c.attr("hidden"))
        .map(|h| h == "1" || h == "true")
        .unwrap_or(false)
}

fn placeholder_of(el: &Element, nv: &str) -> Option<(String, Option<String>)> {
    let ph = el.child(nv)?.child("nvPr")?.child("ph")?;
    Some((
        ph.attr("type").unwrap_or("body").to_string(),
        ph.attr("idx").map(str::to_owned),
    ))
}

fn find_placeholder<'a>(list: &'a [Placeholder], kind: &str, idx: Option<&str>) -> Option<&'a Placeholder> {
    fn norm(t: &str) -> &str {
        match t {
            "ctrTitle" => "title",
            "subTitle" | "obj" => "body",
            other => other,
        }
    }
    let by_type = matches!(norm(kind), "title" | "dt" | "ftr" | "sldNum");
    if by_type {
        return list.iter().find(|p| norm(&p.kind) == norm(kind));
    }
    let wanted = idx.unwrap_or("0");
    list.iter().find(|p| {
        !matches!(norm(&p.kind), "title" | "dt" | "ftr" | "sldNum")
            && p.idx.as_deref().unwrap_or("0") == wanted
    })
}

fn shape(
    el: &Element,
    transform: &Transform,
    ctx: &SlideCtx,
    render_placeholders: bool,
    out: &mut Vec<Anchor>,
    placeholders: &mut Vec<Placeholder>,
) {
    if is_hidden(el, "nvSpPr") {
        return;
    }
    let sp_pr = el.child("spPr");
    let style = el.child("style");
    let own_xfrm = parse_xfrm(sp_pr);
    let ph = placeholder_of(el, "nvSpPr");
    let body = el.child("txBody");
    let list_style = body
        .and_then(|b| b.child("lstStyle"))
        .map(|l| LevelStyles::parse(l, ctx.theme, &ctx.colors))
        .unwrap_or_default();

    let mut chain: Vec<&LevelStyles> = vec![ctx.default_text];
    let mut inherited_xfrm = None;
    let mut body_chain: Vec<&Element> = Vec::new();

    if let Some((kind, idx)) = &ph {
        let layout_ph = if render_placeholders {
            find_placeholder(ctx.layout_placeholders, kind, idx.as_deref())
        } else {
            None
        };
        let master_kind = layout_ph.map(|l| l.kind.as_str()).unwrap_or(kind.as_str());
        let master_idx = layout_ph.and_then(|l| l.idx.as_deref()).or(idx.as_deref());
        let master_ph = find_placeholder(ctx.master_placeholders, master_kind, master_idx);
        let styled = !render_placeholders || layout_ph.is_some() || master_ph.is_some();
        let master_style = match kind.as_str() {
            _ if !styled => ctx.other_style,
            "title" | "ctrTitle" => ctx.title_style,
            "dt" | "ftr" | "sldNum" => ctx.other_style,
            _ => ctx.body_style,
        };
        chain.push(master_style);
        if let Some(m) = master_ph {
            chain.push(&m.styles);
            inherited_xfrm = m.xfrm;
            if let Some(b) = m.body.as_ref() {
                body_chain.push(b);
            }
        }
        if let Some(l) = layout_ph {
            chain.push(&l.styles);
            if l.xfrm.is_some() {
                inherited_xfrm = l.xfrm;
            }
            if let Some(b) = l.body.as_ref() {
                body_chain.push(b);
            }
        }
        if !render_placeholders {
            placeholders.push(Placeholder {
                kind: kind.clone(),
                idx: idx.clone(),
                xfrm: own_xfrm.or(inherited_xfrm),
                styles: list_style.clone(),
                body: body.and_then(|b| b.child("bodyPr")).cloned(),
            });
            return;
        }
    } else {
        chain.push(ctx.other_style);
    }
    chain.push(&list_style);

    let Some(xfrm) = own_xfrm.or(inherited_xfrm) else { return };
    let placed = transform.apply(xfrm);

    let ph_color = style_color(style, "fillRef", &ctx.colors);
    let colors = ColorContext { theme: ctx.theme, clr_map: ctx.colors.clr_map, placeholder: ph_color };
    let shape_fill = match sp_pr.map(|p| fill(p, &colors, ctx.media)) {
        Some(Fill::Solid(c)) => Some(c),
        Some(Fill::Image(image)) => {
            out.push(page_anchor(placed.x, placed.y, placed.w, placed.h, DrawingContent::Image(image), placed.rot, placed.flip_h, placed.flip_v, false));
            None
        }
        Some(Fill::None) if sp_pr.map(|p| p.child("noFill").is_some()).unwrap_or(false) => None,
        _ => {
            if ph.is_none() {
                style_color(style, "fillRef", &ctx.colors)
            } else {
                None
            }
        }
    };
    let stroke = line(sp_pr, style, &colors);

    let custom = sp_pr.and_then(|p| p.child("custGeom")).and_then(custom_path);
    let geometry = sp_pr
        .and_then(|p| p.child("prstGeom"))
        .and_then(|g| g.attr("prst"))
        .unwrap_or("rect");
    let kind = match geometry {
        _ if custom.is_some() => ShapeKind::Path(custom.unwrap()),
        "ellipse" => ShapeKind::Ellipse,
        "roundRect" => ShapeKind::RoundRect(placed.w.min(placed.h) * 0.16667),
        "line" | "straightConnector1" | "bentConnector3" => ShapeKind::Line,
        other => preset_polygon(other).map(ShapeKind::Polygon).unwrap_or(ShapeKind::Rect),
    };

    if let Some(own) = body.and_then(|b| b.child("bodyPr")) {
        body_chain.push(own);
    }
    let (inset, valign, scale, reduction) = body_settings(&body_chain);

    let text_ctx = TextContext {
        theme: ctx.theme,
        colors: &ctx.colors,
        chain,
        font_scale: scale,
        spacing_reduction: reduction,
        slide_number: ctx.slide_number,
    };
    let blocks = body.map(|b| text_blocks(b, &text_ctx)).unwrap_or_default();
    let has_text = blocks.iter().any(|b| match b {
        Block::Paragraph(p) => !p.inlines.is_empty(),
        _ => false,
    });
    if !has_text && shape_fill.is_none() && stroke.is_none() {
        return;
    }

    let content = DrawingContent::TextBox(TextBox {
        blocks,
        fill: shape_fill,
        stroke,
        inset,
        auto_height: false,
        min_height: None,
        valign,
        shape: kind,
        ..TextBox::default()
    });
    out.push(page_anchor(placed.x, placed.y, placed.w, placed.h, content, placed.rot, placed.flip_h, placed.flip_v, false));
}

fn body_settings(chain: &[&Element]) -> ((f64, f64, f64, f64), VAlign, f64, f64) {
    let pick = |name: &str| chain.iter().rev().find_map(|b| b.attr(name));
    let inset = |name: &str, default: f64| pick(name).and_then(emu).unwrap_or(default);
    let insets = (inset("tIns", 3.6), inset("lIns", 7.2), inset("bIns", 3.6), inset("rIns", 7.2));
    let valign = match pick("anchor") {
        Some("ctr") => VAlign::Center,
        Some("b") => VAlign::Bottom,
        _ => VAlign::Top,
    };
    let autofit = chain.iter().rev().find_map(|b| b.child("normAutofit"));
    let scale = autofit
        .and_then(|a| a.attr("fontScale"))
        .and_then(|v| v.parse::<f64>().ok())
        .map(|v| v / 100000.0)
        .unwrap_or(1.0);
    let reduction = autofit
        .and_then(|a| a.attr("lnSpcReduction"))
        .and_then(|v| v.parse::<f64>().ok())
        .map(|v| v / 100000.0)
        .unwrap_or(0.0);
    (insets, valign, scale, reduction)
}

fn picture(el: &Element, transform: &Transform, ctx: &SlideCtx, out: &mut Vec<Anchor>) {
    if is_hidden(el, "nvPicPr") {
        return;
    }
    let Some(xfrm) = parse_xfrm(el.child("spPr")) else { return };
    let placed = transform.apply(xfrm);
    let image = el
        .child("blipFill")
        .and_then(|b| b.child("blip"))
        .and_then(|b| b.attr("embed"))
        .and_then(|id| ctx.media.get(id))
        .cloned();
    let content = match image {
        Some(img) => DrawingContent::Image(img),
        None => DrawingContent::Placeholder,
    };
    out.push(page_anchor(placed.x, placed.y, placed.w, placed.h, content, placed.rot, placed.flip_h, placed.flip_v, false));
    if let Some(stroke) = line(el.child("spPr"), el.child("style"), &ctx.colors) {
        let frame = TextBox { stroke: Some(stroke), inset: (0.0, 0.0, 0.0, 0.0), ..TextBox::default() };
        out.push(page_anchor(placed.x, placed.y, placed.w, placed.h, DrawingContent::TextBox(frame), placed.rot, false, false, false));
    }
}

fn connector(el: &Element, transform: &Transform, ctx: &SlideCtx, out: &mut Vec<Anchor>) {
    if is_hidden(el, "nvCxnSpPr") {
        return;
    }
    let Some(xfrm) = parse_xfrm(el.child("spPr")) else { return };
    let placed = transform.apply(xfrm);
    let Some(stroke) = line(el.child("spPr"), el.child("style"), &ctx.colors) else { return };
    let content = DrawingContent::TextBox(TextBox {
        stroke: Some(stroke),
        inset: (0.0, 0.0, 0.0, 0.0),
        shape: ShapeKind::Line,
        ..TextBox::default()
    });
    out.push(page_anchor(placed.x, placed.y, placed.w.max(0.01), placed.h.max(0.01), content, placed.rot, placed.flip_h, placed.flip_v, false));
}

fn graphic_frame(el: &Element, transform: &Transform, ctx: &SlideCtx, out: &mut Vec<Anchor>) {
    if is_hidden(el, "nvGraphicFramePr") {
        return;
    }
    let Some(xfrm) = parse_xfrm(Some(el)) else { return };
    let placed = transform.apply(xfrm);
    let Some(data) = el.child("graphic").and_then(|g| g.child("graphicData")) else { return };
    if let Some(tbl) = data.child("tbl") {
        let table = parse_table(tbl, ctx);
        out.push(page_anchor(placed.x, placed.y, placed.w, placed.h, DrawingContent::Table(table), placed.rot, false, false, false));
    } else if data.attr("uri").is_some_and(|u| u.contains("/chart") || u.contains("/diagram")) {
        out.push(page_anchor(placed.x, placed.y, placed.w, placed.h, DrawingContent::Placeholder, placed.rot, false, false, false));
    }
}

fn parse_table(tbl: &Element, ctx: &SlideCtx) -> Table {
    let mut table = Table {
        cell_margins: CellMargins { top: Some(3.6), left: Some(7.2), bottom: Some(3.6), right: Some(7.2) },
        ..Table::default()
    };
    if let Some(grid) = tbl.child("tblGrid") {
        table.columns = grid.children("gridCol").filter_map(|c| c.attr("w").and_then(emu)).collect();
    }
    let tbl_pr = tbl.child("tblPr");
    let style_id = tbl_pr.and_then(|p| p.child("tableStyleId")).map(|s| s.text());
    let first_row = tbl_pr.and_then(|p| p.attr("firstRow")).map(|v| v == "1").unwrap_or(false);
    let band_row = tbl_pr.and_then(|p| p.attr("bandRow")).map(|v| v == "1").unwrap_or(false);
    let styled = match style_id.as_deref() {
        Some(NO_STYLE_NO_GRID) => TableLook::Plain,
        Some(NO_STYLE_TABLE_GRID) => TableLook::Grid,
        Some(_) => TableLook::Medium,
        None => TableLook::Plain,
    };
    let accent = ctx.colors.theme.colors.get("accent1").copied().unwrap_or(Color(0x44, 0x72, 0xC4));
    let white = Color(255, 255, 255);
    let text_default = ctx.colors.theme.colors.get("dk1").copied().unwrap_or(Color(0, 0, 0));

    for (ri, tr) in tbl.children("tr").enumerate() {
        let mut row = Row {
            height: tr.attr("h").and_then(emu),
            exact_height: false,
            ..Row::default()
        };
        let header = first_row && ri == 0;
        for tc in tr.children("tc") {
            if tc.attr("hMerge") == Some("1") {
                continue;
            }
            let tc_pr = tc.child("tcPr");
            let mut cell = Cell {
                span: tc.attr("gridSpan").and_then(|v| v.parse().ok()).unwrap_or(1),
                ..Cell::default()
            };
            if tc.attr("vMerge") == Some("1") {
                cell.vertical_merge = Some(VerticalMerge::Continue);
            } else if tc.attr("rowSpan").and_then(|v| v.parse::<usize>().ok()).unwrap_or(1) > 1 {
                cell.vertical_merge = Some(VerticalMerge::Restart);
            }
            if let Some(pr) = tc_pr {
                let m = |name: &str| pr.attr(name).and_then(emu);
                cell.margins = CellMargins { top: m("marT"), left: m("marL"), bottom: m("marB"), right: m("marR") };
                cell.valign = match pr.attr("anchor") {
                    Some("ctr") => VAlign::Center,
                    Some("b") => VAlign::Bottom,
                    _ => VAlign::Top,
                };
                if let Fill::Solid(c) = fill(pr, &ctx.colors, ctx.media) {
                    cell.shading = Some(c);
                }
                for (name, side) in [("lnT", 0), ("lnL", 1), ("lnB", 2), ("lnR", 3)] {
                    if let Some(ln) = pr.child(name) {
                        let value = if ln.child("noFill").is_some() {
                            BorderSide::None
                        } else {
                            BorderSide::Line {
                                width: ln.attr("w").and_then(emu).unwrap_or(0.75),
                                color: ln.child("solidFill").and_then(|f| resolve_child(f, &ctx.colors)).unwrap_or(Color(0, 0, 0)),
                                style: LineStyle::Solid,
                            }
                        };
                        match side {
                            0 => cell.borders.top = value,
                            1 => cell.borders.left = value,
                            2 => cell.borders.bottom = value,
                            _ => cell.borders.right = value,
                        }
                    }
                }
            }

            let mut text_color = None;
            let mut bold = false;
            match styled {
                TableLook::Medium => {
                    if cell.shading.is_none() {
                        cell.shading = Some(if header {
                            accent
                        } else if band_row && (ri % 2 == 1) {
                            tint(accent, 0.8)
                        } else {
                            tint(accent, 0.6)
                        });
                    }
                    if header {
                        text_color = Some(white);
                        bold = true;
                    }
                    let border = BorderSide::Line { width: 1.0, color: white, style: LineStyle::Solid };
                    for side in [&mut cell.borders.top, &mut cell.borders.left, &mut cell.borders.bottom, &mut cell.borders.right] {
                        if *side == BorderSide::Unset {
                            *side = border;
                        }
                    }
                }
                TableLook::Grid => {
                    let border = BorderSide::Line { width: 0.75, color: text_default, style: LineStyle::Solid };
                    for side in [&mut cell.borders.top, &mut cell.borders.left, &mut cell.borders.bottom, &mut cell.borders.right] {
                        if *side == BorderSide::Unset {
                            *side = border;
                        }
                    }
                }
                TableLook::Plain => {}
            }

            if let Some(body) = tc.child("txBody") {
                let text_ctx = TextContext {
                    theme: ctx.theme,
                    colors: &ctx.colors,
                    chain: vec![ctx.default_text, ctx.other_style],
                    font_scale: 1.0,
                    spacing_reduction: 0.0,
                    slide_number: ctx.slide_number,
                };
                let mut blocks = text_blocks(body, &text_ctx);
                if text_color.is_some() || bold {
                    for block in &mut blocks {
                        if let Block::Paragraph(p) = block {
                            for inline in &mut p.inlines {
                                if let Inline::Text { props, .. } = inline {
                                    if let Some(c) = text_color {
                                        props.color = Some(c);
                                    }
                                    if bold {
                                        props.bold = Some(true);
                                    }
                                }
                            }
                        }
                    }
                }
                cell.blocks = blocks;
            }
            row.cells.push(cell);
        }
        table.rows.push(row);
    }
    table
}

fn custom_path(geom: &Element) -> Option<Vec<PathCommand>> {
    let mut commands = Vec::new();
    for path in geom.child("pathLst")?.children("path") {
        let w = path.attr("w").and_then(|v| v.parse::<f64>().ok()).filter(|v| *v > 0.0).unwrap_or(1.0);
        let h = path.attr("h").and_then(|v| v.parse::<f64>().ok()).filter(|v| *v > 0.0).unwrap_or(1.0);
        let pt = |el: &Element| -> Option<(f64, f64)> {
            let x = el.attr("x")?.parse::<f64>().ok()? / w;
            let y = el.attr("y")?.parse::<f64>().ok()? / h;
            Some((x, y))
        };
        let mut current = (0.0, 0.0);
        for cmd in path.elements() {
            let pts: Vec<(f64, f64)> = cmd.children("pt").filter_map(pt).collect();
            match (cmd.name.as_str(), pts.as_slice()) {
                ("moveTo", [p]) => {
                    commands.push(PathCommand::Move(p.0, p.1));
                    current = *p;
                }
                ("lnTo", [p]) => {
                    commands.push(PathCommand::Line(p.0, p.1));
                    current = *p;
                }
                ("quadBezTo", [c, p]) => {
                    commands.push(PathCommand::Quad(c.0, c.1, p.0, p.1));
                    current = *p;
                }
                ("cubicBezTo", [c1, c2, p]) => {
                    commands.push(PathCommand::Cubic(c1.0, c1.1, c2.0, c2.1, p.0, p.1));
                    current = *p;
                }
                ("arcTo", _) => {
                    let angle = |name: &str| cmd.attr(name).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) / 60000.0;
                    let rw = cmd.attr("wR").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) / w;
                    let rh = cmd.attr("hR").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) / h;
                    let start = angle("stAng").to_radians();
                    let sweep = angle("swAng").to_radians();
                    let cx = current.0 - rw * start.cos();
                    let cy = current.1 - rh * start.sin();
                    let steps = ((sweep.abs() / (std::f64::consts::PI / 8.0)).ceil() as usize).max(1);
                    for i in 1..=steps {
                        let a = start + sweep * i as f64 / steps as f64;
                        current = (cx + rw * a.cos(), cy + rh * a.sin());
                        commands.push(PathCommand::Line(current.0, current.1));
                    }
                }
                ("close", _) => commands.push(PathCommand::Close),
                _ => {}
            }
        }
    }
    if commands.iter().any(|c| matches!(c, PathCommand::Line(..) | PathCommand::Cubic(..) | PathCommand::Quad(..))) {
        Some(commands)
    } else {
        None
    }
}

fn preset_polygon(name: &str) -> Option<Vec<(f64, f64)>> {
    let points: &[(f64, f64)] = match name {
        "rightArrow" => &[(0.0, 0.25), (0.65, 0.25), (0.65, 0.0), (1.0, 0.5), (0.65, 1.0), (0.65, 0.75), (0.0, 0.75)],
        "leftArrow" => &[(1.0, 0.25), (0.35, 0.25), (0.35, 0.0), (0.0, 0.5), (0.35, 1.0), (0.35, 0.75), (1.0, 0.75)],
        "upArrow" => &[(0.25, 1.0), (0.25, 0.35), (0.0, 0.35), (0.5, 0.0), (1.0, 0.35), (0.75, 0.35), (0.75, 1.0)],
        "downArrow" => &[(0.25, 0.0), (0.25, 0.65), (0.0, 0.65), (0.5, 1.0), (1.0, 0.65), (0.75, 0.65), (0.75, 0.0)],
        "leftRightArrow" => &[(0.0, 0.5), (0.25, 0.0), (0.25, 0.25), (0.75, 0.25), (0.75, 0.0), (1.0, 0.5), (0.75, 1.0), (0.75, 0.75), (0.25, 0.75), (0.25, 1.0)],
        "homePlate" => &[(0.0, 0.0), (0.75, 0.0), (1.0, 0.5), (0.75, 1.0), (0.0, 1.0)],
        "chevron" => &[(0.0, 0.0), (0.75, 0.0), (1.0, 0.5), (0.75, 1.0), (0.0, 1.0), (0.25, 0.5)],
        "triangle" | "isocelesTriangle" => &[(0.5, 0.0), (1.0, 1.0), (0.0, 1.0)],
        "rtTriangle" => &[(0.0, 0.0), (1.0, 1.0), (0.0, 1.0)],
        "diamond" => &[(0.5, 0.0), (1.0, 0.5), (0.5, 1.0), (0.0, 0.5)],
        "parallelogram" => &[(0.25, 0.0), (1.0, 0.0), (0.75, 1.0), (0.0, 1.0)],
        "trapezoid" => &[(0.25, 0.0), (0.75, 0.0), (1.0, 1.0), (0.0, 1.0)],
        "pentagon" => &[(0.5, 0.0), (1.0, 0.38), (0.81, 1.0), (0.19, 1.0), (0.0, 0.38)],
        "hexagon" => &[(0.25, 0.0), (0.75, 0.0), (1.0, 0.5), (0.75, 1.0), (0.25, 1.0), (0.0, 0.5)],
        "octagon" => &[(0.29, 0.0), (0.71, 0.0), (1.0, 0.29), (1.0, 0.71), (0.71, 1.0), (0.29, 1.0), (0.0, 0.71), (0.0, 0.29)],
        "flowChartDecision" => &[(0.5, 0.0), (1.0, 0.5), (0.5, 1.0), (0.0, 0.5)],
        "notchedRightArrow" => &[(0.0, 0.25), (0.65, 0.25), (0.65, 0.0), (1.0, 0.5), (0.65, 1.0), (0.65, 0.75), (0.0, 0.75), (0.15, 0.5)],
        _ => return None,
    };
    Some(points.to_vec())
}

enum TableLook {
    Plain,
    Grid,
    Medium,
}

fn tint(c: Color, amount: f64) -> Color {
    let mix = |x: u8| (x as f64 + (255.0 - x as f64) * amount).round() as u8;
    Color(mix(c.0), mix(c.1), mix(c.2))
}

impl Default for Paragraph {
    fn default() -> Self {
        Paragraph {
            props: ParagraphProps::default(),
            mark: RunProps::default(),
            inlines: Vec::new(),
            anchors: Vec::new(),
            list: None,
        }
    }
}
