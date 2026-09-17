mod sprm;

use crate::error::Error;
use crate::model::*;
use sprm::{Sprm, SprmIter};
use std::collections::HashMap;
use std::io::{Cursor, Read};

pub fn read(bytes: &[u8]) -> Result<Document, Error> {
    let mut cfb = cfb::CompoundFile::open(Cursor::new(bytes))
        .map_err(|e| Error::new(format!("not a Word binary document: {e}")))?;
    let word = read_stream(&mut cfb, "WordDocument")?;
    if word.len() < 0x200 {
        return Err(Error::new("WordDocument stream is too short"));
    }
    let fib = Fib::parse(&word)?;
    let table = read_stream(&mut cfb, if fib.which_table == 1 { "1Table" } else { "0Table" })
        .or_else(|_| read_stream(&mut cfb, "0Table"))
        .or_else(|_| read_stream(&mut cfb, "1Table"))?;
    let data = read_stream(&mut cfb, "Data").unwrap_or_default();

    let file = File { word, table, data, fib };
    let pieces = file.pieces()?;
    let fonts = file.fonts();
    let styles = file.styles(&fonts)?;
    let lists = file.lists(&fonts);

    let reader = Reader {
        file: &file,
        pieces: &pieces,
        fonts: &fonts,
        styles: &styles,
        lists: &lists,
        chpx_fkps: file.fkps(file.fib.fc_lcb(12))?,
        papx_fkps: file.fkps(file.fib.fc_lcb(13))?,
        counters: std::cell::RefCell::new(HashMap::new()),
    };

    let main_end = file.fib.ccp_text;
    let sections = reader.sections()?;
    let footnotes = reader.footnotes();
    let headers = reader.headers(sections.len());

    let mut doc = Document {
        borders_outside_indent: true,
        footnote_separator_width: Some(144.0),
        default_tab: 36.0,
        additive_spacing: false,
        ..Document::default()
    };
    let mut blocks_start = 0u32;
    for (index, (end_cp, page)) in sections.iter().enumerate() {
        let end = (*end_cp).min(main_end);
        let blocks = reader.blocks(blocks_start, end, &footnotes);
        blocks_start = end;
        let mut section = Section {
            page: page.page.clone(),
            blocks,
            columns: page.columns,
            column_gap: page.column_gap,
            title_page: page.title_page,
            page_start: page.page_start,
            content_scale: 1.0,
            ..Section::default()
        };
        if let Some(h) = headers.get(index) {
            section.header_default = h.header_odd.clone();
            section.header_even = h.header_even.clone();
            section.header_first = h.header_first.clone();
            section.footer_default = h.footer_odd.clone();
            section.footer_even = h.footer_even.clone();
            section.footer_first = h.footer_first.clone();
        }
        doc.sections.push(section);
    }
    doc.even_odd_headers = file.dop_even_odd();
    if doc.sections.is_empty() {
        return Err(Error::new("document has no sections"));
    }
    Ok(doc)
}

pub fn debug(bytes: &[u8]) -> Result<String, Error> {
    let mut cfb = cfb::CompoundFile::open(Cursor::new(bytes))
        .map_err(|e| Error::new(format!("not a Word binary document: {e}")))?;
    let word = read_stream(&mut cfb, "WordDocument")?;
    let fib = Fib::parse(&word)?;
    let table = read_stream(&mut cfb, if fib.which_table == 1 { "1Table" } else { "0Table" })?;
    let data = read_stream(&mut cfb, "Data").unwrap_or_default();
    let file = File { word, table, data, fib };
    let pieces = file.pieces()?;
    let fonts = file.fonts();
    let styles = file.styles(&fonts)?;
    let lists = file.lists(&fonts);
    let mut out = format!(
        "nFib={:#x} ccpText={} ccpHdd={} pieces={} fonts={:?} styles={} lists={} lfos={}\n",
        file.fib.n_fib,
        file.fib.ccp_text,
        file.fib.ccp_hdd,
        pieces.len(),
        fonts,
        styles.styles.len(),
        lists.lists.len(),
        lists.lfos.len()
    );
    for (lsid, def) in &lists.lists {
        out.push_str(&format!("  list {lsid:#x}: {} levels\n", def.levels.len()));
        for (i, level) in def.levels.iter().enumerate() {
            out.push_str(&format!(
                "    lvl{i}: nfc={} start={} follow={} text={:?} papx={:02x?} chpx={:02x?}\n",
                level.nfc,
                level.start,
                level.follow,
                String::from_utf16_lossy(&level.text),
                level.papx,
                level.chpx
            ));
        }
    }
    for (i, lfo) in lists.lfos.iter().enumerate() {
        out.push_str(&format!("  lfo {}: lsid={:#x} overrides={}\n", i + 1, lfo.lsid, lfo.overrides.len()));
    }
    for (i, style) in styles.styles.iter().enumerate().take(12) {
        if let Some(s) = style {
            out.push_str(&format!("  style {i}: {:?} kind={} base={} numbering={:?}\n", s.name, s.kind, s.base, s.ppr.numbering));
        }
    }
    Ok(out)
}

fn read_stream<F: Read + std::io::Seek>(cfb: &mut cfb::CompoundFile<F>, name: &str) -> Result<Vec<u8>, Error> {
    let mut stream = cfb
        .open_stream(format!("/{name}"))
        .map_err(|e| Error::new(format!("missing stream {name}: {e}")))?;
    let mut data = Vec::new();
    stream.read_to_end(&mut data)?;
    Ok(data)
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    if at + 2 > b.len() { 0 } else { u16::from_le_bytes([b[at], b[at + 1]]) }
}
fn i16_at(b: &[u8], at: usize) -> i16 {
    u16_at(b, at) as i16
}
fn u32_at(b: &[u8], at: usize) -> u32 {
    if at + 4 > b.len() { 0 } else { u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]) }
}
fn i32_at(b: &[u8], at: usize) -> i32 {
    u32_at(b, at) as i32
}

struct Fib {
    n_fib: u16,
    which_table: u8,
    encrypted: bool,
    ccp_text: u32,
    ccp_ftn: u32,
    ccp_hdd: u32,
    ccp_atn: u32,
    ccp_edn: u32,
    ccp_txbx: u32,
    fc_lcb: Vec<(u32, u32)>,
    fc_dop: (u32, u32),
}

impl Fib {
    fn parse(word: &[u8]) -> Result<Fib, Error> {
        if u16_at(word, 0) != 0xA5EC {
            return Err(Error::new("not a Word binary document (bad FIB signature)"));
        }
        let n_fib = u16_at(word, 2);
        if n_fib < 0x00C1 {
            return Err(Error::new("Word 6/95 documents are not supported yet"));
        }
        let flags = u16_at(word, 0x0A);
        let encrypted = flags & 0x0100 != 0;
        let which_table = ((flags & 0x0200) >> 9) as u8;
        let csw = u16_at(word, 0x20) as usize;
        let lw_start = 0x22 + csw * 2 + 2;
        let cslw = u16_at(word, lw_start - 2) as usize;
        let lw = |i: usize| u32_at(word, lw_start + i * 4);
        let fc_lcb_count_at = lw_start + cslw * 4;
        let cb_rg_fc_lcb = u16_at(word, fc_lcb_count_at) as usize;
        let blob_start = fc_lcb_count_at + 2;
        let mut fc_lcb = Vec::with_capacity(cb_rg_fc_lcb);
        for i in 0..cb_rg_fc_lcb {
            fc_lcb.push((u32_at(word, blob_start + i * 8), u32_at(word, blob_start + i * 8 + 4)));
        }
        let fc_dop = fc_lcb.get(31).copied().unwrap_or((0, 0));
        Ok(Fib {
            n_fib,
            which_table,
            encrypted,
            ccp_text: lw(3),
            ccp_ftn: lw(4),
            ccp_hdd: lw(5),
            ccp_atn: lw(7),
            ccp_edn: lw(8),
            ccp_txbx: lw(9),
            fc_lcb,
            fc_dop,
        })
    }

    fn fc_lcb(&self, index: usize) -> (u32, u32) {
        self.fc_lcb.get(index).copied().unwrap_or((0, 0))
    }
}

struct File {
    word: Vec<u8>,
    table: Vec<u8>,
    data: Vec<u8>,
    fib: Fib,
}

#[derive(Debug, Clone, Copy)]
struct Piece {
    cp_start: u32,
    cp_end: u32,
    fc: u32,
    compressed: bool,
}

