mod numbering;
pub use numbering::format_number;
mod styles;
pub mod writer;

use crate::error::Error;
use crate::model::*;
use crate::xml::{self, Element};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{Cursor, Read};

use numbering::{Counters, Numbering};
use styles::{Styles, ThemeFonts};

const EMU_PER_PT: f64 = 12700.0;

pub fn read(bytes: &[u8]) -> Result<Document, Error> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| Error::new(format!("not a docx container: {e}")))?;

    let document = entry(&mut archive, "word/document.xml")?
        .ok_or_else(|| Error::new("docx has no word/document.xml"))?;
    let styles = parse_optional(entry(&mut archive, "word/styles.xml")?)?;
    let theme = parse_optional(entry(&mut archive, "word/theme/theme1.xml")?)?;
    let font_table = parse_optional(entry(&mut archive, "word/fontTable.xml")?)?;
    let settings = parse_optional(entry(&mut archive, "word/settings.xml")?)?;
    let rels = parse_optional(entry(&mut archive, "word/_rels/document.xml.rels")?)?;
    let numbering_part = parse_optional(entry(&mut archive, "word/numbering.xml")?)?;
    let footnotes_part = parse_optional(entry(&mut archive, "word/footnotes.xml")?)?;
    let endnotes_part = parse_optional(entry(&mut archive, "word/endnotes.xml")?)?;

    let theme = theme.as_ref().map(ThemeFonts::parse).unwrap_or_default();
    let styles = Styles::parse(styles.as_ref(), &theme);
    let numbering = Numbering::parse(numbering_part.as_ref(), &theme);
    let media = match rels.as_ref() {
        Some(r) => load_media(&mut archive, r, "word")?,
        None => HashMap::new(),
    };
    let charts = match rels.as_ref() {
        Some(r) => load_charts(&mut archive, r, "word")?,
        None => HashMap::new(),
    };
    let targets: HashMap<String, String> = rels
        .as_ref()
        .map(|r| {
            r.children("Relationship")
                .filter_map(|rel| Some((rel.attr("Id")?.to_string(), rel.attr("Target")?.to_string())))
                .collect()
        })
        .unwrap_or_default();

    let root = xml::parse(&document)?;
    let body = root
        .child("body")
        .ok_or_else(|| Error::new("word/document.xml has no body"))?;

    let empty_notes = HashMap::new();
    let mut notes: HashMap<String, Vec<Block>> = HashMap::new();
    for part in [footnotes_part.as_ref(), endnotes_part.as_ref()].into_iter().flatten() {
        let note_reader = Reader {
            styles: &styles,
            theme: &theme,
            media: &media,
            charts: &charts,
            numbering: &numbering,
            counters: RefCell::new(Counters::default()),
            footnotes: &empty_notes,
        };
        for note in part.elements() {
            if !matches!(note.name.as_str(), "footnote" | "endnote") {
                continue;
            }
            if matches!(note.attr("type"), Some("separator") | Some("continuationSeparator")) {
                continue;
            }
            if let Some(id) = note.attr("id") {
                notes.insert(id.to_string(), note_reader.blocks(note));
            }
        }
    }

    let reader = Reader {
        styles: &styles,
        theme: &theme,
        media: &media,
        charts: &charts,
        numbering: &numbering,
        counters: RefCell::new(Counters::default()),
        footnotes: &notes,
    };

    let mut doc = Document {
        borders_outside_indent: true,
        table_at_border_center: true,
        writer_text_offset: true,
        footnote_separator_width: Some(144.0),
        default_tab: 36.0,
        ..Document::default()
    };
    doc.generic_families = font_table.as_ref().map(generic_families).unwrap_or_default();
    if let Some(settings) = settings.as_ref() {
        if let Some(tab) = settings
            .child("defaultTabStop")
            .and_then(|t| t.attr("val"))
            .and_then(twips)
        {
            doc.default_tab = tab;
        }
        doc.even_odd_headers = settings
            .child("evenAndOddHeaders")
            .map(flag)
            .unwrap_or(false);
        doc.additive_spacing = settings
            .child("compat")
            .and_then(|c| c.child("doNotUseHTMLParagraphAutoSpacing"))
            .map(flag)
            .unwrap_or(false);
        let compat_mode = settings
            .child("compat")
            .and_then(|c| {
                c.children("compatSetting")
                    .find(|s| s.attr("name") == Some("compatibilityMode"))
            })
            .and_then(|s| s.attr("val"))
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(0);
        doc.table_at_border_center = compat_mode == 0 || compat_mode >= 15;
    }

    let mut blocks = Vec::new();
    let mut previous: Option<Section> = None;
    let mut joined: Option<Paragraph> = None;
    for el in body.elements() {
        match el.name.as_str() {
            "p" => {
                let mut paragraph = reader.paragraph(el);
                let sect = el.child("pPr").and_then(|p| p.child("sectPr"));
                if sect.is_some() && paragraph.inlines.is_empty() {
                    paragraph.list = None;
                }
                if let Some(previous) = joined.take() {
                    prepend_paragraph(previous, &mut paragraph);
                }
                if mark_deleted(el) && sect.is_none() {
                    joined = Some(paragraph);
                    continue;
                }
                blocks.push(Block::Paragraph(paragraph));
                if let Some(sect) = sect {
                    let section = build_section(
                        &mut archive, &styles, &theme, &targets, sect,
                        std::mem::take(&mut blocks), previous.as_ref(),
                    )?;
                    doc.sections.push(section.clone());
                    previous = Some(section);
                }
            }
            "sectPr" => {}
            _ => blocks.extend(reader.blocks_of(el)),
        }
    }
    if let Some(paragraph) = joined {
        blocks.push(Block::Paragraph(paragraph));
    }
    let last = match body.child("sectPr") {
        Some(sect) => build_section(&mut archive, &styles, &theme, &targets, sect, blocks, previous.as_ref())?,
        None => {
            let mut section = previous.clone().unwrap_or_default();
            section.blocks = blocks;
            section
        }
    };
    if !last.blocks.is_empty() || doc.sections.is_empty() {
        doc.sections.push(last);
    }
    Ok(doc)
}

