use super::{
    auto_row_heights, parse_range, sheet_extent, workbook_defaults, workbook_document, CellData, CellValue, CondRule, Dxf, Font, PageOptions,
    RowData, Sheet, SheetDrawing, Styles, Xf,
};
use crate::error::Error;
use crate::model::*;
use crate::odt::styles::{length, Styles as OdfStyles};
use crate::xml::{self, Element, Node};
use std::collections::{BTreeMap, HashMap};
use std::io::{Cursor, Read};

const CM: f64 = 72.0 / 2.54;
const DEFAULT_COLUMN: f64 = 0.889 * 72.0;
const DEFAULT_ROW: f64 = 12.8;
const MAX_ROWS: u32 = 1_048_576;
const MAX_COLUMNS: u32 = 16_384;

struct Package {
    content: Element,
    styles: Option<Element>,
    files: HashMap<String, Vec<u8>>,
}

fn package(bytes: &[u8]) -> Result<Package, Error> {
    if !bytes.starts_with(b"PK") {
        let root = xml::parse(bytes)?;
        let styles = Some(root.clone());
        return Ok(Package { content: root, styles, files: HashMap::new() });
    }
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| Error::new(format!("not an ods container: {e}")))?;
    let mut read = |name: &str| -> Result<Option<Vec<u8>>, Error> {
        match archive.by_name(name) {
            Ok(mut file) => {
                let mut data = Vec::new();
                file.read_to_end(&mut data).map_err(|e| Error::new(format!("reading {name}: {e}")))?;
                Ok(Some(data))
            }
            Err(_) => Ok(None),
        }
    };
    let content = read("content.xml")?.ok_or_else(|| Error::new("ods has no content.xml"))?;
    let styles = read("styles.xml")?;
    let names: Vec<String> = archive.file_names().map(str::to_owned).collect();
    let mut files = HashMap::new();
    for name in names {
        if name.starts_with("Pictures/") || name.starts_with("Object ") || name.starts_with("ObjectReplacements/") {
            if let Ok(mut file) = archive.by_name(&name) {
                let mut data = Vec::new();
                if file.read_to_end(&mut data).is_ok() {
                    files.insert(name, data);
                }
            }
        }
    }
    Ok(Package { content: xml::parse(&content)?, styles: styles.as_deref().map(xml::parse).transpose()?, files })
}

pub fn read(bytes: &[u8]) -> Result<Document, Error> {
    let pkg = package(bytes)?;
    let odf = OdfStyles::parse(pkg.styles.as_ref(), &pkg.content);
    let spreadsheet = pkg
        .content
        .child("body")
        .and_then(|b| b.child("spreadsheet"))
        .ok_or_else(|| Error::new("ods has no office:spreadsheet body"))?;
    let mut formats = Formats::new(&odf);
    let mut parsed = Vec::new();
    for table in spreadsheet.children("table") {
        let hidden = table
            .attr("style-name")
            .and_then(|s| odf.elements("table", s).into_iter().rev().find_map(|e| e.child("table-properties")?.attr("display")))
            == Some("false");
        if hidden {
            continue;
        }
        let name = table.attr("name").unwrap_or("Sheet").to_string();
        let mut sheet = read_table(table, &odf, &mut formats, &pkg, &name);
        conditional_formats(table, spreadsheet, &name, &mut sheet, &mut formats);
        auto_row_heights(&mut sheet, &formats.styles);
        parsed.push((name, sheet));
    }
    Ok(workbook_document(workbook_defaults(), &parsed, &formats.styles))
}

struct Formats<'a> {
    odf: &'a OdfStyles,
    styles: Styles,
    by_name: HashMap<String, usize>,
    base_font: RunProps,
}