impl Piece {
    fn fc_of(&self, cp: u32) -> u32 {
        let offset = cp - self.cp_start;
        if self.compressed { self.fc + offset } else { self.fc + offset * 2 }
    }
}

#[derive(Debug, Clone)]
struct Fkp {
    fcs: Vec<u32>,
    entries: Vec<Vec<u8>>,
    istds: Vec<u16>,
}

#[derive(Debug, Clone, Default)]
struct Style {
    name: String,
    kind: u16,
    base: u16,
    next: u16,
    ppr: ParagraphProps,
    rpr: RunProps,
    papx: Vec<u8>,
    chpx: Vec<u8>,
}

struct Styles {
    styles: Vec<Option<Style>>,
    default_rpr: RunProps,
}

impl Styles {
    fn chain(&self, istd: u16) -> Vec<&Style> {
        let mut chain = Vec::new();
        let mut current = istd;
        let mut guard = 0;
        while let Some(Some(style)) = self.styles.get(current as usize) {
            chain.push(style);
            if style.base == 0x0FFF || style.base == current {
                break;
            }
            current = style.base;
            guard += 1;
            if guard > 20 {
                break;
            }
        }
        chain.reverse();
        chain
    }

    fn paragraph(&self, istd: u16) -> (ParagraphProps, RunProps) {
        let mut ppr = ParagraphProps::default();
        let mut rpr = self.default_rpr.clone();
        for style in self.chain(istd) {
            ppr.merge(&style.ppr);
            rpr.merge(&style.rpr);
        }
        (ppr, rpr)
    }

    fn character(&self, istd: u16, props: &mut RunProps) {
        for style in self.chain(istd) {
            if style.kind == 2 {
                props.merge(&style.rpr);
            }
        }
    }

    fn table_papx(&self, istd: u16) -> Vec<Vec<u8>> {
        self.chain(istd).iter().map(|s| s.papx.clone()).collect()
    }
}

#[derive(Debug, Clone)]
struct ListLevelDef {
    start: i64,
    nfc: u8,
    jc: u8,
    text: Vec<u16>,
    follow: u8,
    papx: Vec<u8>,
    chpx: Vec<u8>,
}

#[derive(Debug, Clone)]
struct ListDef {
    levels: Vec<ListLevelDef>,
}

#[derive(Debug, Clone)]
struct Lfo {
    lsid: u32,
    overrides: Vec<(u8, Option<i64>, Option<ListLevelDef>)>,
}

struct Lists {
    lists: HashMap<u32, ListDef>,
    lfos: Vec<Lfo>,
}

impl Lists {
    fn level(&self, ilfo: u16, ilvl: u8) -> Option<(u32, ListLevelDef)> {
        let lfo = self.lfos.get(ilfo.checked_sub(1)? as usize)?;
        let list = self.lists.get(&lfo.lsid)?;
        let mut level = list.levels.get(ilvl as usize).cloned().or_else(|| list.levels.first().cloned())?;
        for (lvl, start, replacement) in &lfo.overrides {
            if *lvl == ilvl {
                if let Some(r) = replacement {
                    level = r.clone();
                }
                if let Some(s) = start {
                    level.start = *s;
                }
            }
        }
        Some((lfo.lsid, level))
    }
}

#[derive(Debug, Clone)]
struct SectionInfo {
    page: PageSetup,
    columns: usize,
    column_gap: f64,
    title_page: bool,
    page_start: Option<i64>,
}

#[derive(Debug, Clone, Default)]
struct HeaderSet {
    header_even: Option<Vec<Block>>,
    header_odd: Option<Vec<Block>>,
    footer_even: Option<Vec<Block>>,
    footer_odd: Option<Vec<Block>>,
    header_first: Option<Vec<Block>>,
    footer_first: Option<Vec<Block>>,
}

impl File {
    fn pieces(&self) -> Result<Vec<Piece>, Error> {
        let (fc_clx, lcb_clx) = self.fib.fc_lcb(33);
        let clx = self
            .table
            .get(fc_clx as usize..(fc_clx + lcb_clx) as usize)
            .ok_or_else(|| Error::new("piece table is out of range"))?;
        let mut pos = 0;
        while pos < clx.len() {
            match clx[pos] {
                1 => {
                    let cb = u16_at(clx, pos + 1) as usize;
                    pos += 3 + cb;
                }
                2 => {
                    let lcb = u32_at(clx, pos + 1) as usize;
                    let plc = &clx[pos + 5..(pos + 5 + lcb).min(clx.len())];
                    let n = (plc.len().saturating_sub(4)) / 12;
                    let mut pieces = Vec::with_capacity(n);
                    for i in 0..n {
                        let cp_start = u32_at(plc, i * 4);
                        let cp_end = u32_at(plc, (i + 1) * 4);
                        let pcd = (n + 1) * 4 + i * 8;
                        let fc_raw = u32_at(plc, pcd + 2);
                        let compressed = fc_raw & 0x4000_0000 != 0;
                        let fc = if compressed { (fc_raw & 0x3FFF_FFFF) / 2 } else { fc_raw & 0x3FFF_FFFF };
                        pieces.push(Piece { cp_start, cp_end, fc, compressed });
                    }
                    return Ok(pieces);
                }
                _ => break,
            }
        }
        Err(Error::new("piece table not found"))
    }

    fn fonts(&self) -> Vec<String> {
        let (fc, lcb) = self.fib.fc_lcb(15);
        let Some(sttb) = self.table.get(fc as usize..(fc + lcb) as usize) else { return Vec::new() };
        let mut fonts = Vec::new();
        let mut pos = 0;
        if u16_at(sttb, 0) == 0xFFFF {
            pos = 2;
        }
        let count = u16_at(sttb, pos) as usize;
        pos += 4;
        for _ in 0..count {
            if pos >= sttb.len() {
                break;
            }
            let cb = sttb[pos] as usize;
            let ffn = &sttb[pos + 1..(pos + 1 + cb).min(sttb.len())];
            let name = if ffn.len() > 39 {
                let mut chars = Vec::new();
                let mut i = 39;
                while i + 1 < ffn.len() {
                    let c = u16_at(ffn, i);
                    if c == 0 {
                        break;
                    }
                    chars.push(c);
                    i += 2;
                }
                String::from_utf16_lossy(&chars)
            } else {
                String::new()
            };
            fonts.push(name);
            pos += 1 + cb;
        }
        fonts
    }

    fn styles(&self, fonts: &[String]) -> Result<Styles, Error> {
        let (fc, lcb) = self.fib.fc_lcb(1);
        let stsh = self
            .table
            .get(fc as usize..(fc + lcb) as usize)
            .ok_or_else(|| Error::new("stylesheet is out of range"))?;
        let cb_stshi = u16_at(stsh, 0) as usize;
        let stshi = &stsh[2..(2 + cb_stshi).min(stsh.len())];
        let cstd = u16_at(stshi, 0) as usize;
        let cb_std_base = u16_at(stshi, 2) as usize;
        let default_ftc = u16_at(stshi, 12) as usize;
        let mut default_rpr = RunProps {
            font: Some(fonts.get(default_ftc).cloned().filter(|f| !f.is_empty()).unwrap_or_else(|| "Times New Roman".into())),
            size: Some(10.0),
            ..RunProps::default()
        };
        let _ = &mut default_rpr;

        let mut styles: Vec<Option<Style>> = Vec::with_capacity(cstd);
        let mut pos = 2 + cb_stshi;
        let mut raw: Vec<Option<(u16, u16, u16, String, Vec<u8>, Vec<u8>)>> = Vec::with_capacity(cstd);
        for _ in 0..cstd {
            if pos + 2 > stsh.len() {
                break;
            }
            let cb_std = u16_at(stsh, pos) as usize;
            pos += 2;
            if cb_std == 0 {
                raw.push(None);
                continue;
            }
            let std = &stsh[pos..(pos + cb_std).min(stsh.len())];
            pos += cb_std;
            let kind = u16_at(std, 2) & 0x000F;
            let base = (u16_at(std, 2) >> 4) & 0x0FFF;
            let cupx = (u16_at(std, 4) & 0x000F) as usize;
            let next = (u16_at(std, 4) >> 4) & 0x0FFF;
            let name_at = cb_std_base;
            let cch = u16_at(std, name_at) as usize;
            let mut name_chars = Vec::new();
            for i in 0..cch {
                name_chars.push(u16_at(std, name_at + 2 + i * 2));
            }
            let name = String::from_utf16_lossy(&name_chars);
            let mut upx_pos = name_at + 2 + cch * 2 + 2;
            if upx_pos % 2 == 1 {
                upx_pos += 1;
            }
            let mut papx = Vec::new();
            let mut chpx = Vec::new();
            for u in 0..cupx {
                if upx_pos + 2 > std.len() {
                    break;
                }
                let cb = u16_at(std, upx_pos) as usize;
                let body = std.get(upx_pos + 2..(upx_pos + 2 + cb).min(std.len())).unwrap_or(&[]).to_vec();
                if kind == 1 || kind == 3 {
                    if u == 0 {
                        papx = body.get(2..).map(|b| b.to_vec()).unwrap_or_default();
                    } else if u == 1 {
                        chpx = body;
                    }
                } else if kind == 2 && u == 0 {
                    chpx = body;
                }
                upx_pos += 2 + cb;
                if upx_pos % 2 == 1 {
                    upx_pos += 1;
                }
            }
            raw.push(Some((kind, base, next, name, papx, chpx)));
        }

        for entry in raw {
            match entry {
                None => styles.push(None),
                Some((kind, base, next, name, papx, chpx)) => {
                    let mut style = Style { name, kind, base, next, papx, chpx, ..Style::default() };
                    let mut ppr = ParagraphProps::default();
                    let mut rpr = RunProps::default();
                    for sprm in SprmIter::new(&style.papx) {
                        apply_paragraph_sprm(&sprm, &mut ppr, None);
                    }
                    for sprm in SprmIter::new(&style.chpx) {
                        apply_character_sprm(&sprm, &mut rpr, fonts, None);
                    }
                    style.ppr = ppr;
                    style.rpr = rpr;
                    styles.push(Some(style));
                }
            }
        }
        Ok(Styles { styles, default_rpr })
    }

