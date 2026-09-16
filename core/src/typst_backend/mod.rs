pub mod emit;
pub mod fonts;
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
    let emitted = emit::emit(doc, &fonts);

    if let Ok(path) = std::env::var("SIMPLE_CONVERTER_DUMP_TYPST") {
        let _ = std::fs::write(path, &emitted.source);
    }

    let mut world = ConverterWorld::new(emitted.source, fonts);
    for (path, data) in emitted.files {
        world.add_file(&path, data)?;
    }
    let compiled = typst::compile::<PagedDocument>(&world);
    let paged = compiled
        .output
        .map_err(|errors| Error::new(format!("layout failed: {}", diagnostics(&errors))))?;

    let pdf = typst_pdf::pdf(&paged, &PdfOptions::default())
        .map_err(|errors| Error::new(format!("pdf export failed: {}", diagnostics(&errors))))?;

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
