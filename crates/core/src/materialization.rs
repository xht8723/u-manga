//! Disposable original-image cache: atomic publication, bounded verification, and single-flight.
use crate::{documents, image_input, safety, store, types::Page};
use anyhow::{Context, Result, ensure};
use parking_lot::Mutex;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
};
#[derive(Default)]
struct Entry {
    verified: Mutex<Option<String>>,
    operation: Mutex<()>,
}
type Registry = (u64, HashMap<PathBuf, (u64, Arc<Entry>)>);
static ENTRIES: LazyLock<Mutex<Registry>> = LazyLock::new(|| Mutex::new((0, HashMap::new())));
fn entry(path: &Path) -> Result<Arc<Entry>> {
    let mut entries = ENTRIES.lock();
    entries.0 += 1;
    let sequence = entries.0;
    if let Some((age, state)) = entries.1.get_mut(path) {
        *age = sequence;
        return Ok(state.clone());
    }
    if entries.1.len() >= 512 {
        let oldest = entries
            .1
            .iter()
            .filter(|(_, (_, entry))| Arc::strong_count(entry) == 1)
            .min_by_key(|(_, (age, _))| age)
            .map(|(path, _)| path.clone());
        if let Some(oldest) = oldest {
            entries.1.remove(&oldest);
        }
    }
    ensure!(
        entries.1.len() < 512,
        "Too many active page-image requests; retry shortly"
    );
    let state = Arc::new(Entry::default());
    entries.1.insert(path.to_owned(), (sequence, state.clone()));
    Ok(state)
}
fn file_stamp(path: &Path, recipe: &str) -> Result<String> {
    let metadata = std::fs::metadata(path)?;
    Ok(format!(
        "{recipe}:{}:{:?}",
        metadata.len(),
        metadata.modified()?
    ))
}
pub fn get(book: &Path, page: &Page) -> Result<PathBuf> {
    safety::page(page)?;
    documents::source_health(page)?;
    std::fs::create_dir_all(store::assets(book))?;
    let path = safety::page_asset(book, &page.id, "original.png")?;
    let stamp_path = safety::page_asset(book, &page.id, "stamp")?;
    let identity = store::digest(&serde_json::to_vec(&(
        "materialization-v2",
        &page.source,
        &page.source_stamp,
        page.width,
        page.height,
    ))?);
    let state = entry(&safety::resolved(&path)?)?;
    let _operation = state.operation.lock();
    let valid = std::fs::read_to_string(&stamp_path).ok().as_deref() == Some(&identity);
    let fingerprint = file_stamp(&path, &identity).ok();
    if valid && fingerprint.is_some() {
        if *state.verified.lock() == fingerprint {
            return Ok(path);
        }
        if let Ok(image) = image_input::open(&path)
            && documents::matches_dimensions(page, (image.width(), image.height()))
        {
            *state.verified.lock() = fingerprint;
            return Ok(path);
        }
    }
    let image = documents::load(&page.source)?;
    ensure!(
        documents::matches_dimensions(page, (image.width(), image.height())),
        "Original dimensions changed"
    );
    documents::source_health(page)?;
    crate::assets::save_image(&image, &path)?;
    store::atomic_write(&stamp_path, identity.as_bytes())
        .context("Page image saved, but its disposable cache stamp could not be saved")?;
    *state.verified.lock() = Some(file_stamp(&path, &identity)?);
    Ok(path)
}