fn build_section<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    styles: &Styles,
    theme: &ThemeFonts,
    targets: &HashMap<String, String>,
    sect: &Element,
    blocks: Vec<Block>,
    previous: Option<&Section>,
) -> Result<Section, Error> {
    let mut section = Section {
        page: page_setup(sect),
        blocks,
        columns: 1,
        column_gap: 36.0,
        ..Section::default()
    };
    if let Some(cols) = sect.child("cols") {
        section.columns = cols.attr("num").and_then(|n| n.parse().ok()).unwrap_or(1).max(1);
        if let Some(space) = cols.attr("space").and_then(twips) {
            section.column_gap = space;
        }
    }
    section.title_page = sect.child("titlePg").map(flag).unwrap_or(false);
    if let Some(num) = sect.child("pgNumType") {
        section.page_start = num.attr("start").and_then(|v| v.parse().ok());
        section.page_format = match num.attr("fmt") {
            Some("lowerRoman") => PageNumberFormat::LowerRoman,
            Some("upperRoman") => PageNumberFormat::UpperRoman,
            Some("lowerLetter") => PageNumberFormat::LowerLetter,
            Some("upperLetter") => PageNumberFormat::UpperLetter,
            _ => PageNumberFormat::Decimal,
        };
    }

    if let Some(prev) = previous {
        section.header_default = prev.header_default.clone();
        section.header_first = prev.header_first.clone();
        section.header_even = prev.header_even.clone();
        section.footer_default = prev.footer_default.clone();
        section.footer_first = prev.footer_first.clone();
        section.footer_even = prev.footer_even.clone();
    }

    for reference in sect.elements() {
        let is_header = reference.name == "headerReference";
        if !is_header && reference.name != "footerReference" {
            continue;
        }
        let Some(id) = reference.attr("id") else { continue };
        let Some(target) = targets.get(id) else { continue };
        let path = format!("word/{}", target.trim_start_matches('/').trim_start_matches("word/"));
        let Some(data) = entry(archive, &path)? else { continue };
        let root = xml::parse(&data)?;
        let rels_path = {
            let (dir, file) = path.rsplit_once('/').unwrap_or(("word", &path));
            format!("{dir}/_rels/{file}.rels")
        };
        let part_media = match parse_optional(entry(archive, &rels_path)?)? {
            Some(rels) => load_media(archive, &rels, "word")?,
            None => HashMap::new(),
        };
        let part_charts: HashMap<String, DrawingContent> = HashMap::new();
        let reader = Reader {
            styles,
            theme,
            media: &part_media,
            charts: &part_charts,
            numbering: &Numbering::default(),
            counters: RefCell::new(Counters::default()),
            footnotes: &HashMap::new(),
        };
        let content = reader.blocks(&root);
        let slot = match (is_header, reference.attr("type")) {
            (true, Some("first")) => &mut section.header_first,
            (true, Some("even")) => &mut section.header_even,
            (true, _) => &mut section.header_default,
            (false, Some("first")) => &mut section.footer_first,
            (false, Some("even")) => &mut section.footer_even,
            (false, _) => &mut section.footer_default,
        };
        *slot = Some(content);
    }
    Ok(section)
}

fn entry<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<Option<Vec<u8>>, Error> {
    match archive.by_name(name) {
        Ok(mut file) => {
            let mut data = Vec::new();
            file.read_to_end(&mut data)?;
            Ok(Some(data))
        }
        Err(zip::result::ZipError::FileNotFound) => Ok(None),
        Err(e) => Err(Error::new(format!("reading {name}: {e}"))),
    }
}