impl<'a> Formats<'a> {
    fn new(odf: &'a OdfStyles) -> Self {
        let mut base_font = RunProps { font: Some("Liberation Sans".into()), size: Some(10.0), color: Some(Color(0, 0, 0)), ..RunProps::default() };
        if let Some(t) = odf.default_style("table-cell").and_then(|d| d.child("text-properties")) {
            base_font.merge(&odf.run_props(t));
        }
        for el in odf.elements("table-cell", "Default") {
            if let Some(t) = el.child("text-properties") {
                base_font.merge(&odf.run_props(t));
            }
        }
        let styles = Styles {
            fonts: vec![Font { props: base_font.clone() }],
            xfs: vec![Xf { font: 0, v_align: VAlign::Bottom, ..Xf::default() }],
            num_fmts: HashMap::new(),
            dxfs: Vec::new(),
        };
        let mut formats = Formats { odf, styles, by_name: HashMap::new(), base_font };
        let default = formats.xf("Default");
        formats.styles.xfs[0] = formats.styles.xfs[default].clone();
        formats
    }

    fn xf(&mut self, name: &str) -> usize {
        if let Some(&i) = self.by_name.get(name) {
            return i;
        }
        let mut font = self.base_font.clone();
        let mut xf = Xf { v_align: VAlign::Bottom, ..Xf::default() };
        let mut align_source_fixed = false;
        for el in self.odf.elements("table-cell", name) {
            if let Some(t) = el.child("text-properties") {
                font.merge(&self.odf.run_props(t));
            }
            if let Some(p) = el.child("table-cell-properties") {
                if let Some(v) = p.attr("vertical-align") {
                    xf.v_align = match v {
                        "top" => VAlign::Top,
                        "middle" => VAlign::Center,
                        _ => VAlign::Bottom,
                    };
                }
                if let Some(w) = p.attr("wrap-option") {
                    xf.wrap = w == "wrap";
                }
                if let Some(src) = p.attr("text-align-source") {
                    align_source_fixed = src == "fix";
                }
            }
            if let Some(p) = el.child("paragraph-properties") {
                if let Some(a) = p.attr("text-align") {
                    xf.h_align = match a {
                        "center" => Some(Align::Center),
                        "end" | "right" => Some(Align::Right),
                        "justify" => Some(Align::Justify),
                        _ => Some(Align::Left),
                    };
                }
                if let Some(m) = p.attr("margin-left").and_then(length) {
                    xf.indent = m;
                }
            }
        }
        if !align_source_fixed {
            xf.h_align = None;
        }
        let cell = self.odf.cell(name);
        xf.fill = cell.fill;
        xf.borders = cell.borders;
        self.styles.fonts.push(Font { props: font });
        xf.font = self.styles.fonts.len() - 1;
        self.styles.xfs.push(xf);
        let index = self.styles.xfs.len() - 1;
        self.by_name.insert(name.to_string(), index);
        index
    }

    fn dxf(&self, name: &str) -> Dxf {
        let mut dxf = Dxf::default();
        let encoded = encode_style_name(name);
        let name = if self.odf.elements("table-cell", name).is_empty() { encoded.as_str() } else { name };
        for el in self.odf.elements("table-cell", name) {
            if let Some(t) = el.child("text-properties") {
                dxf.font.merge(&self.odf.run_props(t));
            }
        }
        dxf.fill = self.odf.cell(name).fill;
        dxf
    }
}

fn encode_style_name(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '.' {
            out.push(ch);
        } else {
            out.push_str(&format!("_{:x}_", ch as u32));
        }
    }
    out
}

fn repeat(el: &Element, attr: &str, limit: u32) -> u32 {
    el.attr(attr).and_then(|v| v.parse::<u32>().ok()).unwrap_or(1).clamp(1, limit)
}

fn rows<'e>(el: &'e Element, out: &mut Vec<&'e Element>) {
    for child in el.elements() {
        match child.name.as_str() {
            "table-row" => out.push(child),
            "table-header-rows" | "table-row-group" | "table-rows" => rows(child, out),
            _ => {}
        }
    }
}

