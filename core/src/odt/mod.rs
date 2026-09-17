mod styles;

use crate::error::Error;
use crate::model::*;
use crate::xml::{self, Element, Node};
use std::collections::HashMap;
use std::io::{Cursor, Read};
use styles::{length, ListLevel, Styles};

pub fn read(bytes: &[u8]) -> Result<Document, Error> {
    let (content, styles_root, media, settings) = if bytes.starts_with(b"PK") {
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
            .map_err(|e| Error::new(format!("not an odt container: {e}")))?;
        let content = entry(&mut archive, "content.xml")?
            .ok_or_else(|| Error::new("odt has no content.xml"))?;
        let styles_xml = entry(&mut archive, "styles.xml")?;
        let mut media = HashMap::new();
        for i in 0..archive.len() {
            let mut file = archive.by_index(i)?;
            let name = file.name().to_string();
            if name.starts_with("Pictures/") && !file.is_dir() {
                let mut data = Vec::new();
                file.read_to_end(&mut data)?;
                if let Some(format) = ImageFormat::sniff(&data) {
                    media.insert(name, ImageData { data, format });
                }
            }
        }
        let settings_xml = entry(&mut archive, "settings.xml")?;
        (
            xml::parse(&content)?,
            styles_xml.as_deref().map(xml::parse).transpose()?,
            media,
            settings_xml.as_deref().map(xml::parse).transpose()?,
        )
    } else {
        let root = xml::parse(bytes)?;
        (root.clone(), Some(root.clone()), HashMap::new(), Some(root))
    };
    let additive_spacing = settings
        .as_ref()
        .and_then(|s| find_config_item(s, "AddParaTableSpacing"))
        .map(|v| v == "true")
        .unwrap_or(true);
    let tabs_relative_to_indent = settings
        .as_ref()
        .and_then(|s| find_config_item(s, "TabsRelativeToIndent"))
        .map(|v| v == "true")
        .unwrap_or(true);
    let borders_outside_indent = settings
        .as_ref()
        .and_then(|s| find_config_item(s, "InvertBorderSpacing"))
        .map(|v| v == "true")
        .unwrap_or(false);

    let styles = Styles::parse(styles_root.as_ref(), &content);
    let body = content
        .child("body")
        .and_then(|b| b.child("text"))
        .ok_or_else(|| Error::new("odt has no office:text body"))?;

    let reader = Reader {
        styles: &styles,
        media: &media,
        counters: std::cell::RefCell::new(HashMap::new()),
        list_keys: std::cell::RefCell::new(HashMap::new()),
        style_keys: std::cell::RefCell::new(HashMap::new()),
        next_key: std::cell::Cell::new(0),
    };

    let mut doc = Document {
        default_tab: styles.default_tab,
        additive_spacing,
        tabs_relative_to_indent,
        borders_outside_indent,
        ..Document::default()
    };

    let mut current_master = None::<String>;
    let mut blocks: Vec<Block> = Vec::new();
    for el in body.elements() {
        if matches!(el.name.as_str(), "p" | "h") {
            if let Some(master) = reader.master_page_of(el) {
                if current_master.as_deref() != Some(master.as_str()) {
                    if current_master.is_some() || !blocks.is_empty() {
                        doc.sections.push(reader.section(current_master.as_deref(), std::mem::take(&mut blocks)));
                    }
                    current_master = Some(master);
                }
            }
        }
        reader.block(el, &mut blocks);
    }
    doc.sections.push(reader.section(current_master.as_deref(), blocks));
    Ok(doc)
}

fn find_config_item<'a>(el: &'a Element, name: &str) -> Option<String> {
    for child in el.elements() {
        if child.name == "config-item" && child.attr("name") == Some(name) {
            return Some(child.text());
        }
        if let Some(found) = find_config_item(child, name) {
            return Some(found);
        }
    }
    None
}

fn entry<R: Read + std::io::Seek>(archive: &mut zip::ZipArchive<R>, name: &str) -> Result<Option<Vec<u8>>, Error> {
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

struct Reader<'a> {
    styles: &'a Styles,
    media: &'a HashMap<String, ImageData>,
    counters: std::cell::RefCell<HashMap<String, Vec<i64>>>,
    list_keys: std::cell::RefCell<HashMap<String, String>>,
    style_keys: std::cell::RefCell<HashMap<String, String>>,
    next_key: std::cell::Cell<usize>,
}