fn parse_optional(data: Option<Vec<u8>>) -> Result<Option<Element>, Error> {
    data.as_deref().map(xml::parse).transpose()
}

fn load_charts<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    rels: &Element,
    base: &str,
) -> Result<HashMap<String, DrawingContent>, Error> {
    let mut charts = HashMap::new();
    for rel in rels.children("Relationship") {
        let (Some(id), Some(target), Some(kind)) = (rel.attr("Id"), rel.attr("Target"), rel.attr("Type")) else {
            continue;
        };
        if !kind.ends_with("/chart") || rel.attr("TargetMode") == Some("External") {
            continue;
        }
        let path = if let Some(stripped) = target.strip_prefix('/') {
            stripped.to_string()
        } else {
            format!("{base}/{}", target.trim_start_matches("./"))
        };
        if let Some(data) = entry(archive, &path)? {
            charts.insert(id.to_string(), crate::pptx::chart_content(&xml::parse(&data)?));
        }
    }
    Ok(charts)
}

fn load_media<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    rels: &Element,
    base: &str,
) -> Result<HashMap<String, ImageData>, Error> {
    let mut media = HashMap::new();
    for rel in rels.children("Relationship") {
        let (Some(id), Some(target), Some(kind)) = (rel.attr("Id"), rel.attr("Target"), rel.attr("Type")) else {
            continue;
        };
        if !kind.ends_with("/image") || rel.attr("TargetMode") == Some("External") {
            continue;
        }
        let path = if let Some(stripped) = target.strip_prefix('/') {
            stripped.to_string()
        } else {
            format!("{base}/{}", target.trim_start_matches("./"))
        };
        if let Some(data) = entry(archive, &path)? {
            if let Some(format) = ImageFormat::sniff(&data) {
                media.insert(id.to_string(), ImageData { data, format });
            }
        }
    }
    Ok(media)
}

struct Reader<'a> {
    styles: &'a Styles,
    theme: &'a ThemeFonts,
    media: &'a HashMap<String, ImageData>,
    charts: &'a HashMap<String, DrawingContent>,
    numbering: &'a Numbering,
    counters: RefCell<Counters>,
    footnotes: &'a HashMap<String, Vec<Block>>,
}