fn columns<'e>(el: &'e Element, out: &mut Vec<&'e Element>) {
    for child in el.elements() {
        match child.name.as_str() {
            "table-column" => out.push(child),
            "table-header-columns" | "table-column-group" | "table-columns" => columns(child, out),
            _ => {}
        }
    }
}

fn cell_runs(cell: &Element, odf: &OdfStyles, base_size: f64) -> Vec<(String, RunProps)> {
    let mut runs: Vec<(String, RunProps)> = Vec::new();
    let mut first = true;
    for p in cell.children("p") {
        if !first {
            runs.push(("\n".into(), RunProps::default()));
        }
        first = false;
        inline_runs(p, odf, &RunProps::default(), base_size, &mut runs);
    }
    runs.retain(|(t, _)| !t.is_empty());
    runs
}

fn inline_runs(el: &Element, odf: &OdfStyles, props: &RunProps, base_size: f64, out: &mut Vec<(String, RunProps)>) {
    for node in &el.children {
        match node {
            Node::Text(t) => out.push((t.clone(), props.clone())),
            Node::Element(child) => match child.name.as_str() {
                "s" => {
                    let count = child.attr("c").and_then(|c| c.parse::<usize>().ok()).unwrap_or(1);
                    out.push((" ".repeat(count), props.clone()));
                }
                "tab" => out.push(("\t".into(), props.clone())),
                "line-break" => out.push(("\n".into(), props.clone())),
                "annotation" | "annotation-end" => {}
                "span" | "a" => {
                    let mut inner = props.clone();
                    if let Some(style) = child.attr("style-name") {
                        inner.merge(&odf.text(style, base_size));
                    }
                    inline_runs(child, odf, &inner, base_size, out);
                }
                _ => inline_runs(child, odf, props, base_size, out),
            },
        }
    }
}

fn annotation_text(cell: &Element) -> Option<String> {
    let note = cell.child("annotation")?;
    let text: Vec<String> = note.children("p").map(|p| p.text()).collect();
    let text = text.join("\n");
    (!text.trim().is_empty()).then_some(text)
}

fn cell_value(cell: &Element, runs: Vec<(String, RunProps)>) -> CellValue {
    let number = |attr: &str| cell.attr(attr).and_then(|v| v.parse::<f64>().ok());
    match cell.attr("value-type") {
        Some("float") | Some("percentage") | Some("currency") => match number("value") {
            Some(v) => CellValue::Shown(v, runs),
            None => CellValue::Text(runs),
        },
        Some("boolean") => {
            let v = if cell.attr("boolean-value") == Some("true") { 1.0 } else { 0.0 };
            CellValue::Shown(v, runs)
        }
        Some("date") | Some("time") => CellValue::Shown(0.0, runs),
        _ if runs.is_empty() => CellValue::Empty,
        _ => CellValue::Text(runs),
    }
}

