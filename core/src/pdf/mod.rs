use crate::error::Error;
use crate::model::*;
use pdfium_render::prelude::*;
use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;
use std::sync::OnceLock;

static PDFIUM: OnceLock<Pdfium> = OnceLock::new();

const GAP_FACTOR: f64 = 0.6;
const BASELINE_TOLERANCE: f64 = 0.3;

fn pdfium(path: &Path) -> Result<&'static Pdfium, Error> {
    if let Some(instance) = PDFIUM.get() {
        return Ok(instance);
    }
    let bindings = Pdfium::bind_to_library(path)
        .map_err(|e| Error::new(format!("loading PDFium from {}: {e:?}", path.display())))?;
    let _ = PDFIUM.set(Pdfium::new(bindings));
    PDFIUM
        .get()
        .ok_or_else(|| Error::new("PDFium failed to initialise"))
}

fn err(e: PdfiumError) -> Error {
    Error::new(format!("pdfium: {e:?}"))
}

pub fn read(bytes: &[u8], pdfium_path: &Path, background_scale: f32) -> Result<FixedDocument, Error> {
    let pdfium = pdfium(pdfium_path)?;
    let document = pdfium
        .load_pdf_from_byte_slice(bytes, None)
        .map_err(|e| Error::new(format!("not a readable pdf: {e:?}")))?;

    let mut out = FixedDocument::default();
    for mut page in document.pages().iter() {
        out.pages.push(read_page(&mut page, background_scale)?);
    }
    dump(&out);
    Ok(out)
}

struct CharInfo {
    ch: char,
    left: f64,
    right: f64,
    baseline: f64,
    ascent: f64,
    descent: f64,
    style: TextRun,
}

fn read_page(page: &mut PdfPage<'_>, scale: f32) -> Result<FixedPage, Error> {
    let width = page.width().value as f64;
    let height = page.height().value as f64;
    let mut fixed = FixedPage {
        width,
        height,
        ..FixedPage::default()
    };

    let chars = collect_chars(page)?;
    fixed.lines = group_lines(&chars, height);
    fixed.background = background(page, scale)?;
    Ok(fixed)
}

fn collect_chars(page: &PdfPage<'_>) -> Result<Vec<CharInfo>, Error> {
    let text = page.text().map_err(err)?;
    let mut metrics: HashMap<(String, i64), (f64, f64)> = HashMap::new();
    let mut chars = Vec::new();

    for ch in text.chars().iter() {
        let Some(c) = ch.unicode_char() else { continue };
        if c == '\n' || c == '\r' || c == '\u{fffe}' || c == '\u{ffff}' {
            continue;
        }
        let Ok(bounds) = ch.loose_bounds().or_else(|_| ch.tight_bounds()) else { continue };
        let Ok((_, oy)) = ch.origin() else { continue };

        let mut size = ch.scaled_font_size().value as f64;
        if size <= 0.1 {
            size = ch.unscaled_font_size().value as f64;
        }
        if size <= 0.1 {
            size = (bounds.height().value as f64).max(1.0);
        }

        let raw_name = ch.font_name();
        let (family, bold_name, italic_name) = normalize_font(&raw_name);
        let key = (raw_name.clone(), (size * 100.0).round() as i64);
        let (ascent, descent) = *metrics.entry(key).or_insert_with(|| {
            let fallback = (size * 0.9, size * 0.22);
            ch.text_object()
                .ok()
                .map(|object| {
                    let font = object.font();
                    let points = PdfPoints::new(size as f32);
                    let asc = font.ascent(points).map(|p| p.value as f64).unwrap_or(fallback.0);
                    let desc = font.descent(points).map(|p| (p.value as f64).abs()).unwrap_or(fallback.1);
                    if asc > 0.0 { (asc, desc) } else { fallback }
                })
                .unwrap_or(fallback)
        });

        let color = ch
            .fill_color()
            .map(|c| Color(c.red(), c.green(), c.blue()))
            .unwrap_or(Color(0, 0, 0));

        chars.push(CharInfo {
            ch: c,
            left: bounds.left().value as f64,
            right: bounds.right().value as f64,
            baseline: oy.value as f64,
            ascent,
            descent,
            style: TextRun {
                text: String::new(),
                font: family,
                size,
                bold: bold_name || ch.font_is_bold_reenforced(),
                italic: italic_name || ch.font_is_italic(),
                color,
            },
        });
    }
    Ok(chars)
}

