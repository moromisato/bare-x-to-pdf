use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(root().join("test/fixtures").join(name)).unwrap()
}

fn out_dir() -> PathBuf {
    let dir = std::env::var("SIMPLE_CONVERTER_TEST_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("simple-converter-tests"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn options() -> simple_converter_core::Options<'static> {
    use std::sync::OnceLock;
    static FONTS: OnceLock<PathBuf> = OnceLock::new();
    static PDFIUM: OnceLock<PathBuf> = OnceLock::new();
    simple_converter_core::Options {
        fonts_dir: FONTS.get_or_init(|| root().join("fonts")),
        pdfium_path: PDFIUM.get_or_init(|| {
            root().join("prebuilds/darwin-arm64/simple-converter/libpdfium.dylib")
        }),
        background_scale: 2.0,
    }
}

fn convert_docx(name: &str) -> Vec<u8> {
    let stem = name.trim_end_matches(".docx");
    std::env::set_var(
        "SIMPLE_CONVERTER_DUMP_TYPST",
        out_dir().join(format!("{stem}.typ")),
    );
    let pdf = simple_converter_core::convert(&fixture(name), "docx", "pdf", &options())
        .unwrap_or_else(|e| panic!("{name}: {e}"));
    std::fs::write(out_dir().join(format!("{stem}.pdf")), &pdf).unwrap();
    pdf
}

#[test]
fn minimal_docx_to_pdf() {
    let pdf = convert_docx("minimal-table-unicode.docx");
    assert!(pdf.starts_with(b"%PDF-"));
}

#[test]
fn styled_docx_to_pdf() {
    let pdf = convert_docx("sdk-sample.docx");
    assert!(pdf.starts_with(b"%PDF-"));
}

#[test]
fn rejects_unknown_pair() {
    let err = simple_converter_core::convert(b"", "pdf", "pdf", &options()).unwrap_err();
    assert!(err.message().contains("not supported"));
}

#[test]
fn rejects_garbage_docx() {
    let err = simple_converter_core::convert(b"PK\x03\x04garbage", "docx", "pdf", &options()).unwrap_err();
    assert!(!err.message().is_empty());
}

#[test]
fn pdf_to_docx_round_trip() {
    let pdf = convert_docx("sdk-sample.docx");
    let docx = simple_converter_core::convert(&pdf, "pdf", "docx", &options())
        .unwrap_or_else(|e| panic!("pdf to docx: {e}"));
    assert!(docx.starts_with(b"PK"));
    std::fs::write(out_dir().join("sdk-sample-roundtrip.docx"), &docx).unwrap();
    let again = simple_converter_core::convert(&docx, "docx", "pdf", &options());
    assert!(again.is_ok(), "fixed-layout docx should be readable: {:?}", again.err());
}