fn read_table(table: &Element, odf: &OdfStyles, formats: &mut Formats, pkg: &Package, sheet_name: &str) -> Sheet {
    let mut col_specs: Vec<(u32, u32, f64, bool)> = Vec::new();
    let mut column_styles: Vec<(u32, u32, String)> = Vec::new();
    let mut col = 1u32;
    let mut column_elements = Vec::new();
    columns(table, &mut column_elements);
    for c in column_elements {
        let count = repeat(c, "number-columns-repeated", MAX_COLUMNS);
        let last = (col + count - 1).min(MAX_COLUMNS);
        let width = c.attr("style-name").and_then(|s| odf.column_width(s)).unwrap_or(DEFAULT_COLUMN);
        let hidden = matches!(c.attr("visibility"), Some("collapse") | Some("filter"));
        col_specs.push((col, last, width, hidden));
        if let Some(style) = c.attr("default-cell-style-name") {
            column_styles.push((col, last, style.to_string()));
        }
        col = last + 1;
        if col > MAX_COLUMNS {
            break;
        }
    }
    let column_style = |c: u32| column_styles.iter().find(|(a, b, _)| c >= *a && c <= *b).map(|(_, _, s)| s.clone());

    let base_size = formats.base_font.size.unwrap_or(10.0);
    let mut sheet_rows: BTreeMap<u32, RowData> = BTreeMap::new();
    let mut merges = Vec::new();
    let mut notes = BTreeMap::new();
    let mut drawings_src: Vec<(u32, u32, &Element)> = Vec::new();
    let (mut max_row, mut max_col) = (0u32, 0u32);
    let mut row = 1u32;
    let mut row_elements = Vec::new();
    rows(table, &mut row_elements);
    for r in row_elements {
        let count = repeat(r, "number-rows-repeated", MAX_ROWS);
        let (height, custom) = r.attr("style-name").map(|s| odf.row_height(s)).unwrap_or((None, false));
        let optimal = r
            .attr("style-name")
            .and_then(|s| odf.elements("table-row", s).into_iter().rev().find_map(|e| e.child("table-row-properties")?.attr("use-optimal-row-height")))
            .map_or(true, |v| v == "true");
        let hidden = matches!(r.attr("visibility"), Some("collapse") | Some("filter"));
        let row_style = r.attr("default-cell-style-name").map(str::to_owned);
        let has_content = r.children("table-cell").any(|c| c.attr("value-type").is_some() || c.child("p").is_some() || c.child("frame").is_some());
        let mut template = RowData { height, custom_height: custom, hidden, cells: BTreeMap::new() };
        let mut tallest = 0.0f64;
        let mut c = 1u32;
        for cell in r.elements().filter(|e| matches!(e.name.as_str(), "table-cell" | "covered-table-cell")) {
            let span = repeat(cell, "number-columns-repeated", MAX_COLUMNS);
            if cell.name == "table-cell" {
                let style = cell
                    .attr("style-name")
                    .map(str::to_owned)
                    .or_else(|| row_style.clone())
                    .or_else(|| column_style(c));
                let runs = cell_runs(cell, odf, base_size);
                let value = cell_value(cell, runs);
                let xf = style.as_deref().map(|s| formats.xf(s)).unwrap_or(0);
                if c <= MAX_COLUMNS && (span < 1024 || style.as_deref() != Some("Default")) {
                    tallest = tallest.max(line_height(formats.styles.font(formats.styles.xf(xf).font)));
                }
                let styled = style.is_some() && {
                    let x = formats.styles.xf(xf);
                    x.fill.is_some() || super::has_border(&x.borders)
                };
                if !matches!(value, CellValue::Empty) || styled || cell.child("frame").is_some() {
                    for k in 0..span.min(if matches!(value, CellValue::Empty) { 256 } else { span }) {
                        template.cells.insert(c + k, CellData { value: value.clone(), style: xf });
                    }
                    if !matches!(value, CellValue::Empty) || styled {
                        max_col = max_col.max(c + span.min(256) - 1);
                    }
                }
                let (cols_spanned, rows_spanned) = (repeat(cell, "number-columns-spanned", MAX_COLUMNS), repeat(cell, "number-rows-spanned", MAX_ROWS));
                if cols_spanned > 1 || rows_spanned > 1 {
                    merges.push((row, c, row + rows_spanned - 1, c + cols_spanned - 1));
                    max_col = max_col.max(c + cols_spanned - 1);
                }
                if let Some(text) = annotation_text(cell) {
                    notes.insert((row, c), text);
                    max_col = max_col.max(c);
                }
                for frame in cell.children("frame") {
                    drawings_src.push((row, c, frame));
                }
            }
            c += span;
            if c > MAX_COLUMNS {
                break;
            }
        }
        if optimal {
            template.height = (tallest > 0.0).then(|| (tallest + 1.15).max(DEFAULT_ROW));
        }
        let repeats = if has_content || !template.cells.is_empty() { count.min(4096) } else { count };
        if !template.cells.is_empty() || height.is_some() || hidden {
            for k in 0..repeats.min(if template.cells.is_empty() { 1 } else { repeats }) {
                sheet_rows.insert(row + k, template.clone());
            }
            if !template.cells.is_empty() {
                max_row = max_row.max(row + repeats - 1);
            }
        }
        if height.is_some() && template.cells.is_empty() && count > 1 && count < 4096 {
            for k in 1..count {
                sheet_rows.insert(row + k, template.clone());
            }
        }
        row += count;
        if row > MAX_ROWS {
            break;
        }
    }
    if let Some(shapes) = table.child("shapes") {
        for frame in shapes.children("frame") {
            drawings_src.push((0, 0, frame));
        }
    }
    for &(r, c) in notes.keys() {
        max_row = max_row.max(r);
        max_col = max_col.max(c);
    }
    for (r1, c1, r2, c2) in &merges {
        let _ = (r1, c1);
        max_row = max_row.max(*r2);
        max_col = max_col.max(*c2);
    }

    let default_row_height = odf
        .default_style("table-row")
        .and_then(|d| d.child("table-row-properties"))
        .and_then(|p| p.attr("row-height"))
        .and_then(length)
        .unwrap_or(DEFAULT_ROW);
    let col_width = |c: u32| -> f64 {
        match col_specs.iter().find(|(a, b, _, _)| c >= *a && c <= *b) {
            Some((_, _, _, true)) => 0.0,
            Some((_, _, w, _)) => *w,
            None => DEFAULT_COLUMN,
        }
    };
    let row_height = |r: u32| sheet_rows.get(&r).and_then(|d| d.height).unwrap_or(default_row_height);
    let x_of = |c: u32| (1..c).map(col_width).sum::<f64>();
    let y_of = |r: u32| (1..r).map(row_height).sum::<f64>();
    let mut drawings = Vec::new();
    for (r, c, frame) in drawings_src {
        let (base_x, base_y) = if r == 0 { (0.0, 0.0) } else { (x_of(c), y_of(r)) };
        let get = |n: &str| frame.attr(n).and_then(length).unwrap_or(0.0);
        let (x, y) = (base_x + get("x"), base_y + get("y"));
        let (mut w, mut h) = (get("width"), get("height"));
        if let Some((end_col, end_row)) = frame
            .attr("end-cell-address")
            .map(|a| a.rsplit('.').next().unwrap_or(a).replace('$', ""))
            .and_then(|a| super::column_index(&a))
        {
            w = (x_of(end_col) + get("end-x") - x).max(0.0);
            h = (y_of(end_row) + get("end-y") - y).max(0.0);
        }
        let Some(content) = frame_content(frame, pkg) else { continue };
        drawings.push(SheetDrawing { x, y, width: w, height: h, content });
        let (mut right_col, mut bottom_row) = (1u32, 1u32);
        let mut acc = 0.0;
        while acc + col_width(right_col) < x + w && right_col < 1024 {
            acc += col_width(right_col);
            right_col += 1;
        }
        acc = 0.0;
        while acc + row_height(bottom_row) < y + h && bottom_row < 65_536 {
            acc += row_height(bottom_row);
            bottom_row += 1;
        }
        max_col = max_col.max(right_col);
        max_row = max_row.max(bottom_row);
    }

    let print_area = table.attr("print-ranges").and_then(|r| r.split_whitespace().next()).and_then(|r| {
        let cleaned: String = r.split(':').map(|part| part.rsplit('.').next().unwrap_or(part).to_string()).collect::<Vec<_>>().join(":");
        parse_range(&cleaned)
    });
    let specs_pt: Vec<(u32, u32, f64, bool)> = col_specs.iter().map(|&(a, b, w, h)| (a, b, w, h)).collect();
    let extent = sheet_extent(&formats.styles, &sheet_rows, &specs_pt, DEFAULT_COLUMN, 1.0, max_row, max_col, print_area);
    let page = page_options(table, odf, formats, sheet_name);
    Sheet {
        notes,
        conditional: Vec::new(),
        drawings,
        col_widths: extent.col_widths,
        rows: sheet_rows,
        merges,
        default_row_height,
        page,
        first_row: extent.first_row,
        last_row: extent.last_row,
        first_col: extent.first_col,
        last_col: extent.last_col,
        row_breaks: Vec::new(),
        col_breaks: Vec::new(),
    }
}

