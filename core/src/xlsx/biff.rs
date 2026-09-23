use super::biff_art;
use super::biff_cf;
use super::{
    auto_row_heights, border_side, column_width, digit_width, has_border, indexed_color, paper_size, sheet_extent, workbook_defaults,
    workbook_document, CellData, CellValue, Dxf, Font, PageOptions, RowData, Sheet, SheetDrawing, Styles, Xf,
};
use crate::error::Error;
use crate::model::*;
use std::collections::{BTreeMap, HashMap};
use std::io::{Cursor, Read};

const BOF: u16 = 0x0809;
const EOF: u16 = 0x000A;
const CONTINUE: u16 = 0x003C;
const FILEPASS: u16 = 0x002F;
const FONT: u16 = 0x0031;
const FORMAT: u16 = 0x041E;
const XF: u16 = 0x00E0;
const PALETTE: u16 = 0x0092;
const BOUNDSHEET: u16 = 0x0085;
const SST: u16 = 0x00FC;
const NAME: u16 = 0x0018;
const DEFCOLWIDTH: u16 = 0x0055;
const STANDARDWIDTH: u16 = 0x0099;
const COLINFO: u16 = 0x007D;
const DEFAULTROWHEIGHT: u16 = 0x0225;
const ROW: u16 = 0x0208;
const NUMBER: u16 = 0x0203;
const RK: u16 = 0x027E;
const MULRK: u16 = 0x00BD;
const LABELSST: u16 = 0x00FD;
const LABEL: u16 = 0x0204;
const RSTRING: u16 = 0x00D6;
const BLANK: u16 = 0x0201;
const MULBLANK: u16 = 0x00BE;
const BOOLERR: u16 = 0x0205;
const FORMULA: u16 = 0x0006;
const STRING: u16 = 0x0207;
const MERGECELLS: u16 = 0x00E5;
const LEFTMARGIN: u16 = 0x0026;
const RIGHTMARGIN: u16 = 0x0027;
const TOPMARGIN: u16 = 0x0028;
const BOTTOMMARGIN: u16 = 0x0029;
const SETUP: u16 = 0x00A1;
const WSBOOL: u16 = 0x0081;
const PRINTGRIDLINES: u16 = 0x002B;
const HCENTER: u16 = 0x0083;
const HEADER: u16 = 0x0014;
const FOOTER: u16 = 0x0015;
const HORIZONTALPAGEBREAKS: u16 = 0x001B;
const VERTICALPAGEBREAKS: u16 = 0x001A;
const OBJ: u16 = 0x005D;
const TXO: u16 = 0x01B6;
const NOTE: u16 = 0x001C;
const MSODRAWINGGROUP: u16 = 0x00EB;
const MSODRAWING: u16 = 0x00EC;
const CONDFMT: u16 = 0x01B0;
const CF: u16 = 0x01B1;

const BORDER_STYLES: [&str; 14] = [
    "none",
    "thin",
    "medium",
    "dashed",
    "dotted",
    "thick",
    "double",
    "hair",
    "mediumDashed",
    "dashDot",
    "mediumDashDot",
    "dashDotDot",
    "mediumDashDotDot",
    "slantDashDot",
];

struct Record {
    kind: u16,
    offset: usize,
    parts: Vec<Vec<u8>>,
}

impl Record {
    fn data(&self) -> &[u8] {
        &self.parts[0]
    }

    fn reader(&self) -> Reader<'_> {
        Reader { parts: &self.parts, part: 0, pos: 0 }
    }
}

struct Reader<'a> {
    parts: &'a [Vec<u8>],
    part: usize,
    pos: usize,
}