fn group_lines(chars: &[CharInfo], page_height: f64) -> Vec<TextLine> {
    let mut lines = Vec::new();
    let mut current: Vec<&CharInfo> = Vec::new();

    let flush = |current: &mut Vec<&CharInfo>, lines: &mut Vec<TextLine>| {
        if let Some(line) = build_line(current, page_height) {
            lines.push(line);
        }
        current.clear();
    };

    for ch in chars {
        if let Some(prev) = current.last() {
            let size = prev.style.size.max(ch.style.size).max(1.0);
            let same_baseline = (prev.baseline - ch.baseline).abs() <= BASELINE_TOLERANCE * size;
            let gap = ch.left - prev.right;
            let backwards = gap < -0.5 * size;
            let wide_gap = gap > GAP_FACTOR * size;
            if !same_baseline || backwards || wide_gap || ch.ch.is_whitespace() {
                flush(&mut current, &mut lines);
            }
        }
        if ch.ch.is_whitespace() {
            continue;
        }
        current.push(ch);
    }
    flush(&mut current, &mut lines);
    lines
}

fn dump(doc: &FixedDocument) {
    let Ok(path) = std::env::var("SIMPLE_CONVERTER_DUMP_FIXED") else { return };
    let mut out = String::new();
    for (i, page) in doc.pages.iter().enumerate() {
        out.push_str(&format!("page {} {}x{} background={}\n", i + 1, page.width, page.height, page.background.is_some()));
        for line in &page.lines {
            let text: String = line.runs.iter().map(|r| r.text.as_str()).collect();
            let run = &line.runs[0];
            out.push_str(&format!(
                "  x={:.2} top={:.2} baseline={:.2} w={:.2} h={:.2} asc={:.2} {} {}{}{} {:?}\n",
                line.x,
                line.top,
                page.height - line.top - line.ascent,
                line.width,
                line.height,
                line.ascent,
                run.font,
                run.size,
                if run.bold { " bold" } else { "" },
                if run.italic { " italic" } else { "" },
                text
            ));
        }
    }
    let _ = std::fs::write(path, out);
}

fn build_line(chars: &[&CharInfo], page_height: f64) -> Option<TextLine> {
    let visible: Vec<&&CharInfo> = chars.iter().filter(|c| !c.ch.is_whitespace()).collect();
    if visible.is_empty() {
        return None;
    }
    let left = visible.iter().map(|c| c.left).fold(f64::INFINITY, f64::min);
    let right = visible.iter().map(|c| c.right).fold(f64::NEG_INFINITY, f64::max);
    let baseline = visible.iter().map(|c| c.baseline).sum::<f64>() / visible.len() as f64;
    let ascent = visible.iter().map(|c| c.ascent).fold(0.0, f64::max);
    let descent = visible.iter().map(|c| c.descent).fold(0.0, f64::max);

    let mut runs: Vec<TextRun> = Vec::new();
    let mut trailing_space = false;
    for ch in chars {
        if ch.ch.is_whitespace() {
            if runs.is_empty() {
                continue;
            }
            trailing_space = true;
            continue;
        }
        let mut style = ch.style.clone();
        if trailing_space {
            if let Some(last) = runs.last_mut() {
                last.text.push(' ');
            }
            trailing_space = false;
        }
        match runs.last_mut() {
            Some(last)
                if last.font == style.font
                    && (last.size - style.size).abs() < 0.05
                    && last.bold == style.bold
                    && last.italic == style.italic
                    && last.color == style.color =>
            {
                last.text.push(ch.ch);
            }
            _ => {
                style.text.push(ch.ch);
                runs.push(style);
            }
        }
    }
    if runs.is_empty() {
        return None;
    }

    Some(TextLine {
        x: left,
        top: page_height - baseline - ascent,
        width: right - left,
        height: ascent + descent,
        ascent,
        runs,
    })
}