impl Reader<'_> {
    fn blocks_of(&self, el: &Element) -> Vec<Block> {
        let holder = Element {
            name: String::from("#holder"),
            attrs: Vec::new(),
            children: vec![crate::xml::Node::Element(el.clone())],
        };
        self.blocks(&holder)
    }

    fn blocks(&self, parent: &Element) -> Vec<Block> {
        let mut blocks = Vec::new();
        let mut joined: Option<Paragraph> = None;
        for el in parent.elements() {
            match el.name.as_str() {
                "p" => {
                    let mut paragraph = self.paragraph(el);
                    if let Some(previous) = joined.take() {
                        prepend_paragraph(previous, &mut paragraph);
                    }
                    if mark_deleted(el) {
                        joined = Some(paragraph);
                    } else {
                        blocks.push(Block::Paragraph(paragraph));
                    }
                }
                "tbl" => blocks.push(Block::Table(self.table(el))),
                "sdt" => {
                    if let Some(content) = el.child("sdtContent") {
                        blocks.extend(self.blocks(content));
                    }
                }
                "ins" | "customXml" | "smartTag" | "moveTo" => blocks.extend(self.blocks(el)),
                _ => {}
            }
        }
        if let Some(paragraph) = joined {
            blocks.push(Block::Paragraph(paragraph));
        }
        blocks
    }

    fn paragraph(&self, el: &Element) -> Paragraph {
        let ppr = el.child("pPr");
        let style_id = ppr
            .and_then(|p| p.child("pStyle"))
            .and_then(|s| s.attr("val"));

        let direct = ppr.map(styles::parse_ppr).unwrap_or_default();
        let mut props = self.styles.paragraph_props(style_id);
        let numbering = direct
            .numbering
            .clone()
            .or_else(|| props.numbering.clone())
            .filter(|(id, _)| id != "0");

        let level = numbering
            .as_ref()
            .and_then(|(id, ilvl)| self.numbering.level(id, *ilvl));
        if let Some(level) = &level {
            props.merge(&level.ppr);
        }
        props.merge(&direct);
        props.style_id = style_id.map(str::to_owned);

        let base = self.styles.run_base(style_id);
        let mut mark = base.clone();
        if let Some(rpr) = ppr.and_then(|p| p.child("rPr")) {
            mark.merge(&styles::parse_rpr(rpr, self.theme));
        }

        let list = numbering.and_then(|(id, ilvl)| {
            let (text, level) = self.counters.borrow_mut().next(self.numbering, &id, ilvl)?;
            let mut label_props = mark.clone();
            label_props.merge(&level.rpr);
            Some(ListLabel {
                text,
                props: label_props,
                suffix: level.suffix,
                tab_pos: None,
            })
        });

        let mut inlines = Vec::new();
        let mut anchors = Vec::new();
        let mut field = FieldState::default();
        self.runs(el, &base, &mut inlines, &mut anchors, &mut field);

        Paragraph {
            props,
            mark,
            inlines,
            anchors,
            list,
        }
    }

    fn runs(
        &self,
        parent: &Element,
        base: &RunProps,
        out: &mut Vec<Inline>,
        anchors: &mut Vec<Anchor>,
        field: &mut FieldState,
    ) {
        for el in parent.elements() {
            match el.name.as_str() {
                "r" => self.run(el, base, out, anchors, field),
                "fldSimple" => {
                    let instr = el.attr("instr").unwrap_or("");
                    match field_kind(instr) {
                        Some(kind) => {
                            let props = el
                                .elements()
                                .find(|r| r.name == "r")
                                .and_then(|r| r.child("rPr"))
                                .map(|rpr| {
                                    let mut p = base.clone();
                                    p.merge(&styles::parse_rpr(rpr, self.theme));
                                    p
                                })
                                .unwrap_or_else(|| base.clone());
                            out.push(Inline::Field { kind, props });
                        }
                        None => self.runs(el, base, out, anchors, field),
                    }
                }
                "hyperlink" | "smartTag" | "ins" | "customXml" | "dir" | "bdo" | "sdt"
                | "sdtContent" | "moveTo" => self.runs(el, base, out, anchors, field),
                _ => {}
            }
        }
    }

    fn run(
        &self,
        el: &Element,
        base: &RunProps,
        out: &mut Vec<Inline>,
        anchors: &mut Vec<Anchor>,
        field: &mut FieldState,
    ) {
        let mut props = base.clone();
        if let Some(rpr) = el.child("rPr") {
            if let Some(style) = rpr.child("rStyle").and_then(|s| s.attr("val")) {
                self.styles.apply_character_style(style, &mut props);
            }
            props.merge(&styles::parse_rpr(rpr, self.theme));
        }
        if props.hidden == Some(true) {
            return;
        }

        for child in el.elements() {
            match child.name.as_str() {
                "fldChar" => match child.attr("fldCharType") {
                    Some("begin") => {
                        field.depth += 1;
                        field.instr.clear();
                        field.skipping = false;
                    }
                    Some("separate") => {
                        if field.depth > 0 {
                            if let Some(kind) = field_kind(&field.instr) {
                                out.push(Inline::Field {
                                    kind,
                                    props: props.clone(),
                                });
                                field.skipping = true;
                            }
                        }
                    }
                    Some("end") => {
                        if field.depth > 0 {
                            field.depth -= 1;
                        }
                        field.skipping = false;
                        field.instr.clear();
                    }
                    _ => {}
                },
                "instrText" => {
                    if field.depth > 0 {
                        field.instr.push_str(&child.text());
                    }
                }
                _ if field.skipping => {}
                "t" => out.push(Inline::Text {
                    text: child.text(),
                    props: props.clone(),
                }),
                "tab" | "ptab" => out.push(Inline::Tab),
                "br" => match child.attr("type") {
                    Some("page") => out.push(Inline::PageBreak),
                    _ => out.push(Inline::LineBreak),
                },
                "cr" => out.push(Inline::LineBreak),
                "noBreakHyphen" => out.push(Inline::Text {
                    text: "\u{2011}".into(),
                    props: props.clone(),
                }),
                "softHyphen" => out.push(Inline::Text {
                    text: "\u{ad}".into(),
                    props: props.clone(),
                }),
                "sym" => {
                    if let Some(ch) = child
                        .attr("char")
                        .and_then(|c| u32::from_str_radix(c, 16).ok())
                        .and_then(char::from_u32)
                    {
                        let mut sym = props.clone();
                        if let Some(font) = child.attr("font") {
                            sym.font = Some(font.to_owned());
                        }
                        out.push(Inline::Text {
                            text: ch.to_string(),
                            props: sym,
                        });
                    }
                }
                "footnoteReference" | "endnoteReference" => {
                    if let Some(blocks) = child.attr("id").and_then(|id| self.footnotes.get(id)) {
                        out.push(Inline::Footnote(blocks.clone()));
                    }
                }
                "drawing" => self.drawing(child, out, anchors),
                "pict" | "object" => self.pict(child, out, anchors),
                "AlternateContent" => {
                    if let Some(choice) = child.child("Choice") {
                        self.run(choice, base, out, anchors, field);
                    }
                }
                _ => {}
            }
        }
    }

    fn drawing(&self, el: &Element, out: &mut Vec<Inline>, anchors: &mut Vec<Anchor>) {
        for child in el.elements() {
            match child.name.as_str() {
                "inline" => {
                    if let Some(drawing) = self.drawing_body(child) {
                        out.push(Inline::Drawing(drawing));
                    }
                }
                "anchor" => {
                    if let Some(drawing) = self.drawing_body(child) {
                        anchors.push(anchor(child, drawing));
                    }
                }
                _ => {}
            }
        }
    }

    fn drawing_body(&self, el: &Element) -> Option<Drawing> {
        let extent = el.child("extent")?;
        let width = extent.attr("cx").and_then(emu)?;
        let height = extent.attr("cy").and_then(emu)?;
        let data = el.child("graphic").and_then(|g| g.child("graphicData"));
        let content = match data.and_then(|d| d.elements().next()) {
            Some(node) if node.name == "pic" => self.picture(node),
            Some(node) if node.name == "wsp" => self.shape(node),
            Some(node) if node.name == "wgp" => find_descendant(node, "pic")
                .map(|pic| self.picture(pic))
                .unwrap_or(DrawingContent::Placeholder(None)),
            Some(node) if node.name == "chart" => node.attr("id").and_then(|id| self.charts.get(id)).cloned().unwrap_or(DrawingContent::Placeholder(None)),
            _ => DrawingContent::Placeholder(None),
        };
        Some(Drawing::new(width, height, content))
    }

    fn picture(&self, pic: &Element) -> DrawingContent {
        pic.child("blipFill")
            .and_then(|b| b.child("blip"))
            .and_then(|b| b.attr("embed").or(b.attr("link")))
            .and_then(|id| self.media.get(id))
            .cloned()
            .map(DrawingContent::Image)
            .unwrap_or(DrawingContent::Placeholder(None))
    }

    fn shape(&self, wsp: &Element) -> DrawingContent {
        let sppr = wsp.child("spPr");
        let fill = sppr
            .and_then(|s| s.child("solidFill"))
            .and_then(|f| f.child("srgbClr"))
            .and_then(|c| c.attr("val"))
            .and_then(Color::parse_hex);
        let stroke = match sppr.and_then(|s| s.child("ln")) {
            Some(ln) if ln.child("noFill").is_some() => None,
            Some(ln) => Some((
                ln.attr("w").and_then(emu).unwrap_or(0.75),
                ln.child("solidFill")
                    .and_then(|f| f.child("srgbClr"))
                    .and_then(|c| c.attr("val"))
                    .and_then(Color::parse_hex)
                    .unwrap_or(Color(0, 0, 0)),
            )),
            None => match wsp
                .child("style")
                .and_then(|s| s.child("lnRef"))
                .and_then(|l| l.attr("idx"))
            {
                Some(idx) if idx != "0" => Some((0.75, Color(0, 0, 0))),
                _ => None,
            },
        };
        let body = wsp.child("bodyPr");
        let inset = |name: &str, default: f64| body.and_then(|b| b.attr(name)).and_then(emu).unwrap_or(default);
        let insets = (inset("tIns", 3.6), inset("lIns", 7.2), inset("bIns", 3.6), inset("rIns", 7.2));
        let auto_height = body.map(|b| b.child("spAutoFit").is_some()).unwrap_or(false);
        let blocks = wsp
            .child("txbx")
            .and_then(|t| t.child("txbxContent"))
            .map(|c| self.blocks(c))
            .unwrap_or_default();
        if blocks.is_empty() && fill.is_none() && stroke.is_none() {
            return DrawingContent::Placeholder(None);
        }
        DrawingContent::TextBox(TextBox {
            blocks,
            fill,
            stroke,
            inset: insets,
            auto_height,
            ..TextBox::default()
        })
    }

    fn pict(&self, el: &Element, out: &mut Vec<Inline>, anchors: &mut Vec<Anchor>) {
        for shape in el.elements() {
            if !matches!(shape.name.as_str(), "shape" | "rect" | "roundrect" | "oval" | "group") {
                continue;
            }
            let style = vml_style(shape.attr("style").unwrap_or(""));
            let width = style.get("width").and_then(|v| css_length(v));
            let height = style.get("height").and_then(|v| css_length(v));
            let (Some(width), Some(height)) = (width, height) else { continue };

            let content = if let Some(image) = find_descendant(shape, "imagedata") {
                image
                    .attr("id")
                    .or(image.attr("href"))
                    .and_then(|id| self.media.get(id))
                    .cloned()
                    .map(DrawingContent::Image)
                    .unwrap_or(DrawingContent::Placeholder(None))
            } else if let Some(content) = find_descendant(shape, "txbxContent") {
                let fill = shape
                    .attr("fillcolor")
                    .and_then(|c| Color::parse_hex(c.trim_start_matches('#')))
                    .filter(|_| shape.attr("filled") != Some("f"));
                let stroke = if shape.attr("stroked") == Some("f") {
                    None
                } else {
                    Some((
                        shape.attr("strokeweight").and_then(css_length).unwrap_or(0.75),
                        shape
                            .attr("strokecolor")
                            .and_then(|c| Color::parse_hex(c.trim_start_matches('#')))
                            .unwrap_or(Color(0, 0, 0)),
                    ))
                };
                DrawingContent::TextBox(TextBox {
                    blocks: self.blocks(content),
                    fill,
                    stroke,
                    ..TextBox::default()
                })
            } else {
                DrawingContent::Placeholder(None)
            };

            let drawing = Drawing::new(width, height, content);

            if style.get("position").map(|p| p.as_str()) == Some("absolute") {
                let href = match style.get("mso-position-horizontal-relative").map(|s| s.as_str()) {
                    Some("page") => HRef::Page,
                    Some("text") | Some("char") => HRef::Column,
                    _ => HRef::Margin,
                };
                let vref = match style.get("mso-position-vertical-relative").map(|s| s.as_str()) {
                    Some("page") => VRef::Page,
                    Some("margin") => VRef::Margin,
                    Some("line") => VRef::Line,
                    _ => VRef::Paragraph,
                };
                let x = style.get("margin-left").and_then(|v| css_length(v)).unwrap_or(0.0);
                let y = style.get("margin-top").and_then(|v| css_length(v)).unwrap_or(0.0);
                let behind = style
                    .get("z-index")
                    .and_then(|z| z.parse::<i64>().ok())
                    .map(|z| z < 0)
                    .unwrap_or(false);
                let wrap = match shape.child("wrap").and_then(|w| w.attr("type")) {
                    Some("topAndBottom") => Wrap::TopAndBottom,
                    Some("square") | Some("tight") | Some("through") => Wrap::Square,
                    _ => Wrap::None,
                };
                anchors.push(Anchor {
                    drawing,
                    horizontal: HPosition::Offset(href, x),
                    vertical: VPosition::Offset(vref, y),
                    wrap,
                    behind,
                    dist_top: 0.0,
                    dist_bottom: 0.0,
                    dist_left: 0.0,
                    dist_right: 0.0,
                });
            } else {
                out.push(Inline::Drawing(drawing));
            }
        }
    }

    fn table(&self, el: &Element) -> Table {
        let tbl_pr = el.child("tblPr");
        let style_id = tbl_pr
            .and_then(|p| p.child("tblStyle"))
            .and_then(|s| s.attr("val"));

        let (style_borders, style_margins) = self.styles.table_style(style_id);
        let mut table = Table {
            borders: style_borders,
            cell_margins: style_margins,
            ..Table::default()
        };

        if let Some(grid) = el.child("tblGrid") {
            table.columns = grid
                .children("gridCol")
                .filter_map(|c| c.attr("w").and_then(twips))
                .collect();
        }

        if let Some(pr) = tbl_pr {
            if let Some(borders) = pr.child("tblBorders") {
                table.borders.merge(&parse_borders(borders));
            }
            if let Some(ind) = pr.child("tblInd").and_then(|i| i.attr("w")).and_then(twips) {
                table.indent = ind;
            }
            if let Some(margins) = pr.child("tblCellMar") {
                table.cell_margins.merge(&parse_cell_margins(margins));
            }
        }

        for tr in el.children("tr") {
            let mut row = Row::default();
            if let Some(height) = tr.child("trPr").and_then(|p| p.child("trHeight")) {
                row.height = height.attr("val").and_then(twips);
                row.exact_height = height.attr("hRule") == Some("exact");
            }
            for tc in tr.children("tc") {
                let pr = tc.child("tcPr");
                let mut cell = Cell {
                    blocks: self.blocks(tc),
                    span: 1,
                    ..Cell::default()
                };
                if let Some(pr) = pr {
                    cell.span = pr
                        .child("gridSpan")
                        .and_then(|s| s.attr("val"))
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(1)
                        .max(1);
                    cell.width = pr
                        .child("tcW")
                        .filter(|w| w.attr("type") != Some("pct"))
                        .and_then(|w| w.attr("w"))
                        .and_then(twips);
                    cell.shading = pr
                        .child("shd")
                        .and_then(|s| s.attr("fill"))
                        .filter(|f| *f != "auto")
                        .and_then(Color::parse_hex);
                    cell.vertical_merge = pr.child("vMerge").map(|m| match m.attr("val") {
                        Some("restart") => VerticalMerge::Restart,
                        _ => VerticalMerge::Continue,
                    });
                    if let Some(borders) = pr.child("tcBorders") {
                        cell.borders = parse_borders(borders);
                    }
                    if let Some(margins) = pr.child("tcMar") {
                        cell.margins = parse_cell_margins(margins);
                    }
                    cell.valign = match pr.child("vAlign").and_then(|v| v.attr("val")) {
                        Some("center") => VAlign::Center,
                        Some("bottom") => VAlign::Bottom,
                        _ => VAlign::Top,
                    };
                }
                row.cells.push(cell);
            }
            if !row.cells.is_empty() {
                table.rows.push(row);
            }
        }

        if table.columns.is_empty() {
            if let Some(first) = table.rows.first() {
                table.columns = first
                    .cells
                    .iter()
                    .flat_map(|c| {
                        let w = c.width.unwrap_or(72.0) / c.span as f64;
                        std::iter::repeat(w).take(c.span)
                    })
                    .collect();
            }
        }

        table
    }
}