impl Reader<'_> {
    fn remaining_in_part(&self) -> usize {
        self.parts.get(self.part).map_or(0, |p| p.len().saturating_sub(self.pos))
    }

    fn next_part(&mut self) -> bool {
        if self.part + 1 < self.parts.len() {
            self.part += 1;
            self.pos = 0;
            true
        } else {
            false
        }
    }

    fn u8(&mut self) -> Option<u8> {
        while self.remaining_in_part() == 0 {
            if !self.next_part() {
                return None;
            }
        }
        let value = self.parts[self.part][self.pos];
        self.pos += 1;
        Some(value)
    }

    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes([self.u8()?, self.u8()?]))
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes([self.u8()?, self.u8()?, self.u8()?, self.u8()?]))
    }

    fn skip(&mut self, mut count: usize) {
        while count > 0 {
            let here = self.remaining_in_part();
            if here == 0 {
                if !self.next_part() {
                    return;
                }
                continue;
            }
            let step = here.min(count);
            self.pos += step;
            count -= step;
        }
    }

    fn chars(&mut self, count: usize, mut wide: bool) -> Option<String> {
        let mut units: Vec<u16> = Vec::with_capacity(count);
        while units.len() < count {
            if self.remaining_in_part() == 0 {
                if !self.next_part() {
                    break;
                }
                wide = self.u8()? & 1 == 1;
                continue;
            }
            if wide {
                units.push(self.u16()?);
            } else {
                units.push(self.u8()? as u16);
            }
        }
        Some(String::from_utf16_lossy(&units))
    }

    fn unicode_string(&mut self, count: usize) -> Option<(String, Vec<(usize, usize)>)> {
        let flags = self.u8()?;
        let runs = if flags & 0x08 != 0 { self.u16()? as usize } else { 0 };
        let ext = if flags & 0x04 != 0 { self.u32()? as usize } else { 0 };
        let text = self.chars(count, flags & 1 == 1)?;
        let mut formatting = Vec::with_capacity(runs);
        for _ in 0..runs {
            let at = self.u16()? as usize;
            let font = self.u16()? as usize;
            formatting.push((at, font));
        }
        self.skip(ext);
        Some((text, formatting))
    }
}

fn records(stream: &[u8]) -> Vec<Record> {
    let mut out: Vec<Record> = Vec::new();
    let mut pos = 0;
    while pos + 4 <= stream.len() {
        let kind = u16::from_le_bytes([stream[pos], stream[pos + 1]]);
        let len = u16::from_le_bytes([stream[pos + 2], stream[pos + 3]]) as usize;
        let start = pos + 4;
        let end = (start + len).min(stream.len());
        let data = stream[start..end].to_vec();
        if kind == CONTINUE {
            if let Some(last) = out.last_mut() {
                last.parts.push(data);
            }
        } else {
            out.push(Record { kind, offset: pos, parts: vec![data] });
        }
        pos = end;
    }
    out
}

fn le16(data: &[u8], at: usize) -> u16 {
    data.get(at..at + 2).map_or(0, |b| u16::from_le_bytes([b[0], b[1]]))
}

