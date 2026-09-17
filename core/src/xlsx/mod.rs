mod format;

use crate::error::Error;
use crate::model::*;
use crate::pptx::color::{apply_tint, Theme};
use crate::pptx::text::emu;
use crate::pptx::{page_anchor, resolve_path, simple_shape};
use crate::xml::{self, Element};
use std::collections::{BTreeMap, HashMap};
use std::io::{Cursor, Read};

const PX_TO_PT: f64 = 0.75;

pub fn read(bytes: &[u8]) -> Result<Document, Error> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| Error::new(format!("not an xlsx container: {e}")))?;
    let workbook = entry(&mut archive, "xl/workbook.xml")?
        .ok_or_else(|| Error::new("xlsx has no xl/workbook.xml"))?;
    let workbook = xml::parse(&workbook)?;
    let rels = entry(&mut archive, "xl/_rels/workbook.xml.rels")?
        .as_deref()
        .map(xml::parse)
        .transpose()?;
    let targets: HashMap<String, String> = rels
        .as_ref()
        .map(|r| {
            r.children("Relationship")
                .filter_map(|rel| {
                    let target = rel.attr("Target")?;
                    let path = target
                        .strip_prefix('/')
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("xl/{}", target.trim_start_matches("./")));
                    Some((rel.attr("Id")?.to_string(), path))
                })
                .collect()
        })
        .unwrap_or_default();

    let theme = entry(&mut archive, "xl/theme/theme1.xml")?
        .as_deref()
        .map(xml::parse)
        .transpose()?
        .map(|t| Theme::parse(&t))
        .unwrap_or_default();
    let styles = Styles::parse(
        entry(&mut archive, "xl/styles.xml")?.as_deref().map(xml::parse).transpose()?.as_ref(),
        &theme,
    );
    let shared = entry(&mut archive, "xl/sharedStrings.xml")?
        .as_deref()
        .map(xml::parse)
        .transpose()?
        .map(|root| parse_shared_strings(&root, &styles))
        .unwrap_or_default();

    let mut print_areas: HashMap<usize, String> = HashMap::new();
    if let Some(names) = workbook.child("definedNames") {
        for name in names.children("definedName") {
            if name.attr("name") == Some("_xlnm.Print_Area") {
                if let Some(id) = name.attr("localSheetId").and_then(|v| v.parse::<usize>().ok()) {
                    print_areas.insert(id, name.text());
                }
            }
        }
    }

    let mut doc = Document {
        default_tab: 36.0,
        additive_spacing: true,
        ..Document::default()
    };

    let sheets: Vec<&Element> = workbook
        .child("sheets")
        .map(|s| s.children("sheet").collect())
        .unwrap_or_default();
    let mut parsed_sheets = Vec::new();
    for (index, sheet) in sheets.iter().enumerate() {
        if matches!(sheet.attr("state"), Some("hidden") | Some("veryHidden")) {
            continue;
        }
        let Some(rid) = sheet.attr("r:id").or(sheet.attr("id")) else { continue };
        let Some(path) = targets.get(rid) else { continue };
        let Some(data) = entry(&mut archive, path)? else { continue };
        let root = xml::parse(&data)?;
        let name = sheet.attr("name").unwrap_or("Sheet").to_string();
        let mut parsed = parse_sheet(&root, &styles, &shared, print_areas.get(&index).map(String::as_str));
        parsed.drawings = read_drawings(&mut archive, path, &parsed, &theme)?;
        for drawing in &parsed.drawings {
            let (col, row) = cell_at(&parsed, drawing.x + drawing.width, drawing.y + drawing.height);
            if parsed.first_row == 0 {
                parsed.first_row = 1;
                parsed.first_col = 1;
            }
            parsed.last_row = parsed.last_row.max(row);
            parsed.last_col = parsed.last_col.max(col);
        }
        parsed_sheets.push((name, parsed));
    }
    let any_content = parsed_sheets.iter().any(|(_, s)| s.last_row > 0 && s.last_col > 0);
    for (index, (name, parsed)) in parsed_sheets.iter().enumerate() {
        let empty = parsed.last_row == 0 || parsed.last_col == 0;
        if empty && (any_content || index > 0) {
            continue;
        }
        doc.sections.extend(paginate(parsed, name, &styles));
    }
    if doc.sections.is_empty() {
        doc.sections.push(Section {
            page: PageSetup::default(),
            blocks: vec![Block::Paragraph(Paragraph {
                props: ParagraphProps::default(),
                mark: RunProps::default(),
                inlines: Vec::new(),
                anchors: Vec::new(),
                list: None,
            })],
            columns: 1,
            content_scale: 1.0,
            ..Section::default()
        });
    }
    Ok(doc)
}

fn sheet_col_width(sheet: &Sheet, col: u32) -> f64 {
    sheet.col_widths.get(col as usize).copied().unwrap_or(48.0)
}

fn sheet_row_height(sheet: &Sheet, row: u32) -> f64 {
    sheet
        .rows
        .get(&(row + 1))
        .and_then(|d| d.height)
        .unwrap_or(sheet.default_row_height)
}

fn sheet_x(sheet: &Sheet, col: u32) -> f64 {
    (0..col).map(|c| sheet_col_width(sheet, c)).sum()
}

fn sheet_y(sheet: &Sheet, row: u32) -> f64 {
    (0..row).map(|r| sheet_row_height(sheet, r)).sum()
}

fn cell_at(sheet: &Sheet, x: f64, y: f64) -> (u32, u32) {
    let mut col = 0;
    let mut cx = 0.0;
    while cx + sheet_col_width(sheet, col) < x - 0.01 && col < 16_384 {
        cx += sheet_col_width(sheet, col);
        col += 1;
    }
    let mut row = 0;
    let mut cy = 0.0;
    while cy + sheet_row_height(sheet, row) < y - 0.01 && row < 1_048_576 {
        cy += sheet_row_height(sheet, row);
        row += 1;
    }
    (col + 1, row + 1)
}