fn anchor(el: &Element, drawing: Drawing) -> Anchor {
    let behind = matches!(el.attr("behindDoc"), Some("1") | Some("true"));

    let horizontal = el
        .child("positionH")
        .map(|p| {
            let relative = match p.attr("relativeFrom") {
                Some("page") => HRef::Page,
                Some("character") => HRef::Character,
                Some("column") => HRef::Column,
                _ => HRef::Margin,
            };
            match p.child("posOffset").and_then(|o| emu(&o.text())) {
                Some(offset) => HPosition::Offset(relative, offset),
                None => HPosition::Align(
                    relative,
                    match p.child("align").map(|a| a.text()).as_deref() {
                        Some("center") => HAlign::Center,
                        Some("right") | Some("outside") => HAlign::Right,
                        _ => HAlign::Left,
                    },
                ),
            }
        })
        .unwrap_or(HPosition::Offset(HRef::Column, 0.0));

    let vertical = el
        .child("positionV")
        .map(|p| {
            let relative = match p.attr("relativeFrom") {
                Some("page") | Some("topMargin") | Some("bottomMargin") => VRef::Page,
                Some("margin") => VRef::Margin,
                Some("line") => VRef::Line,
                _ => VRef::Paragraph,
            };
            match p.child("posOffset").and_then(|o| emu(&o.text())) {
                Some(offset) => VPosition::Offset(relative, offset),
                None => VPosition::Align(
                    relative,
                    match p.child("align").map(|a| a.text()).as_deref() {
                        Some("center") => VAnchorAlign::Center,
                        Some("bottom") | Some("outside") => VAnchorAlign::Bottom,
                        _ => VAnchorAlign::Top,
                    },
                ),
            }
        })
        .unwrap_or(VPosition::Offset(VRef::Paragraph, 0.0));

    let wrap = if behind || el.child("wrapNone").is_some() {
        Wrap::None
    } else if el.child("wrapTopAndBottom").is_some() {
        Wrap::TopAndBottom
    } else if el.child("wrapSquare").is_some()
        || el.child("wrapTight").is_some()
        || el.child("wrapThrough").is_some()
    {
        Wrap::Square
    } else {
        Wrap::None
    };

    Anchor {
        drawing,
        horizontal,
        vertical,
        wrap,
        behind,
        dist_top: el.attr("distT").and_then(emu).unwrap_or(0.0),
        dist_bottom: el.attr("distB").and_then(emu).unwrap_or(0.0),
        dist_left: el.attr("distL").and_then(emu).unwrap_or(0.0),
        dist_right: el.attr("distR").and_then(emu).unwrap_or(0.0),
    }
}

