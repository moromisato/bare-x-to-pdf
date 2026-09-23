pub mod emit;
pub mod fonts;
mod notes;
pub mod world;

use crate::error::Error;
use crate::model::Document;
use fonts::FontSet;
use std::path::Path;
use typst::diag::SourceDiagnostic;
use typst_layout::PagedDocument;
use typst_pdf::PdfOptions;
use world::ConverterWorld;

pub fn render_pdf(doc: &Document, fonts_dir: &Path) -> Result<Vec<u8>, Error> {
    let fonts = FontSet::load(fonts_dir)?;
    let mut emitted = emit::emit(doc, &fonts);

    if let Ok(path) = std::env::var("SIMPLE_CONVERTER_DUMP_TYPST") {
        let _ = std::fs::write(path, &emitted.source);
    }
    if let Ok(path) = std::env::var("SIMPLE_CONVERTER_LOAD_TYPST") {
        emitted.source = std::fs::read_to_string(path).map_err(|e| Error::new(format!("cannot read typst source: {e}")))?;
    }

    let mut world = ConverterWorld::new(emitted.source, fonts);
    for (path, data) in emitted.files {
        world.add_file(&path, data)?;
    }
    let compiled = typst::compile::<PagedDocument>(&world);
    let paged = compiled
        .output
        .map_err(|errors| Error::new(format!("layout failed: {}", diagnostics(&errors))))?;

    let mut pdf = typst_pdf::pdf(&paged, &PdfOptions::default())
        .map_err(|errors| Error::new(format!("pdf export failed: {}", diagnostics(&errors))))?;

    let has_notes = doc.sections.iter().any(|s| !s.notes.is_empty());
    if has_notes && paged.pages().len() == doc.sections.len() {
        let pages: Vec<(f64, &[crate::model::PageNote])> =
            doc.sections.iter().map(|s| (s.page.height, s.notes.as_slice())).collect();
        pdf = notes::append_notes(pdf, &pages)?;
    }

    typst::comemo::evict(0);
    Ok(pdf)
}

fn diagnostics(errors: &[SourceDiagnostic]) -> String {
    errors
        .iter()
        .map(|e| e.message.to_string())
        .collect::<Vec<_>>()
        .join("; ")
}