impl Reader<'_> {
    fn master_page_of(&self, p: &Element) -> Option<String> {
        let style = p.attr("style-name")?;
        self.styles.master_page(style)
    }

    fn section(&self, master: Option<&str>, blocks: Vec<Block>) -> Section {
        let master = master
            .and_then(|m| self.styles.master_pages.get(m))
            .or_else(|| self.styles.master_pages.get("Standard"))
            .or_else(|| self.styles.master_pages.values().next());
        let mut section = Section {
            blocks,
            columns: 1,
            column_gap: 0.0,
            content_scale: 1.0,
            ..Section::default()
        };
        let writer_margin = 2.0 / 2.54 * 72.0;
        section.page.margin = Margins {
            top: writer_margin,
            right: writer_margin,
            bottom: writer_margin,
            left: writer_margin,
            header: writer_margin,
            footer: writer_margin,
        };
        if let Some(master) = master {
            if let Some(layout) = self.styles.page_layouts.get(&master.layout) {
                section.page = layout.page.clone();
                section.columns = layout.columns.max(1);
                section.column_gap = layout.column_gap;
            }
            if let Some(header) = &master.header {
                let mut blocks = Vec::new();
                for el in header.elements() {
                    self.block(el, &mut blocks);
                }
                section.header_default = Some(blocks);
            }
            if let Some(footer) = &master.footer {
                let mut blocks = Vec::new();
                for el in footer.elements() {
                    self.block(el, &mut blocks);
                }
                section.footer_default = Some(blocks);
            }
        }
        section
    }

    fn blocks(&self, parent: &Element) -> Vec<Block> {
        let mut out = Vec::new();
        for el in parent.elements() {
            self.block(el, &mut out);
        }
        out
    }

    fn block(&self, el: &Element, out: &mut Vec<Block>) {
        match el.name.as_str() {
            "p" | "h" => out.push(Block::Paragraph(self.paragraph(el, None, 0, None))),
            "list" => self.list(el, None, 0, None, out),
            "table" => out.push(Block::Table(self.table(el))),
            "section" => {
                let columns = el.attr("style-name").and_then(|name| self.styles.section_columns(name));
                match columns {
                    Some(mut columns) if columns.count > 1 => {
                        columns.blocks = self.blocks(el);
                        out.push(Block::Columns(columns));
                    }
                    _ => {
                        for child in el.elements() {
                            self.block(child, out);
                        }
                    }
                }
            }
            "index-body" | "table-of-content" | "alphabetical-index" | "illustration-index"
            | "bibliography" | "user-index" | "text-box" => {
                for child in el.elements() {
                    self.block(child, out);
                }
            }
            "soft-page-break" | "sequence-decls" | "variable-decls" | "user-field-decls" | "forms"
            | "tracked-changes" | "change" | "change-start" | "change-end" | "bookmark" | "bookmark-start"
            | "bookmark-end" | "annotation" => {}
            _ => {}
        }
    }

    fn list(&self, el: &Element, inherited_style: Option<&str>, level: usize, inherited_key: Option<&str>, out: &mut Vec<Block>) {
        let style_name = el.attr("style-name").or(inherited_style);
        let key: String = match inherited_key {
            Some(k) => k.to_string(),
            None => {
                let continued = el
                    .attr("continue-list")
                    .and_then(|id| self.list_keys.borrow().get(id).cloned())
                    .or_else(|| {
                        if el.attr("continue-numbering") == Some("true") {
                            style_name.and_then(|s| self.style_keys.borrow().get(s).cloned())
                        } else {
                            None
                        }
                    });
                match continued {
                    Some(k) => k,
                    None => {
                        let n = self.next_key.get() + 1;
                        self.next_key.set(n);
                        let k = format!("list-{n}");
                        self.counters.borrow_mut().remove(&k);
                        k
                    }
                }
            }
        };
        if let Some(id) = el.attr("xml:id").or(el.attr("id")) {
            self.list_keys.borrow_mut().insert(id.to_string(), key.clone());
        }
        if let Some(style) = style_name {
            self.style_keys.borrow_mut().insert(style.to_string(), key.clone());
        }
        for item in el.elements() {
            if !matches!(item.name.as_str(), "list-item" | "list-header") {
                continue;
            }
            let is_header = item.name == "list-header";
            let mut first = true;
            for child in item.elements() {
                match child.name.as_str() {
                    "p" | "h" => {
                        let label = if first && !is_header { style_name } else { None };
                        let mut paragraph = self.paragraph(child, label, level, Some(&key));
                        if !first || is_header {
                            paragraph.list = None;
                        }
                        first = false;
                        out.push(Block::Paragraph(paragraph));
                    }
                    "list" => self.list(child, style_name, level + 1, Some(&key), out),
                    _ => {}
                }
            }
        }
    }

    fn paragraph(&self, el: &Element, list_style: Option<&str>, level: usize, list_key: Option<&str>) -> Paragraph {
        let style_name = el.attr("style-name");
        let (mut props, base) = self.styles.paragraph(style_name);
        let mut list_style_name = list_style
            .map(str::to_owned)
            .or_else(|| style_name.and_then(|s| self.styles.list_style_of(s)));
        let mut level = level;
        let mut list_key = list_key;
        let mut outline = false;
        if list_style_name.is_none() && el.name == "h" {
            let outline_level = self
                .styles
                .outline_level_of(style_name)
                .or_else(|| el.attr("outline-level").and_then(|v| v.parse::<usize>().ok()))
                .filter(|l| *l >= 1);
            if let (Some(outline_level), Some(name)) = (outline_level, self.styles.outline_style.as_deref()) {
                let numbered = matches!(
                    self.styles.list_level(name, outline_level - 1).map(|l| l.kind),
                    Some(styles::LevelKind::Number { .. }) | Some(styles::LevelKind::Bullet(_))
                );
                if numbered {
                    list_style_name = Some(name.to_string());
                    level = outline_level - 1;
                    list_key = Some("outline");
                    outline = true;
                }
            }
        }

        let list_level = list_style_name
            .as_deref()
            .and_then(|name| self.styles.list_level(name, level));
        if let Some(lvl) = &list_level {
            if let Some(left) = lvl.left {
                props.indent_left = Some(left);
            }
            if let Some(hanging) = lvl.hanging {
                props.indent_hanging = Some(hanging);
                props.indent_first_line = Some(0.0);
            }
        }

        let mut inlines = Vec::new();
        let mut anchors = Vec::new();
        self.inlines(el, &base, &mut inlines, &mut anchors);
        trim_inlines(&mut inlines);

        let mut mark = base.clone();
        if let Some(last) = inlines.iter().rev().find_map(|i| match i {
            Inline::Text { props, .. } => Some(props.clone()),
            _ => None,
        }) {
            mark = last;
        }

        let list = match (&list_level, list_style_name.as_deref()) {
            (Some(lvl), Some(name)) if list_style.is_some() || outline => {
                let key = list_key.unwrap_or(name);
                let text = self.label(key, name, level, lvl);
                text.map(|text| ListLabel {
                    text,
                    props: {
                        let mut p = base.clone();
                        if let Some(rp) = &lvl.rpr {
                            p.merge(rp);
                        }
                        p
                    },
                    suffix: lvl.suffix,
                    tab_pos: lvl.tab_pos,
                })
            }
            _ => None,
        };

        Paragraph {
            props,
            mark,
            inlines,
            anchors,
            list,
        }
    }

    fn label(&self, key: &str, style: &str, level: usize, lvl: &ListLevel) -> Option<String> {
        match &lvl.kind {
            styles::LevelKind::Bullet(ch) => Some(ch.clone()),
            styles::LevelKind::Number { format, prefix, suffix, start, display_levels } => {
                let mut counters = self.counters.borrow_mut();
                let values = counters.entry(key.to_string()).or_insert_with(|| vec![0; 10]);
                for higher in 0..level {
                    if values[higher] == 0 {
                        values[higher] = self
                            .styles
                            .list_level(style, higher)
                            .and_then(|x| match x.kind {
                                styles::LevelKind::Number { start, .. } => Some(start),
                                _ => None,
                            })
                            .unwrap_or(1);
                    }
                }
                if values[level] == 0 {
                    values[level] = *start - 1;
                }
                values[level] += 1;
                for deeper in level + 1..values.len() {
                    values[deeper] = 0;
                }
                let mut parts = Vec::new();
                let first = level + 1 - (*display_levels).min(level + 1);
                for l in first..=level {
                    let v = if values[l] == 0 { 1 } else { values[l] };
                    let fmt = self
                        .styles
                        .list_level(style, l)
                        .and_then(|x| match &x.kind {
                            styles::LevelKind::Number { format, .. } => Some(format.clone()),
                            _ => None,
                        })
                        .unwrap_or_else(|| format.clone());
                    parts.push(crate::docx::format_number(v, &fmt));
                }
                Some(format!("{prefix}{}{suffix}", parts.join(".")))
            }
            styles::LevelKind::None => None,
        }
    }

    fn inlines(&self, parent: &Element, base: &RunProps, out: &mut Vec<Inline>, anchors: &mut Vec<Anchor>) {
        for node in &parent.children {
            match node {
                Node::Text(text) => {
                    let collapsed = collapse(text);
                    if !collapsed.is_empty() {
                        out.push(Inline::Text { text: collapsed, props: base.clone() });
                    }
                }
                Node::Element(el) => match el.name.as_str() {
                    "span" => {
                        let mut props = base.clone();
                        if let Some(name) = el.attr("style-name") {
                            props.merge(&self.styles.text(name, base.size.unwrap_or(12.0)));
                        }
                        self.inlines(el, &props, out, anchors);
                    }
                    "a" => self.inlines(el, base, out, anchors),
                    "s" => {
                        let count = el.attr("c").and_then(|c| c.parse::<usize>().ok()).unwrap_or(1);
                        out.push(Inline::Text { text: " ".repeat(count), props: base.clone() });
                    }
                    "tab" => out.push(Inline::Tab),
                    "line-break" => out.push(Inline::LineBreak),
                    "page-number" => out.push(Inline::Field { kind: FieldKind::Page, props: base.clone() }),
                    "page-count" => out.push(Inline::Field { kind: FieldKind::NumPages, props: base.clone() }),
                    "note" => {
                        if let Some(body) = el.child("note-body") {
                            out.push(Inline::Footnote(self.blocks(body)));
                        }
                    }
                    "frame" => self.frame(el, out, anchors),
                    "soft-page-break" | "bookmark" | "bookmark-start" | "bookmark-end" | "reference-mark"
                    | "reference-mark-start" | "reference-mark-end" | "annotation" | "annotation-end"
                    | "change-start" | "change-end" | "change" | "alphabetical-index-mark" | "toc-mark"
                    | "sequence-decls" => {}
                    "sequence" | "date" | "time" | "author-name" | "title" | "subject" | "chapter"
                    | "file-name" | "variable-set" | "variable-get" | "user-field-get" | "conditional-text"
                    | "hidden-text" | "text-input" | "expression" | "database-display" | "placeholder"
                    | "sender-firstname" | "sender-lastname" | "creator" | "initial-creator" | "description"
                    | "keywords" | "meta-field" | "bibliography-mark" | "ruby" | "measure" | "word-count"
                    | "page-variable-get" | "template-name" | "print-date" | "creation-date"
                    | "modification-date" | "editing-cycles" | "editing-duration" | "user-defined"
                    | "character-count" | "paragraph-count" | "table-count" | "image-count" | "object-count"
                    | "page-continuation" => {
                        let text = collapse(&el.text());
                        if !text.is_empty() {
                            out.push(Inline::Text { text, props: base.clone() });
                        }
                    }
                    _ => self.inlines(el, base, out, anchors),
                },
            }
        }
    }

    fn frame(&self, el: &Element, out: &mut Vec<Inline>, anchors: &mut Vec<Anchor>) {
        let width = el.attr("svg:width").or(el.attr("width")).and_then(length);
        let height = el.attr("svg:height").or(el.attr("height")).and_then(length);
        let graphic = el.attr("style-name").map(|s| self.styles.graphic(s)).unwrap_or_default();

        let content = if let Some(image) = el.child("image") {
            let data = image
                .attr("xlink:href")
                .or(image.attr("href"))
                .and_then(|href| self.media.get(href.trim_start_matches("./")))
                .cloned()
                .or_else(|| {
                    image
                        .child("binary-data")
                        .and_then(|b| base64_decode(&b.text()))
                        .and_then(|data| ImageFormat::sniff(&data).map(|format| ImageData { data, format }))
                });
            match data {
                Some(image) => DrawingContent::Image(image),
                None => DrawingContent::Placeholder,
            }
        } else if let Some(text_box) = el.child("text-box") {
            let blocks = self.blocks(text_box);
            let fill_style = match &graphic.fill_style {
                Some(styles::GraphicFill::Gradient { start, end, angle, radial }) => Some(FillStyle::Gradient { start: *start, end: *end, angle: *angle, radial: *radial }),
                Some(styles::GraphicFill::Hatch { color, distance, angle, background }) => Some(FillStyle::Hatch { color: *color, distance: *distance, angle: *angle, background: *background }),
                Some(styles::GraphicFill::ImageRef { href, repeat }) => self.media.get(href.as_str()).map(|data| FillStyle::Image { data: data.clone(), repeat: *repeat }),
                None => None,
            };
            DrawingContent::TextBox(TextBox {
                blocks,
                fill: graphic.fill,
                fill_style,
                opacity: graphic.opacity,
                stroke: graphic.stroke,
                inset: graphic.padding,
                auto_height: height.is_none() || text_box.attr("min-height").is_some(),
                min_height: text_box.attr("fo:min-height").or(text_box.attr("min-height")).and_then(length),
                ..TextBox::default()
            })
        } else if el.child("object").is_some() || el.child("object-ole").is_some() || el.child("plugin").is_some() {
            DrawingContent::Placeholder
        } else {
            return;
        };

        let (Some(width), height) = (width, height.unwrap_or(0.0)) else { return };
        let auto_height = matches!(&content, DrawingContent::TextBox(tb) if tb.auto_height);
        let drawing = Drawing::new(width, if height <= 0.0 { 20.0 } else { height }, content);
        let _ = auto_height;

        let anchor_type = el.attr("text:anchor-type").or(el.attr("anchor-type")).unwrap_or("paragraph");
        if anchor_type == "as-char" {
            out.push(Inline::Drawing(drawing));
            return;
        }

        let x = el.attr("svg:x").or(el.attr("x")).and_then(length).unwrap_or(0.0);
        let y = el.attr("svg:y").or(el.attr("y")).and_then(length).unwrap_or(0.0);
        let (href, vref) = match anchor_type {
            "page" => (HRef::Page, VRef::Page),
            _ => match (graphic.h_rel.as_str(), graphic.v_rel.as_str()) {
                ("page", "page") => (HRef::Page, VRef::Page),
                ("page", _) => (HRef::Page, VRef::Paragraph),
                (_, "page") => (HRef::Margin, VRef::Page),
                _ => (HRef::Margin, VRef::Paragraph),
            },
        };
        let horizontal = match graphic.h_pos.as_str() {
            "center" => HPosition::Align(href, HAlign::Center),
            "right" => HPosition::Align(href, HAlign::Right),
            "left" if x == 0.0 => HPosition::Align(href, HAlign::Left),
            _ => HPosition::Offset(href, x),
        };
        anchors.push(Anchor {
            drawing,
            horizontal,
            vertical: VPosition::Offset(vref, y),
            wrap: graphic.wrap,
            behind: graphic.behind,
            dist_top: graphic.margins.0,
            dist_bottom: graphic.margins.2,
            dist_left: graphic.margins.1,
            dist_right: graphic.margins.3,
        });
    }

    fn table(&self, el: &Element) -> Table {
        let mut table = Table {
            cell_margins: CellMargins { top: Some(0.0), left: Some(2.7), bottom: Some(0.0), right: Some(2.7) },
            ..Table::default()
        };
        if let Some(name) = el.attr("style-name") {
            let props = self.styles.table(name);
            table.indent = props.indent;
        }
        for col in el.elements() {
            match col.name.as_str() {
                "table-column" => self.columns(col, &mut table.columns),
                "table-columns" | "table-header-columns" | "table-column-group" => {
                    for c in col.elements() {
                        self.columns(c, &mut table.columns);
                    }
                }
                _ => {}
            }
        }
        for child in el.elements() {
            match child.name.as_str() {
                "table-row" => table.rows.push(self.row(child)),
                "table-header-rows" | "table-rows" | "table-row-group" => {
                    for row in child.children("table-row") {
                        table.rows.push(self.row(row));
                    }
                }
                _ => {}
            }
        }
        if table.columns.is_empty() {
            let count = table.rows.iter().map(|r| r.cells.iter().map(|c| c.span).sum::<usize>()).max().unwrap_or(1);
            table.columns = vec![468.0 / count as f64; count];
        }
        table
    }

    fn columns(&self, col: &Element, out: &mut Vec<f64>) {
        let repeat = col.attr("number-columns-repeated").and_then(|v| v.parse::<usize>().ok()).unwrap_or(1).min(256);
        let width = col
            .attr("style-name")
            .and_then(|s| self.styles.column_width(s))
            .unwrap_or(72.0);
        for _ in 0..repeat {
            out.push(width);
        }
    }

    fn row(&self, tr: &Element) -> Row {
        let mut row = Row::default();
        if let Some(name) = tr.attr("style-name") {
            let (height, exact) = self.styles.row_height(name);
            row.height = height;
            row.exact_height = exact;
        }
        let mut pending_vmerge: Vec<usize> = Vec::new();
        let _ = &mut pending_vmerge;
        for tc in tr.elements() {
            match tc.name.as_str() {
                "table-cell" => {
                    let repeat = tc.attr("number-columns-repeated").and_then(|v| v.parse::<usize>().ok()).unwrap_or(1).min(64);
                    for _ in 0..repeat {
                        let mut cell = Cell {
                            blocks: self.blocks(tc),
                            span: tc.attr("number-columns-spanned").and_then(|v| v.parse().ok()).unwrap_or(1),
                            ..Cell::default()
                        };
                        if tc.attr("number-rows-spanned").and_then(|v| v.parse::<usize>().ok()).unwrap_or(1) > 1 {
                            cell.vertical_merge = Some(VerticalMerge::Restart);
                        }
                        if let Some(name) = tc.attr("style-name") {
                            let props = self.styles.cell(name);
                            cell.borders = props.borders;
                            cell.margins = props.margins;
                            cell.shading = props.fill;
                            cell.valign = props.valign;
                        }
                        row.cells.push(cell);
                    }
                }
                "covered-table-cell" => {
                    let repeat = tc.attr("number-columns-repeated").and_then(|v| v.parse::<usize>().ok()).unwrap_or(1).min(64);
                    for _ in 0..repeat {
                        row.cells.push(Cell {
                            span: 1,
                            vertical_merge: Some(VerticalMerge::Continue),
                            ..Cell::default()
                        });
                    }
                }
                _ => {}
            }
        }
        row
    }
}

fn collapse(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for c in text.chars() {
        if c.is_whitespace() {
            if !space {
                out.push(' ');
                space = true;
            }
        } else {
            out.push(c);
            space = false;
        }
    }
    out
}

fn trim_inlines(inlines: &mut Vec<Inline>) {
    if let Some(Inline::Text { text, .. }) = inlines.first_mut() {
        let trimmed = text.trim_start().to_string();
        *text = trimmed;
    }
    if let Some(Inline::Text { text, .. }) = inlines.last_mut() {
        let trimmed = text.trim_end().to_string();
        *text = trimmed;
    }
    inlines.retain(|i| !matches!(i, Inline::Text { text, .. } if text.is_empty()));
}

fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0;
    for c in text.bytes() {
        let value = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            _ => continue,
        } as u32;
        buffer = (buffer << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    if out.is_empty() { None } else { Some(out) }
}