fn background(page: &mut PdfPage<'_>, scale: f32) -> Result<Option<RasterImage>, Error> {
    if scale <= 0.0 {
        return Ok(None);
    }
    let count = page.objects().len();
    let has_graphics = page
        .objects()
        .iter()
        .any(|object| object.object_type() != PdfPageObjectType::Text);
    if !has_graphics {
        return Ok(None);
    }

    page.set_content_regeneration_strategy(PdfPageContentRegenerationStrategy::Manual);
    for index in 0..count {
        if let Ok(mut object) = page.objects().get(index) {
            if let Some(text) = object.as_text_object_mut() {
                let _ = text.set_render_mode(PdfPageTextRenderMode::Invisible);
            }
        }
    }

    let bitmap = page
        .render_with_config(&PdfRenderConfig::new().scale_page_by_factor(scale))
        .map_err(err)?;
    let image = bitmap.as_image().map_err(err)?.to_rgba8();
    let (w, h) = image.dimensions();
    if w == 0 || h == 0 || uniform(&image) {
        return Ok(None);
    }

    let mut png = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|e| Error::new(format!("png encode: {e}")))?;

    Ok(Some(RasterImage {
        png: png.into_inner(),
        width_px: w,
        height_px: h,
    }))
}

fn uniform(image: &image::RgbaImage) -> bool {
    let raw = image.as_raw();
    if raw.len() < 4 {
        return true;
    }
    let first = &raw[..4];
    raw.chunks_exact(4).all(|px| px == first)
}

fn normalize_font(raw: &str) -> (String, bool, bool) {
    let name = match raw.split_once('+') {
        Some((prefix, rest)) if prefix.len() == 6 && prefix.chars().all(|c| c.is_ascii_uppercase()) => rest,
        _ => raw,
    };
    let (family, style) = name
        .split_once(|c| c == '-' || c == ',')
        .unwrap_or((name, ""));
    let style_lower = style.to_lowercase();
    let family_lower = family.to_lowercase();

    let bold = ["bold", "black", "heavy", "semibold", "demibold"]
        .iter()
        .any(|w| style_lower.contains(w) || family_lower.ends_with(w));
    let italic = ["italic", "oblique"]
        .iter()
        .any(|w| style_lower.contains(w) || family_lower.ends_with(w));

    let mut base = family.to_string();
    for word in ["BoldItalic", "BoldOblique", "Bold", "Italic", "Oblique", "Regular", "PSMT", "MT", "PS"] {
        if base.len() > word.len() + 2 && base.ends_with(word) {
            base.truncate(base.len() - word.len());
        }
    }

    let known = match base.to_lowercase().as_str() {
        "dejavusans" => Some("DejaVu Sans"),
        "dejavuserif" => Some("DejaVu Serif"),
        "dejavusansmono" => Some("DejaVu Sans Mono"),
        "timesnewroman" | "times" | "timesnewromanps" => Some("Times New Roman"),
        "arial" | "helvetica" | "arialmt" => Some("Arial"),
        "couriernew" | "courier" => Some("Courier New"),
        _ => None,
    };
    let family = match known {
        Some(name) => name.to_string(),
        None => split_camel(&base),
    };
    (family, bold, italic)
}

fn split_camel(name: &str) -> String {
    if name.contains(' ') {
        return name.to_string();
    }
    let mut out = String::with_capacity(name.len() + 4);
    let chars: Vec<char> = name.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if i > 0 && c.is_ascii_uppercase() && chars[i - 1].is_ascii_lowercase() {
            out.push(' ');
        }
        out.push(c);
    }
    out
}