    fn lists(&self, fonts: &[String]) -> Lists {
        let mut lists = HashMap::new();
        let (fc, lcb) = self.fib.fc_lcb(73);
        if let Some(plf) = self.table.get(fc as usize..(fc as usize + lcb as usize).min(self.table.len())) {
            let c_lst = u16_at(plf, 0) as usize;
            let mut defs = Vec::with_capacity(c_lst);
            let mut pos = 2;
            for _ in 0..c_lst {
                if pos + 28 > plf.len() {
                    break;
                }
                let lsid = u32_at(plf, pos);
                let simple = plf[pos + 26] & 0x01 != 0;
                defs.push((lsid, simple));
                pos += 28;
            }
            let remainder = &self.table[(fc as usize + pos).min(self.table.len())..];
            let mut rpos = 0;
            for (lsid, simple) in defs {
                let count = if simple { 1 } else { 9 };
                let mut levels = Vec::with_capacity(count);
                for _ in 0..count {
                    let Some((level, consumed)) = parse_lvl(&remainder[rpos.min(remainder.len())..], fonts) else { break };
                    levels.push(level);
                    rpos += consumed;
                }
                lists.insert(lsid, ListDef { levels });
            }
        }

        let mut lfos = Vec::new();
        let (fc, lcb) = self.fib.fc_lcb(74);
        if let Some(plf) = self.table.get(fc as usize..(fc as usize + lcb as usize).min(self.table.len())) {
            let c_lfo = u32_at(plf, 0) as usize;
            let mut pos = 4;
            let mut heads = Vec::with_capacity(c_lfo);
            for _ in 0..c_lfo {
                if pos + 16 > plf.len() {
                    break;
                }
                heads.push((u32_at(plf, pos), plf[pos + 12] as usize));
                pos += 16;
            }
            for (lsid, clfolvl) in heads {
                let mut overrides = Vec::new();
                pos += 4;
                for _ in 0..clfolvl {
                    if pos + 8 > plf.len() {
                        break;
                    }
                    let start = i32_at(plf, pos) as i64;
                    let flags = plf[pos + 4];
                    let ilvl = flags & 0x0F;
                    let f_start = flags & 0x10 != 0;
                    let f_formatting = flags & 0x20 != 0;
                    pos += 8;
                    let mut level = None;
                    if f_formatting {
                        if let Some((lvl, consumed)) = parse_lvl(&plf[pos.min(plf.len())..], fonts) {
                            level = Some(lvl);
                            pos += consumed;
                        }
                    }
                    overrides.push((ilvl, if f_start { Some(start) } else { None }, level));
                }
                lfos.push(Lfo { lsid, overrides });
            }
        }
        Lists { lists, lfos }
    }

    fn fkps(&self, plc: (u32, u32)) -> Result<Vec<(u32, u32, Fkp)>, Error> {
        let (fc, lcb) = plc;
        let Some(bte) = self.table.get(fc as usize..(fc as usize + lcb as usize).min(self.table.len())) else {
            return Ok(Vec::new());
        };
        let n = bte.len().saturating_sub(4) / 8;
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let fc_start = u32_at(bte, i * 4);
            let fc_end = u32_at(bte, (i + 1) * 4);
            let pn = u32_at(bte, (n + 1) * 4 + i * 4) as usize;
            let start = pn * 512;
            let Some(page) = self.word.get(start..start + 512) else { continue };
            out.push((fc_start, fc_end, parse_fkp(page)));
        }
        Ok(out)
    }

    fn dop_even_odd(&self) -> bool {
        let (fc, lcb) = self.fib.fc_dop;
        if lcb < 4 {
            return false;
        }
        self.table
            .get(fc as usize + 2)
            .map(|b| b & 0x01 != 0)
            .unwrap_or(false)
    }
}

fn parse_fkp(page: &[u8]) -> Fkp {
    let crun = page[511] as usize;
    let mut fcs = Vec::with_capacity(crun + 1);
    for i in 0..=crun {
        fcs.push(u32_at(page, i * 4));
    }
    let base = (crun + 1) * 4;
    let mut entries = Vec::with_capacity(crun);
    let mut istds = Vec::with_capacity(crun);
    for i in 0..crun {
        let offset_byte = page[base + i];
        if offset_byte == 0 {
            entries.push(Vec::new());
            istds.push(0);
            continue;
        }
        let at = offset_byte as usize * 2;
        let cb = page[at] as usize;
        let grpprl = &page[at + 1..(at + 1 + cb).min(512)];
        entries.push(grpprl.to_vec());
        istds.push(0);
    }
    Fkp { fcs, entries, istds }
}

fn parse_papx_fkp(page: &[u8]) -> Fkp {
    let cpara = page[511] as usize;
    let mut fcs = Vec::with_capacity(cpara + 1);
    for i in 0..=cpara {
        fcs.push(u32_at(page, i * 4));
    }
    let base = (cpara + 1) * 4;
    let mut entries = Vec::with_capacity(cpara);
    let mut istds = Vec::with_capacity(cpara);
    for i in 0..cpara {
        let bx = page[base + i * 13];
        if bx == 0 {
            entries.push(Vec::new());
            istds.push(0);
            continue;
        }
        let at = bx as usize * 2;
        let mut cb = page[at] as usize;
        let mut start = at + 1;
        if cb == 0 {
            cb = page[at + 1] as usize * 2;
            start = at + 2;
        } else {
            cb = cb * 2 - 1;
        }
        let body = &page[start..(start + cb).min(512)];
        let istd = u16_at(body, 0);
        entries.push(body.get(2..).map(|b| b.to_vec()).unwrap_or_default());
        istds.push(istd);
    }
    Fkp { fcs, entries, istds }
}

fn parse_lvl(buf: &[u8], fonts: &[String]) -> Option<(ListLevelDef, usize)> {
    if buf.len() < 28 {
        return None;
    }
    let start = i32_at(buf, 0) as i64;
    let nfc = buf[4];
    let jc = buf[5] & 0x03;
    let mut placeholders = [0u8; 9];
    placeholders.copy_from_slice(&buf[6..15]);
    let follow = buf[15];
    let cb_chpx = buf[24] as usize;
    let cb_papx = buf[25] as usize;
    let mut pos = 28;
    let papx = buf.get(pos..pos + cb_papx)?.to_vec();
    pos += cb_papx;
    let chpx = buf.get(pos..pos + cb_chpx)?.to_vec();
    pos += cb_chpx;
    let cch = u16_at(buf, pos) as usize;
    pos += 2;
    let mut text = Vec::with_capacity(cch);
    for i in 0..cch {
        text.push(u16_at(buf, pos + i * 2));
    }
    pos += cch * 2;
    let _ = fonts;
    let _ = placeholders;
    Some((ListLevelDef { start, nfc, jc, text, follow, papx, chpx }, pos))
}

