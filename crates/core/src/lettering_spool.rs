//! Lossless temporary storage keeps only one region pixmap resident at a time.
use anyhow::{Context, Result};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
pub(crate) struct Spool {
    file: Option<File>,
    path: PathBuf,
    entries: Vec<(usize, u64, u32, u32)>,
}
impl Spool {
    pub fn new(book: &Path, page: &str) -> Result<Self> {
        let path = crate::safety::page_asset(
            book,
            page,
            &format!("lettering-{}.tmp", crate::types::uid()),
        )?;
        std::fs::create_dir_all(path.parent().context("Missing asset directory")?)?;
        let file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path)?;
        Ok(Self {
            file: Some(file),
            path,
            entries: vec![],
        })
    }
    pub fn push(&mut self, index: usize, pixmap: tiny_skia::Pixmap) -> Result<()> {
        let file = self.file.as_mut().unwrap();
        let offset = file.stream_position()?;
        file.write_all(pixmap.data())?;
        self.entries
            .push((index, offset, pixmap.width(), pixmap.height()));
        Ok(())
    }
    pub fn replay(
        &mut self,
        mut visit: impl FnMut(usize, &tiny_skia::Pixmap) -> Result<()>,
    ) -> Result<()> {
        let file = self.file.as_mut().unwrap();
        for &(index, offset, width, height) in &self.entries {
            file.seek(SeekFrom::Start(offset))?;
            let mut pixmap =
                tiny_skia::Pixmap::new(width, height).context("Invalid lettering size")?;
            file.read_exact(pixmap.data_mut())?;
            visit(index, &pixmap)?;
        }
        Ok(())
    }
}
impl Drop for Spool {
    fn drop(&mut self) {
        drop(self.file.take());
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pixels_and_layer_order_survive_spooling_and_temporary_files_are_removed() {
        let temp = tempfile::tempdir().unwrap();
        let book = temp.path().join("book.umanga");
        let page = crate::types::uid();
        let mut spool = Spool::new(&book, &page).unwrap();
        let path = spool.path.clone();
        let mut original = tiny_skia::Pixmap::new(7, 9).unwrap();
        original.fill(tiny_skia::Color::from_rgba8(20, 90, 230, 110));
        spool.push(3, original.clone()).unwrap();
        spool.push(1, original.clone()).unwrap();
        let mut order = vec![];
        spool
            .replay(|index, pixels| {
                order.push(index);
                assert_eq!(pixels.data(), original.data());
                Ok(())
            })
            .unwrap();
        assert_eq!(order, [3, 1]);
        drop(spool);
        assert!(!path.exists());
    }
}