#[derive(Default)]
struct FieldState {
    depth: usize,
    instr: String,
    skipping: bool,
}

fn field_kind(instr: &str) -> Option<FieldKind> {
    let mut words = instr.split_whitespace();
    match words.next()?.to_ascii_uppercase().as_str() {
        "PAGE" => Some(FieldKind::Page),
        "NUMPAGES" | "SECTIONPAGES" => Some(FieldKind::NumPages),
        _ => None,
    }
}

fn find_descendant<'a>(el: &'a Element, name: &str) -> Option<&'a Element> {
    for child in el.elements() {
        if child.name == name {
            return Some(child);
        }
        if let Some(found) = find_descendant(child, name) {
            return Some(found);
        }
    }
    None
}

fn vml_style(style: &str) -> HashMap<String, String> {
    style
        .split(';')
        .filter_map(|item| {
            let (k, v) = item.split_once(':')?;
            Some((k.trim().to_lowercase(), v.trim().to_string()))
        })
        .collect()
}

fn css_length(value: &str) -> Option<f64> {
    let v = value.trim();
    let split = v
        .find(|c: char| c.is_ascii_alphabetic() || c == '%')
        .unwrap_or(v.len());
    let number: f64 = v[..split].trim().parse().ok()?;
    Some(match &v[split..] {
        "pt" | "" => number,
        "in" => number * 72.0,
        "cm" => number * 72.0 / 2.54,
        "mm" => number * 72.0 / 25.4,
        "px" => number * 0.75,
        "pc" => number * 12.0,
        _ => return None,
    })
}