struct Reader<'a> {
    file: &'a File,
    pieces: &'a [Piece],
    fonts: &'a [String],
    styles: &'a Styles,
    lists: &'a Lists,
    chpx_fkps: Vec<(u32, u32, Fkp)>,
    papx_fkps: Vec<(u32, u32, Fkp)>,
    counters: std::cell::RefCell<HashMap<u32, Vec<i64>>>,
}

#[derive(Debug, Clone)]
struct Char {
    ch: char,
    cp: u32,
    fc: u32,
}

impl Reader<'_> {
    fn piece_for(&self, cp: u32) -> Option<&Piece> {
        self.pieces.iter().find(|p| cp >= p.cp_start && cp < p.cp_end)
    }

    fn text(&self, start: u32, end: u32) -> Vec<Char> {
        let mut out = Vec::new();
        for piece in self.pieces {
            if piece.cp_end <= start || piece.cp_start >= end {
                continue;
            }
            let from = start.max(piece.cp_start);
            let to = end.min(piece.cp_end);
            if piece.compressed {
                let fc = piece.fc_of(from) as usize;
                let len = (to - from) as usize;
                let Some(bytes) = self.file.word.get(fc..(fc + len).min(self.file.word.len())) else { continue };
                for (i, b) in bytes.iter().enumerate() {
                    let ch = cp1252(*b);
                    out.push(Char { ch, cp: from + i as u32, fc: (fc + i) as u32 });
                }
            } else {
                let fc = piece.fc_of(from) as usize;
                let len = (to - from) as usize * 2;
                let Some(bytes) = self.file.word.get(fc..(fc + len).min(self.file.word.len())) else { continue };
                let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                let mut i = 0;
                while i < units.len() {
                    let u = units[i];
                    let (ch, width) = if (0xD800..0xDC00).contains(&u) && i + 1 < units.len() {
                        (char::decode_utf16([u, units[i + 1]]).next().and_then(|r| r.ok()).unwrap_or('\u{FFFD}'), 2)
                    } else {
                        (char::from_u32(u as u32).unwrap_or('\u{FFFD}'), 1)
                    };
                    out.push(Char { ch, cp: from + i as u32, fc: (fc + i * 2) as u32 });
                    if width == 2 {
                        out.push(Char { ch: '\u{0}', cp: from + i as u32 + 1, fc: (fc + i * 2 + 2) as u32 });
                    }
                    i += width;
                }
            }
        }
        out
    }

    fn papx_at(&self, fc: u32) -> (u16, Vec<u8>) {
        for (fc_start, fc_end, _) in &self.papx_fkps {
            if fc >= *fc_start && fc < *fc_end {
                let pn_page = self.papx_page(*fc_start);
                if let Some(fkp) = pn_page {
                    for i in 0..fkp.entries.len() {
                        if fc >= fkp.fcs[i] && fc < fkp.fcs[i + 1] {
                            return (fkp.istds[i], fkp.entries[i].clone());
                        }
                    }
                }
            }
        }
        (0, Vec::new())
    }

    fn papx_page(&self, fc_start: u32) -> Option<Fkp> {
        let (fc, lcb) = self.file.fib.fc_lcb(13);
        let bte = self.file.table.get(fc as usize..(fc as usize + lcb as usize).min(self.file.table.len()))?;
        let n = bte.len().saturating_sub(4) / 8;
        for i in 0..n {
            if u32_at(bte, i * 4) == fc_start {
                let pn = u32_at(bte, (n + 1) * 4 + i * 4) as usize;
                let page = self.file.word.get(pn * 512..pn * 512 + 512)?;
                return Some(parse_papx_fkp(page));
            }
        }
        None
    }

    fn chpx_runs(&self, fc_from: u32, fc_to: u32) -> Vec<(u32, u32, Vec<u8>)> {
        let mut runs = Vec::new();
        for (fc_start, fc_end, fkp) in &self.chpx_fkps {
            if *fc_end <= fc_from || *fc_start >= fc_to {
                continue;
            }
            for i in 0..fkp.entries.len() {
                let a = fkp.fcs[i].max(fc_from);
                let b = fkp.fcs[i + 1].min(fc_to);
                if a < b {
                    runs.push((a, b, fkp.entries[i].clone()));
                }
            }
        }
        runs.sort_by_key(|r| r.0);
        runs
    }

    fn sections(&self) -> Result<Vec<(u32, SectionInfo)>, Error> {
        let (fc, lcb) = self.file.fib.fc_lcb(6);
        let mut out = Vec::new();
        if let Some(plc) = self.file.table.get(fc as usize..(fc as usize + lcb as usize).min(self.file.table.len())) {
            let n = plc.len().saturating_sub(4) / 16;
            for i in 0..n {
                let cp_end = u32_at(plc, (i + 1) * 4);
                let sed = (n + 1) * 4 + i * 12;
                let fc_sepx = u32_at(plc, sed + 2);
                let mut info = SectionInfo {
                    page: PageSetup::default(),
                    columns: 1,
                    column_gap: 36.0,
                    title_page: false,
                    page_start: None,
                };
                if fc_sepx != 0xFFFF_FFFF {
                    let at = fc_sepx as usize;
                    let cb = u16_at(&self.file.word, at) as usize;
                    if let Some(grpprl) = self.file.word.get(at + 2..(at + 2 + cb).min(self.file.word.len())) {
                        for sprm in SprmIter::new(grpprl) {
                            apply_section_sprm(&sprm, &mut info);
                        }
                    }
                }
                out.push((cp_end, info));
            }
        }
        if out.is_empty() {
            out.push((
                self.file.fib.ccp_text,
                SectionInfo { page: PageSetup::default(), columns: 1, column_gap: 36.0, title_page: false, page_start: None },
            ));
        }
        Ok(out)
    }

    fn footnotes(&self) -> HashMap<u32, Vec<Block>> {
        let mut map = HashMap::new();
        let (fc_ref, lcb_ref) = self.file.fib.fc_lcb(2);
        let (fc_txt, lcb_txt) = self.file.fib.fc_lcb(3);
        let Some(refs) = self.file.table.get(fc_ref as usize..(fc_ref as usize + lcb_ref as usize).min(self.file.table.len())) else { return map };
        let Some(txt) = self.file.table.get(fc_txt as usize..(fc_txt as usize + lcb_txt as usize).min(self.file.table.len())) else { return map };
        let n = refs.len().saturating_sub(4) / 6;
        let story = self.file.fib.ccp_text;
        let empty = HashMap::new();
        for i in 0..n {
            let ref_cp = u32_at(refs, i * 4);
            let start = u32_at(txt, i * 4);
            let end = u32_at(txt, (i + 1) * 4);
            if end <= start {
                continue;
            }
            let blocks = self.blocks(story + start, story + end, &empty);
            map.insert(ref_cp, blocks);
        }
        map
    }

    fn headers(&self, section_count: usize) -> Vec<HeaderSet> {
        let mut out = Vec::new();
        let (fc, lcb) = self.file.fib.fc_lcb(11);
        let Some(plc) = self.file.table.get(fc as usize..(fc as usize + lcb as usize).min(self.file.table.len())) else {
            return out;
        };
        let n = plc.len() / 4;
        let story = self.file.fib.ccp_text + self.file.fib.ccp_ftn;
        let empty = HashMap::new();
        let cp = |i: usize| u32_at(plc, i * 4);
        for s in 0..section_count {
            let base = 6 + s * 6;
            if base + 6 >= n {
                break;
            }
            let mut set = HeaderSet::default();
            let slot = |i: usize| -> Option<Vec<Block>> {
                let start = cp(base + i);
                let end = cp(base + i + 1);
                if end > start + 1 {
                    Some(self.blocks(story + start, story + end, &empty))
                } else {
                    None
                }
            };
            set.header_even = slot(0);
            set.header_odd = slot(1);
            set.footer_even = slot(2);
            set.footer_odd = slot(3);
            set.header_first = slot(4);
            set.footer_first = slot(5);
            out.push(set);
        }
        out
    }

    fn blocks(&self, start: u32, end: u32, footnotes: &HashMap<u32, Vec<Block>>) -> Vec<Block> {
        let chars = self.text(start, end);
        let mut paragraphs: Vec<Vec<Char>> = Vec::new();
        let mut current: Vec<Char> = Vec::new();
        for c in chars {
            let terminator = matches!(c.ch, '\r' | '\u{7}');
            current.push(c);
            if terminator {
                paragraphs.push(std::mem::take(&mut current));
            }
        }
        if !current.is_empty() {
            paragraphs.push(current);
        }
        if start >= self.file.fib.ccp_text && paragraphs.len() > 1 {
            if let Some(last) = paragraphs.last() {
                if last.len() == 1 && last[0].ch == '\r' {
                    paragraphs.pop();
                }
            }
        }

        let mut blocks: Vec<Block> = Vec::new();
        let mut table_rows: Vec<Vec<(Vec<Char>, ParagraphProps, u16, Vec<u8>)>> = Vec::new();
        let mut current_row: Vec<(Vec<Char>, ParagraphProps, u16, Vec<u8>)> = Vec::new();
        let mut cell_paragraphs: Vec<Vec<Char>> = Vec::new();

        for para in paragraphs {
            let Some(last) = para.last() else { continue };
            let (istd, papx) = self.papx_at(last.fc);
            let (mut props, _) = self.styles.paragraph(istd);
            let mut in_table = false;
            let mut row_end = false;
            let mut ilfo = 0u16;
            let mut ilvl = 0u8;
            for sprm in SprmIter::new(&papx) {
                match sprm.opcode {
                    0x2416 => in_table = sprm.byte() != 0,
                    0x2417 => row_end = sprm.byte() != 0,
                    _ => {}
                }
            }
            for style in self.styles.chain(istd) {
                for sprm in SprmIter::new(&style.papx) {
                    match sprm.opcode {
                        0x2416 => in_table = sprm.byte() != 0,
                        0x2417 => row_end = sprm.byte() != 0,
                        _ => {}
                    }
                }
            }
            for sprm in SprmIter::new(&papx) {
                apply_paragraph_sprm(&sprm, &mut props, Some((&mut ilfo, &mut ilvl)));
            }
            if ilfo == 0 {
                if let Some(chain_numbering) = props.numbering.clone() {
                    ilfo = chain_numbering.0.parse().unwrap_or(0);
                    ilvl = chain_numbering.1 as u8;
                }
            }

            if in_table || row_end {
                if row_end && last.ch == '\u{7}' {
                    current_row.push((para.clone(), props.clone(), istd, papx.clone()));
                    table_rows.push(std::mem::take(&mut current_row));
                    cell_paragraphs.clear();
                    continue;
                }
                cell_paragraphs.push(para.clone());
                if last.ch == '\u{7}' {
                    let mut cell_chars: Vec<Char> = Vec::new();
                    for p in cell_paragraphs.drain(..) {
                        cell_chars.extend(p);
                    }
                    current_row.push((cell_chars, props.clone(), istd, papx.clone()));
                }
                continue;
            }
            if !table_rows.is_empty() {
                blocks.push(Block::Table(self.table(std::mem::take(&mut table_rows), footnotes)));
            }
            if std::env::var_os("SIMPLE_CONVERTER_DOC_TRACE").is_some() {
                let text: String = para.iter().map(|c| c.ch).filter(|c| !c.is_control()).take(40).collect();
                eprintln!("para istd={istd} ilfo={ilfo} ilvl={ilvl} {text:?}");
            }
            blocks.push(Block::Paragraph(self.paragraph(&para, props, istd, ilfo, ilvl, footnotes)));
        }
        if !table_rows.is_empty() {
            blocks.push(Block::Table(self.table(table_rows, footnotes)));
        }
        blocks
    }

    fn paragraph(
        &self,
        chars: &[Char],
        props: ParagraphProps,
        istd: u16,
        ilfo: u16,
        ilvl: u8,
        footnotes: &HashMap<u32, Vec<Block>>,
    ) -> Paragraph {
        let (_, base) = self.styles.paragraph(istd);
        let mut props = props;
        let mut list = None;

        let first_fc = chars.first().map(|c| c.fc).unwrap_or(0);
        let last_fc = chars.last().map(|c| c.fc + if self.piece_for(chars.last().unwrap().cp).map(|p| p.compressed).unwrap_or(true) { 1 } else { 2 }).unwrap_or(first_fc);
        let runs = self.chpx_runs(first_fc, last_fc);

        let mut inlines = Vec::new();
        let mut field_depth = 0;
        let mut field_skip = false;
        let mut field_instr = String::new();
        let mut mark = base.clone();
        let mut buffer = String::new();
        let mut buffer_props: Option<RunProps> = None;

        let flush = |buffer: &mut String, props: &Option<RunProps>, inlines: &mut Vec<Inline>| {
            if !buffer.is_empty() {
                if let Some(p) = props {
                    inlines.push(Inline::Text { text: std::mem::take(buffer), props: p.clone() });
                }
            }
        };

        for c in chars {
            if c.ch == '\u{0}' {
                continue;
            }
            let mut props = base.clone();
            let mut special = false;
            let mut pic_location = None;
            if let Some((_, _, grpprl)) = runs.iter().find(|(a, b, _)| c.fc >= *a && c.fc < *b) {
                let mut istd_char = None;
                for sprm in SprmIter::new(grpprl) {
                    if sprm.opcode == 0x4A30 {
                        istd_char = Some(sprm.word());
                    }
                }
                if let Some(istd_char) = istd_char {
                    self.styles.character(istd_char, &mut props);
                }
                for sprm in SprmIter::new(grpprl) {
                    match sprm.opcode {
                        0x0855 => special = sprm.byte() != 0,
                        0x6A03 => pic_location = Some(sprm.dword()),
                        _ => apply_character_sprm(&sprm, &mut props, self.fonts, Some(&base)),
                    }
                }
            }
            if matches!(c.ch, '\r' | '\u{7}') {
                mark = props;
                continue;
            }
            if props.hidden == Some(true) && !matches!(c.ch, '\u{13}' | '\u{14}' | '\u{15}') {
                continue;
            }
            match c.ch {
                '\u{13}' => {
                    flush(&mut buffer, &buffer_props, &mut inlines);
                    field_depth += 1;
                    field_instr.clear();
                    field_skip = false;
                    continue;
                }
                '\u{14}' => {
                    if field_depth > 0 {
                        let instr = field_instr.trim().to_ascii_uppercase();
                        let kind = if instr.starts_with("PAGE") && !instr.starts_with("PAGEREF") {
                            Some(FieldKind::Page)
                        } else if instr.starts_with("NUMPAGES") || instr.starts_with("SECTIONPAGES") {
                            Some(FieldKind::NumPages)
                        } else {
                            None
                        };
                        if let Some(kind) = kind {
                            flush(&mut buffer, &buffer_props, &mut inlines);
                            inlines.push(Inline::Field { kind, props: props.clone() });
                            field_skip = true;
                        }
                    }
                    continue;
                }
                '\u{15}' => {
                    if field_depth > 0 {
                        field_depth -= 1;
                    }
                    field_skip = false;
                    continue;
                }
                _ => {}
            }
            if field_depth > 0 && field_instr.len() < 512 && !field_skip && field_instr_active(&inlines, field_depth) {
                field_instr.push(c.ch);
            }
            if field_skip {
                continue;
            }
            if field_depth > 0 && !field_started_result(&field_instr) {
                field_instr.push(c.ch);
                continue;
            }
            let inline = match c.ch {
                '\t' => Some(Inline::Tab),
                '\u{b}' => Some(Inline::LineBreak),
                '\u{c}' => Some(Inline::PageBreak),
                '\u{1}' if special => {
                    pic_location.and_then(|offset| self.picture(offset)).map(Inline::Drawing)
                }
                '\u{2}' if special => footnotes.get(&c.cp).map(|blocks| Inline::Footnote(blocks.clone())),
                '\u{1e}' => {
                    buffer.push('\u{2011}');
                    None
                }
                '\u{1f}' => {
                    buffer.push('\u{ad}');
                    None
                }
                '\u{5}' | '\u{8}' | '\u{3}' | '\u{4}' => None,
                _ if special && (c.ch as u32) < 0x20 => None,
                _ => {
                    match &buffer_props {
                        Some(p) if *p == props => {}
                        _ => {
                            flush(&mut buffer, &buffer_props, &mut inlines);
                            buffer_props = Some(props.clone());
                        }
                    }
                    buffer.push(c.ch);
                    None
                }
            };
            if let Some(inline) = inline {
                flush(&mut buffer, &buffer_props, &mut inlines);
                inlines.push(inline);
            }
        }
        flush(&mut buffer, &buffer_props, &mut inlines);
        if ilfo != 0 && ilfo != 0x07FF {
            if let Some((lsid, level)) = self.lists.level(ilfo, ilvl) {
                let mut lvl_props = ParagraphProps::default();
                for sprm in SprmIter::new(&level.papx) {
                    apply_paragraph_sprm(&sprm, &mut lvl_props, None);
                }
                let mut merged = lvl_props;
                merged.merge(&props);
                if props.indent_left.is_none() {
                    merged.indent_left = merged.indent_left.or(props.indent_left);
                }
                props = merged;
                let mut label_props = mark.clone();
                for sprm in SprmIter::new(&level.chpx) {
                    apply_character_sprm(&sprm, &mut label_props, self.fonts, None);
                }
                if let Some(text) = self.list_label(lsid, ilvl, &level) {
                    list = Some(ListLabel {
                        text,
                        props: label_props,
                        tab_pos: None,
                        suffix: match level.follow {
                            1 => ListSuffix::Space,
                            2 => ListSuffix::Nothing,
                            _ => ListSuffix::Tab,
                        },
                    });
                }
            }
        }

        Paragraph { props, mark, inlines, anchors: Vec::new(), list }
    }

    fn list_label(&self, lsid: u32, ilvl: u8, level: &ListLevelDef) -> Option<String> {
        if level.nfc == 0xFF {
            return None;
        }
        let mut counters = self.counters.borrow_mut();
        let values = counters.entry(lsid).or_insert_with(|| vec![0; 9]);
        let ilvl = ilvl.min(8) as usize;
        if values[ilvl] == 0 {
            values[ilvl] = level.start - 1;
        }
        values[ilvl] += 1;
        for deeper in ilvl + 1..9 {
            values[deeper] = 0;
        }
        if level.nfc == 23 {
            let s: String = level.text.iter().filter_map(|u| char::from_u32(*u as u32)).collect();
            return Some(crate::pptx::map_bullet_pub(&s));
        }
        let mut out = String::new();
        for &u in &level.text {
            if (u as usize) < 9 {
                let l = u as usize;
                let v = values[l].max(1);
                let nfc = self
                    .lists
                    .lists
                    .get(&lsid)
                    .and_then(|d| d.levels.get(l))
                    .map(|d| d.nfc)
                    .unwrap_or(level.nfc);
                out.push_str(&format_nfc(v, nfc));
            } else if let Some(c) = char::from_u32(u as u32) {
                out.push(c);
            }
        }
        Some(out)
    }

    fn picture(&self, offset: u32) -> Option<Drawing> {
        let data = &self.file.data;
        let at = offset as usize;
        if at + 0x44 > data.len() {
            return None;
        }
        let lcb = u32_at(data, at) as usize;
        let cb_header = u16_at(data, at + 4) as usize;
        let dxa_goal = i16_at(data, at + 0x1C) as f64 / 20.0;
        let dya_goal = i16_at(data, at + 0x1E) as f64 / 20.0;
        let mx = u16_at(data, at + 0x20) as f64 / 1000.0;
        let my = u16_at(data, at + 0x22) as f64 / 1000.0;
        let width = if mx > 0.0 { dxa_goal * mx } else { dxa_goal };
        let height = if my > 0.0 { dya_goal * my } else { dya_goal };
        if width <= 0.0 || height <= 0.0 {
            return None;
        }
        let body = data.get(at + cb_header..(at + lcb).min(data.len()))?;
        let content = find_blip(body)
            .and_then(|bytes| ImageFormat::sniff(bytes).map(|format| ImageData { data: bytes.to_vec(), format }))
            .map(DrawingContent::Image)
            .unwrap_or(DrawingContent::Placeholder);
        Some(Drawing::new(width, height, content))
    }

    fn table(&self, rows: Vec<Vec<(Vec<Char>, ParagraphProps, u16, Vec<u8>)>>, footnotes: &HashMap<u32, Vec<Block>>) -> Table {
        let mut table = Table {
            cell_margins: CellMargins { top: Some(0.0), left: Some(5.4), bottom: Some(0.0), right: Some(5.4) },
            ..Table::default()
        };
        let mut left_edge: Option<f64> = None;
        for row_cells in rows {
            let Some((_, _, istd, row_papx)) = row_cells.last() else { continue };
            let mut tap = Tap::default();
            for papx in self.styles.table_papx(*istd) {
                for sprm in SprmIter::new(&papx) {
                    apply_table_sprm(&sprm, &mut tap);
                }
            }
            for sprm in SprmIter::new(row_papx) {
                apply_table_sprm(&sprm, &mut tap);
            }
            apply_table_borders(&mut tap);
            let mut row = Row { height: tap.height, exact_height: tap.exact, ..Row::default() };
            let cells = &row_cells[..row_cells.len() - 1];
            if left_edge.is_none() {
                left_edge = tap.centers.first().copied();
                table.indent = tap.centers.first().copied().unwrap_or(0.0) + tap.gap_half;
                if table.columns.is_empty() && tap.centers.len() > 1 {
                    table.columns = tap.centers.windows(2).map(|w| w[1] - w[0]).collect();
                }
            }
            for (i, (chars, props, cell_istd, _)) in cells.iter().enumerate() {
                let tc = tap.cells.get(i).cloned().unwrap_or_default();
                if tc.merged_continuation {
                    if let Some(last) = row.cells.last_mut() {
                        last.span += 1;
                    }
                    continue;
                }
                let mut cell = Cell {
                    span: 1,
                    width: tap.centers.get(i + 1).zip(tap.centers.get(i)).map(|(b, a)| b - a),
                    shading: tc.shading,
                    borders: tc.borders,
                    valign: tc.valign,
                    vertical_merge: if tc.vert_restart {
                        Some(VerticalMerge::Restart)
                    } else if tc.vert_merge {
                        Some(VerticalMerge::Continue)
                    } else {
                        None
                    },
                    ..Cell::default()
                };
                if tap.borders_default {
                    let border = BorderSide::Line { width: 0.5, color: Color(0, 0, 0), style: LineStyle::Solid };
                    for side in [&mut cell.borders.top, &mut cell.borders.left, &mut cell.borders.bottom, &mut cell.borders.right] {
                        if *side == BorderSide::Unset {
                            *side = border;
                        }
                    }
                }
                let mut paragraphs: Vec<Vec<Char>> = Vec::new();
                let mut current = Vec::new();
                for c in chars {
                    current.push(c.clone());
                    if matches!(c.ch, '\r' | '\u{7}') {
                        paragraphs.push(std::mem::take(&mut current));
                    }
                }
                for p in paragraphs {
                    let last_fc = p.last().map(|c| c.fc).unwrap_or(0);
                    let (istd_p, papx) = self.papx_at(last_fc);
                    let (mut pprops, _) = self.styles.paragraph(istd_p);
                    let mut ilfo = 0u16;
                    let mut ilvl = 0u8;
                    for sprm in SprmIter::new(&papx) {
                        apply_paragraph_sprm(&sprm, &mut pprops, Some((&mut ilfo, &mut ilvl)));
                    }
                    let _ = (props, cell_istd);
                    cell.blocks.push(Block::Paragraph(self.paragraph(&p, pprops, istd_p, ilfo, ilvl, footnotes)));
                }
                row.cells.push(cell);
            }
            if !row.cells.is_empty() {
                table.rows.push(row);
            }
        }
        if table.columns.is_empty() {
            let count = table.rows.iter().map(|r| r.cells.len()).max().unwrap_or(1);
            table.columns = vec![468.0 / count as f64; count];
        }
        table
    }
}