fn le32(data: &[u8], at: usize) -> u32 {
    data.get(at..at + 4).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn f64_at(data: &[u8], at: usize) -> Option<f64> {
    data.get(at..at + 8).map(|b| f64::from_le_bytes(b.try_into().unwrap()))
}

fn rk_value(rk: u32) -> f64 {
    let value = if rk & 2 != 0 {
        ((rk as i32) >> 2) as f64
    } else {
        f64::from_bits(((rk & 0xFFFF_FFFC) as u64) << 32)
    };
    if rk & 1 != 0 { value / 100.0 } else { value }
}

struct SheetEntry {
    name: String,
    offset: usize,
    visible: bool,
    worksheet: bool,
}

struct Globals {
    palette: Palette,
    styles: Styles,
    blips: Vec<Option<ImageData>>,
    strings: Vec<Vec<(String, RunProps)>>,
    sheets: Vec<SheetEntry>,
    print_areas: HashMap<usize, (u32, u32, u32, u32)>,
}

struct Palette(HashMap<usize, Color>);

impl Palette {
    fn color(&self, index: usize) -> Option<Color> {
        match index {
            0x40 | 0x7FFF => None,
            i => self.0.get(&i).copied().or_else(|| indexed_color(i)),
        }
    }
}

pub fn read(bytes: &[u8]) -> Result<Document, Error> {
    let mut cfb = cfb::CompoundFile::open(Cursor::new(bytes)).map_err(|e| Error::new(format!("not an xls container: {e}")))?;
    if cfb.exists("/Book") && !cfb.exists("/Workbook") {
        return Err(Error::new("Excel 5.0/95 workbooks are not supported, only Excel 97-2003"));
    }
    let mut stream = Vec::new();
    cfb.open_stream("/Workbook")
        .map_err(|e| Error::new(format!("xls has no Workbook stream: {e}")))?
        .read_to_end(&mut stream)
        .map_err(|e| Error::new(format!("reading the xls Workbook stream: {e}")))?;
    let records = records(&stream);
    match records.first() {
        Some(r) if r.kind == BOF && le16(r.data(), 0) == 0x0600 => {}
        Some(r) if r.kind == BOF => return Err(Error::new("only Excel 97-2003 (BIFF8) workbooks are supported")),
        _ => return Err(Error::new("xls Workbook stream does not start with a BOF record")),
    }
    if records.iter().any(|r| r.kind == FILEPASS) {
        return Err(Error::new("encrypted xls workbooks are not supported"));
    }
    let mut globals = read_globals(&records);
    let by_offset: HashMap<usize, usize> = records.iter().enumerate().map(|(i, r)| (r.offset, i)).collect();

    let mut parsed = Vec::new();
    for (index, entry) in globals.sheets.iter().enumerate() {
        if !entry.visible || !entry.worksheet {
            continue;
        }
        let Some(&start) = by_offset.get(&entry.offset) else { continue };
        let (mut sheet, dxfs) = read_sheet(&records[start..], &globals, globals.print_areas.get(&index).copied());
        let base = globals.styles.dxfs.len();
        for rule in &mut sheet.conditional {
            rule.dxf += base;
        }
        globals.styles.dxfs.extend(dxfs);
        auto_row_heights(&mut sheet, &globals.styles);
        parsed.push((entry.name.clone(), sheet));
    }
    Ok(workbook_document(workbook_defaults(), &parsed, &globals.styles))
}

fn read_globals(records: &[Record]) -> Globals {
    let mut palette = Palette(HashMap::new());
    for record in records.iter().take_while(|r| r.kind != EOF) {
        if record.kind == PALETTE {
            let data = record.data();
            let count = le16(data, 0) as usize;
            for i in 0..count {
                let at = 2 + i * 4;
                if let Some(rgb) = data.get(at..at + 3) {
                    palette.0.insert(8 + i, Color(rgb[0], rgb[1], rgb[2]));
                }
            }
        }
    }

    let mut fonts: Vec<Font> = Vec::new();
    let mut xfs: Vec<Xf> = Vec::new();
    let mut num_fmts = HashMap::new();
    let mut strings = Vec::new();
    let mut sheets = Vec::new();
    let mut print_areas = HashMap::new();
    let mut raw_xfs: Vec<Vec<u8>> = Vec::new();
    let mut drawing_group: Vec<u8> = Vec::new();

    for record in records.iter().skip(1).take_while(|r| r.kind != EOF) {
        let data = record.data();
        match record.kind {
            FONT => fonts.push(Font { props: font_props(record, &palette) }),
            FORMAT => {
                let id = le16(data, 0) as u32;
                let mut reader = record.reader();
                reader.skip(2);
                if let Some(count) = reader.u16() {
                    if let Some((code, _)) = reader.unicode_string(count as usize) {
                        num_fmts.insert(id, code);
                    }
                }
            }
            XF => raw_xfs.push(data.to_vec()),
            BOUNDSHEET => {
                let mut reader = record.reader();
                reader.skip(6);
                let name = reader
                    .u8()
                    .and_then(|count| reader.unicode_string(count as usize))
                    .map(|(n, _)| n)
                    .unwrap_or_else(|| "Sheet".into());
                sheets.push(SheetEntry {
                    name,
                    offset: le32(data, 0) as usize,
                    visible: data.get(4).copied().unwrap_or(0) & 0x03 == 0,
                    worksheet: data.get(5).copied().unwrap_or(0) == 0,
                });
            }
            SST => strings = shared_strings(record, &fonts),
            MSODRAWINGGROUP => record.parts.iter().for_each(|part| drawing_group.extend_from_slice(part)),
            NAME => {
                if let Some((sheet, area)) = print_area(data) {
                    print_areas.insert(sheet, area);
                }
            }
            _ => {}
        }
    }

    if fonts.is_empty() {
        fonts.push(Font {
            props: RunProps { font: Some("Arial".into()), size: Some(10.0), color: Some(Color(0, 0, 0)), ..RunProps::default() },
        });
    }
    let font_count = fonts.len();
    for data in &raw_xfs {
        xfs.push(cell_format(data, &palette, font_count));
    }
    if xfs.is_empty() {
        xfs.push(Xf { v_align: VAlign::Bottom, ..Xf::default() });
    }
    Globals {
        palette,
        blips: biff_art::blip_store(&drawing_group),
        styles: Styles { fonts, xfs, num_fmts, dxfs: Vec::new() },
        strings,
        sheets,
        print_areas,
    }
}

fn font_props(record: &Record, palette: &Palette) -> RunProps {
    let data = record.data();
    let flags = le16(data, 2);
    let mut reader = record.reader();
    reader.skip(14);
    let name = reader.u8().and_then(|count| reader.unicode_string(count as usize)).map(|(n, _)| n);
    RunProps {
        font: name.filter(|n| !n.is_empty()).or_else(|| Some("Arial".into())),
        size: Some(le16(data, 0) as f64 / 20.0),
        bold: Some(le16(data, 6) >= 700),
        italic: Some(flags & 0x02 != 0),
        strike: Some(flags & 0x08 != 0),
        underline: Some(data.get(10).copied().unwrap_or(0) != 0),
        color: Some(palette.color(le16(data, 4) as usize).unwrap_or(Color(0, 0, 0))),
        vertical: match le16(data, 8) {
            1 => Some(VerticalAlign::Superscript),
            2 => Some(VerticalAlign::Subscript),
            _ => None,
        },
        ..RunProps::default()
    }
}

fn font_index(ifnt: usize) -> usize {
    if ifnt >= 4 { ifnt - 1 } else { ifnt }
}

fn cell_format(data: &[u8], palette: &Palette, font_count: usize) -> Xf {
    let align = data.get(6).copied().unwrap_or(0);
    let indent = data.get(8).copied().unwrap_or(0) & 0x0F;
    let border1 = le32(data, 10);
    let border2 = le32(data, 14);
    let colors = le16(data, 18);
    let side = |style: u32, color: u32| -> BorderSide {
        let name = BORDER_STYLES.get(style as usize).copied().unwrap_or("thin");
        border_side(name, palette.color(color as usize).unwrap_or(Color(0, 0, 0)))
    };
    let pattern = (border2 >> 26) & 0x3F;
    let fore = palette.color((colors & 0x7F) as usize);
    let back = palette.color(((colors >> 7) & 0x7F) as usize);
    let fill = match pattern {
        0 => None,
        1 => fore.or(back),
        _ => fore.map(|c| crate::pptx::color::apply_tint(c, 0.5)).or(back),
    };
    Xf {
        font: font_index(le16(data, 0) as usize).min(font_count.saturating_sub(1)),
        fill,
        borders: Borders {
            left: side(border1 & 0x0F, (border1 >> 16) & 0x7F),
            right: side((border1 >> 4) & 0x0F, (border1 >> 23) & 0x7F),
            top: side((border1 >> 8) & 0x0F, border2 & 0x7F),
            bottom: side((border1 >> 12) & 0x0F, (border2 >> 7) & 0x7F),
            ..Borders::default()
        },
        num_fmt: le16(data, 2) as u32,
        h_align: match align & 0x07 {
            1 | 4 => Some(Align::Left),
            2 | 6 => Some(Align::Center),
            3 => Some(Align::Right),
            5 | 7 => Some(Align::Justify),
            _ => None,
        },
        v_align: match (align >> 4) & 0x07 {
            0 => VAlign::Top,
            1 | 3 | 4 => VAlign::Center,
            _ => VAlign::Bottom,
        },
        wrap: align & 0x08 != 0,
        indent: indent as f64 * 9.0,
    }
}

fn runs_to_text(text: String, runs: &[(usize, usize)], fonts: &[Font]) -> Vec<(String, RunProps)> {
    if runs.is_empty() {
        return vec![(text, RunProps::default())];
    }
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut start = 0;
    let mut props = RunProps::default();
    for &(at, ifnt) in runs {
        let at = at.min(chars.len());
        if at > start {
            out.push((chars[start..at].iter().collect(), props.clone()));
        }
        props = fonts.get(font_index(ifnt)).map(|f| f.props.clone()).unwrap_or_default();
        start = at.max(start);
    }
    if start < chars.len() {
        out.push((chars[start..].iter().collect(), props));
    }
    out
}

fn shared_strings(record: &Record, fonts: &[Font]) -> Vec<Vec<(String, RunProps)>> {
    let mut reader = record.reader();
    reader.skip(4);
    let unique = reader.u32().unwrap_or(0) as usize;
    let mut out = Vec::with_capacity(unique.min(1 << 20));
    for _ in 0..unique {
        let Some(count) = reader.u16() else { break };
        let Some((text, runs)) = reader.unicode_string(count as usize) else { break };
        out.push(runs_to_text(text, &runs, fonts));
    }
    out
}

fn print_area(data: &[u8]) -> Option<(usize, (u32, u32, u32, u32))> {
    let flags = le16(data, 0);
    if flags & 0x20 == 0 || data.get(3).copied()? != 1 {
        return None;
    }
    let formula_len = le16(data, 4) as usize;
    let sheet = le16(data, 8) as usize;
    let name_flags = *data.get(14)?;
    let name_len = if name_flags & 1 == 1 { 2 } else { 1 };
    if *data.get(15)? != 0x06 || sheet == 0 {
        return None;
    }
    let formula = data.get(15 + name_len..15 + name_len + formula_len)?;
    let mut pos = 0;
    while pos < formula.len() {
        match formula[pos] {
            0x29 | 0x49 | 0x69 => pos += 3,
            0x3B | 0x5B | 0x7B => {
                let row1 = le16(formula, pos + 3) as u32 + 1;
                let row2 = le16(formula, pos + 5) as u32 + 1;
                let col1 = (le16(formula, pos + 7) & 0x3FFF) as u32 + 1;
                let col2 = (le16(formula, pos + 9) & 0x3FFF) as u32 + 1;
                return Some((sheet - 1, (row1.min(row2), col1.min(col2), row1.max(row2), col1.max(col2))));
            }
            _ => return None,
        }
    }
    None
}

struct SheetBuilder<'a> {
    globals: &'a Globals,
    rows: BTreeMap<u32, RowData>,
    max_row: u32,
    max_col: u32,
}