fn line_height(font: &RunProps) -> f64 {
    let size = font.size.unwrap_or(10.0);
    let name = font.font.as_deref().unwrap_or("").to_lowercase();
    let factor = if name.contains("calibri") || name.contains("carlito") {
        1.221
    } else if name.contains("mono") || name.contains("courier") {
        1.133
    } else if name.contains("cambria") || name.contains("caladea") {
        1.172
    } else {
        1.149
    };
    size * factor
}

fn frame_content(frame: &Element, pkg: &Package) -> Option<DrawingContent> {
    if let Some(object) = frame.child("object") {
        let href = object.attr("href").unwrap_or("").trim_start_matches("./").trim_end_matches('/');
        if let Some(root) = pkg.files.get(&format!("{href}/content.xml")).and_then(|c| xml::parse(c).ok()) {
            return Some(crate::odf_chart::chart_content(&root));
        }
    }
    let image = frame.child("image")?;
    let href = image.attr("href")?.trim_start_matches("./");
    let data = pkg.files.get(href)?.clone();
    let format = ImageFormat::sniff(&data)?;
    Some(DrawingContent::Image(ImageData { data, format }))
}

fn header_code(el: Option<&Element>) -> Option<String> {
    let el = el?;
    if el.attr("display") == Some("false") {
        return None;
    }
    let region = |r: &Element| -> String {
        let mut out = String::new();
        for (i, p) in r.children("p").enumerate() {
            if i > 0 {
                out.push('\n');
            }
            field_text(p, &mut out);
        }
        out
    };
    let mut code = String::new();
    let mut any = false;
    for (name, tag) in [("region-left", "&L"), ("region-center", "&C"), ("region-right", "&R")] {
        if let Some(r) = el.child(name) {
            code.push_str(tag);
            code.push_str(&region(r));
            any = true;
        }
    }
    if !any {
        code.push_str("&C");
        code.push_str(&region(el));
    }
    let visible = code.replace("&L", "").replace("&C", "").replace("&R", "");
    (!visible.trim().is_empty()).then_some(code)
}