fn field_instr_active(_inlines: &[Inline], _depth: usize) -> bool {
    false
}

fn field_started_result(_instr: &str) -> bool {
    false
}

#[derive(Debug, Clone, Default)]
struct Tc {
    merged_continuation: bool,
    vert_merge: bool,
    vert_restart: bool,
    valign: VAlign,
    borders: Borders,
    shading: Option<Color>,
}

#[derive(Debug, Clone, Default)]
struct Tap {
    centers: Vec<f64>,
    cells: Vec<Tc>,
    height: Option<f64>,
    exact: bool,
    gap_half: f64,
    borders_default: bool,
    table_borders: Option<[BorderSide; 6]>,
}

fn apply_table_borders(tap: &mut Tap) {
    let Some([top, left, bottom, right, _inside_h, inside_v]) = tap.table_borders else { return };
    let count = tap.cells.len();
    for (i, tc) in tap.cells.iter_mut().enumerate() {
        let own = [tc.borders.top, tc.borders.left, tc.borders.bottom, tc.borders.right]
            .iter()
            .any(|b| matches!(b, BorderSide::Line { .. }));
        if own {
            continue;
        }
        tc.borders.top = top;
        tc.borders.bottom = bottom;
        tc.borders.left = if i == 0 { left } else { inside_v };
        tc.borders.right = if i + 1 == count { right } else { inside_v };
    }
}

