//! Disposable previews shared by draft imports and saved books.
use crate::{documents, store, types::Source};
use anyhow::Result;
use base64::{Engine, engine::general_purpose::STANDARD};
use image::{DynamicImage, Rgb, RgbImage, codecs::jpeg::JpegEncoder};
use std::{
    fs,
    path::PathBuf,
    time::{Duration, SystemTime},
};

pub const MAX_BYTES: u64 = 64 * 1024 * 1024;
const MAX_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);

pub struct Cache {
    folder: PathBuf,
    limit: u64,
}

impl Cache {
    pub fn new(folder: PathBuf) -> Result<Self> {
        fs::create_dir_all(&folder)?;
        let cache = Self {
            folder,
            limit: MAX_BYTES,
        };
        cache.prune()?;
        Ok(cache)
    }

    // The application serializes this method with clear and pruning under one mutex.
    pub fn get(&mut self, source: &Source) -> Result<String> {
        let stamp = documents::source_stamp(source)?;
        let mut normalized = source.clone();
        normalized.path = fs::canonicalize(&source.path)?.to_string_lossy().into();
        let key = store::digest(&serde_json::to_vec(&("jpeg-v1", normalized, stamp))?);
        let file = self.folder.join(format!("{key}.jpg"));
        let bytes = if file.is_file() {
            let bytes = fs::read(&file)?;
            fs::File::options()
                .write(true)
                .open(&file)?
                .set_modified(SystemTime::now())?;
            bytes
        } else {
            let thumb = documents::load(source)?.thumbnail(320, 480).to_rgba8();
            let mut rgb = RgbImage::new(thumb.width(), thumb.height());
            for (x, y, pixel) in thumb.enumerate_pixels() {
                let alpha = pixel[3] as u16;
                rgb.put_pixel(
                    x,
                    y,
                    Rgb(std::array::from_fn(|i| {
                        ((pixel[i] as u16 * alpha + 255 * (255 - alpha) + 127) / 255) as u8
                    })),
                );
            }
            let mut bytes = Vec::new();
            JpegEncoder::new_with_quality(&mut bytes, 80)
                .encode_image(&DynamicImage::ImageRgb8(rgb))?;
            if bytes.len() as u64 <= self.limit {
                store::atomic_write(&file, &bytes)?;
            }
            self.prune()?;
            bytes
        };
        // Returning data keeps already-requested previews valid after eviction or Clear.
        Ok(format!("data:image/jpeg;base64,{}", STANDARD.encode(bytes)))
    }

    fn entries(&self) -> Result<Vec<(PathBuf, u64, SystemTime)>> {
        let mut entries = Vec::new();
        for entry in fs::read_dir(&self.folder)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.strip_suffix(".jpg").is_some_and(is_digest) {
                continue;
            }
            let meta = entry.metadata()?;
            entries.push((entry.path(), meta.len(), meta.modified()?));
        }
        Ok(entries)
    }

    fn prune(&self) -> Result<()> {
        let mut entries = self.entries()?;
        entries.sort_by_key(|(_, _, modified)| *modified);
        let mut total: u64 = entries.iter().map(|(_, bytes, _)| bytes).sum();
        for (path, bytes, modified) in entries {
            if total > self.limit || modified.elapsed().unwrap_or_default() > MAX_AGE {
                fs::remove_file(path)?;
                total -= bytes;
            }
        }
        Ok(())
    }

    pub fn clear(&mut self) -> Result<usize> {
        let entries = self.entries()?;
        for (path, _, _) in &entries {
            fs::remove_file(path)?;
        }
        Ok(entries.len())
    }
}

fn is_digest(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::library;
    use crate::library::Metadata;
    use std::path::Path;
    use tokio_util::sync::CancellationToken;

    fn source(path: &Path) -> Source {
        Source {
            path: path.to_string_lossy().into(),
            kind: "image".into(),
            entry: None,
            index: 0,
        }
    }
    fn picture(path: &Path, width: u32) {
        RgbImage::from_pixel(width, 600, Rgb([40, 80, 120]))
            .save(path)
            .unwrap();
    }
    fn meta() -> Metadata {
        Metadata {
            title: "試験".into(),
            ..Default::default()
        }
    }

    #[test]
    fn import_and_saved_page_reuse_one_entry_and_source_changes_invalidate_it() {
        let temp = tempfile::tempdir().unwrap();
        let original = temp.path().join("原稿.png");
        picture(&original, 400);
        let before = store::file_hash(&original).unwrap();
        let mut cache = Cache::new(temp.path().join("cache")).unwrap();
        let imported = cache.get(&source(&original)).unwrap();
        let preview = library::scan(
            &[original.to_string_lossy().into()],
            false,
            &CancellationToken::new(),
        )
        .unwrap();
        let book = library::create(&temp.path().join("books"), meta(), preview, None).unwrap();
        let page = store::page(Path::new(&book.path), &book.chapters[0].page_ids[0]).unwrap();
        assert_eq!(imported, cache.get(&page.source).unwrap());
        assert_eq!(cache.entries().unwrap().len(), 1);
        assert_eq!(store::file_hash(&original).unwrap(), before);
        assert!(
            fs::read_dir(store::assets(Path::new(&book.path)))
                .unwrap()
                .next()
                .is_none()
        );
        picture(&original, 500);
        assert_ne!(imported, cache.get(&source(&original)).unwrap());
        assert_eq!(cache.entries().unwrap().len(), 2);
        cache.clear().unwrap();
        assert!(cache.entries().unwrap().is_empty());
        // Images already sent to the webview survive deletion of their cache files.
        let bytes = STANDARD
            .decode(imported.split_once(',').unwrap().1)
            .unwrap();
        let image = image::load_from_memory(&bytes).unwrap();
        assert_eq!((image.width(), image.height()), (320, 480));
        assert!(cache.get(&source(&original)).is_ok());
    }

    #[test]
    fn disk_budget_evicts_least_recently_used_and_startup_expires_old_entries() {
        let temp = tempfile::tempdir().unwrap();
        let mut cache = Cache::new(temp.path().join("cache")).unwrap();
        cache.limit = 150;
        let old = cache.folder.join(format!("{}.jpg", "a".repeat(64)));
        let recent = cache.folder.join(format!("{}.jpg", "b".repeat(64)));
        fs::write(&old, vec![0; 100]).unwrap();
        fs::write(&recent, vec![0; 100]).unwrap();
        fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(120))
            .unwrap();
        cache.prune().unwrap();
        assert!(!old.exists());
        assert!(recent.exists());
        fs::File::options()
            .write(true)
            .open(&recent)
            .unwrap()
            .set_modified(SystemTime::now() - MAX_AGE - Duration::from_secs(1))
            .unwrap();
        let reopened = Cache::new(cache.folder.clone()).unwrap();
        assert!(reopened.entries().unwrap().is_empty());
        // A single preview larger than the configured budget is returned without persisting it.
        let original = temp.path().join("source.png");
        picture(&original, 400);
        cache.limit = 1;
        assert!(
            cache
                .get(&source(&original))
                .unwrap()
                .starts_with("data:image/jpeg;base64,")
        );
        assert!(cache.entries().unwrap().is_empty());
    }
}
