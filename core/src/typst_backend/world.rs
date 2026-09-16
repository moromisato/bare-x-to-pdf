use super::fonts::FontSet;
use std::collections::HashMap;
use typst::diag::{FileError, FileResult};
use typst::foundations::{Bytes, Datetime, Duration};
use crate::error::Error;
use typst::syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt, World};

pub struct ConverterWorld {
    library: LazyHash<Library>,
    fonts: FontSet,
    source: Source,
    files: HashMap<FileId, Bytes>,
}

impl ConverterWorld {
    pub fn new(source: String, fonts: FontSet) -> Self {
        ConverterWorld {
            library: LazyHash::new(Library::builder().build()),
            fonts,
            source: Source::detached(source),
            files: HashMap::new(),
        }
    }

    pub fn add_file(&mut self, path: &str, data: Vec<u8>) -> Result<(), Error> {
        let vpath = VirtualPath::new(path).map_err(|e| Error::new(format!("virtual path {path}: {e:?}")))?;
        let id = RootedPath::new(VirtualRoot::Project, vpath).intern();
        self.files.insert(id, Bytes::new(data));
        Ok(())
    }
}

impl World for ConverterWorld {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }

    fn book(&self) -> &LazyHash<FontBook> {
        &self.fonts.book
    }

    fn main(&self) -> FileId {
        self.source.id()
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.source.id() {
            Ok(self.source.clone())
        } else {
            Err(not_found(id))
        }
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        self.files.get(&id).cloned().ok_or_else(|| not_found(id))
    }

    fn font(&self, index: usize) -> Option<Font> {
        self.fonts.fonts.get(index).cloned()
    }

    fn today(&self, _offset: Option<Duration>) -> Option<Datetime> {
        None
    }
}

fn not_found(id: FileId) -> FileError {
    FileError::NotFound(std::path::PathBuf::from(id.get().vpath().get_without_slash()))
}