fn apply_table_sprm(sprm: &Sprm, tap: &mut Tap) {
    match sprm.opcode {
        0xD608 => {
            let op = sprm.operand;
            if op.is_empty() {
                return;
            }
            let itc = op[0] as usize;
            tap.centers = (0..=itc).map(|i| i16_at(op, 1 + i * 2) as f64 / 20.0).collect();
            let tc_start = 1 + (itc + 1) * 2;
            tap.cells = (0..itc)
                .map(|i| {
                    let at = tc_start + i * 20;
                    let flags = u16_at(op, at);
                    let border = |off: usize| brc80(op, at + off);
                    Tc {
                        merged_continuation: flags & 0x0002 != 0 && flags & 0x0001 == 0,
                        vert_merge: flags & 0x0020 != 0 && flags & 0x0040 == 0,
                        vert_restart: flags & 0x0040 != 0,
                        valign: match (flags >> 7) & 0x3 {
                            1 => VAlign::Center,
                            2 => VAlign::Bottom,
                            _ => VAlign::Top,
                        },
                        borders: Borders {
                            top: border(4),
                            left: border(8),
                            bottom: border(12),
                            right: border(16),
                            ..Borders::default()
                        },
                        shading: None,
                    }
                })
                .collect();
        }
        0x9407 => {
            let v = sprm.word() as i16;
            if v != 0 {
                tap.height = Some((v.abs() as f64) / 20.0);
                tap.exact = v < 0;
            }
        }
        0x9601 => tap.gap_half = sprm.word() as i16 as f64 / 20.0,
        0xD605 | 0xD613 => {
            let op = sprm.operand;
            if std::env::var_os("SIMPLE_CONVERTER_DOC_TRACE").is_some() {
                eprintln!("table borders {:#x} {:02x?} cells={:?}", sprm.opcode, op, tap.cells.iter().map(|c| c.borders.top).collect::<Vec<_>>());
            }
            let size = if sprm.opcode == 0xD605 { 4 } else { 8 };
            let read = |i: usize| if size == 4 { brc80(op, i * 4) } else { brc(op, i * 8) };
            tap.table_borders = Some([read(0), read(1), read(2), read(3), read(4), read(5)]);
            tap.borders_default = false;
        }
        0xD609 | 0xD612 | 0xD616 | 0xD617 => {
            let op = sprm.operand;
            let size = if sprm.opcode == 0xD609 { 2 } else { 10 };
            let count = op.len() / size;
            let base = match sprm.opcode {
                0xD616 => 22,
                0xD617 => 44,
                _ => 0,
            };
            for i in 0..count {
                let Some(tc) = tap.cells.get_mut(base + i) else { break };
                let shading = if size == 2 {
                    let shd = u16_at(op, i * 2);
                    shd80_color(shd)
                } else {
                    let back = u32_at(op, i * 10 + 4);
                    let pattern = u16_at(op, i * 10 + 8);
                    if back == 0xFF00_0000 || (back == 0xFFFF_FFFF && pattern == 0) { None } else { Some(colorref(back)) }
                };
                if shading.is_some() {
                    tc.shading = shading;
                }
            }
        }
        _ => {}
    }
}