impl SheetBuilder<'_> {
    fn put(&mut self, row: u16, col: u16, style: u16, value: CellValue) {
        let (r, c) = (row as u32 + 1, col as u32 + 1);
        let style = style as usize;
        let xf = self.globals.styles.xf(style);
        if !matches!(value, CellValue::Empty) || xf.fill.is_some() || has_border(&xf.borders) {
            self.max_row = self.max_row.max(r);
            self.max_col = self.max_col.max(c);
        }
        self.rows.entry(r).or_default().cells.insert(c, CellData { value, style });
    }
}

fn read_sheet(records: &[Record], globals: &Globals, print_area: Option<(u32, u32, u32, u32)>) -> (Sheet, Vec<Dxf>) {
    let mut builder = SheetBuilder { globals, rows: BTreeMap::new(), max_row: 0, max_col: 0 };
    let mut col_specs: Vec<(u32, u32, f64, bool)> = Vec::new();
    let mut default_col_chars = 8.43;
    let mut standard_width = None;
    let mut default_row_height = 12.8;
    let mut merges = Vec::new();
    let mm = 72.0 / 25.4;
    let mut page = PageOptions {
        margin_left: 19.0 * mm,
        margin_right: 19.0 * mm,
        margin_top: 25.0 * mm,
        margin_bottom: 25.0 * mm,
        margin_header: 13.0 * mm,
        margin_footer: 13.0 * mm,
        ..PageOptions::default()
    };
    let mut row_breaks = Vec::new();
    let mut col_breaks = Vec::new();
    let mut pending_string: Option<(u16, u16, u16)> = None;
    let mut depth = 0;
    let mut objects: Vec<(u16, u16)> = Vec::new();
    let mut texts: HashMap<u16, String> = HashMap::new();
    let mut note_cells: Vec<(u32, u32, u16)> = Vec::new();
    let mut drawing: Vec<u8> = Vec::new();
    let mut conditional = Vec::new();
    let mut dxfs: Vec<Dxf> = Vec::new();
    let mut condition_group: Option<(&[u8], Vec<&[u8]>)> = None;
    let mut priority = 0;
    let palette = |i: usize| globals.palette.color(i);
    let mut flush = |group: &mut Option<(&[u8], Vec<&[u8]>)>, conditional: &mut Vec<_>, dxfs: &mut Vec<Dxf>| {
        if let Some((head, rules)) = group.take() {
            for condition in biff_cf::conditions(head, &rules, &palette, &mut priority) {
                let mut rule = condition.rule;
                rule.dxf = dxfs.len();
                dxfs.push(condition.dxf);
                conditional.push(rule);
            }
        }
    };

    for record in records {
        match record.kind {
            BOF => {
                depth += 1;
                continue;
            }
            EOF => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
                continue;
            }
            _ if depth != 1 => continue,
            _ => {}
        }
        let data = record.data();
        let (row, col, style) = (le16(data, 0), le16(data, 2), le16(data, 4));
        match record.kind {
            CONDFMT => {
                flush(&mut condition_group, &mut conditional, &mut dxfs);
                condition_group = Some((data, Vec::new()));
                continue;
            }
            CF => {
                if let Some((_, rules)) = condition_group.as_mut() {
                    rules.push(data);
                }
                continue;
            }
            _ => flush(&mut condition_group, &mut conditional, &mut dxfs),
        }
        match record.kind {
            DEFCOLWIDTH => {
                let font_twips = globals.styles.font(0).size.unwrap_or(10.0) * 20.0;
                default_col_chars = le16(data, 0) as f64 + (40960.0 / (font_twips - 15.0).max(60.0) + 50.0) / 256.0;
            }
            STANDARDWIDTH => standard_width = Some(le16(data, 0) as f64 / 256.0),
            COLINFO => col_specs.push((
                le16(data, 0) as u32 + 1,
                le16(data, 2) as u32 + 1,
                le16(data, 4) as f64 / 256.0,
                le16(data, 8) & 0x01 != 0,
            )),
            DEFAULTROWHEIGHT => default_row_height = le16(data, 2) as f64 / 20.0,
            ROW => {
                let flags = le32(data, 12);
                let entry = builder.rows.entry(row as u32 + 1).or_default();
                entry.height = Some((le16(data, 6) & 0x7FFF) as f64 / 20.0);
                entry.custom_height = flags & 0x40 != 0;
                entry.hidden = flags & 0x20 != 0;
            }
            NUMBER => {
                if let Some(value) = f64_at(data, 6) {
                    builder.put(row, col, style, CellValue::Number(value));
                }
            }
            RK => builder.put(row, col, style, CellValue::Number(rk_value(le32(data, 6)))),
            MULRK => {
                let count = data.len().saturating_sub(6) / 6;
                for i in 0..count {
                    let at = 4 + i * 6;
                    builder.put(row, col + i as u16, le16(data, at), CellValue::Number(rk_value(le32(data, at + 2))));
                }
            }
            LABELSST => {
                let text = globals.strings.get(le32(data, 6) as usize).cloned().unwrap_or_default();
                builder.put(row, col, style, CellValue::Text(text));
            }
            LABEL | RSTRING => {
                let mut reader = record.reader();
                reader.skip(6);
                if let Some((text, runs)) = reader.u16().and_then(|count| reader.unicode_string(count as usize)) {
                    let text = runs_to_text(text, &runs, &globals.styles.fonts);
                    builder.put(row, col, style, CellValue::Text(text));
                }
            }
            BLANK => builder.put(row, col, style, CellValue::Empty),
            MULBLANK => {
                let count = data.len().saturating_sub(6) / 2;
                for i in 0..count {
                    builder.put(row, col + i as u16, le16(data, 4 + i * 2), CellValue::Empty);
                }
            }
            BOOLERR => {
                let value = data.get(6).copied().unwrap_or(0);
                let cell = if data.get(7).copied().unwrap_or(0) == 0 {
                    CellValue::Bool(value != 0)
                } else {
                    CellValue::Error(error_text(value).into())
                };
                builder.put(row, col, style, cell);
            }
            FORMULA => {
                let result = data.get(6..14).unwrap_or(&[0; 8]);
                if result[6] == 0xFF && result[7] == 0xFF {
                    match result[0] {
                        0 => pending_string = Some((row, col, style)),
                        1 => builder.put(row, col, style, CellValue::Number(if result[2] != 0 { 1.0 } else { 0.0 })),
                        2 => builder.put(row, col, style, CellValue::Error(error_text(result[2]).into())),
                        _ => builder.put(row, col, style, CellValue::Empty),
                    }
                } else if let Some(value) = f64_at(data, 6) {
                    builder.put(row, col, style, CellValue::Number(value));
                }
            }
            STRING => {
                if let Some((row, col, style)) = pending_string.take() {
                    let mut reader = record.reader();
                    if let Some((text, _)) = reader.u16().and_then(|count| reader.unicode_string(count as usize)) {
                        builder.put(row, col, style, CellValue::Text(vec![(text, RunProps::default())]));
                    }
                }
            }
            MERGECELLS => {
                let count = le16(data, 0) as usize;
                for i in 0..count {
                    let at = 2 + i * 8;
                    if data.len() < at + 8 {
                        break;
                    }
                    let (r1, r2) = (le16(data, at) as u32 + 1, le16(data, at + 2) as u32 + 1);
                    let (c1, c2) = (le16(data, at + 4) as u32 + 1, le16(data, at + 6) as u32 + 1);
                    merges.push((r1, c1, r2, c2));
                    builder.max_row = builder.max_row.max(r2);
                    builder.max_col = builder.max_col.max(c2);
                }
            }
            LEFTMARGIN | RIGHTMARGIN | TOPMARGIN | BOTTOMMARGIN => {
                if let Some(inches) = f64_at(data, 0) {
                    let value = inches * 72.0;
                    match record.kind {
                        LEFTMARGIN => page.margin_left = value,
                        RIGHTMARGIN => page.margin_right = value,
                        TOPMARGIN => page.margin_top = value,
                        _ => page.margin_bottom = value,
                    }
                }
            }
            SETUP => {
                let flags = le16(data, 10);
                if flags & 0x04 == 0 {
                    let (w, h) = paper_size(le16(data, 0) as u32);
                    let landscape = flags & 0x40 == 0 && flags & 0x02 == 0;
                    (page.width, page.height) = if landscape { (h, w) } else { (w, h) };
                    let scale = le16(data, 2) as f64;
                    if scale > 0.0 {
                        page.scale = (scale / 100.0).clamp(0.1, 4.0);
                    }
                }
                page.fit_width = le16(data, 6) as usize;
                page.fit_height = le16(data, 8) as usize;
                page.over_then_down = flags & 0x01 != 0;
                if let (Some(header), Some(footer)) = (f64_at(data, 16), f64_at(data, 24)) {
                    page.margin_header = header * 72.0;
                    page.margin_footer = footer * 72.0;
                }
            }
            WSBOOL => page.fit_to_page = le16(data, 0) & 0x0100 != 0,
            PRINTGRIDLINES => page.grid_lines = le16(data, 0) != 0,
            HCENTER => page.h_center = le16(data, 0) != 0,
            HEADER | FOOTER => {
                let mut reader = record.reader();
                let text = reader
                    .u16()
                    .and_then(|count| reader.unicode_string(count as usize))
                    .map(|(t, _)| t)
                    .filter(|t| !t.is_empty());
                if record.kind == HEADER {
                    page.header = text;
                } else {
                    page.footer = text;
                }
            }
            OBJ => {
                if le16(data, 0) == 0x15 {
                    objects.push((le16(data, 4), le16(data, 6)));
                }
            }
            TXO => {
                if let (Some(&(_, id)), Some(text)) = (objects.last(), text_object(record)) {
                    texts.insert(id, text);
                }
            }
            NOTE => note_cells.push((row as u32 + 1, col as u32 + 1, le16(data, 6))),
            MSODRAWING => record.parts.iter().for_each(|part| drawing.extend_from_slice(part)),
            HORIZONTALPAGEBREAKS | VERTICALPAGEBREAKS => {
                let count = le16(data, 0) as usize;
                let breaks: Vec<u32> = (0..count).map(|i| le16(data, 2 + i * 6) as u32).filter(|b| *b > 0).collect();
                if record.kind == HORIZONTALPAGEBREAKS {
                    row_breaks = breaks;
                } else {
                    col_breaks = breaks;
                }
            }
            _ => {}
        }
    }

    flush(&mut condition_group, &mut conditional, &mut dxfs);
    let default_col_chars = standard_width.unwrap_or(default_col_chars);
    let styles = &globals.styles;
    let digit = digit_width(styles.font(0));
    let default_size = styles.font(0).size.unwrap_or(10.0);
    if let Some(header) = &page.header {
        page.body_top = Some(calc_body_edge(header, default_size, page.margin_top, page.margin_header));
    }
    if let Some(footer) = &page.footer {
        page.body_bottom = Some(calc_body_edge(footer, default_size, page.margin_bottom, page.margin_footer));
    }
    let col_width = |c: u32| column_width(&col_specs, default_col_chars, digit, c);
    let row_height = |r: u32| builder.rows.get(&r).and_then(|d| d.height).unwrap_or(default_row_height);
    let x_of = |col: u16, dx: u16| -> f64 {
        (1..=col as u32).map(col_width).sum::<f64>() + col_width(col as u32 + 1) * (dx.min(1024) as f64 / 1024.0)
    };
    let y_of = |row: u16, dy: u16| -> f64 {
        (1..=row as u32).map(row_height).sum::<f64>() + row_height(row as u32 + 1) * (dy.min(256) as f64 / 256.0)
    };
    let mut drawings = Vec::new();
    for shape in biff_art::shapes(&drawing) {
        let (Some(anchor), Some(index)) = (&shape.anchor, shape.picture) else { continue };
        if shape.hidden || shape.child {
            continue;
        }
        let Some(Some(image)) = globals.blips.get(index) else { continue };
        let (x, y) = (x_of(anchor.col1, anchor.dx1), y_of(anchor.row1, anchor.dy1));
        let (x2, y2) = (x_of(anchor.col2, anchor.dx2), y_of(anchor.row2, anchor.dy2));
        drawings.push(SheetDrawing { x, y, width: (x2 - x).max(0.0), height: (y2 - y).max(0.0), content: DrawingContent::Image(image.clone()) });
        builder.max_row = builder.max_row.max(anchor.row2 as u32 + 1);
        builder.max_col = builder.max_col.max(anchor.col2 as u32 + 1);
    }
    let extent = sheet_extent(styles, &builder.rows, &col_specs, default_col_chars, digit, builder.max_row, builder.max_col, print_area);
    let notes = note_cells
        .into_iter()
        .filter_map(|(r, c, id)| texts.get(&id).map(|t| ((r, c), t.clone())))
        .collect();
    let sheet = Sheet {
        notes,
        conditional,
        drawings,
        col_widths: extent.col_widths,
        rows: builder.rows,
        merges,
        default_row_height,
        page,
        first_row: extent.first_row,
        last_row: extent.last_row,
        first_col: extent.first_col,
        last_col: extent.last_col,
        row_breaks,
        col_breaks,
    };
    (sheet, dxfs)
}