fn mark_deleted(paragraph: &Element) -> bool {
    paragraph
        .child("pPr")
        .and_then(|p| p.child("rPr"))
        .is_some_and(|r| r.child("del").is_some() || r.child("moveFrom").is_some())
}

fn prepend_paragraph(previous: Paragraph, paragraph: &mut Paragraph) {
    let mut inlines = previous.inlines;
    inlines.extend(std::mem::take(&mut paragraph.inlines));
    paragraph.inlines = inlines;
    let mut anchors = previous.anchors;
    anchors.extend(std::mem::take(&mut paragraph.anchors));
    paragraph.anchors = anchors;
}

pub(crate) fn parse_border_side(side: &Element) -> Option<BorderSide> {
    Some(match side.attr("val")? {
        "nil" | "none" => BorderSide::None,
        val => BorderSide::Line {
            width: side
                .attr("sz")
                .and_then(|v| v.trim().parse::<f64>().ok())
                .map(|v| (v / 8.0).max(0.25))
                .unwrap_or(0.5),
            color: side
                .attr("color")
                .filter(|c| *c != "auto")
                .and_then(Color::parse_hex)
                .unwrap_or(Color(0, 0, 0)),
            style: LineStyle::from_name(val),
        },
    })
}

pub(crate) fn parse_borders(el: &Element) -> Borders {
    let mut borders = Borders::default();
    for side in el.elements() {
        let Some(value) = parse_border_side(side) else { continue };
        match side.name.as_str() {
            "top" => borders.top = value,
            "left" | "start" => borders.left = value,
            "bottom" => borders.bottom = value,
            "right" | "end" => borders.right = value,
            "insideH" => borders.inside_h = value,
            "insideV" => borders.inside_v = value,
            _ => {}
        }
    }
    borders
}

