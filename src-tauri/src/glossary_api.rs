use super::*;
use umanga_core::{
    glossary::{self, BookGlossary},
    library::{self, Book},
};

#[tauri::command]
pub async fn book_glossary_save(
    state: State<'_, AppState>,
    path: String,
    expected: u64,
    glossary: BookGlossary,
) -> Api<Book> {
    let _storage = state.storage_change.lock().await;
    require_managed(&state, &path)?;
    let root = library_api::root(&state);
    let saved_path = path.clone();
    let saved = tauri::async_runtime::spawn_blocking(move || {
        library::save_glossary(&root, Path::new(&path), expected, glossary)
    })
    .await
    .map_err(error)?
    .map_err(error)?;
    hosting::notify_clients(&state.app, &saved_path, None);
    state.host.publish_requirements();
    Ok(saved)
}

#[tauri::command]
pub async fn glossary_parse(text: String, format: String) -> Api<Vec<GlossaryEntry>> {
    tauri::async_runtime::spawn_blocking(move || glossary::parse(&text, &format))
        .await
        .map_err(error)?
        .map_err(error)
}

#[tauri::command]
pub async fn glossary_export(
    state: State<'_, AppState>,
    path: String,
    destination: String,
    entries: Vec<GlossaryEntry>,
    format: String,
) -> Api<()> {
    let _storage = state.storage_change.lock().await;
    require_managed(&state, &path)?;
    let app_folder = state.folder.clone();
    tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<()> {
        let mut protected = library::export_protection(Path::new(&path))?;
        protected
            .directories
            .push(umanga_core::safety::resolved(&app_folder)?);
        let destination = Path::new(&destination);
        protected.check(destination)?;
        let content = glossary::encode(&entries, &format)?;
        let parent = destination
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Choose a CSV or TSV file."))?;
        let temporary = parent.join(format!(".umanga-glossary-{}.tmp", uid()));
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let result = (|| -> anyhow::Result<()> {
            file.write_all(content.as_bytes())?;
            file.sync_all()?;
            drop(file);
            protected.check(destination)?;
            std::fs::rename(&temporary, destination)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result
    })
    .await
    .map_err(error)?
    .map_err(error)
}