fn field_text(el: &Element, out: &mut String) {
    for node in &el.children {
        match node {
            Node::Text(t) => out.push_str(&t.replace('&', "&&")),
            Node::Element(child) => match child.name.as_str() {
                "sheet-name" => out.push_str("&A"),
                "page-number" => out.push_str("&P"),
                "page-count" => out.push_str("&N"),
                "s" => out.push_str(&" ".repeat(child.attr("c").and_then(|c| c.parse().ok()).unwrap_or(1))),
                "tab" => out.push(' '),
                _ => field_text(child, out),
            },
        }
    }
}

fn page_options(table: &Element, odf: &OdfStyles, formats: &Formats, _sheet_name: &str) -> PageOptions {
    let master_name = table
        .attr("style-name")
        .and_then(|s| odf.elements("table", s).into_iter().rev().find_map(|e| e.attr("master-page-name")))
        .unwrap_or("Default")
        .to_string();
    let master = odf.master_pages.get(&master_name).or_else(|| odf.master_pages.get("Default"));
    let mut page = PageOptions {
        width: 612.0,
        height: 792.0,
        margin_left: 2.0 * CM,
        margin_right: 2.0 * CM,
        margin_top: 2.0 * CM,
        margin_bottom: 2.0 * CM,
        margin_header: 2.0 * CM,
        margin_footer: 2.0 * CM,
        ..PageOptions::default()
    };
    let layout = master.and_then(|m| odf.page_layout_elements.get(&m.layout));
    let mut header_extent = (0.75 * CM, 0.25 * CM);
    let mut footer_extent = (0.75 * CM, 0.25 * CM);
    if let Some(layout) = layout {
        if let Some(p) = layout.child("page-layout-properties") {
            let get = |n: &str| p.attr(n).and_then(length);
            page.width = get("page-width").unwrap_or(page.width);
            page.height = get("page-height").unwrap_or(page.height);
            page.margin_left = get("margin-left").unwrap_or(page.margin_left);
            page.margin_right = get("margin-right").unwrap_or(page.margin_right);
            page.margin_top = get("margin-top").unwrap_or(page.margin_top);
            page.margin_bottom = get("margin-bottom").unwrap_or(page.margin_bottom);
            if let Some(scale) = p.attr("scale-to").and_then(|v| v.trim_end_matches('%').parse::<f64>().ok()) {
                page.scale = (scale / 100.0).clamp(0.1, 4.0);
            }
            let x = p.attr("scale-to-X").and_then(|v| v.parse::<usize>().ok());
            let y = p.attr("scale-to-Y").and_then(|v| v.parse::<usize>().ok());
            let pages = p.attr("scale-to-pages").and_then(|v| v.parse::<usize>().ok());
            if x.is_some() || y.is_some() {
                page.fit_to_page = true;
                page.fit_width = x.unwrap_or(0);
                page.fit_height = y.unwrap_or(0);
            } else if let Some(n) = pages {
                page.fit_to_page = true;
                page.fit_width = n;
                page.fit_height = n;
            }
            page.over_then_down = p.attr("print-page-order") == Some("ltr");
            page.grid_lines = p.attr("print").is_some_and(|v| v.split_whitespace().any(|w| w == "grid"));
            page.h_center = matches!(p.attr("table-centering"), Some("horizontal") | Some("both"));
        }
        for (name, slot) in [("header-style", &mut header_extent), ("footer-style", &mut footer_extent)] {
            if let Some(hf) = layout.child(name).and_then(|h| h.child("header-footer-properties")) {
                let min = hf.attr("min-height").and_then(length).or_else(|| hf.attr("height").and_then(length)).unwrap_or(0.0);
                let gap = hf.attr(if name == "header-style" { "margin-bottom" } else { "margin-top" }).and_then(length).unwrap_or(0.0);
                *slot = (min, gap);
            }
        }
    }
    page.header = header_code(master.and_then(|m| m.header.as_ref()));
    page.footer = header_code(master.and_then(|m| m.footer.as_ref()));
    let line = formats.base_font.size.unwrap_or(10.0) * 1.149;
    page.margin_header = page.margin_top;
    page.margin_footer = page.margin_bottom;
    if page.header.is_some() {
        page.body_top = Some(page.margin_top + header_extent.0.max(line + header_extent.1));
    }
    if page.footer.is_some() {
        page.body_bottom = Some(page.margin_bottom + footer_extent.0.max(line + footer_extent.1));
    }
    page
}