fn brc80(op: &[u8], at: usize) -> BorderSide {
    if at + 4 > op.len() {
        return BorderSide::Unset;
    }
    let width = op[at];
    let kind = op[at + 1];
    let ico = op[at + 2];
    if width == 0xFF && kind == 0xFF {
        return BorderSide::Unset;
    }
    if kind == 0 || width == 0 {
        return BorderSide::None;
    }
    BorderSide::Line { width: (width as f64 / 8.0).max(0.25), color: ico_color(ico).unwrap_or(Color(0, 0, 0)), style: brc_style(kind) }
}

fn brc_style(kind: u8) -> LineStyle {
    match kind {
        3 | 10..=19 | 21 => LineStyle::Double,
        6 => LineStyle::Dotted,
        7 | 8 | 9 | 22 | 23 => LineStyle::Dashed,
        _ => LineStyle::Solid,
    }
}

fn brc(op: &[u8], at: usize) -> BorderSide {
    if at + 8 > op.len() {
        return BorderSide::Unset;
    }
    let cv = u32_at(op, at);
    let width = op[at + 4];
    let kind = op[at + 5];
    if kind == 0 || kind == 0xFF || width == 0 {
        return BorderSide::None;
    }
    BorderSide::Line { width: (width as f64 / 8.0).max(0.25), color: if cv == 0xFF00_0000 { Color(0, 0, 0) } else { colorref(cv) }, style: brc_style(kind) }
}

fn shd80_color(shd: u16) -> Option<Color> {
    if shd == 0xFFFF {
        return None;
    }
    let icob = ((shd >> 5) & 0x1F) as u8;
    let ipat = shd >> 10;
    if ipat == 1 {
        return ico_color(icob & 0x1F).filter(|_| icob != 0);
    }
    ico_color(icob).filter(|_| icob != 0)
}

fn colorref(v: u32) -> Color {
    Color((v & 0xFF) as u8, ((v >> 8) & 0xFF) as u8, ((v >> 16) & 0xFF) as u8)
}

fn ico_color(ico: u8) -> Option<Color> {
    Some(match ico {
        1 => Color(0, 0, 0),
        2 => Color(0, 0, 255),
        3 => Color(0, 255, 255),
        4 => Color(0, 255, 0),
        5 => Color(255, 0, 255),
        6 => Color(255, 0, 0),
        7 => Color(255, 255, 0),
        8 => Color(255, 255, 255),
        9 => Color(0, 0, 128),
        10 => Color(0, 128, 128),
        11 => Color(0, 128, 0),
        12 => Color(128, 0, 128),
        13 => Color(128, 0, 0),
        14 => Color(128, 128, 0),
        15 => Color(128, 128, 128),
        16 => Color(192, 192, 192),
        _ => return None,
    })
}

fn format_nfc(value: i64, nfc: u8) -> String {
    use crate::docx::format_number;
    match nfc {
        1 => format_number(value, "upperRoman"),
        2 => format_number(value, "lowerRoman"),
        3 => format_number(value, "upperLetter"),
        4 => format_number(value, "lowerLetter"),
        5 => format_number(value, "ordinal"),
        22 => format!("{value:02}"),
        _ => value.to_string(),
    }
}

