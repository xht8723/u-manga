//! Local native integration fixture; never invokes OCR or a provider.
use anyhow::Result;
use std::{fs, path::PathBuf, time::Instant};
use umanga_core::{library, store, types::AppSettings};
fn main() -> Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let source = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("Pass an existing .umanga book"))?;
    let output = root.join("test-output/redesign-state");
    let lib = output.join("library");
    fs::create_dir_all(&lib)?;
    let before = store::file_hash(&source)?;
    let start = Instant::now();
    let book = library::import_book(&lib, &source)?;
    let summary = library::list(&lib)?;
    assert_eq!(store::file_hash(&source)?, before);
    let mut settings = AppSettings {
        library_directory: lib.to_string_lossy().into(),
        library_sort: "title".into(),
        ..Default::default()
    };
    settings.appearance.active = "night".into();
    store::atomic_write(
        &output.join("preferences.json"),
        &serde_json::to_vec_pretty(&settings)?,
    )?;
    store::atomic_write(
        &output.join("smoke.json"),
        &serde_json::to_vec_pretty(
            &serde_json::json!({"bookId":book.id,"chapters":book.chapters.len(),"pages":summary[0].pages,"translated":summary[0].translated,"milliseconds":start.elapsed().as_millis(),"sourceUnchanged":true}),
        )?,
    )?;
    println!("{}", output.display());
    Ok(())
}
