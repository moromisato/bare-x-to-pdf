use crate::error::Error;
use crate::model::Generic;
use std::collections::BTreeSet;
use std::path::Path;
use typst::foundations::Bytes;
use typst::text::{Font, FontBook, FontStretch, FontStyle, FontVariant, FontWeight};
use typst::utils::LazyHash;

const SANS: &str = "Liberation Sans";
const SERIF: &str = "Liberation Serif";
const MONO: &str = "Liberation Mono";

pub struct FontSet {
    pub fonts: Vec<Font>,
    pub book: LazyHash<FontBook>,
    families: BTreeSet<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct FontMetricsEm {
    pub ascender: f64,
    pub descender: f64,
    pub line_gap: f64,
}

impl FontMetricsEm {
    pub fn line_height(&self) -> f64 {
        self.ascender + self.descender + self.line_gap
    }
}

impl FontSet {
    pub fn load(dir: &Path) -> Result<FontSet, Error> {
        let mut paths: Vec<_> = std::fs::read_dir(dir)
            .map_err(|e| Error::new(format!("fonts directory {}: {e}", dir.display())))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                matches!(
                    path.extension().and_then(|e| e.to_str()).map(str::to_lowercase).as_deref(),
                    Some("ttf") | Some("otf") | Some("ttc") | Some("otc")
                )
            })
            .collect();
        paths.sort();

        let mut fonts = Vec::new();
        for path in paths {
            let data = std::fs::read(&path)?;
            fonts.extend(Font::iter(Bytes::new(data)));
        }
        if fonts.is_empty() {
            return Err(Error::new(format!("no fonts found in {}", dir.display())));
        }

        let book = FontBook::from_fonts(&fonts);
        let families = fonts
            .iter()
            .map(|f| f.info().family.to_lowercase())
            .collect();

        Ok(FontSet {
            fonts,
            book: LazyHash::new(book),
            families,
        })
    }

    pub fn has_family(&self, name: &str) -> bool {
        self.families.contains(&name.to_lowercase())
    }

    pub fn resolve(&self, requested: &str, generic: Option<Generic>) -> String {
        let lower = requested.trim().to_lowercase();
        if let Some(font) = self.fonts.iter().find(|f| f.info().family.to_lowercase() == lower) {
            return font.info().family.to_string();
        }

        let alias = match lower.as_str() {
            "calibri" | "calibri light" | "carlito" => "Carlito",
            "cambria" | "cambria math" | "caladea" => "Caladea",
            "arial" | "arial narrow" | "helvetica" | "helvetica neue" | "arimo" | "albany" => SANS,
            "times new roman" | "times" | "tinos" | "thorndale" | "nimbus roman" => SERIF,
            "courier new" | "courier" | "cousine" | "cumberland" | "consolas" | "lucida console"
            | "menlo" | "monaco" | "source code pro" | "fira code" | "cascadia code" => MONO,
            "georgia" | "garamond" | "book antiqua" | "palatino" | "palatino linotype"
            | "century schoolbook" | "bookman old style" | "constantia" | "minion pro"
            | "baskerville" | "perpetua" => SERIF,
            "verdana" | "tahoma" | "segoe ui" | "trebuchet ms" | "century gothic" | "franklin gothic"
            | "franklin gothic book" | "gill sans" | "gill sans mt" | "lucida sans" | "candara"
            | "corbel" | "open sans" | "roboto" | "avenir" | "futura" | "aptos" | "aptos display"
            | "lato" | "montserrat" | "inter" | "dejavu sans" | "noto sans" => SANS,
            _ => match generic {
                Some(Generic::Serif) => SERIF,
                Some(Generic::Mono) => MONO,
                Some(Generic::Sans) => SANS,
                None => {
                    if lower.contains("mono") || lower.contains("courier") || lower.contains("code") {
                        MONO
                    } else if lower.contains("serif") && !lower.contains("sans") {
                        SERIF
                    } else if lower.contains("roman") || lower.contains("times") || lower.contains("book") {
                        SERIF
                    } else {
                        SANS
                    }
                }
            },
        };

        if self.has_family(alias) {
            alias.to_string()
        } else {
            self.fonts[0].info().family.to_string()
        }
    }

    pub fn metrics(&self, family: &str, bold: bool, italic: bool) -> FontMetricsEm {
        let fallback = FontMetricsEm {
            ascender: 0.9,
            descender: 0.22,
            line_gap: 0.03,
        };
        let variant = FontVariant {
            style: if italic { FontStyle::Italic } else { FontStyle::Normal },
            weight: if bold { FontWeight::BOLD } else { FontWeight::REGULAR },
            stretch: FontStretch::NORMAL,
        };
        let key = family.to_lowercase();
        let index = self
            .book
            .select(&key, variant)
            .or_else(|| self.book.select_family(&key).next());

        let Some(font) = index.and_then(|i| self.fonts.get(i)) else {
            return fallback;
        };
        let Ok(face) = ttf_parser::Face::parse(font.data().as_slice(), font.index()) else {
            return fallback;
        };
        let upm = face.units_per_em() as f64;
        FontMetricsEm {
            ascender: face.ascender() as f64 / upm,
            descender: face.descender().abs() as f64 / upm,
            line_gap: face.line_gap().max(0) as f64 / upm,
        }
    }
}
