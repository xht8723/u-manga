//! Retire only generated outputs whose ownership and lack of references are provable.
use crate::{safety, store};
use anyhow::Result;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};
static ASSET_USE: parking_lot::RwLock<()> = parking_lot::RwLock::new(());
static ACTIVE_OUTPUTS: std::sync::LazyLock<parking_lot::Mutex<HashMap<PathBuf, usize>>> =
    std::sync::LazyLock::new(Default::default);
/// Only newly created, uncommitted managed outputs may be retired on failure.
pub struct ProvisionalOutput {
    book: PathBuf,
    path: PathBuf,
    created: bool,
    committed: bool,
}
impl ProvisionalOutput {
    pub fn image(book: &Path, path: &Path, image: &image::DynamicImage) -> Result<Self> {
        let _publication = ASSET_USE.write();
        safety::no_link(&store::assets(book))?;
        if path.exists() {
            safety::no_link(path)?;
        }
        let path = safety::resolved(path)?;
        anyhow::ensure!(
            path.parent() == Some(safety::resolved(&store::assets(book))?.as_path()),
            "Output is outside managed assets"
        );
        let created = !path.exists();
        if created {
            save_image(image, &path)?;
        }
        *ACTIVE_OUTPUTS.lock().entry(path.clone()).or_default() += 1;
        Ok(Self {
            book: book.to_owned(),
            path,
            created,
            committed: false,
        })
    }
    pub fn commit(&mut self) {
        self.committed = true;
    }
}
impl Drop for ProvisionalOutput {
    fn drop(&mut self) {
        {
            let mut active = ACTIVE_OUTPUTS.lock();
            if let Some(count) = active.get_mut(&self.path) {
                *count -= 1;
                if *count == 0 {
                    active.remove(&self.path);
                }
            }
        }
        if self.created
            && !self.committed
            && let Err(e) = collect_known(&self.book, None, &HashSet::new(), Some(&self.path))
        {
            eprintln!("Provisional output retirement deferred: {e}");
        }
    }
}
/// A short publication gate captures references; long inference/export uses only these paths.
pub struct AssetReferences {
    paths: Vec<PathBuf>,
}
impl Drop for AssetReferences {
    fn drop(&mut self) {
        let mut active = ACTIVE_OUTPUTS.lock();
        for path in &self.paths {
            if let Some(count) = active.get_mut(path) {
                *count -= 1;
                if *count == 0 {
                    active.remove(path);
                }
            }
        }
    }
}
pub fn page_references(page: &crate::types::Page) -> Vec<PathBuf> {
    [
        Some(&page.source.path),
        page.rendered.as_ref(),
        page.background.as_ref(),
        page.cleanup.as_ref().map(|c| &c.path),
    ]
    .into_iter()
    .flatten()
    .map(PathBuf::from)
    .collect()
}
pub fn protect_snapshot<T>(
    snapshot: impl FnOnce() -> Result<(T, Vec<PathBuf>)>,
) -> Result<(T, AssetReferences)> {
    let _publication = ASSET_USE.read();
    let (value, paths) = snapshot()?;
    let paths = paths
        .iter()
        .map(|p| safety::resolved(p))
        .collect::<Result<Vec<_>>>()?;
    let mut active = ACTIVE_OUTPUTS.lock();
    for path in &paths {
        *active.entry(path.clone()).or_default() += 1;
    }
    Ok((value, AssetReferences { paths }))
}

pub struct PageOutputs {
    book: PathBuf,
    page: String,
}
impl PageOutputs {
    pub fn new(book: &Path, page: &str) -> Result<Self> {
        safety::identity(page)?;
        Ok(Self {
            book: book.to_owned(),
            page: page.to_owned(),
        })
    }
}
impl Drop for PageOutputs {
    fn drop(&mut self) {
        if let Err(e) = collect(&self.book, Some(&self.page)) {
            eprintln!("Generated output retirement deferred: {e}");
        }
    }
}
pub fn collect(book: &Path, page: Option<&str>) -> Result<usize> {
    collect_protected(book, page, &HashSet::new())
}
/// Preserve pre-commit sources as well as current references while retiring removed pages.
pub fn collect_protected(
    book: &Path,
    page: Option<&str>,
    extra: &HashSet<PathBuf>,
) -> Result<usize> {
    collect_known(book, page, extra, None)
}
fn collect_known(
    book: &Path,
    page: Option<&str>,
    extra: &HashSet<PathBuf>,
    known: Option<&Path>,
) -> Result<usize> {
    let _retirement = ASSET_USE.write();
    let root = book
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Book has no parent"))?;
    let assets = store::assets(book);
    if !book.exists() || !assets.exists() {
        return Ok(0);
    }
    safety::no_link(&assets)?;
    let mut retained: HashSet<PathBuf> = if root.join("library.sqlite").is_file() {
        crate::library::linked_originals(root)?
    } else {
        HashSet::new()
    };
    retained.extend(extra.iter().cloned());
    retained.extend(ACTIVE_OUTPUTS.lock().keys().cloned());
    let connection = store::connection(book)?;
    let mut q = connection.prepare("SELECT data FROM pages")?;
    for row in q.query_map([], |r| r.get::<_, String>(0))? {
        let p: crate::types::Page = serde_json::from_str(&row?)?;
        for path in [
            Some(p.source.path),
            p.rendered,
            p.background,
            p.cleanup.map(|c| c.path),
        ]
        .into_iter()
        .flatten()
        {
            retained.insert(safety::resolved(Path::new(&path))?);
        }
    }
    if let Some(cover) = crate::library::open(book)?.cover {
        retained.insert(safety::resolved(Path::new(&cover))?);
    }
    let mut removed = 0;
    for item in std::fs::read_dir(&assets)? {
        let item = item?;
        if !item.file_type()?.is_file() {
            continue;
        }
        let name = item.file_name();
        let name = name.to_string_lossy();
        let stem = name.strip_suffix(".png").unwrap_or("");
        let tail = if let Some(id) = page {
            stem.strip_prefix(&format!("{id}-"))
                .map(|s| s.strip_prefix("cleanup-").unwrap_or(s))
        } else {
            stem.strip_prefix("cover-")
        };
        if known.is_none() && tail.is_none_or(|s| safety::identity(s).is_err()) {
            continue;
        }
        safety::no_link(&item.path())?;
        let canonical = item.path().canonicalize()?;
        if known.is_some_and(|p| canonical != p) {
            continue;
        }
        if !retained.contains(&canonical) {
            std::fs::remove_file(item.path())?;
            removed += 1;
        }
    }
    Ok(removed)
}
pub fn save_image(image: &image::DynamicImage, path: &Path) -> Result<()> {
    let temporary = path.with_extension(format!("{}.partial", crate::types::uid()));
    let result = (|| {
        image.save_with_format(&temporary, image::ImageFormat::Png)?;
        store::replace_file(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}