fn conditional_formats(table: &Element, _spreadsheet: &Element, sheet_name: &str, sheet: &mut Sheet, formats: &mut Formats) {
    let Some(list) = table.child("conditional-formats") else { return };
    let mut priority = 0i64;
    for format in list.children("conditional-format") {
        let ranges: Vec<(u32, u32, u32, u32)> = format
            .attr("target-range-address")
            .unwrap_or("")
            .split_whitespace()
            .filter_map(|r| {
                let cleaned: String = r.split(':').map(|part| part.rsplit('.').next().unwrap_or(part).to_string()).collect::<Vec<_>>().join(":");
                parse_range(&cleaned)
            })
            .collect();
        let Some(first) = ranges.first() else { continue };
        let origin = (first.0, first.1);
        let base = format.attr("base-cell-address").and_then(|a| {
            let cell = a.rsplit('.').next().unwrap_or(a).replace('$', "");
            super::column_index(&cell).map(|(c, r)| (r, c))
        });
        let origin = base.unwrap_or(origin);
        for condition in format.children("condition") {
            let Some(style) = condition.attr("apply-style-name") else { continue };
            let Some(value) = condition.attr("value") else { continue };
            let Some((kind, operator, formulas, text)) = parse_condition(value, sheet_name) else { continue };
            priority += 1;
            let dxf = formats.dxf(style);
            formats.styles.dxfs.push(dxf);
            let index = formats.styles.dxfs.len() - 1;
            for range in &ranges {
                sheet.conditional.push(CondRule {
                    range: *range,
                    origin,
                    priority,
                    dxf: index,
                    kind: kind.into(),
                    operator: operator.into(),
                    formulas: formulas.clone(),
                    text: text.clone(),
                });
            }
        }
    }
}