fn apply_paragraph_sprm(sprm: &Sprm, props: &mut ParagraphProps, numbering: Option<(&mut u16, &mut u8)>) {
    match sprm.opcode {
        0x2403 | 0x2461 => {
            props.align = Some(match sprm.byte() {
                1 => Align::Center,
                2 => Align::Right,
                3 | 4 | 5 => Align::Justify,
                _ => Align::Left,
            })
        }
        0x840E | 0x845D => props.indent_right = Some(sprm.word() as i16 as f64 / 20.0),
        0x840F | 0x845E => props.indent_left = Some(sprm.word() as i16 as f64 / 20.0),
        0x8411 | 0x8460 => {
            let v = sprm.word() as i16 as f64 / 20.0;
            if v < 0.0 {
                props.indent_hanging = Some(-v);
                props.indent_first_line = Some(0.0);
            } else {
                props.indent_first_line = Some(v);
                props.indent_hanging = Some(0.0);
            }
        }
        0x6412 => {
            let dya = sprm.word() as i16;
            let mult = u16_at(sprm.operand, 2);
            props.line_spacing = Some(if mult != 0 {
                LineSpacing::Multiple(dya as f64 / 240.0)
            } else if dya < 0 {
                LineSpacing::Exact(-(dya as f64) / 20.0)
            } else {
                LineSpacing::AtLeast(dya as f64 / 20.0)
            });
        }
        0xA413 => props.space_before = Some(sprm.word() as f64 / 20.0),
        0xA414 => props.space_after = Some(sprm.word() as f64 / 20.0),
        0x6424 | 0x6425 | 0x6426 | 0x6427 => {
            let side = brc80(sprm.operand, 0);
            let space = sprm.operand.get(3).map(|s| (s & 0x1F) as f64).unwrap_or(0.0);
            set_border(props, sprm.opcode - 0x6424, side, space);
        }
        0xC64E | 0xC64F | 0xC650 | 0xC651 => {
            let side = brc(sprm.operand, 0);
            let space = sprm.operand.get(6).map(|s| (s & 0x1F) as f64).unwrap_or(0.0);
            set_border(props, sprm.opcode - 0xC64E, side, space);
        }
        0x442D => props.shading = shd80_color(sprm.word()),
        0xC64D => {
            let op = sprm.operand;
            if op.len() >= 10 {
                let back = u32_at(op, 4);
                props.shading = if back == 0xFF00_0000 { None } else { Some(colorref(back)) };
            }
        }
        0x2407 => props.keep_next = Some(sprm.byte() != 0),
        0x2408 => props.page_break_before = Some(sprm.byte() != 0),
        0x246D => props.contextual_spacing = Some(sprm.byte() != 0),
        0x260A => {
            if let Some((_, ilvl)) = numbering {
                *ilvl = sprm.byte();
            } else {
                let current = props.numbering.clone().map(|n| n.0).unwrap_or_else(|| "0".into());
                props.numbering = Some((current, sprm.byte() as usize));
            }
        }
        0x460B => {
            if let Some((ilfo, _)) = numbering {
                *ilfo = sprm.word();
            } else {
                let level = props.numbering.clone().map(|n| n.1).unwrap_or(0);
                props.numbering = Some((sprm.word().to_string(), level));
            }
        }
        0xC60D | 0xC615 => {
            let op = sprm.operand;
            if op.is_empty() {
                return;
            }
            let del = op[0] as usize;
            let mut pos = 1 + del * 2;
            if let Some(&add) = op.get(pos) {
                pos += 1;
                for i in 0..add as usize {
                    let p = i16_at(op, pos + i * 2) as f64 / 20.0;
                    props.tabs.push(TabStop { pos: p, clear: false });
                }
            }
        }
        _ => {}
    }
}

fn apply_character_sprm(sprm: &Sprm, props: &mut RunProps, fonts: &[String], style_base: Option<&RunProps>) {
    let toggle = |current: Option<bool>, base: Option<bool>| -> Option<bool> {
        match sprm.byte() {
            0 => Some(false),
            1 => Some(true),
            128 => base.or(current),
            129 => Some(!base.unwrap_or(false)),
            _ => current,
        }
    };
    match sprm.opcode {
        0x0835 => props.bold = toggle(props.bold, style_base.and_then(|b| b.bold)),
        0x0836 => props.italic = toggle(props.italic, style_base.and_then(|b| b.italic)),
        0x0837 | 0x2A53 => props.strike = toggle(props.strike, style_base.and_then(|b| b.strike)),
        0x083A => props.small_caps = toggle(props.small_caps, style_base.and_then(|b| b.small_caps)),
        0x083B => props.caps = toggle(props.caps, style_base.and_then(|b| b.caps)),
        0x083C => props.hidden = toggle(props.hidden, style_base.and_then(|b| b.hidden)),
        0x2A3E => props.underline = Some(sprm.byte() != 0),
        0x4A43 => props.size = Some(sprm.word() as f64 / 2.0),
        0x8840 => props.letter_spacing = Some(sprm.word() as i16 as f64 / 20.0),
        0x484B => props.kerning = Some(sprm.word() != 0),
        0x0800 => props.hidden = Some(sprm.byte() != 0),
        0x6865 => {
            let side = brc80(sprm.operand, 0);
            let space = sprm.operand.get(3).map(|s| (s & 0x1F) as f64).unwrap_or(0.0);
            props.border = Some((side, space));
        }
        0xCA72 => {
            let side = brc(sprm.operand, 0);
            let space = sprm.operand.get(6).map(|s| (s & 0x1F) as f64).unwrap_or(0.0);
            props.border = Some((side, space));
        }
        0x4A4F => {
            if let Some(f) = fonts.get(sprm.word() as usize).filter(|f| !f.is_empty()) {
                props.font = Some(f.clone());
            }
        }
        0x2A42 => {
            if let Some(c) = ico_color(sprm.byte()) {
                props.color = Some(c);
            } else if sprm.byte() == 0 {
                props.color = None;
            }
        }
        0x6870 => {
            let v = sprm.dword();
            props.color = if v == 0xFF00_0000 { None } else { Some(colorref(v)) };
        }
        0x2A48 => {
            props.vertical = Some(match sprm.byte() {
                1 => VerticalAlign::Superscript,
                2 => VerticalAlign::Subscript,
                _ => VerticalAlign::Baseline,
            })
        }
        0x2A0C => props.highlight = ico_color(sprm.byte()),
        _ => {}
    }
}

fn apply_section_sprm(sprm: &Sprm, info: &mut SectionInfo) {
    match sprm.opcode {
        0xB01F => info.page.width = sprm.word() as f64 / 20.0,
        0xB020 => info.page.height = sprm.word() as f64 / 20.0,
        0xB021 => info.page.margin.left = sprm.word() as f64 / 20.0,
        0xB022 => info.page.margin.right = sprm.word() as f64 / 20.0,
        0x9023 => info.page.margin.top = (sprm.word() as i16).abs() as f64 / 20.0,
        0x9024 => info.page.margin.bottom = (sprm.word() as i16).abs() as f64 / 20.0,
        0xB017 => info.page.margin.header = sprm.word() as f64 / 20.0,
        0xB018 => info.page.margin.footer = sprm.word() as f64 / 20.0,
        0xB025 => info.page.margin.left += sprm.word() as f64 / 20.0,
        0x500B => info.columns = sprm.word() as usize + 1,
        0x900C => info.column_gap = sprm.word() as f64 / 20.0,
        0x300A => info.title_page = sprm.byte() != 0,
        0x501C => info.page_start = Some(sprm.word() as i64),
        _ => {}
    }
}

fn find_blip(body: &[u8]) -> Option<&[u8]> {
    let mut pos = 0;
    while pos + 8 <= body.len() {
        let ver_inst = u16_at(body, pos);
        let rec_type = u16_at(body, pos + 2);
        let rec_len = u32_at(body, pos + 4) as usize;
        let instance = ver_inst >> 4;
        let version = ver_inst & 0x0F;
        pos += 8;
        match rec_type {
            0xF01D | 0xF01E | 0xF01F | 0xF029 | 0xF02A | 0xF01A | 0xF01B => {
                let uid_len = match rec_type {
                    0xF01D if instance == 0x46B || instance == 0x6E3 => 32,
                    0xF01E if instance == 0x6E1 => 32,
                    0xF01F if instance == 0x7A9 => 32,
                    0xF01A if instance == 0x3D5 => 32,
                    0xF01B if instance == 0x217 => 32,
                    _ => 16,
                };
                if matches!(rec_type, 0xF01A | 0xF01B) {
                    return None;
                }
                let start = pos + uid_len + 1;
                return body.get(start..(pos + rec_len).min(body.len()));
            }
            0xF007 => {
                pos += 36;
                continue;
            }
            _ if version == 0x0F => continue,
            _ => pos += rec_len,
        }
    }
    None
}

fn cp1252(b: u8) -> char {
    let bytes = [b];
    let (out, _) = encoding_rs::WINDOWS_1252.decode_without_bom_handling(&bytes);
    out.chars().next().unwrap_or('\u{FFFD}')
}

fn set_border(props: &mut ParagraphProps, index: u16, side: BorderSide, space: f64) {
    let (target, gap) = match index {
        0 => (&mut props.borders.top, &mut props.border_space.top),
        1 => (&mut props.borders.left, &mut props.border_space.left),
        2 => (&mut props.borders.bottom, &mut props.border_space.bottom),
        _ => (&mut props.borders.right, &mut props.border_space.right),
    };
    *target = side;
    *gap = space;
}