fn read_drawings<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    sheet_path: &str,
    sheet: &Sheet,
    theme: &Theme,
) -> Result<Vec<SheetDrawing>, Error> {
    let (dir, file) = sheet_path.rsplit_once('/').unwrap_or(("", sheet_path));
    let Some(rels) = entry(archive, &format!("{dir}/_rels/{file}.rels"))? else { return Ok(Vec::new()) };
    let rels = xml::parse(&rels)?;
    let mut drawings = Vec::new();
    for rel in rels.children("Relationship") {
        if !rel.attr("Type").is_some_and(|t| t.ends_with("/drawing")) {
            continue;
        }
        let Some(target) = rel.attr("Target") else { continue };
        let path = resolve_path(dir, target);
        let Some(data) = entry(archive, &path)? else { continue };
        let root = xml::parse(&data)?;
        let (ddir, dfile) = path.rsplit_once('/').unwrap_or(("", path.as_str()));
        let mut media = HashMap::new();
        if let Some(drels) = entry(archive, &format!("{ddir}/_rels/{dfile}.rels"))? {
            for r in xml::parse(&drels)?.children("Relationship") {
                if !r.attr("Type").is_some_and(|t| t.ends_with("/image")) {
                    continue;
                }
                if let (Some(id), Some(t)) = (r.attr("Id"), r.attr("Target")) {
                    if let Some(bytes) = entry(archive, &resolve_path(ddir, t))? {
                        if let Some(format) = ImageFormat::sniff(&bytes) {
                            media.insert(id.to_string(), ImageData { data: bytes, format });
                        }
                    }
                }
            }
        }
        for anchor in root.elements() {
            let num = |el: &Element, name: &str| el.child(name).and_then(|c| c.text().trim().parse::<u32>().ok()).unwrap_or(0);
            let off = |el: &Element, name: &str| el.child(name).and_then(|c| emu(c.text().trim())).unwrap_or(0.0);
            let (x, y) = match (anchor.child("from"), anchor.child("pos")) {
                (Some(from), _) => (
                    sheet_x(sheet, num(from, "col")) + off(from, "colOff"),
                    sheet_y(sheet, num(from, "row")) + off(from, "rowOff"),
                ),
                (None, Some(pos)) => (pos.attr("x").and_then(emu).unwrap_or(0.0), pos.attr("y").and_then(emu).unwrap_or(0.0)),
                _ => continue,
            };
            let (width, height) = match (anchor.child("to"), anchor.child("ext")) {
                (Some(to), _) => (
                    sheet_x(sheet, num(to, "col")) + off(to, "colOff") - x,
                    sheet_y(sheet, num(to, "row")) + off(to, "rowOff") - y,
                ),
                (None, Some(ext)) => (ext.attr("cx").and_then(emu).unwrap_or(0.0), ext.attr("cy").and_then(emu).unwrap_or(0.0)),
                _ => continue,
            };
            if width <= 0.0 || height <= 0.0 {
                continue;
            }
            let Some(shape) = anchor.elements().find(|e| matches!(e.name.as_str(), "sp" | "pic" | "cxnSp")) else { continue };
            if let Some(content) = simple_shape(shape, width, height, theme, &media) {
                drawings.push(SheetDrawing { x, y, width, height, content });
            }
        }
    }
    Ok(drawings)
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

#[derive(Debug, Clone)]
struct Font {
    props: RunProps,
}

#[derive(Debug, Clone, Default)]
struct Xf {
    font: usize,
    fill: Option<Color>,
    borders: Borders,
    num_fmt: u32,
    h_align: Option<Align>,
    v_align: VAlign,
    wrap: bool,
    indent: f64,
}

struct Styles {
    fonts: Vec<Font>,
    xfs: Vec<Xf>,
    num_fmts: HashMap<u32, String>,
}

impl Styles {
    fn parse(root: Option<&Element>, theme: &Theme) -> Styles {
        let default_font = Font {
            props: RunProps {
                font: Some("Calibri".into()),
                size: Some(11.0),
                color: Some(Color(0, 0, 0)),
                ..RunProps::default()
            },
        };
        let mut styles = Styles {
            fonts: vec![default_font.clone()],
            xfs: vec![Xf::default()],
            num_fmts: HashMap::new(),
        };
        let Some(root) = root else { return styles };

        if let Some(fonts) = root.child("fonts") {
            styles.fonts = fonts
                .children("font")
                .map(|f| Font { props: parse_font(f, theme) })
                .collect();
            if styles.fonts.is_empty() {
                styles.fonts.push(default_font.clone());
            }
        }
        let fills: Vec<Option<Color>> = root
            .child("fills")
            .map(|fills| {
                fills
                    .children("fill")
                    .map(|fill| {
                        let pattern = fill.child("patternFill")?;
                        let kind = pattern.attr("patternType").unwrap_or("none");
                        if kind == "none" {
                            return None;
                        }
                        let fg = pattern.child("fgColor").and_then(|c| parse_color(c, theme));
                        let bg = pattern.child("bgColor").and_then(|c| parse_color(c, theme));
                        if kind == "solid" { fg.or(bg) } else { fg.map(|c| apply_tint(c, 0.5)).or(bg) }
                    })
                    .collect()
            })
            .unwrap_or_default();
        let borders: Vec<Borders> = root
            .child("borders")
            .map(|b| b.children("border").map(|b| parse_borders(b, theme)).collect())
            .unwrap_or_default();
        if let Some(fmts) = root.child("numFmts") {
            for fmt in fmts.children("numFmt") {
                if let (Some(id), Some(code)) = (fmt.attr("numFmtId").and_then(|v| v.parse().ok()), fmt.attr("formatCode")) {
                    styles.num_fmts.insert(id, code.to_string());
                }
            }
        }
        if let Some(xfs) = root.child("cellXfs") {
            styles.xfs = xfs
                .children("xf")
                .map(|xf| {
                    let get = |name: &str| xf.attr(name).and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
                    let mut out = Xf {
                        font: get("fontId").min(styles.fonts.len().saturating_sub(1)),
                        fill: fills.get(get("fillId")).copied().flatten(),
                        borders: borders.get(get("borderId")).copied().unwrap_or_default(),
                        num_fmt: xf.attr("numFmtId").and_then(|v| v.parse().ok()).unwrap_or(0),
                        v_align: VAlign::Bottom,
                        ..Xf::default()
                    };
                    if let Some(align) = xf.child("alignment") {
                        out.h_align = match align.attr("horizontal") {
                            Some("center") | Some("centerContinuous") => Some(Align::Center),
                            Some("right") => Some(Align::Right),
                            Some("left") => Some(Align::Left),
                            Some("justify") | Some("distributed") => Some(Align::Justify),
                            _ => None,
                        };
                        out.v_align = match align.attr("vertical") {
                            Some("top") => VAlign::Top,
                            Some("center") | Some("justify") | Some("distributed") => VAlign::Center,
                            _ => VAlign::Bottom,
                        };
                        out.wrap = matches!(align.attr("wrapText"), Some("1") | Some("true"));
                        out.indent = align.attr("indent").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) * 9.0;
                    }
                    out
                })
                .collect();
            if styles.xfs.is_empty() {
                styles.xfs.push(Xf { v_align: VAlign::Bottom, ..Xf::default() });
            }
        }
        styles
    }

    fn xf(&self, index: usize) -> &Xf {
        self.xfs.get(index).unwrap_or(&self.xfs[0])
    }

    fn font(&self, index: usize) -> &RunProps {
        &self.fonts.get(index).unwrap_or(&self.fonts[0]).props
    }

    fn format_code(&self, id: u32) -> Option<String> {
        self.num_fmts.get(&id).cloned().or_else(|| format::builtin(id).map(str::to_string))
    }
}

