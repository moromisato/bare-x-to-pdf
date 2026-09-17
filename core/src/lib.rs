pub mod doc;
pub mod docx;
pub mod error;
pub mod model;
pub mod odt;
pub mod pdf;
pub mod pptx;
pub mod typst_backend;
pub mod xlsx;
pub mod xml;

use error::Error;
use std::ffi::CStr;
use std::os::raw::c_char;
use std::panic::{self, AssertUnwindSafe};
use std::path::Path;

pub struct Options<'a> {
    pub fonts_dir: &'a Path,
    pub pdfium_path: &'a Path,
    pub background_scale: f32,
}

pub fn convert(input: &[u8], from: &str, to: &str, options: &Options) -> Result<Vec<u8>, Error> {
    let from = match from {
        "doc" | "dot" if input.starts_with(b"PK") => "docx",
        "docx" | "docm" | "dotx" | "dotm" if input.starts_with(&[0xD0, 0xCF, 0x11, 0xE0]) => "doc",
        "odt" | "ott" if input.starts_with(b"<") || input.starts_with(&[0xEF, 0xBB, 0xBF]) => "fodt",
        other => other,
    };
    match (from, to) {
        ("docx" | "docm" | "dotx" | "dotm", "pdf") => {
            let document = docx::read(input)?;
            typst_backend::render_pdf(&document, options.fonts_dir)
        }
        ("doc" | "dot", "pdf") => {
            let document = doc::read(input)?;
            typst_backend::render_pdf(&document, options.fonts_dir)
        }
        ("odt" | "ott" | "fodt", "pdf") => {
            let document = odt::read(input)?;
            typst_backend::render_pdf(&document, options.fonts_dir)
        }
        ("xlsx" | "xlsm" | "xltx" | "xltm", "pdf") => {
            let document = xlsx::read(input)?;
            typst_backend::render_pdf(&document, options.fonts_dir)
        }
        ("pptx", "pdf") => {
            let document = pptx::read(input)?;
            typst_backend::render_pdf(&document, options.fonts_dir)
        }
        ("pdf", "docx") => {
            let document = pdf::read(input, options.pdfium_path, options.background_scale)?;
            docx::writer::write_fixed(&document)
        }
        _ => Err(Error::new(format!("conversion from {from} to {to} is not supported"))),
    }
}

#[repr(C)]
pub struct Buffer {
    pub data: *mut u8,
    pub len: usize,
}

impl Buffer {
    fn from_vec(bytes: Vec<u8>) -> Buffer {
        let boxed = bytes.into_boxed_slice();
        let len = boxed.len();
        let data = Box::into_raw(boxed) as *mut u8;
        Buffer { data, len }
    }

    fn empty() -> Buffer {
        Buffer {
            data: std::ptr::null_mut(),
            len: 0,
        }
    }
}

unsafe fn c_string(ptr: *const c_char, name: &str) -> Result<String, Error> {
    if ptr.is_null() {
        return Err(Error::new(format!("{name} must not be null")));
    }
    CStr::from_ptr(ptr)
        .to_str()
        .map(str::to_owned)
        .map_err(|_| Error::new(format!("{name} must be valid UTF-8")))
}

#[no_mangle]
pub unsafe extern "C" fn sc_convert(
    input: *const u8,
    input_len: usize,
    from: *const c_char,
    to: *const c_char,
    fonts_dir: *const c_char,
    pdfium_path: *const c_char,
    out: *mut Buffer,
    error: *mut Buffer,
) -> i32 {
    if out.is_null() || error.is_null() {
        return 3;
    }
    *out = Buffer::empty();
    *error = Buffer::empty();

    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        let input: &[u8] = if input.is_null() {
            &[]
        } else {
            std::slice::from_raw_parts(input, input_len)
        };
        let from = c_string(from, "from")?;
        let to = c_string(to, "to")?;
        let fonts_dir = c_string(fonts_dir, "fontsDir")?;
        let pdfium_path = c_string(pdfium_path, "pdfiumPath")?;
        let options = Options {
            fonts_dir: Path::new(&fonts_dir),
            pdfium_path: Path::new(&pdfium_path),
            background_scale: 2.0,
        };
        convert(input, &from, &to, &options)
    }));

    match result {
        Ok(Ok(bytes)) => {
            *out = Buffer::from_vec(bytes);
            0
        }
        Ok(Err(e)) => {
            *error = Buffer::from_vec(e.message().as_bytes().to_vec());
            1
        }
        Err(payload) => {
            let message = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic".to_string());
            *error = Buffer::from_vec(format!("internal error: {message}").into_bytes());
            2
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn sc_free(buffer: *mut Buffer) {
    if buffer.is_null() {
        return;
    }
    let b = &mut *buffer;
    if !b.data.is_null() {
        drop(Box::from_raw(std::slice::from_raw_parts_mut(b.data, b.len)));
    }
    b.data = std::ptr::null_mut();
    b.len = 0;
}