fn calc_body_edge(code: &str, default_size: f64, margin: f64, hf_margin: f64) -> f64 {
    let reserved: f64 = header_lines(code, default_size).iter().sum();
    let rendered: f64 = header_lines(code, default_size).iter().map(|size| (size * 1.117).max(12.0 * 1.117)).sum();
    let distance = margin - hf_margin - reserved;
    if distance >= 0.0 { hf_margin + rendered + distance } else { margin }
}

fn header_lines(code: &str, default_size: f64) -> Vec<f64> {
    let mut sections: Vec<Vec<f64>> = vec![vec![0.0]];
    let mut size = default_size;
    let mut chars = code.chars().peekable();
    let mut current = 0;
    let touch = |sections: &mut Vec<Vec<f64>>, current: usize, size: f64| {
        let line = sections[current].last_mut().unwrap();
        *line = line.max(size);
    };
    while let Some(ch) = chars.next() {
        match ch {
            '&' => match chars.peek().copied() {
                Some('L') | Some('C') | Some('R') => {
                    chars.next();
                    sections.push(vec![0.0]);
                    current = sections.len() - 1;
                    size = default_size;
                }
                Some(d) if d.is_ascii_digit() => {
                    let mut digits = String::new();
                    while let Some(d) = chars.peek().copied().filter(char::is_ascii_digit) {
                        digits.push(d);
                        chars.next();
                    }
                    size = digits.parse().unwrap_or(default_size);
                }
                Some('"') => {
                    chars.next();
                    for c in chars.by_ref() {
                        if c == '"' {
                            break;
                        }
                    }
                }
                Some(_) => {
                    chars.next();
                    touch(&mut sections, current, size);
                }
                None => {}
            },
            '\n' => sections[current].push(0.0),
            _ => touch(&mut sections, current, size),
        }
    }
    sections
        .into_iter()
        .map(|lines| lines.into_iter().map(|s| if s > 0.0 { s } else { default_size }).collect::<Vec<_>>())
        .max_by(|a, b| a.iter().sum::<f64>().total_cmp(&b.iter().sum::<f64>()))
        .unwrap_or_else(|| vec![default_size])
}

fn text_object(record: &Record) -> Option<String> {
    let count = le16(record.data(), 10) as usize;
    if count == 0 || record.parts.len() < 2 {
        return None;
    }
    let mut reader = Reader { parts: &record.parts, part: 1, pos: 0 };
    let wide = reader.u8()? & 1 == 1;
    reader.chars(count, wide).map(|t| t.replace('\r', "\n"))
}

fn error_text(code: u8) -> &'static str {
    match code {
        0x00 => "#NULL!",
        0x07 => "#DIV/0!",
        0x0F => "#VALUE!",
        0x17 => "#REF!",
        0x1D => "#NAME?",
        0x24 => "#NUM!",
        0x2A => "#N/A",
        _ => "#N/A",
    }
}