fn parse_font(f: &Element, theme: &Theme) -> RunProps {
    let mut props = RunProps {
        font: Some("Calibri".into()),
        size: Some(11.0),
        color: Some(Color(0, 0, 0)),
        ..RunProps::default()
    };
    for child in f.elements() {
        match child.name.as_str() {
            "sz" => props.size = child.attr("val").and_then(|v| v.parse().ok()),
            "name" | "rFont" => props.font = child.attr("val").map(str::to_owned),
            "b" => props.bold = Some(!matches!(child.attr("val"), Some("0") | Some("false"))),
            "i" => props.italic = Some(!matches!(child.attr("val"), Some("0") | Some("false"))),
            "u" => props.underline = Some(child.attr("val") != Some("none")),
            "strike" => props.strike = Some(!matches!(child.attr("val"), Some("0") | Some("false"))),
            "color" => {
                if let Some(c) = parse_color(child, theme) {
                    props.color = Some(c);
                }
            }
            "vertAlign" => {
                props.vertical = Some(match child.attr("val") {
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

fn parse_color(el: &Element, theme: &Theme) -> Option<Color> {
    let tint = el.attr("tint").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
    let base = if let Some(rgb) = el.attr("rgb") {
        let hex = if rgb.len() == 8 { &rgb[2..] } else { rgb };
        Color::parse_hex(hex)?
    } else if let Some(index) = el.attr("theme").and_then(|v| v.parse::<usize>().ok()) {
        let key = ["lt1", "dk1", "lt2", "dk2", "accent1", "accent2", "accent3", "accent4", "accent5", "accent6", "hlink", "folHlink"]
            .get(index)?;
        theme.colors.get(*key).copied().unwrap_or(if index % 2 == 0 { Color(255, 255, 255) } else { Color(0, 0, 0) })
    } else if let Some(index) = el.attr("indexed").and_then(|v| v.parse::<usize>().ok()) {
        indexed_color(index)?
    } else {
        return None;
    };
    Some(apply_tint(base, tint))
}

fn indexed_color(index: usize) -> Option<Color> {
    const PALETTE: [u32; 64] = [
        0x000000, 0xFFFFFF, 0xFF0000, 0x00FF00, 0x0000FF, 0xFFFF00, 0xFF00FF, 0x00FFFF, 0x000000, 0xFFFFFF, 0xFF0000, 0x00FF00,
        0x0000FF, 0xFFFF00, 0xFF00FF, 0x00FFFF, 0x800000, 0x008000, 0x000080, 0x808000, 0x800080, 0x008080, 0xC0C0C0, 0x808080,
        0x9999FF, 0x993366, 0xFFFFCC, 0xCCFFFF, 0x660066, 0xFF8080, 0x0066CC, 0xCCCCFF, 0x000080, 0xFF00FF, 0xFFFF00, 0x00FFFF,
        0x800080, 0x800000, 0x008080, 0x0000FF, 0x00CCFF, 0xCCFFFF, 0xCCFFCC, 0xFFFF99, 0x99CCFF, 0xFF99CC, 0xCC99FF, 0xFFCC99,
        0x3366FF, 0x33CCCC, 0x99CC00, 0xFFCC00, 0xFF9900, 0xFF6600, 0x666699, 0x969696, 0x003366, 0x339966, 0x003300, 0x333300,
        0x993300, 0x993366, 0x333399, 0x333333,
    ];
    match index {
        64 => Some(Color(0, 0, 0)),
        65 => Some(Color(255, 255, 255)),
        i if i < 64 => {
            let v = PALETTE[i];
            Some(Color((v >> 16) as u8, (v >> 8) as u8, v as u8))
        }
        _ => None,
    }
}

fn parse_borders(b: &Element, theme: &Theme) -> Borders {
    let side = |name: &str| -> BorderSide {
        let Some(el) = b.child(name) else { return BorderSide::Unset };
        let width = match el.attr("style") {
            None | Some("none") => return BorderSide::None,
            Some("thin") | Some("dashed") | Some("dotted") | Some("dashDot") | Some("dashDotDot") | Some("slantDashDot") => 0.5,
            Some("medium") | Some("mediumDashed") | Some("mediumDashDot") | Some("mediumDashDotDot") => 1.0,
            Some("thick") => 1.5,
            Some("double") => 1.5,
            Some("hair") => 0.25,
            Some(_) => 0.5,
        };
        let color = el.child("color").and_then(|c| parse_color(c, theme)).unwrap_or(Color(0, 0, 0));
        BorderSide::Line { width, color, style: LineStyle::from_name(el.attr("style").unwrap_or("")) }
    };
    Borders {
        top: side("top"),
        left: side("left"),
        bottom: side("bottom"),
        right: side("right"),
        ..Borders::default()
    }
}

#[derive(Debug, Clone)]
enum CellValue {
    Empty,
    Number(f64),
    Text(Vec<(String, RunProps)>),
    Bool(bool),
    Error(String),
}

#[derive(Debug, Clone)]
struct CellData {
    value: CellValue,
    style: usize,
}

#[derive(Debug, Clone, Default)]
struct RowData {
    height: Option<f64>,
    custom_height: bool,
    hidden: bool,
    cells: BTreeMap<u32, CellData>,
}

#[derive(Debug, Clone)]
struct PageOptions {
    width: f64,
    height: f64,
    margin_left: f64,
    margin_right: f64,
    margin_top: f64,
    margin_bottom: f64,
    margin_header: f64,
    margin_footer: f64,
    scale: f64,
    fit_to_page: bool,
    fit_width: usize,
    fit_height: usize,
    over_then_down: bool,
    grid_lines: bool,
    h_center: bool,
    header: Option<String>,
    footer: Option<String>,
}

impl Default for PageOptions {
    fn default() -> Self {
        PageOptions {
            width: 612.0,
            height: 792.0,
            margin_left: 0.7 * 72.0,
            margin_right: 0.7 * 72.0,
            margin_top: 0.75 * 72.0,
            margin_bottom: 0.75 * 72.0,
            margin_header: 0.3 * 72.0,
            margin_footer: 0.3 * 72.0,
            scale: 1.0,
            fit_to_page: false,
            fit_width: 1,
            fit_height: 1,
            over_then_down: false,
            grid_lines: false,
            h_center: false,
            header: None,
            footer: None,
        }
    }
}

struct SheetDrawing {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    content: DrawingContent,
}

struct Sheet {
    drawings: Vec<SheetDrawing>,
    col_widths: Vec<f64>,
    rows: BTreeMap<u32, RowData>,
    merges: Vec<(u32, u32, u32, u32)>,
    default_row_height: f64,
    page: PageOptions,
    first_row: u32,
    last_row: u32,
    first_col: u32,
    last_col: u32,
    row_breaks: Vec<u32>,
    col_breaks: Vec<u32>,
}

fn column_index(reference: &str) -> Option<(u32, u32)> {
    let mut col = 0u32;
    let mut row = 0u32;
    for c in reference.chars() {
        if c.is_ascii_alphabetic() {
            col = col * 26 + (c.to_ascii_uppercase() as u32 - 'A' as u32 + 1);
        } else if c.is_ascii_digit() {
            row = row * 10 + c.to_digit(10)?;
        }
    }
    if col == 0 || row == 0 { None } else { Some((col, row)) }
}

fn parse_range(range: &str) -> Option<(u32, u32, u32, u32)> {
    let range = range.rsplit('!').next()?.replace('$', "");
    let (a, b) = range.split_once(':').unwrap_or((&range, &range));
    let (c1, r1) = column_index(a)?;
    let (c2, r2) = column_index(b)?;
    Some((r1.min(r2), c1.min(c2), r1.max(r2), c1.max(c2)))
}

fn parse_shared_strings(root: &Element, styles: &Styles) -> Vec<Vec<(String, RunProps)>> {
    root.children("si").map(|si| rich_text(si, styles)).collect()
}

fn rich_text(si: &Element, _styles: &Styles) -> Vec<(String, RunProps)> {
    let mut runs = Vec::new();
    for child in si.elements() {
        match child.name.as_str() {
            "t" => runs.push((child.text(), RunProps::default())),
            "r" => {
                let props = child
                    .child("rPr")
                    .map(|rpr| {
                        let mut p = RunProps::default();
                        for el in rpr.elements() {
                            match el.name.as_str() {
                                "sz" => p.size = el.attr("val").and_then(|v| v.parse().ok()),
                                "rFont" => p.font = el.attr("val").map(str::to_owned),
                                "b" => p.bold = Some(!matches!(el.attr("val"), Some("0") | Some("false"))),
                                "i" => p.italic = Some(!matches!(el.attr("val"), Some("0") | Some("false"))),
                                "u" => p.underline = Some(el.attr("val") != Some("none")),
                                "strike" => p.strike = Some(true),
                                "color" => {
                                    if let Some(rgb) = el.attr("rgb") {
                                        p.color = Color::parse_hex(if rgb.len() == 8 { &rgb[2..] } else { rgb });
                                    }
                                }
                                _ => {}
                            }
                        }
                        p
                    })
                    .unwrap_or_default();
                runs.push((child.child("t").map(|t| t.text()).unwrap_or_default(), props));
            }
            _ => {}
        }
    }
    runs
}

fn parse_sheet(root: &Element, styles: &Styles, shared: &[Vec<(String, RunProps)>], print_area: Option<&str>) -> Sheet {
    let format_pr = root.child("sheetFormatPr");
    let default_col_chars = format_pr
        .and_then(|f| f.attr("defaultColWidth"))
        .and_then(|v| v.parse::<f64>().ok())
        .or_else(|| {
            format_pr
                .and_then(|f| f.attr("baseColWidth"))
                .and_then(|v| v.parse::<f64>().ok())
                .map(|b| b + 0.71)
        })
        .unwrap_or(8.43);
    let default_row_height = format_pr
        .and_then(|f| f.attr("defaultRowHeight"))
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(15.0);
    let all_custom = format_pr
        .and_then(|f| f.attr("customHeight"))
        .map(|v| v == "1" || v == "true")
        .unwrap_or(false);

    let mut col_specs: Vec<(u32, u32, f64, bool)> = Vec::new();
    if let Some(cols) = root.child("cols") {
        for col in cols.children("col") {
            let min = col.attr("min").and_then(|v| v.parse().ok()).unwrap_or(1);
            let max = col.attr("max").and_then(|v| v.parse().ok()).unwrap_or(min);
            let width = col.attr("width").and_then(|v| v.parse::<f64>().ok()).unwrap_or(default_col_chars);
            let hidden = matches!(col.attr("hidden"), Some("1") | Some("true"));
            col_specs.push((min, max, width, hidden));
        }
    }

    let mut rows: BTreeMap<u32, RowData> = BTreeMap::new();
    let mut max_col = 0u32;
    let mut max_row = 0u32;
    let mut min_col = u32::MAX;
    let mut min_row = u32::MAX;
    if let Some(data) = root.child("sheetData") {
        for row in data.children("row") {
            let Some(r) = row.attr("r").and_then(|v| v.parse::<u32>().ok()) else { continue };
            let mut row_data = RowData {
                height: row.attr("ht").and_then(|v| v.parse().ok()),
                custom_height: all_custom || matches!(row.attr("customHeight"), Some("1") | Some("true")),
                hidden: matches!(row.attr("hidden"), Some("1") | Some("true")),
                cells: BTreeMap::new(),
            };
            for cell in row.children("c") {
                let Some((c, _)) = cell.attr("r").and_then(column_index) else { continue };
                let style = cell.attr("s").and_then(|v| v.parse().ok()).unwrap_or(0);
                let kind = cell.attr("t").unwrap_or("n");
                let raw = cell.child("v").map(|v| v.text());
                let value = match kind {
                    "s" => raw
                        .and_then(|v| v.trim().parse::<usize>().ok())
                        .and_then(|i| shared.get(i).cloned())
                        .map(CellValue::Text)
                        .unwrap_or(CellValue::Empty),
                    "inlineStr" => cell
                        .child("is")
                        .map(|is| CellValue::Text(rich_text(is, styles)))
                        .unwrap_or(CellValue::Empty),
                    "str" => raw.map(|v| CellValue::Text(vec![(v, RunProps::default())])).unwrap_or(CellValue::Empty),
                    "b" => raw.map(|v| CellValue::Bool(v.trim() == "1")).unwrap_or(CellValue::Empty),
                    "e" => raw.map(CellValue::Error).unwrap_or(CellValue::Empty),
                    _ => raw
                        .and_then(|v| v.trim().parse::<f64>().ok())
                        .map(CellValue::Number)
                        .unwrap_or(CellValue::Empty),
                };
                let xf = styles.xf(style);
                let formatted = xf.fill.is_some() || has_border(&xf.borders);
                if !matches!(value, CellValue::Empty) || formatted {
                    max_col = max_col.max(c);
                    max_row = max_row.max(r);
                    min_col = min_col.min(c);
                    min_row = min_row.min(r);
                }
                row_data.cells.insert(c, CellData { value, style });
            }
            rows.insert(r, row_data);
        }
    }

    let mut merges = Vec::new();
    if let Some(mc) = root.child("mergeCells") {
        for m in mc.children("mergeCell") {
            if let Some(range) = m.attr("ref").and_then(parse_range) {
                merges.push(range);
                max_col = max_col.max(range.3);
                max_row = max_row.max(range.2);
            }
        }
    }

    let (mut first_row, mut first_col, mut last_row, mut last_col) = (1, 1, max_row, max_col);
    if min_row != u32::MAX {
        first_row = 1;
        first_col = 1;
    }
    let width_of = |c: u32| -> f64 {
        let spec = col_specs.iter().find(|(min, max, _, _)| c >= *min && c <= *max);
        match spec {
            Some((_, _, _, true)) => 0.0,
            Some((_, _, w, _)) => chars_to_pt(*w),
            None => chars_to_pt(default_col_chars),
        }
    };
    for (_, row) in &rows {
        for (c, cell) in &row.cells {
            let xf = styles.xf(cell.style);
            if xf.wrap || !matches!(xf.h_align, None | Some(Align::Left)) {
                continue;
            }
            let CellValue::Text(parts) = &cell.value else { continue };
            let font = styles.font(xf.font);
            let size = font.size.unwrap_or(11.0);
            let chars: usize = parts.iter().map(|(t, _)| t.chars().count()).sum();
            let mut needed = chars as f64 * size * 0.5 + 3.0;
            let mut col = *c;
            needed -= width_of(col);
            while needed > 0.0 && col < 200 {
                col += 1;
                if row.cells.get(&col).map(|n| !matches!(n.value, CellValue::Empty)).unwrap_or(false) {
                    break;
                }
                needed -= width_of(col);
            }
            if col > *c && needed <= 0.0 {
                max_col = max_col.max(col);
            } else if col > *c {
                max_col = max_col.max(col);
            }
        }
    }
    last_col = last_col.max(max_col);
    if let Some(area) = print_area.and_then(parse_range) {
        first_row = area.0;
        first_col = area.1;
        last_row = area.2;
        last_col = area.3;
    }

    let mut col_widths = Vec::new();
    for c in 1..=last_col.max(1) {
        let spec = col_specs.iter().find(|(min, max, _, _)| c >= *min && c <= *max);
        let width = match spec {
            Some((_, _, _, true)) => 0.0,
            Some((_, _, w, _)) => chars_to_pt(*w),
            None => chars_to_pt(default_col_chars),
        };
        col_widths.push(width);
    }

    let mut page = PageOptions::default();
    if root.child("pageMargins").is_none() {
        page.margin_left = 53.3;
        page.margin_right = 53.3;
        page.margin_top = 70.9;
        page.margin_bottom = 70.9;
        page.margin_header = 56.7;
        page.margin_footer = 56.7;
    }
    if let Some(m) = root.child("pageMargins") {
        let inches = |name: &str, default: f64| m.attr(name).and_then(|v| v.parse::<f64>().ok()).map(|v| v * 72.0).unwrap_or(default);
        page.margin_left = inches("left", page.margin_left);
        page.margin_right = inches("right", page.margin_right);
        page.margin_top = inches("top", page.margin_top);
        page.margin_bottom = inches("bottom", page.margin_bottom);
        page.margin_header = inches("header", page.margin_header);
        page.margin_footer = inches("footer", page.margin_footer);
    }
    if let Some(setup) = root.child("pageSetup") {
        let (w, h) = paper_size(setup.attr("paperSize").and_then(|v| v.parse().ok()).unwrap_or(1));
        if setup.attr("orientation") == Some("landscape") {
            page.width = h;
            page.height = w;
        } else {
            page.width = w;
            page.height = h;
        }
        page.scale = setup.attr("scale").and_then(|v| v.parse::<f64>().ok()).map(|v| (v / 100.0).clamp(0.1, 4.0)).unwrap_or(1.0);
        page.fit_width = setup.attr("fitToWidth").and_then(|v| v.parse().ok()).unwrap_or(1);
        page.fit_height = setup.attr("fitToHeight").and_then(|v| v.parse().ok()).unwrap_or(1);
        page.over_then_down = setup.attr("pageOrder") == Some("overThenDown");
    }
    page.fit_to_page = root
        .child("sheetPr")
        .and_then(|p| p.child("pageSetUpPr"))
        .and_then(|p| p.attr("fitToPage"))
        .map(|v| v == "1" || v == "true")
        .unwrap_or(false);
    if let Some(opts) = root.child("printOptions") {
        page.grid_lines = matches!(opts.attr("gridLines"), Some("1") | Some("true"));
        page.h_center = matches!(opts.attr("horizontalCentered"), Some("1") | Some("true"));
    }
    if let Some(hf) = root.child("headerFooter") {
        page.header = hf.child("oddHeader").map(|h| h.text()).filter(|t| !t.is_empty());
        page.footer = hf.child("oddFooter").map(|f| f.text()).filter(|t| !t.is_empty());
    }
    let breaks = |name: &str| -> Vec<u32> {
        root.child(name)
            .map(|b| b.children("brk").filter_map(|k| k.attr("id").and_then(|v| v.parse().ok())).collect())
            .unwrap_or_default()
    };

    Sheet {
        drawings: Vec::new(),
        col_widths,
        rows,
        merges,
        default_row_height,
        page,
        first_row,
        last_row,
        first_col,
        last_col,
        row_breaks: breaks("rowBreaks"),
        col_breaks: breaks("colBreaks"),
    }
}

fn has_border(b: &Borders) -> bool {
    [b.top, b.left, b.bottom, b.right].iter().any(|s| matches!(s, BorderSide::Line { .. }))
}

fn chars_to_pt(chars: f64) -> f64 {
    ((chars * 7.0 + 5.0).round()) * PX_TO_PT
}

fn paper_size(code: u32) -> (f64, f64) {
    match code {
        5 => (612.0, 1008.0),
        3 => (792.0, 1224.0),
        7 => (522.0, 756.0),
        8 => (841.9, 1190.6),
        9 => (595.3, 841.9),
        11 => (419.5, 595.3),
        12 => (1031.8, 1459.8),
        13 => (515.9, 728.5),
        14 => (612.0, 936.0),
        _ => (612.0, 792.0),
    }
}

fn paginate(sheet: &Sheet, name: &str, styles: &Styles) -> Vec<Section> {
    let page = &sheet.page;
    if sheet.last_row == 0 || sheet.last_col == 0 {
        let mut section = Section {
            page: PageSetup {
                width: page.width,
                height: page.height,
                margin: Margins {
                    top: page.margin_top,
                    right: page.margin_right,
                    bottom: page.margin_bottom,
                    left: page.margin_left,
                    header: page.margin_header,
                    footer: page.margin_footer,
                },
            },
            blocks: vec![Block::Paragraph(Paragraph {
                props: ParagraphProps::default(),
                mark: RunProps { size: Some(1.0), ..RunProps::default() },
                inlines: Vec::new(),
                anchors: Vec::new(),
                list: None,
            })],
            columns: 1,
            content_scale: 1.0,
            ..Section::default()
        };
        section.header_default = page.header.as_deref().map(|h| header_footer(h, name, page.width - page.margin_left - page.margin_right));
        section.footer_default = page.footer.as_deref().map(|f| header_footer(f, name, page.width - page.margin_left - page.margin_right));
        return vec![section];
    }
    let printable_w = page.width - page.margin_left - page.margin_right;
    let printable_h = page.height - page.margin_top - page.margin_bottom;

    let cols: Vec<u32> = (sheet.first_col..=sheet.last_col).collect();
    let rows: Vec<u32> = (sheet.first_row..=sheet.last_row)
        .filter(|r| !sheet.rows.get(r).map(|d| d.hidden).unwrap_or(false))
        .collect();
    let col_w = |c: u32| sheet.col_widths.get((c - 1) as usize).copied().unwrap_or(48.0);
    let row_h = |r: u32| {
        sheet
            .rows
            .get(&r)
            .and_then(|d| d.height)
            .unwrap_or(sheet.default_row_height)
    };

    let total_w: f64 = cols.iter().map(|c| col_w(*c)).sum();
    let total_h: f64 = rows.iter().map(|r| row_h(*r)).sum();
    let mut scale = page.scale;
    if page.fit_to_page {
        let sw = if page.fit_width > 0 { (printable_w * page.fit_width as f64) / total_w.max(1.0) } else { f64::INFINITY };
        let sh = if page.fit_height > 0 { (printable_h * page.fit_height as f64) / total_h.max(1.0) } else { f64::INFINITY };
        scale = sw.min(sh).min(1.0).max(0.1);
        if !scale.is_finite() {
            scale = 1.0;
        }
    }

    let col_groups = group(&cols, |c| col_w(c) * scale, printable_w, &sheet.col_breaks);
    let row_groups = group(&rows, |r| row_h(r) * scale, printable_h, &sheet.row_breaks);

    let mut order: Vec<(&Vec<u32>, &Vec<u32>)> = Vec::new();
    if page.over_then_down {
        for rg in &row_groups {
            for cg in &col_groups {
                order.push((rg, cg));
            }
        }
    } else {
        for cg in &col_groups {
            for rg in &row_groups {
                order.push((rg, cg));
            }
        }
    }

    let header = page.header.as_deref().map(|h| header_footer(h, name, page.width - page.margin_left - page.margin_right));
    let footer = page.footer.as_deref().map(|f| header_footer(f, name, page.width - page.margin_left - page.margin_right));

    let mut sections = Vec::new();
    for (rg, cg) in order {
        let table = build_table(sheet, rg, cg, styles, scale);
        let origin_x = sheet_x(sheet, cg[0] - 1);
        let origin_y = sheet_y(sheet, rg[0] - 1);
        let anchors: Vec<Anchor> = sheet
            .drawings
            .iter()
            .filter(|d| {
                let (col, row) = cell_at(sheet, d.x + 0.01, d.y + 0.01);
                cg.contains(&col) && rg.contains(&row)
            })
            .map(|d| {
                page_anchor(
                    page.margin_left + (d.x - origin_x) * scale,
                    page.margin_top + (d.y - origin_y) * scale,
                    d.width * scale,
                    d.height * scale,
                    d.content.clone(),
                    0.0,
                    false,
                    false,
                    false,
                )
            })
            .collect();
        let mut section = Section {
            page: PageSetup {
                width: page.width,
                height: page.height,
                margin: Margins {
                    top: page.margin_top,
                    right: page.margin_right,
                    bottom: page.margin_bottom,
                    left: page.margin_left,
                    header: page.margin_header,
                    footer: page.margin_footer,
                },
            },
            blocks: vec![Block::Table(table)],
            columns: 1,
            content_scale: scale,
            anchors,
            ..Section::default()
        };
        section.header_default = header.clone();
        section.footer_default = footer.clone();
        sections.push(section);
    }
    sections
}

fn group(items: &[u32], size: impl Fn(u32) -> f64, limit: f64, breaks: &[u32]) -> Vec<Vec<u32>> {
    let mut groups: Vec<Vec<u32>> = Vec::new();
    let mut current: Vec<u32> = Vec::new();
    let mut used = 0.0;
    for &item in items {
        let s = size(item);
        if !current.is_empty() && (used + s > limit + 0.01 || breaks.contains(&(item - 1))) {
            groups.push(std::mem::take(&mut current));
            used = 0.0;
        }
        current.push(item);
        used += s;
    }
    if !current.is_empty() {
        groups.push(current);
    }
    groups
}

fn build_table(sheet: &Sheet, rows: &[u32], cols: &[u32], styles: &Styles, scale: f64) -> Table {
    let mut table = Table {
        columns: cols.iter().map(|c| sheet.col_widths.get((*c - 1) as usize).copied().unwrap_or(48.0)).collect(),
        cell_margins: CellMargins { top: Some(0.75), left: Some(1.5), bottom: Some(0.75), right: Some(1.5) },
        ..Table::default()
    };
    if sheet.page.h_center {
        let total: f64 = table.columns.iter().sum();
        let printable = (sheet.page.width - sheet.page.margin_left - sheet.page.margin_right) / scale;
        table.indent = ((printable - total) / 2.0).max(0.0);
    }
    let grid_border = BorderSide::Line { width: 0.25, color: Color(128, 128, 128), style: LineStyle::Solid };

    for &r in rows {
        let data = sheet.rows.get(&r);
        let mut row = Row {
            height: Some(data.and_then(|d| d.height).unwrap_or(sheet.default_row_height)),
            exact_height: data.map(|d| d.custom_height).unwrap_or(false),
            ..Row::default()
        };
        let mut skip_until: Option<u32> = None;
        for &c in cols {
            if let Some(until) = skip_until {
                if c <= until {
                    continue;
                }
                skip_until = None;
            }
            let covered = sheet.merges.iter().find(|(r1, c1, r2, c2)| r >= *r1 && r <= *r2 && c >= *c1 && c <= *c2);
            let mut cell = Cell { span: 1, valign: VAlign::Bottom, ..Cell::default() };
            if let Some(&(r1, c1, r2, c2)) = covered {
                if c != c1 {
                    continue;
                }
                if r != r1 {
                    cell.vertical_merge = Some(VerticalMerge::Continue);
                    cell.span = (c2.min(*cols.last().unwrap()) - c1 + 1) as usize;
                    skip_until = Some(c2);
                    row.cells.push(cell);
                    continue;
                }
                cell.span = (c2.min(*cols.last().unwrap()) - c1 + 1) as usize;
                if r2 > r1 {
                    cell.vertical_merge = Some(VerticalMerge::Restart);
                }
                skip_until = Some(c2);
            }
            let cell_data = data.and_then(|d| d.cells.get(&c));
            let xf = styles.xf(cell_data.map(|d| d.style).unwrap_or(0));
            let font = styles.font(xf.font).clone();
            cell.shading = xf.fill;
            cell.borders = xf.borders;
            cell.valign = xf.v_align;
            cell.no_wrap = !xf.wrap;
            if sheet.page.grid_lines {
                for side in [&mut cell.borders.top, &mut cell.borders.left, &mut cell.borders.bottom, &mut cell.borders.right] {
                    if *side == BorderSide::Unset {
                        *side = grid_border;
                    }
                }
            }
            if xf.indent > 0.0 {
                cell.margins.left = Some(1.5 + xf.indent);
            }
            if !xf.wrap {
                let mut remaining = 0.0;
                let mut blocked = false;
                for &next in cols.iter().filter(|n| **n >= c) {
                    if next > c
                        && data
                            .and_then(|d| d.cells.get(&next))
                            .map(|n| !matches!(n.value, CellValue::Empty))
                            .unwrap_or(false)
                    {
                        blocked = true;
                    }
                    if blocked {
                        break;
                    }
                    remaining += sheet.col_widths.get((next - 1) as usize).copied().unwrap_or(48.0);
                }
                cell.overflow_width = Some((remaining - 3.0).max(1.0));
            }

            let (runs, default_align) = match cell_data.map(|d| &d.value) {
                Some(CellValue::Number(v)) => {
                    let code = styles.format_code(xf.num_fmt).unwrap_or_else(|| "General".into());
                    let formatted = format::format_number(*v, &code);
                    let mut props = font.clone();
                    if let Some(c) = formatted.color {
                        props.color = Some(c);
                    }
                    (vec![(formatted.text, props)], Align::Right)
                }
                Some(CellValue::Text(parts)) => {
                    let code = styles.format_code(xf.num_fmt).unwrap_or_default();
                    let runs = parts
                        .iter()
                        .map(|(text, overrides)| {
                            let mut props = font.clone();
                            props.merge(overrides);
                            let text = if parts.len() == 1 && code.contains('@') { format::format_text(text, &code) } else { text.clone() };
                            (text, props)
                        })
                        .collect();
                    (runs, Align::Left)
                }
                Some(CellValue::Bool(b)) => (vec![(if *b { "TRUE".into() } else { "FALSE".into() }, font.clone())], Align::Center),
                Some(CellValue::Error(e)) => (vec![(e.clone(), font.clone())], Align::Center),
                _ => (Vec::new(), Align::Left),
            };
            let align = xf.h_align.unwrap_or(default_align);
            cell.halign = Some(align);
            if !runs.is_empty() {
                let inlines: Vec<Inline> = runs
                    .into_iter()
                    .filter(|(t, _)| !t.is_empty())
                    .flat_map(|(text, props)| {
                        let mut out = Vec::new();
                        for (i, line) in text.split('\n').enumerate() {
                            if i > 0 {
                                out.push(Inline::LineBreak);
                            }
                            out.push(Inline::Text { text: line.to_string(), props: props.clone() });
                        }
                        out
                    })
                    .collect();
                cell.blocks.push(Block::Paragraph(Paragraph {
                    props: ParagraphProps {
                        align: Some(align),
                        line_spacing: Some(LineSpacing::Multiple(1.0)),
                        ..ParagraphProps::default()
                    },
                    mark: font.clone(),
                    inlines,
                    anchors: Vec::new(),
                    list: None,
                }));
            }
            row.cells.push(cell);
        }
        table.rows.push(row);
    }
    table
}

fn header_footer(code: &str, sheet_name: &str, width: f64) -> Vec<Block> {
    let mut parts: [String; 3] = [String::new(), String::new(), String::new()];
    let mut current = 1usize;
    let mut chars = code.chars().peekable();
    let mut fields: [Vec<Inline>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    let mut props = RunProps {
        font: Some("Calibri".into()),
        size: Some(11.0),
        ..RunProps::default()
    };
    let flush = |parts: &mut [String; 3], fields: &mut [Vec<Inline>; 3], current: usize, props: &RunProps| {
        if !parts[current].is_empty() {
            fields[current].push(Inline::Text { text: std::mem::take(&mut parts[current]), props: props.clone() });
        }
    };
    while let Some(c) = chars.next() {
        if c != '&' {
            parts[current].push(c);
            continue;
        }
        match chars.next() {
            Some('L') => { flush(&mut parts, &mut fields, current, &props); current = 0; }
            Some('C') => { flush(&mut parts, &mut fields, current, &props); current = 1; }
            Some('R') => { flush(&mut parts, &mut fields, current, &props); current = 2; }
            Some('P') => { flush(&mut parts, &mut fields, current, &props); fields[current].push(Inline::Field { kind: FieldKind::Page, props: props.clone() }); }
            Some('N') => { flush(&mut parts, &mut fields, current, &props); fields[current].push(Inline::Field { kind: FieldKind::NumPages, props: props.clone() }); }
            Some('A') => parts[current].push_str(sheet_name),
            Some('&') => parts[current].push('&'),
            Some('B') => { flush(&mut parts, &mut fields, current, &props); props.bold = Some(props.bold != Some(true)); }
            Some('I') => { flush(&mut parts, &mut fields, current, &props); props.italic = Some(props.italic != Some(true)); }
            Some('U') => { flush(&mut parts, &mut fields, current, &props); props.underline = Some(props.underline != Some(true)); }
            Some('"') => {
                let mut spec = String::new();
                for q in chars.by_ref() {
                    if q == '"' {
                        break;
                    }
                    spec.push(q);
                }
                flush(&mut parts, &mut fields, current, &props);
                let (font, style) = spec.split_once(',').unwrap_or((spec.as_str(), ""));
                if !font.is_empty() && font != "-" {
                    props.font = Some(font.to_string());
                }
                let style = style.to_ascii_lowercase();
                props.bold = Some(style.contains("bold"));
                props.italic = Some(style.contains("italic"));
            }
            Some(d) if d.is_ascii_digit() => {
                let mut digits = String::from(d);
                while let Some(n) = chars.peek() {
                    if n.is_ascii_digit() { digits.push(*n); chars.next(); } else { break; }
                }
                flush(&mut parts, &mut fields, current, &props);
                if let Ok(size) = digits.parse::<f64>() {
                    props.size = Some(size);
                }
            }
            Some(_) => {}
            None => break,
        }
    }
    flush(&mut parts, &mut fields, current, &props);

    let mut table = Table {
        columns: vec![width / 3.0; 3],
        cell_margins: CellMargins { top: Some(0.0), left: Some(0.0), bottom: Some(0.0), right: Some(0.0) },
        ..Table::default()
    };
    let mut row = Row::default();
    for (i, inlines) in fields.into_iter().enumerate() {
        let align = [Align::Left, Align::Center, Align::Right][i];
        let mut cell = Cell { span: 1, halign: Some(align), ..Cell::default() };
        if !inlines.is_empty() {
            cell.blocks.push(Block::Paragraph(Paragraph {
                props: ParagraphProps { align: Some(align), ..ParagraphProps::default() },
                mark: props.clone(),
                inlines,
                anchors: Vec::new(),
                list: None,
            }));
        }
        row.cells.push(cell);
    }
    table.rows.push(row);
    vec![Block::Table(table)]
}