pub(crate) fn parse_cell_margins(el: &Element) -> CellMargins {
    let mut margins = CellMargins::default();
    for side in el.elements() {
        let value = if side.attr("type") == Some("nil") {
            Some(0.0)
        } else {
            side.attr("w").and_then(twips)
        };
        match side.name.as_str() {
            "top" => margins.top = value,
            "left" | "start" => margins.left = value,
            "bottom" => margins.bottom = value,
            "right" | "end" => margins.right = value,
            _ => {}
        }
    }
    margins
}

fn page_setup(sect: &Element) -> PageSetup {
    let mut page = PageSetup::default();
    if let Some(size) = sect.child("pgSz") {
        if let Some(w) = size.attr("w").and_then(twips) {
            page.width = w;
        }
        if let Some(h) = size.attr("h").and_then(twips) {
            page.height = h;
        }
    }
    if let Some(margin) = sect.child("pgMar") {
        let side = |name: &str| margin.attr(name).and_then(twips).map(f64::abs);
        if let Some(v) = side("top") {
            page.margin.top = v;
        }
        if let Some(v) = side("right") {
            page.margin.right = v;
        }
        if let Some(v) = side("bottom") {
            page.margin.bottom = v;
        }
        if let Some(v) = side("left") {
            page.margin.left = v;
        }
        if let Some(v) = side("header") {
            page.margin.header = v;
        }
        if let Some(v) = side("footer") {
            page.margin.footer = v;
        }
        if let Some(gutter) = side("gutter") {
            page.margin.left += gutter;
        }
    }
    page
}

fn generic_families(fonts: &Element) -> HashMap<String, Generic> {
    let mut map = HashMap::new();
    for font in fonts.children("font") {
        let Some(name) = font.attr("name") else { continue };
        let generic = match font.child("family").and_then(|f| f.attr("val")) {
            Some("roman") => Generic::Serif,
            Some("modern") => Generic::Mono,
            Some("swiss") => Generic::Sans,
            _ => {
                let pitch = font.child("pitch").and_then(|p| p.attr("val"));
                if pitch == Some("fixed") {
                    Generic::Mono
                } else {
                    continue;
                }
            }
        };
        map.insert(name.to_lowercase(), generic);
    }
    map
}

pub(crate) fn emu(value: &str) -> Option<f64> {
    value.trim().parse::<f64>().ok().map(|v| v / EMU_PER_PT)
}

pub(crate) fn twips(value: &str) -> Option<f64> {
    value.trim().parse::<f64>().ok().map(|v| v / 20.0)
}

pub(crate) fn half_points(value: &str) -> Option<f64> {
    value.trim().parse::<f64>().ok().map(|v| v / 2.0)
}

pub(crate) fn flag(el: &Element) -> bool {
    !matches!(el.attr("val"), Some("0") | Some("false") | Some("off"))
}