fn odf_formula(text: &str, sheet_name: &str) -> String {
    let mut out = String::new();
    let mut chars = text.trim().trim_start_matches("of:").trim_start_matches('=').chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '[' => {
                let mut reference = String::new();
                for c in chars.by_ref() {
                    if c == ']' {
                        break;
                    }
                    reference.push(c);
                }
                let parts: Vec<String> = reference
                    .split(':')
                    .map(|p| {
                        let p = p.trim_start_matches('.');
                        let p = if let Some((sheet, cell)) = p.split_once('.') {
                            let sheet = sheet.trim_matches('$').trim_matches('\'');
                            let _ = sheet == sheet_name;
                            cell
                        } else {
                            p
                        };
                        p.to_string()
                    })
                    .collect();
                out.push_str(&parts.join(":"));
            }
            ';' => out.push(','),
            _ => out.push(ch),
        }
    }
    out
}

type Condition = (&'static str, &'static str, Vec<String>, Option<String>);

fn parse_condition(value: &str, sheet_name: &str) -> Option<Condition> {
    let value = value.trim();
    let value = value.strip_prefix("cell-content()").map(str::trim_start).unwrap_or(value);
    let inside = |prefix: &str| value.strip_prefix(prefix).and_then(|v| v.strip_suffix(')'));
    if let Some(args) = inside("formula-is(") {
        return Some(("expression", "", vec![odf_formula(args, sheet_name)], None));
    }
    for (prefix, operator) in [
        ("cell-content-is-between(", "between"),
        ("cell-content-is-not-between(", "notBetween"),
        ("between(", "between"),
        ("not-between(", "notBetween"),
    ] {
        if let Some(args) = inside(prefix) {
            let (a, b) = split_args(args)?;
            return Some(("cellIs", operator, vec![odf_formula(&a, sheet_name), odf_formula(&b, sheet_name)], None));
        }
    }
    for (prefix, kind) in [
        ("begins-with(", "beginsWith"),
        ("ends-with(", "endsWith"),
        ("contains-text(", "containsText"),
        ("not-contains-text(", "notContainsText"),
    ] {
        if let Some(args) = inside(prefix) {
            let needle = args.trim().trim_matches('"').to_string();
            return Some((kind, "", Vec::new(), Some(needle)));
        }
    }
    for (op, operator) in [("<=", "lessThanOrEqual"), (">=", "greaterThanOrEqual"), ("!=", "notEqual"), ("<", "lessThan"), (">", "greaterThan"), ("=", "equal")] {
        if let Some(operand) = value.strip_prefix(op) {
            return Some(("cellIs", operator, vec![odf_formula(operand, sheet_name)], None));
        }
    }
    None
}

fn split_args(args: &str) -> Option<(String, String)> {
    let mut depth = 0;
    for (i, ch) in args.char_indices() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            ',' | ';' if depth == 0 => return Some((args[..i].to_string(), args[i + 1..].to_string())),
            _ => {}
        }
    }
    None
}
