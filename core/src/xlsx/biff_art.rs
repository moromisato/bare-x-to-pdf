use crate::model::{ImageData, ImageFormat};

const CONTAINER_VERSION: u16 = 0x0F;
const BSTORE: u16 = 0xF001;
const BSE: u16 = 0xF007;
const SPGR: u16 = 0xF003;
const SP: u16 = 0xF004;
const FSP: u16 = 0xF00A;
const OPT: u16 = 0xF00B;
const CLIENT_ANCHOR: u16 = 0xF010;
const PROP_PIB: u16 = 0x0104;
const PROP_GROUP_BOOLEANS: u16 = 0x03BF;

struct Art<'a> {
    kind: u16,
    instance: u16,
    container: bool,
    body: &'a [u8],
}

fn art_records(data: &[u8]) -> Vec<Art<'_>> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos + 8 <= data.len() {
        let ver_inst = u16::from_le_bytes([data[pos], data[pos + 1]]);
        let kind = u16::from_le_bytes([data[pos + 2], data[pos + 3]]);
        let len = u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]]) as usize;
        let start = pos + 8;
        let end = start.saturating_add(len).min(data.len());
        out.push(Art { kind, instance: ver_inst >> 4, container: ver_inst & 0x0F == CONTAINER_VERSION, body: &data[start..end] });
        pos = end;
    }
    out
}

pub struct Anchor {
    pub col1: u16,
    pub dx1: u16,
    pub row1: u16,
    pub dy1: u16,
    pub col2: u16,
    pub dx2: u16,
    pub row2: u16,
    pub dy2: u16,
}

pub struct Shape {
    pub anchor: Option<Anchor>,
    pub picture: Option<usize>,
    pub hidden: bool,
    pub child: bool,
}

pub fn blip_store(data: &[u8]) -> Vec<Option<ImageData>> {
    let mut out = Vec::new();
    for group in art_records(data).into_iter().filter(|r| r.container) {
        for store in art_records(group.body).into_iter().filter(|r| r.kind == BSTORE) {
            for entry in art_records(store.body) {
                out.push(if entry.kind == BSE { bse_image(entry.body) } else { None });
            }
        }
    }
    out
}

fn bse_image(body: &[u8]) -> Option<ImageData> {
    let name_len = *body.get(33)? as usize;
    let blip = art_records(body.get(36 + name_len..)?).into_iter().next()?;
    let (uid_count, skip_tag) = match blip.kind {
        0xF01D | 0xF02A => (if matches!(blip.instance, 0x46B | 0x6E3) { 2 } else { 1 }, true),
        0xF01E => (if blip.instance == 0x6E1 { 2 } else { 1 }, true),
        0xF01F => (if blip.instance == 0x7A9 { 2 } else { 1 }, true),
        _ => return None,
    };
    let data = blip.body.get(uid_count * 16 + usize::from(skip_tag)..)?;
    match blip.kind {
        0xF01F => dib_to_png(data).map(|png| ImageData { data: png, format: ImageFormat::Png }),
        0xF01E => Some(ImageData { data: data.to_vec(), format: ImageFormat::Png }),
        _ => Some(ImageData { data: data.to_vec(), format: ImageFormat::Jpeg }),
    }
}

pub fn shapes(data: &[u8]) -> Vec<Shape> {
    let mut out = Vec::new();
    for container in art_records(data).into_iter().filter(|r| r.container) {
        for record in art_records(container.body) {
            if record.kind == SPGR {
                collect_group(record.body, &mut out);
            }
        }
    }
    out
}

fn collect_group(body: &[u8], out: &mut Vec<Shape>) {
    for child in art_records(body) {
        match child.kind {
            SP => out.extend(shape(child.body)),
            SPGR => collect_group(child.body, out),
            _ => {}
        }
    }
}

fn shape(body: &[u8]) -> Option<Shape> {
    let mut result = Shape { anchor: None, picture: None, hidden: false, child: false };
    for record in art_records(body) {
        match record.kind {
            FSP => {
                let flags = u32::from_le_bytes(record.body.get(4..8)?.try_into().ok()?);
                if flags & 0x04 != 0 {
                    return None;
                }
                result.child = flags & 0x03 != 0;
            }
            OPT => {
                for i in 0..record.instance as usize {
                    let at = i * 6;
                    let Some(prop) = record.body.get(at..at + 6) else { break };
                    let id = u16::from_le_bytes([prop[0], prop[1]]) & 0x3FFF;
                    let value = u32::from_le_bytes([prop[2], prop[3], prop[4], prop[5]]);
                    match id {
                        PROP_PIB => result.picture = (value > 0).then(|| value as usize - 1),
                        PROP_GROUP_BOOLEANS => result.hidden = value & 0x0002_0000 != 0 && value & 0x0002 != 0,
                        _ => {}
                    }
                }
            }
            CLIENT_ANCHOR => {
                let b = record.body;
                let at = |i: usize| b.get(i..i + 2).map_or(0, |v| u16::from_le_bytes([v[0], v[1]]));
                if b.len() >= 18 {
                    result.anchor = Some(Anchor {
                        col1: at(2),
                        dx1: at(4),
                        row1: at(6),
                        dy1: at(8),
                        col2: at(10),
                        dx2: at(12),
                        row2: at(14),
                        dy2: at(16),
                    });
                }
            }
            _ => {}
        }
    }
    Some(result)
}

fn dib_to_png(dib: &[u8]) -> Option<Vec<u8>> {
    let u32_at = |i: usize| dib.get(i..i + 4).map(|v| u32::from_le_bytes([v[0], v[1], v[2], v[3]]));
    let header = u32_at(0)? as usize;
    let width = u32_at(4)? as i32;
    let height = u32_at(8)? as i32;
    let bits = u16::from_le_bytes(dib.get(14..16)?.try_into().ok()?);
    let compression = u32_at(16)?;
    if width <= 0 || height == 0 || compression != 0 || !matches!(bits, 1 | 4 | 8 | 24 | 32) {
        return None;
    }
    let (w, h) = (width as usize, height.unsigned_abs() as usize);
    let used = u32_at(32).unwrap_or(0) as usize;
    let colors = if bits <= 8 { if used > 0 { used } else { 1 << bits } } else { 0 };
    let palette = dib.get(header..header + colors * 4)?;
    let pixels = dib.get(header + colors * 4..)?;
    let stride = (w * bits as usize).div_ceil(32) * 4;
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        let source = if height > 0 { h - 1 - y } else { y };
        let row = pixels.get(source * stride..source * stride + stride)?;
        for x in 0..w {
            let (b, g, r, a) = match bits {
                24 => (row[x * 3], row[x * 3 + 1], row[x * 3 + 2], 255),
                32 => (row[x * 4], row[x * 4 + 1], row[x * 4 + 2], 255),
                _ => {
                    let bit = x * bits as usize;
                    let byte = row[bit / 8];
                    let index = ((byte >> (8 - bits as usize - bit % 8)) & ((1u16 << bits) - 1) as u8) as usize;
                    let c = palette.get(index * 4..index * 4 + 3)?;
                    (c[0], c[1], c[2], 255)
                }
            };
            rgba.extend_from_slice(&[r, g, b, a]);
        }
    }
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, w as u32, h as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(&rgba).ok()?;
    }
    Some(out)
}
